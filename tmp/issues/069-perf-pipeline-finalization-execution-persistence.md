# Pipeline Finalization to Overlap Re-Execution With Persistence

**Category**: performance -- consensus
**Severity**: medium

**Labels**: `performance`, `consensus`, `storage`, `reliability`

---

## Summary

The finalization pipeline processes blocks sequentially: for each finalized block, it re-executes the block (for RPC indexing data), then persists the state to QMDB, and blocks until persistence completes before processing the next finalized block. There is no overlap between the CPU-bound re-execution and the I/O-bound persistence. Pipelining these two phases would improve sustained throughput by up to 33%, depending on the relative costs of execution and persistence.

---

## Problem

The finalization reporter receives finalized blocks from the marshal in order. For each block, it calls `finalize_block` which performs three main steps:

1. **Re-execute the block** (CPU-bound): Run EVM execution against the parent snapshot to produce `ExecutionOutcome` (for RPC indexing -- receipts, logs, etc.)
2. **Persist to QMDB** (I/O-bound): Write the merged changeset to QMDB's on-disk storage
3. **Wait for persistence** (`await` on the persist handle): Block until the I/O completes

Steps 1 and 2 are serialized: re-execution of block N+1 cannot begin until persistence of block N completes. Since execution is CPU-bound and persistence is I/O-bound, they could run concurrently.

---

## Code Reference

**The sequential finalization pipeline** -- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:476-615`:

The `finalize_block` function re-executes the block (lines 530-533) and then persists (lines 600-612):

```rust
async fn finalize_block<E, P>(
    state: &LedgerService,
    context: &tokio::Context,
    executor: &E,
    provider: &P,
    block_index: Option<&Arc<BlockIndex>>,
    block: &Block,
    persist_checkpoint: bool,
) -> Result<(Option<ExecutionOutcome>, Option<BlockContext>), FinalizationError>
where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    // ... snapshot lookup and re-execution (lines 489-564) ...

    // Persistence: blocks until QMDB write completes
    if persist_checkpoint {
        let persist_state = state.clone();
        let persist_handle = context
            .child("persist")
            .shared(true)
            .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
        let persist_result = persist_handle
            .await                    // <-- BLOCKS until QMDB write completes
            .map_err(|err| FinalizationError::PersistTaskFailed(format!("{err}")))?;
        if let Err(err) = persist_result {
            return Err(FinalizationError::PersistFailed(err));
        }
    }

    Ok((execution_outcome, execution_context))
}
```

**The caller processes blocks sequentially** -- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:255-270`:

```rust
        Update::Block(block, ack) => {
            // ...
            let result = finalize_with_retry(
                &state,
                &context,
                &executor,
                &provider,
                block_index.as_ref(),
                &block,
                persist_checkpoint,
            )
            .await;
            // finalize_with_retry awaits finalize_block, which awaits persistence
            // Only after this returns does the next Update::Block arrive from the channel
```

The flow is strictly sequential:
```
Block N:   [re-execute] [persist] [wait]
Block N+1:                               [re-execute] [persist] [wait]
```

With pipelining, the flow becomes:
```
Block N:   [re-execute] [persist]
Block N+1:              [re-execute] [persist]
                         ^-- overlapping
```

---

## Impact

If re-execution takes T_exec and persistence takes T_persist:

**Sequential** (current): throughput = 1 / (T_exec + T_persist)
**Pipelined** (proposed): throughput = 1 / max(T_exec, T_persist)

For typical values:
- T_exec = 5ms (empty blocks) to 30ms (contract-heavy blocks)
- T_persist = 10ms (SSD) to 50ms (shared NVMe under contention)

Sequential: 1 / (5 + 10) = 66.7 blocks/s max finalization throughput
Pipelined: 1 / max(5, 10) = 100 blocks/s max finalization throughput (50% improvement)

Sequential: 1 / (30 + 50) = 12.5 blocks/s
Pipelined: 1 / max(30, 50) = 20 blocks/s (60% improvement)

On the 10-node devnet sharing one NVMe drive, I/O contention between QMDB writes from 10 nodes makes T_persist the dominant factor. The sequential pipeline means the finalization reporter is idle during the entire persistence phase -- time that could be spent re-executing the next block.

If finalization throughput drops below consensus throughput (~34 blocks/s), the `unpersisted_snapshot_depth` metric rises, eventually triggering the `MAX_PROPOSAL_LAG` guard that throttles block production.

---

## Root Cause

The finalization reporter was designed with a simple sequential pipeline. The `await` on the persistence handle serializes the pipeline rather than allowing concurrent processing. There is no mechanism to start re-execution of block N+1 while persistence of block N is still running.

---

## Suggested Fix

**Approach 1: Fire-and-forget persistence with a semaphore**

The simplest approach is to spawn persistence as a background task and use a semaphore to limit the number of concurrent persistence operations:

```rust
use tokio::sync::Semaphore;

let persist_semaphore = Arc::new(Semaphore::new(2)); // Allow 2 concurrent persists

async fn finalize_block_pipelined(/* ... */) {
    // Re-execute the block (CPU-bound)
    let (outcome, context) = execute_block(block).await?;

    if persist_checkpoint {
        // Wait for a persistence slot (limits memory pressure)
        let permit = persist_semaphore.clone().acquire_owned().await?;
        let persist_state = state.clone();
        context.child("persist").shared(true).spawn(move |_| async move {
            let result = persist_state.persist_snapshot(digest).await;
            drop(permit); // Release semaphore slot
            result
        });
        // Do NOT await the spawn handle -- continue to the next block
    }

    Ok((outcome, context))
}
```

**Approach 2: Bounded channel worker**

For more control, use a bounded channel separating finalization dispatch from processing:

```rust
let (finalize_tx, finalize_rx) = tokio::sync::mpsc::channel(4);

// Producer: sends blocks to finalize
async fn report_finalized(&self, block: Block) {
    finalize_tx.send(block).await.ok();
}

// Consumer: processes with overlapping execution and persistence
async fn finalization_worker(rx: Receiver<Block>) {
    let mut prev_persist: Option<JoinHandle<_>> = None;
    while let Some(block) = rx.recv().await {
        // Wait for previous persistence to complete before starting another
        if let Some(handle) = prev_persist.take() {
            handle.await??;
        }
        let outcome = execute_block(block).await?;
        prev_persist = Some(spawn_persist(outcome));
        // Continue to the next block immediately
    }
}
```

Both approaches require careful error handling: if persistence of block N fails, the pipeline must halt (consistent with the current abort-on-failure behavior in `handle_finalized_update` at line 312).

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` -- refactor `finalize_block` to separate execution from persistence, add pipelining in `handle_finalized_update`

---

## Related Issues

- `061-metrics-missing-gas-persist-rpc-latency.md` -- persist duration metric is needed to measure the improvement
- `020-qmdb-persistence-blocks-finalization.md` -- describes how persistence blocks finalization (the exact issue this fix addresses)
- `015-ledger-single-mutex-bottleneck.md` -- the central mutex adds latency to persistence
- `064-perf-state-root-reacquires-mutex.md` -- redundant mutex in the state root computation path
