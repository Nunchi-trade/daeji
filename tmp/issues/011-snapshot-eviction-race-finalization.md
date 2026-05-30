# Snapshot Eviction Race with Finalization Pipeline

**Category**: Bug
**Severity**: High
**Labels**: `bug`, `reliability`, `correctness`, `consensus`, `storage`

## Summary

There is a race condition between snapshot eviction and the finalization pipeline in the Kora consensus client. When `persist_snapshot()` evicts old snapshots from the in-memory store, it can remove snapshot data that `finalize_block()` still needs to read. If finalization cannot find the parent snapshot, it falls back to creating an empty-changeset snapshot, which causes that block's state changes to be silently lost from QMDB, leading to state divergence from the network.

## Problem

The Kora node maintains an in-memory `InMemorySnapshotStore` that holds recent block snapshots (state overlays and changesets). When the store grows beyond 256 persisted entries, the oldest snapshots are evicted to free memory. This eviction runs inside `persist_snapshot()`, which is called from the finalization pipeline. A separate part of the finalization pipeline, `finalize_block()`, also needs to read parent snapshots for re-execution.

The race occurs in this sequence:

1. `persist_snapshot()` at `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:464-514` acquires the ledger mutex, collects the changeset chain, releases the mutex, commits to QMDB (disk I/O), re-acquires the mutex, marks the chain as persisted, and then calls `evict_persisted()` to remove old snapshots.

2. `finalize_block()` at `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:500-526` looks up the parent snapshot via `state.parent_snapshot(parent_digest)`. If eviction removed this parent between persistence and finalization of a later block, the parent is not found.

3. When the parent is missing and the block's own snapshot is also missing, the fallback code at `reporters/src/lib.rs:571-596` calls `state.restore_persisted_snapshot(block)`, which creates a snapshot with an **empty changeset** over the current QMDB state. This means the block's actual state changes are never computed or applied.

## Code Reference

**Eviction in persist_snapshot** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:500-507`):

```rust
inner.snapshots.mark_persisted(&chain);
// Evict oldest persisted snapshots to bound memory usage.
// Must happen inside the ledger mutex to prevent a TOCTOU
// race where another thread reads a snapshot between
// mark_persisted() and eviction.
inner.snapshots.evict_persisted();
```

**Eviction implementation** (`/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs:138-171`):

```rust
pub fn evict_persisted(&self) -> usize {
    if self.persisted_order.read().len() <= self.max_persisted_retained {
        return 0;
    }
    let mut snapshots = self.snapshots.write();
    let persisted = self.persisted.read();
    let mut order = self.persisted_order.write();
    let mut evicted = 0usize;
    while order.len() > self.max_persisted_retained {
        let Some(oldest) = order.pop_front() else {
            break;
        };
        if persisted.contains(&oldest) && snapshots.remove(&oldest).is_some() {
            evicted += 1;
        }
    }
    // ...
}
```

**Fallback when parent missing** (`/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:571-596`):

```rust
} else {
    let is_evicted = state.is_snapshot_persisted(&parent_digest).await;
    warn!(
        ?digest,
        ?parent_digest,
        parent_evicted = is_evicted,
        height = block.height,
        "finalize_block: parent snapshot unavailable; restoring block as \
         trusted persisted snapshot to unblock finalization pipeline"
    );
    state.restore_persisted_snapshot(block).await;
}
```

**restore_persisted_snapshot creates empty changeset** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:373-389`):

```rust
pub async fn restore_persisted_snapshot(&self, block: &Block) {
    let mut inner = self.inner.lock().await;
    let digest = block.commitment();
    let state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default()); // EMPTY changeset
    let snapshot = Snapshot::new(
        Some(block.parent()),
        state,
        block.state_root,
        QmdbChangeSet::default(), // EMPTY - actual block changes are lost
        tx_ids(&block.txs),
    );
    inner.snapshots.insert(digest, snapshot);
    inner.snapshots.mark_persisted(&[digest]);
    // ...
}
```

## Impact

When the race triggers:

1. The finalized block's state changes are never applied to QMDB because the restored snapshot has an empty changeset.
2. The local node's QMDB state silently diverges from the network consensus state.
3. RPC queries (`eth_getBalance`, `eth_getStorageAt`, etc.) return stale data for accounts modified in the missed block.
4. When the node next becomes the consensus leader, it proposes blocks built on an incorrect state root, which other validators will reject as invalid.

The race is more likely under:
- High block throughput (33+ blocks/s on the devnet) with the 256-entry eviction limit
- CPU contention (Docker containers with 1.2 cores) where finalization falls behind production
- Long QMDB persistence times due to disk I/O contention on shared storage

## Root Cause

The snapshot eviction policy (oldest-first, bounded at 256 entries via `DEFAULT_MAX_PERSISTED_RETAINED`) does not coordinate with the finalization pipeline. Eviction can remove snapshots that are about to be read by a concurrent `finalize_block()` call. The `mark_persisting_chain()` mechanism exists to prevent double-persistence, but there is no analogous "pin" mechanism for snapshots that finalization needs to read.

## Suggested Fix

**Option 1 (recommended): Pin mechanism.** Before `finalize_block()` reads a parent snapshot, pin it to prevent eviction. Unpin after finalization completes. The snapshot store already has `mark_persisting_chain()` / `clear_persisting_chain()` for the persistence path -- extend this pattern for finalization reads.

```rust
// Before looking up parent:
state.pin_snapshot(parent_digest).await;

// ... do finalization work ...

// After finalization:
state.unpin_snapshot(parent_digest).await;

// In evict_persisted(), skip pinned digests:
if pinned.contains(&oldest) {
    continue; // do not evict
}
```

**Option 2: Increase eviction window.** Make the eviction limit large enough to cover the maximum finalization pipeline depth. If finalization can lag by at most N blocks behind the tip, set `DEFAULT_MAX_PERSISTED_RETAINED` to at least `N * 2`.

**Option 3: Return early instead of restoring.** When the parent snapshot is missing due to eviction (detected via `is_snapshot_persisted`), defer finalization of that block rather than creating a lossy snapshot. Retry when the parent becomes available via re-execution.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (lines 464-514) -- `persist_snapshot()` evicts old snapshots after QMDB commit
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` (lines 500-526) -- `finalize_block()` reads parent snapshot with retry loop
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` (lines 571-596) -- fallback to empty-changeset restored snapshot
- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs` (lines 138-171) -- `evict_persisted()` removes old snapshots without coordination

## Related Issues

- `020-qmdb-persistence-blocks-finalization.md` -- QMDB persistence blocking the finalization pipeline increases the window for this race
- `015-ledger-single-mutex-bottleneck.md` -- the single ledger mutex serializes persistence and finalization, contributing to the timing conditions
- `146-restore-persisted-snapshot-stale-state.md` -- the restored snapshot uses stale QMDB state
- `205-snapshot-persisted-set-unbounded.md` -- the persisted marker set itself grows without bound
