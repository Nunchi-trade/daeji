# 150: finalize_lock Serializes Entire Finalization Pipeline, Starving the Proposal Path

**Category**: bug
**Severity**: medium
**Component**: consensus

## Summary

The `FinalizedReporter` holds a `tokio::sync::Mutex<()>` called `finalize_lock` that serializes all finalized block processing. This lock is held throughout the entire finalization pipeline -- including execution replay, state root computation, QMDB persistence, mempool pruning, and block indexing. While the lock is held, the proposal path (which needs the same ledger mutex for `wait_for_snapshot`, `proposal_components`, etc.) is blocked. Since the `SNAPSHOT_WAIT_TIMEOUT` is only 100ms, slow finalization under heavy I/O load can cause the proposer to time out and nullify its round.

## Problem

In `crates/node/reporters/src/lib.rs`, the `FinalizedReporter::report()` method spawns a background task that acquires `finalize_lock` and then calls `handle_finalized_update()`:

```rust
// crates/node/reporters/src/lib.rs:1442-1459
self.context.child("report_task").spawn(move |_| async move {
    let _guard = finalize_lock.lock().await;
    handle_finalized_update(
        state, context, executor, provider, block_index,
        mempool_broadcast, gc_log, metrics, checkpoint_interval,
        pending_acks, node_state, update,
    ).await;
});
```

Inside `handle_finalized_update()`, the function performs multiple expensive operations, each of which re-acquires the ledger's inner `futures::lock::Mutex` via calls to `state.*` methods:

1. `finalize_with_retry()` -- re-executes the block, computes state root, persists to QMDB
2. `index_finalized_block()` -- converts execution results to indexer format, inserts into BlockIndex
3. `acknowledge_checkpoint()` -- processes deferred marshal acknowledgments
4. `state.prune_mempool()` -- acquires ledger mutex, removes transactions
5. `state.prune_stale_nonces()` -- acquires ledger mutex, queries QMDB nonces for each sender

The `finalize_lock` prevents any interleaving between these operations, meaning the proposal path cannot acquire the ledger mutex between steps. The total time holding `finalize_lock` can exceed 100ms under heavy I/O (especially during QMDB persistence), which is longer than the `SNAPSHOT_WAIT_TIMEOUT` used by the proposal path.

**File**: `crates/node/reporters/src/lib.rs` (lines 1317-1318, 1442-1459)

## Code Reference

```rust
// crates/node/reporters/src/lib.rs:1317-1318
/// Serializes finalized-block persistence so marshal acknowledgements advance in chain order.
finalize_lock: Arc<::tokio::sync::Mutex<()>>,
```

```rust
// crates/node/reporters/src/lib.rs:1429-1461
fn report(&mut self, update: Self::Activity) -> Feedback {
    let state = self.state.clone();
    // ... clone all fields ...
    let finalize_lock = self.finalize_lock.clone();
    self.context.child("report_task").spawn(move |_| async move {
        let _guard = finalize_lock.lock().await;  // <-- Held for entire pipeline
        handle_finalized_update(
            state, context, executor, provider, block_index,
            mempool_broadcast, gc_log, metrics, checkpoint_interval,
            pending_acks, node_state, update,
        ).await;
        // _guard dropped here -- after ALL finalization work is done
    });
    Feedback::Ok
}
```

The ledger mutex contention point in the proposal path:

```rust
// crates/node/ledger/src/lib.rs:397-420
pub async fn wait_for_snapshot(
    &self,
    parent: ConsensusDigest,
    timeout: Duration,  // 100ms
) -> Option<LedgerSnapshot> {
    let deadline = ::tokio::time::Instant::now() + timeout;
    loop {
        let notified = self.snapshot_notify.notified();
        if let Some(snap) = self.parent_snapshot(parent).await {
            // parent_snapshot() acquires self.inner.lock().await
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

## Impact

1. **Nullification spikes under I/O load**: When QMDB persistence takes >100ms (e.g., disk contention, large changesets), the `finalize_lock` blocks the proposal path long enough to exceed `SNAPSHOT_WAIT_TIMEOUT`, causing the proposer to skip its turn and nullify.
2. **Throughput reduction**: Each nullification wastes a consensus round, reducing effective block throughput.
3. **Cascading delays**: Multiple finalized blocks queuing up behind the `finalize_lock` create a backlog where each block's finalization is delayed by all preceding blocks' full pipelines.

This was observed on the 10-node devnet where periodic nullification spikes correlated with QMDB flush latency.

## Root Cause

The `finalize_lock` creates an unnecessarily coarse serialization layer. Its purpose is to ensure marshal acknowledgements advance in chain order, but it holds through the entire finalization pipeline including I/O-heavy operations (QMDB persistence, nonce querying) that do not need to be serialized for acknowledgement ordering.

The individual ledger methods (`insert_snapshot`, `persist_snapshot`, `prune_mempool`, etc.) already release and re-acquire the ledger's inner mutex between operations. The `finalize_lock` prevents any interleaving, blocking the proposal path from acquiring the mutex between finalization steps.

## Suggested Fix

1. **Reduce lock scope**: Only hold `finalize_lock` around the acknowledgement ordering logic (the `acknowledge_checkpoint` call), not the entire pipeline:

```rust
// Execution and persistence outside the lock:
let result = finalize_with_retry(&state, &context, &executor, &provider, ...).await;

// Only hold finalize_lock for acknowledgment ordering:
{
    let _guard = finalize_lock.lock().await;
    acknowledge_checkpoint(pending_acks, block.height, checkpoint_interval, ack).await;
}

// Post-finalization cleanup outside the lock:
state.prune_mempool(&block.txs).await;
state.prune_stale_nonces().await;
```

2. **Use a sequencing queue**: Replace the mutex with an ordered queue (e.g., `tokio::sync::mpsc`) that preserves acknowledgement order without blocking the finalization pipeline.

3. **Increase SNAPSHOT_WAIT_TIMEOUT**: As a stopgap, increase the timeout from 100ms to 200ms or 500ms to reduce nullifications, though this trades latency for reliability.

## Files to Modify

- `crates/node/reporters/src/lib.rs` -- reduce scope of `finalize_lock` or replace with a sequencing mechanism

## Related Issues

- [015-ledger-single-mutex-bottleneck.md](./015-ledger-single-mutex-bottleneck.md) -- related mutex contention at the ledger layer
- [029-snapshot-wait-timeout-nullifications.md](./029-snapshot-wait-timeout-nullifications.md) -- downstream effect: snapshot wait timeouts causing nullifications
- [153-wait-for-snapshot-thundering-herd.md](./153-wait-for-snapshot-thundering-herd.md) -- thundering herd effect from global Notify on snapshot insertion

## Labels

`bug`, `performance`, `consensus`, `reliability`
