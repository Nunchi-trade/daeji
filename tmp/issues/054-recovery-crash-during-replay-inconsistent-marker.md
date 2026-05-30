# Crash During Replay Recovery Can Leave Inconsistent Commit Marker

**Category:** recovery, reliability
**Severity:** medium

## Summary

During crash recovery, the `replay_finalized_block()` function re-executes finalized blocks and inserts their state snapshots into the in-memory store, but does not update the commit marker (which tracks the last successfully committed block on disk). The commit marker is only updated asynchronously later, via the `LedgerEvent::SnapshotPersisted` event handler. If the node crashes again during the replay sequence -- after state has been partially written to QMDB but before the commit marker is updated -- the commit marker will point to a stale block, potentially causing state inconsistency on the next restart.

## Problem

Kora is an EVM execution client that uses QMDB (a Merkle database) for persistent state storage and a commonware archive for finalized block history. On startup, the node recovers by reading blocks from the archive and replaying them through the EVM executor to rebuild state. The `restore_checkpoint_and_replay_tail` function (in `crates/node/runner/src/runner.rs`, starting at line 360) orchestrates this process.

The replay calls `replay_finalized_block()` (lines 463-531) for each block in the archive tail. This function:
1. Executes the block through the EVM executor (line 480-482)
2. Computes the state root from QMDB (line 483-486)
3. Verifies the state root matches (line 487-493)
4. Inserts the snapshot into the in-memory store (line 520-529)

Critically, the function does **not** update the commit marker. The commit marker is only written in `spawn_ledger_observers()` (line 667-691), which is a separate async task that subscribes to `LedgerEvent::SnapshotPersisted` events. During normal operation, this works because the async event pipeline is reliable. During crash recovery replay, however, snapshots are inserted but may not be persisted (to QMDB) and the commit marker may not be updated before a second crash.

## Code Reference

File: `crates/node/runner/src/runner.rs`, lines 463-531 (the replay function that does NOT update the commit marker):

```rust
async fn replay_finalized_block(
    ledger: &LedgerService,
    provider: &RevmContextProvider,
    executor: &RevmExecutor,
    block: &Block,
    block_index: &BlockIndex,
) -> anyhow::Result<()> {
    let digest = block.commitment();
    if ledger.query_state_root(digest).await.is_some() {
        return Ok(());
    }

    let parent_digest = block.parent();
    let parent_snapshot = ledger.parent_snapshot(parent_digest).await.with_context(|| {
        format!("missing parent snapshot while replaying height {}", block.height)
    })?;
    let block_context = provider.context(block);
    let execution = BlockExecution::execute(&parent_snapshot, executor, &block_context, &block.txs)
        .await
        .with_context(|| format!("failed to replay finalized block at height {}", block.height))?;
    let state_root = ledger
        .compute_root_from_store(parent_digest, &execution.outcome.changes)
        .await
        .with_context(|| format!("failed to compute replay root at height {}", block.height))?;
    // ... state root verification ...

    let merged_changes = parent_snapshot.state.merge_changes(execution.outcome.changes.clone());
    let next_state = kora_overlay::OverlayState::new(parent_snapshot.state.base(), merged_changes);
    ledger
        .insert_snapshot(
            digest,
            parent_digest,
            next_state,
            state_root,
            execution.outcome.changes,
            &block.txs,
        )
        .await;
    Ok(())
    // NOTE: commit marker is NOT updated here
}
```

File: `crates/node/runner/src/runner.rs`, lines 667-691 (the async observer that DOES update the commit marker -- but only on `SnapshotPersisted` events):

```rust
fn spawn_ledger_observers<S: Spawner>(service: LedgerService, spawner: S, data_dir: PathBuf) {
    let mut receiver = service.subscribe();
    spawner.shared(true).spawn(move |_| async move {
        while let Some(event) = receiver.next().await {
            match event {
                // ...
                LedgerEvent::SnapshotPersisted(digest) => {
                    trace!(?digest, "snapshot persisted");
                    if let Err(e) = crate::commit_marker::write_commit_marker(&data_dir, &digest) {
                        warn!(
                            error = %e,
                            ?digest,
                            "failed to write commit marker after persist"
                        );
                    }
                }
            }
        }
    });
}
```

There is a guard at line 471 that checks if the state root is already known (`if ledger.query_state_root(digest).await.is_some()`), which prevents re-execution of blocks whose snapshots are already in the store. However, this guard does not protect against QMDB having received partial writes from a previous crashed replay attempt.

## Impact

1. **Inconsistent restart state**: If the node crashes during the replay sequence (e.g., OOM kill, power loss, SIGKILL), the commit marker may still point to block N while QMDB has partial writes from block N+1. On the next restart, `restore_checkpoint_and_replay_tail` will attempt to replay from block N+1 again, but QMDB may already contain stale partial data from the previous attempt.

2. **State root mismatch**: The re-replay of block N+1 over a QMDB with partial prior writes may compute a different state root than expected (because the QMDB base state is not clean). This causes the `anyhow::ensure!` at line 487 to fail, aborting the node with a "replayed root mismatch" error.

3. **Manual recovery required**: An operator facing this situation must manually delete the QMDB state directory and the commit marker, forcing a full replay from genesis -- which can take a very long time for chains with significant history.

The window for this bug is narrow (a crash must occur during the replay sequence, between QMDB write and commit marker update), but it becomes more likely with longer replay sequences (more blocks to replay) or under resource pressure (e.g., Docker containers with memory limits that trigger OOM kills).

## Root Cause

The commit marker update is architecturally decoupled from the replay execution path. During normal operation, it runs through the async event pipeline (`LedgerEvent::SnapshotPersisted` -> `write_commit_marker`). During crash recovery, the replay function operates synchronously in the startup path, but there is no synchronous commit marker update to match.

## Suggested Fix

Write the commit marker synchronously after each successful block replay during the recovery sequence. This ensures each replayed block is durably marked before proceeding to the next.

**Add after line 529** in `replay_finalized_block()` (or in the calling loop in `restore_checkpoint_and_replay_tail()`):

```rust
// During recovery replay, persist the snapshot and update the commit marker
// synchronously so that a crash does not leave the marker pointing to a
// stale block.
ledger.persist_snapshot(digest).await?;
crate::commit_marker::write_commit_marker(data_dir, &digest)?;
```

Alternatively, the commit marker update could be added to the calling loop in `restore_checkpoint_and_replay_tail()` at lines 398-420, after each `replay_finalized_block()` call returns successfully.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- add synchronous commit marker update to `replay_finalized_block()` or its calling loop in `restore_checkpoint_and_replay_tail()`

## Related Issues

- `056-docker-devnet-run-always-clears-state.md` -- the devnet script clears runtime state (including QMDB) on every start, which can create commit marker vs. QMDB mismatches

## Labels

bug, reliability, recovery
