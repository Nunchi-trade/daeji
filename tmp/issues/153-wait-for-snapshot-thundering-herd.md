# 153: wait_for_snapshot Uses Global Notify -- Thundering Herd on Every Snapshot Insertion

**Category**: performance
**Severity**: medium
**Component**: consensus

## Summary

The `wait_for_snapshot()` method in `LedgerView` uses a single `tokio::sync::Notify` to wake up all waiters whenever any snapshot is inserted into the store. When N waiters are each waiting for a different parent snapshot, every snapshot insertion causes all N waiters to wake up, re-acquire the ledger mutex, check if their specific snapshot arrived, and go back to sleep. This O(N) "thundering herd" effect creates unnecessary mutex contention on the hot path and can contribute to snapshot wait timeouts, especially under high concurrency.

## Problem

The `LedgerView` struct (in `crates/node/ledger/src/lib.rs`) uses a single `tokio::sync::Notify` called `snapshot_notify`:

```rust
snapshot_notify: Arc<::tokio::sync::Notify>,
```

When any snapshot is inserted (via `insert_snapshot`, `cache_snapshot`, or `restore_persisted_snapshot`), the code drops the ledger mutex and calls `self.snapshot_notify.notify_waiters()`, which wakes up ALL waiters:

```rust
// crates/node/ledger/src/lib.rs:358-361
inner.snapshots.insert(digest, Snapshot::new(...));
inner.head = digest;
drop(inner);                              // Release ledger mutex
self.snapshot_notify.notify_waiters();    // Wake ALL waiters
```

The `wait_for_snapshot()` method (lines 397-420) loops: it registers a `notified()` future, checks the snapshot store, and if the desired snapshot is not present, waits for the notification:

```rust
// crates/node/ledger/src/lib.rs:397-420
pub async fn wait_for_snapshot(
    &self,
    parent: ConsensusDigest,
    timeout: Duration,
) -> Option<LedgerSnapshot> {
    let deadline = ::tokio::time::Instant::now() + timeout;
    loop {
        let notified = self.snapshot_notify.notified();
        if let Some(snap) = self.parent_snapshot(parent).await {
            //                  ^^^^^^^^^^^^^^^^^^^^^^^^
            //                  Acquires self.inner.lock().await on each iteration
            return Some(snap);
        }
        let remaining = deadline.saturating_duration_since(::tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let _ = ::tokio::time::timeout(remaining, notified).await;
    }
    None
}
```

Each spurious wake-up (from a snapshot insertion for a different digest) causes a full iteration: re-register `notified()`, acquire the ledger mutex, read from the snapshot store, release the mutex. With N waiters and M snapshot insertions, this creates N * M mutex acquisitions.

**File**: `crates/node/ledger/src/lib.rs` (lines 358-361, 397-420)

## Code Reference

```rust
// crates/node/ledger/src/lib.rs:345-361
pub async fn insert_snapshot(
    &self,
    digest: ConsensusDigest,
    parent: ConsensusDigest,
    state: OverlayState<QmdbState>,
    root: StateRoot,
    qmdb_changes: QmdbChangeSet,
    txs: &[Tx],
) {
    let mut inner = self.inner.lock().await;
    let ids = tx_ids(txs);
    inner.snapshots.insert(digest, Snapshot::new(Some(parent), state, root, qmdb_changes, ids));
    inner.head = digest;
    drop(inner);
    self.snapshot_notify.notify_waiters();  // <-- Wakes ALL waiters
}
```

```rust
// crates/node/ledger/src/lib.rs:397-420
pub async fn wait_for_snapshot(
    &self,
    parent: ConsensusDigest,
    timeout: Duration,
) -> Option<LedgerSnapshot> {
    let deadline = ::tokio::time::Instant::now() + timeout;
    loop {
        let notified = self.snapshot_notify.notified();
        if let Some(snap) = self.parent_snapshot(parent).await {
            return Some(snap);
        }
        let remaining = deadline.saturating_duration_since(::tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let _ = ::tokio::time::timeout(remaining, notified).await;
        // <-- On wake, loops back and re-acquires mutex even if the inserted
        //     snapshot was for a different digest
    }
    None
}
```

## Impact

1. **Unnecessary mutex contention**: On a 10-node network at 33 blocks/second, each snapshot insertion wakes all waiters. If there are 3 concurrent waiters (verify + propose + finalize paths), each insertion causes 3 extra mutex acquisitions.
2. **Wasted CPU**: Each spurious wake-up involves a `Notify::notified()` future allocation, a mutex lock/unlock, a BTreeMap lookup, and a timeout recalculation.
3. **Contributes to snapshot wait timeouts**: Under high concurrency, the thundering herd effect adds latency to each `wait_for_snapshot()` iteration. Combined with the `SNAPSHOT_WAIT_TIMEOUT` of 100ms (see issue #029), this can push waiters past their deadline.

The impact is currently bounded because the number of concurrent waiters is small (typically 1-3), but it becomes more significant if the architecture is modified to support parallel block verification or if the timeout is tightened.

## Root Cause

A single global `tokio::sync::Notify` is used for all snapshot insertions, with no mechanism to filter wake-ups by digest. All waiters are woken regardless of which specific snapshot was inserted.

## Suggested Fix

**Option A**: Use a `tokio::sync::broadcast` channel that includes the inserted digest, so waiters can filter without acquiring the mutex:

```rust
// In LedgerView:
snapshot_broadcast: tokio::sync::broadcast::Sender<ConsensusDigest>,

// In insert_snapshot:
let _ = self.snapshot_broadcast.send(digest);

// In wait_for_snapshot:
loop {
    let mut rx = self.snapshot_broadcast.subscribe();
    if let Some(snap) = self.parent_snapshot(parent).await {
        return Some(snap);
    }
    loop {
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(inserted_digest)) if inserted_digest == parent => break,
            Ok(Ok(_)) => continue,  // Different digest, keep waiting
            _ => return None,       // Timeout or channel closed
        }
    }
}
```

**Option B**: Use a per-digest `tokio::sync::watch` channel (more memory but zero spurious wakes):

```rust
// Maintain a map of digest -> Notify:
digest_notifiers: Arc<RwLock<HashMap<ConsensusDigest, Arc<Notify>>>>,
```

**Option C**: Accept the current design as adequate for the small number of concurrent waiters (1-3), and document the thundering herd behavior as a known limitation.

## Files to Modify

- `crates/node/ledger/src/lib.rs` -- replace global `Notify` with per-digest or filtered notification mechanism

## Related Issues

- [029-snapshot-wait-timeout-nullifications.md](./029-snapshot-wait-timeout-nullifications.md) -- downstream effect of contention: snapshot wait timeouts causing nullifications
- [015-ledger-single-mutex-bottleneck.md](./015-ledger-single-mutex-bottleneck.md) -- the single ledger mutex that is contended by spurious wakeups
- [150-finalize-lock-starves-proposal.md](./150-finalize-lock-starves-proposal.md) -- another source of mutex contention on the proposal path

## Labels

`performance`, `consensus`
