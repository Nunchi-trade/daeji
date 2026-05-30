# 154: FinalizedReporter::report Returns Feedback::Ok Before Finalization Actually Completes

**Category**: bug
**Severity**: medium
**Component**: consensus

## Summary

The `FinalizedReporter::report()` method spawns finalization work (block execution, state root computation, QMDB persistence, mempool pruning) as a fire-and-forget background task and immediately returns `Feedback::Ok` to the consensus engine. This means the consensus engine believes finalization succeeded before the actual work has even started. If the spawned task is delayed or dropped (e.g., during shutdown or runtime overload), finalized blocks may never be persisted to QMDB despite the consensus engine having already moved on to the next round.

## Problem

In `crates/node/reporters/src/lib.rs`, the `FinalizedReporter` implements the `Reporter` trait with a `report()` method that is called by the commonware simplex consensus engine when a block is finalized:

```rust
// crates/node/reporters/src/lib.rs:1429-1461
fn report(&mut self, update: Self::Activity) -> Feedback {
    let state = self.state.clone();
    let context = self.context.child("report");
    let executor = self.executor.clone();
    // ... clone all fields ...
    let finalize_lock = self.finalize_lock.clone();

    self.context.child("report_task").spawn(move |_| async move {
        let _guard = finalize_lock.lock().await;
        handle_finalized_update(
            state, context, executor, provider, block_index,
            mempool_broadcast, gc_log, metrics, checkpoint_interval,
            pending_acks, node_state, update,
        ).await;
    });

    Feedback::Ok  // <-- Returned BEFORE the spawned task starts executing
}
```

The `handle_finalized_update()` function performs the actual heavy lifting:

1. **Block execution replay**: Re-executes the block to verify state transitions (`finalize_with_retry()`)
2. **State root verification**: Computes the QMDB root and checks it matches the block header
3. **QMDB persistence**: Commits the changeset to disk (`state.persist_snapshot()`)
4. **Block indexing**: Updates the RPC-visible block index (`index_finalized_block()`)
5. **Marshal acknowledgment**: Tells the marshal that the block is durably persisted (`acknowledge_checkpoint()`)
6. **Mempool pruning**: Removes finalized transactions (`state.prune_mempool()`)
7. **Stale nonce pruning**: Evicts transactions with consumed nonces (`state.prune_stale_nonces()`)

If any of these steps fails permanently, the function calls `std::process::abort()` to prevent state divergence. But if the task is never scheduled or is dropped by the runtime (e.g., during graceful shutdown when `tokio::Runtime::shutdown_timeout` expires), none of these steps execute.

**File**: `crates/node/reporters/src/lib.rs` (lines 1422-1461)

## Code Reference

```rust
// crates/node/reporters/src/lib.rs:1422-1461
impl<E, P> Reporter for FinalizedReporter<E, P>
where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    type Activity = Update<Block>;

    fn report(&mut self, update: Self::Activity) -> Feedback {
        let state = self.state.clone();
        let context = self.context.child("report");
        let executor = self.executor.clone();
        let provider = self.provider.clone();
        let block_index = self.block_index.clone();
        let mempool_broadcast = self.mempool_broadcast.clone();
        let gc_log = self.gc_log.clone();
        let metrics = self.metrics.clone();
        let checkpoint_interval = self.checkpoint_interval;
        let pending_acks = self.pending_acks.clone();
        let finalize_lock = self.finalize_lock.clone();
        let node_state = self.node_state.clone();
        self.context.child("report_task").spawn(move |_| async move {
            let _guard = finalize_lock.lock().await;
            handle_finalized_update(
                state,
                context,
                executor,
                provider,
                block_index,
                mempool_broadcast,
                gc_log,
                metrics,
                checkpoint_interval,
                pending_acks,
                node_state,
                update,
            )
            .await;
        });
        Feedback::Ok  // <-- Consensus engine thinks finalization is done
    }
}
```

## Impact

1. **Lost finalization on shutdown**: If the node shuts down while finalization tasks are queued, the `finalize_lock` serialization means tasks waiting for the lock are dropped by the runtime. These blocks are finalized in consensus but not persisted to QMDB.

2. **In-memory state inconsistency**: The consensus engine's snapshot store may have moved the head forward based on finalization reports, but QMDB has not committed the corresponding state transitions. After restart, the node must re-derive the state from the marshal's archive, which adds recovery time.

3. **Backpressure blindness**: The consensus engine has no mechanism to detect that finalization is falling behind. It continues to propose and finalize new blocks even if the finalization pipeline is backlogged. This can cause the snapshot store to grow unbounded (tracked by issue #015).

**Mitigating factors**:
- The `acknowledge_checkpoint()` mechanism defers marshal acknowledgment until QMDB actually commits. This means the marshal will not garbage-collect block data for blocks that were never acknowledged, so the data is available for re-derivation on restart.
- The `finalize_lock` ensures in-order processing, preventing partial persistence (e.g., block N+1 persisted but block N not).
- On permanent finalization failure, `std::process::abort()` halts the node immediately, preventing further divergence.

## Root Cause

The `Reporter::report()` trait method is synchronous (returns `Feedback`) but the finalization work is inherently async and I/O-heavy. The fire-and-forget spawn pattern was chosen to avoid blocking the consensus engine's event loop, but it decouples the success signal from the actual work. The `Feedback` enum does not have a variant for "work is in progress" or "apply backpressure."

## Suggested Fix

1. **Track outstanding tasks**: Maintain a counter of outstanding finalization tasks. If the counter exceeds a threshold (e.g., 4), return `Feedback::Backpressure` (if supported by the commonware API) or log a warning and skip proposals.

```rust
fn report(&mut self, update: Self::Activity) -> Feedback {
    let outstanding = self.outstanding_tasks.fetch_add(1, Ordering::Relaxed);
    if outstanding > MAX_OUTSTANDING_FINALIZATIONS {
        warn!(outstanding, "finalization backlog exceeded threshold");
    }
    // ... spawn task ...
    // At the end of handle_finalized_update:
    // outstanding_tasks.fetch_sub(1, Ordering::Relaxed);
    Feedback::Ok
}
```

2. **Graceful shutdown drain**: During shutdown, wait for all outstanding finalization tasks to complete before dropping the runtime. This requires a `JoinHandle` or `tokio::sync::Semaphore` to track in-flight tasks.

3. **Use JoinHandle instead of fire-and-forget**: Store the `JoinHandle` from `spawn()` and check for panics on subsequent `report()` calls:

```rust
if let Some(handle) = self.last_task.take() {
    if handle.is_finished() {
        handle.await.expect("finalization task panicked");
    }
}
```

## Files to Modify

- `crates/node/reporters/src/lib.rs` -- add outstanding task tracking, graceful shutdown drain, or JoinHandle monitoring to `FinalizedReporter`

## Related Issues

- [152-acknowledge-checkpoint-std-mutex-async.md](./152-acknowledge-checkpoint-std-mutex-async.md) -- the fire-and-forget pattern enables the cascading mutex poisoning bug described there
- [150-finalize-lock-starves-proposal.md](./150-finalize-lock-starves-proposal.md) -- the `finalize_lock` serialization that causes task queuing
- [101-shutdown-graceful-shutdown.md](./101-shutdown-graceful-shutdown.md) -- broader graceful shutdown issue where in-flight finalization tasks are dropped

## Labels

`bug`, `reliability`, `consensus`, `shutdown`
