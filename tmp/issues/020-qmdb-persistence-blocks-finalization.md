# QMDB Persistence Blocks the Finalization Pipeline

**Category**: Performance
**Severity**: High
**Labels**: `performance`, `storage`, `consensus`

## Summary

The finalization pipeline blocks synchronously on QMDB disk persistence at every checkpoint interval (every 256 blocks by default). The `finalize_block()` function spawns a persistence task but immediately `.await`s it, halting all finalization progress until the disk I/O completes. During this stall, new finalized blocks accumulate in memory, increasing the gap between the consensus tip and finalized height, and exacerbating the snapshot eviction race condition (issue #011).

## Problem

The `finalize_block()` function at `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:600-612` spawns a background task for persistence but immediately awaits its result, making it effectively synchronous:

```rust
if persist_checkpoint {
    let persist_state = state.clone();
    let persist_handle = context
        .child("persist")
        .shared(true)
        .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
    let persist_result = persist_handle
        .await  // <-- BLOCKS finalization until disk I/O completes
        .map_err(|err| FinalizationError::PersistTaskFailed(format!("{err}")))?;
    if let Err(err) = persist_result {
        return Err(FinalizationError::PersistFailed(err));
    }
}
```

The `persist_snapshot()` method at `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:464-514` performs substantial work that involves both CPU and disk I/O:

```rust
pub async fn persist_snapshot(&self, digest: ConsensusDigest) -> LedgerResult<bool> {
    // Step 1: Acquire mutex, collect changeset chain
    let (changes, qmdb, chain) = {
        let inner = self.inner.lock().await;  // Mutex acquisition #1
        let (chain, changes) = inner.snapshots.changes_for_persist(digest)?;
        if chain.is_empty() { return Ok(false); }
        if !inner.snapshots.can_persist_chain(&chain) { return Ok(false); }
        inner.snapshots.mark_persisting_chain(&chain);
        (changes, inner.qmdb.clone(), chain)
    };

    // Step 2: QMDB commit -- writes to 3 partitions sequentially (DISK I/O)
    let result = qmdb.commit_changes(changes).await;

    // Step 3: Re-acquire mutex, update markers, evict old snapshots
    {
        let inner = self.inner.lock().await;  // Mutex acquisition #2
        inner.snapshots.clear_persisting_chain(&chain);
        match result {
            Ok(_) => {
                // ... replace snapshots with compact versions ...
                inner.snapshots.mark_persisted(&chain);
                inner.snapshots.evict_persisted();
                Ok(())
            }
            Err(err) => Err(LedgerError::from(err)),
        }
    }?;
    Ok(true)
}
```

The QMDB `commit_changes()` method at `/Users/will/dev/nunchi/daeji/crates/storage/qmdb-ledger/src/ledger.rs:112-125` acquires its own internal locks and writes to three storage partitions (accounts, storage, code) sequentially:

```rust
pub async fn commit_changes(&self, changes: QmdbChangeSet) -> Result<StateRoot, Error> {
    let _storage_access = self.handle.storage_access().await;
    let mut store = self.handle.write().await;
    store
        .commit_changes(changes)
        .await
        .map_err(|e| kora_traits::StateDbError::Storage(e.to_string()))?;
    // ...
}
```

The `apply_batches()` method at `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs:334` writes to each partition sequentially.

## Code Reference

**Persistence checkpoint trigger** in the `FinalizedReporter` (`/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:600-612`):

```rust
if persist_checkpoint {
    let persist_state = state.clone();
    let persist_handle = context
        .child("persist")
        .shared(true)
        .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
    let persist_result = persist_handle
        .await
        .map_err(|err| FinalizationError::PersistTaskFailed(format!("{err}")))?;
    if let Err(err) = persist_result {
        return Err(FinalizationError::PersistFailed(err));
    }
}
```

**persist_snapshot two-phase lock** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:464-514`) -- see full code above.

## Impact

At each persistence checkpoint (every 256 blocks, roughly every 7.7 seconds at 33 blocks/s), the finalization pipeline stalls for the duration of the QMDB commit. On the devnet's shared NVMe storage with 10 validator nodes, this commit can take tens to hundreds of milliseconds depending on the changeset size and I/O contention.

During this stall:

1. **Finalized blocks queue up in memory**: New blocks continue to be consensus-finalized while persistence is blocking, increasing the gap between the consensus tip and the last finalized height.

2. **Snapshot eviction race window widens**: The growing gap increases the likelihood that the snapshot eviction race (issue #011) will trigger, because more snapshots accumulate in memory while persistence is stuck.

3. **LedgerView mutex contention increases**: The `persist_snapshot` method acquires the global `LedgerView` mutex twice (issue #015). During the second acquisition (after disk I/O), all other ledger operations (proposal, verification, transaction submission, state queries) are blocked.

4. **Sawtooth finalization throughput**: The periodic stalls create a "sawtooth" pattern in finalization throughput -- fast progress for 256 blocks, then a pause, then fast progress again. This limits sustained average throughput.

5. **Increased nullification risk**: If the proposer's finalization pipeline is stalled during persistence and it falls behind by more than `MAX_PROPOSAL_LAG` (64 blocks), the proposal lag guard at `app.rs:792` kicks in and skips proposals, causing nullification.

## Root Cause

The persistence was designed synchronously for correctness: ensuring blocks are written to QMDB before acknowledging finalization. However, the consensus system does not actually require persistence before advancing -- blocks are already consensus-finalized (backed by a threshold signature from 2/3+ validators) and can safely survive in memory until the next checkpoint. The `persist_checkpoint` flag already batches persistence to every 256 blocks rather than every block, showing that deferred persistence is acceptable.

## Suggested Fix

### Option 1: Async persistence pipeline (recommended)

Submit persistence work to a bounded background channel and continue finalization immediately. The background worker processes persistence requests sequentially, ensuring QMDB writes are ordered. The next checkpoint waits for the previous one to complete, but individual block finalization does not wait.

```rust
// In FinalizedReporter setup:
let (persist_tx, persist_rx) = tokio::sync::mpsc::channel(4);
// Spawn background persistence worker
context.child("persist_worker").shared(true).spawn(move |_| async move {
    while let Some((digest, state)) = persist_rx.recv().await {
        if let Err(e) = state.persist_snapshot(digest).await {
            error!(?digest, error = %e, "background persistence failed");
        }
    }
});

// In finalize_block:
if persist_checkpoint {
    persist_tx.send((digest, state.clone())).await
        .map_err(|_| FinalizationError::PersistTaskFailed("channel closed".into()))?;
    // Don't await -- continue finalizing immediately
}
```

### Option 2: Batch persistence with coalescing

Instead of one commit per checkpoint, allow multiple checkpoints to coalesce when the disk is slow. The changeset merge in `changes_for_persist()` already handles multi-block ranges, so if a second checkpoint arrives before the first finishes, the two can be merged into a single QMDB commit.

### Option 3: Parallel partition writes

The QMDB `apply_batches()` method writes to accounts, storage, and code partitions sequentially. These writes are independent and could be parallelized with `tokio::join!` or similar, reducing the total disk I/O time per checkpoint by up to 3x.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` (lines 600-612) -- `finalize_block` blocks on persistence; change to async submission
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (lines 464-514) -- `persist_snapshot()` performs disk I/O between two mutex acquisitions
- `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs` (line 334) -- `apply_batches()` writes partitions sequentially (could be parallelized)
- `/Users/will/dev/nunchi/daeji/crates/storage/qmdb-ledger/src/ledger.rs` (lines 112-125) -- `commit_changes()` acquires storage access lock and write lock

## Related Issues

- `011-snapshot-eviction-race-finalization.md` -- the persistence stall increases the window for the snapshot eviction race
- `015-ledger-single-mutex-bottleneck.md` -- the single LedgerView mutex amplifies the contention during persistence
- `014-triple-block-execution.md` -- eliminating the finalization re-execution would reduce the per-block cost and make persistence the dominant bottleneck more visible
- `001-qmdb-non-atomic-cross-partition-writes.md` -- QMDB writes are not atomic across partitions; parallelizing them must preserve crash consistency
