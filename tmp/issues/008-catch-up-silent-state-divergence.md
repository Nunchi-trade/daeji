# 008: Catch-Up Mode Creates Silent State Divergence Via Empty Changesets

**Category:** bug / consensus
**Severity:** critical
**Labels:** bug, correctness, consensus, recovery

---

## Summary

During catch-up after a restart, the node trusts finality certificates without fully re-executing blocks. When a block cannot be fully verified (missing parent snapshot, execution failure, or state root mismatch), the code creates a "restored" snapshot with an **empty changeset** and marks the block as verified. For chains of consecutive certificate-trusted blocks, the node's local QMDB state silently diverges from the network because none of those blocks' state changes are ever applied. The node appears healthy but will produce invalid blocks when it becomes leader.

## Problem

After a restart, the node enters "catch-up mode" controlled by `RevmApplication::is_catching_up()` at `crates/node/runner/src/app.rs:452-467`. During catch-up, the `verify_block()` method has four fallback paths that all create empty-changeset snapshots instead of rejecting the block:

**Fallback 1 -- Parent snapshot missing** (lines 552-573):
When the parent snapshot is not in the cache (common when multiple blocks were finalized during the restart window), the code trusts the finality certificate and creates a persisted snapshot with `QmdbChangeSet::default()`:

```rust
if self.is_catching_up(block.height) {
    debug!(
        ?digest, ?parent_digest,
        height = block.height,
        "verify_block: parent snapshot missing during catch-up; \
         trusting finality certificate"
    );
    self.ledger.restore_persisted_snapshot(block).await;
    return true;  // Block accepted without execution
}
```

**Fallback 2 -- Execution failure** (lines 603-613): If block execution fails (because the parent snapshot has an empty changeset from a prior trust), falls back to certificate trust.

**Fallback 3 -- Root computation failure** (lines 628-637): Same pattern if `compute_root()` fails.

**Fallback 4 -- State root mismatch** (lines 653-664): If the computed state root does not match the block's declared root (because the parent snapshot was empty), falls back to certificate trust.

The `restore_persisted_snapshot()` method at `crates/node/ledger/src/lib.rs:372-389` creates the snapshot with an empty changeset:

```rust
pub async fn restore_persisted_snapshot(&self, block: &Block) {
    let mut inner = self.inner.lock().await;
    let digest = block.commitment();
    let state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
    let snapshot = Snapshot::new(
        Some(block.parent()),
        state,
        block.state_root,
        QmdbChangeSet::default(),  // Empty changeset -- block's changes are lost
        tx_ids(&block.txs),
    );
    inner.snapshots.insert(digest, snapshot);
    inner.snapshots.mark_persisted(&[digest]);
    inner.head = digest;
    // ...
}
```

The `FinalizedReporter` at `crates/node/reporters/src/lib.rs:571-596` has the same problem. When it encounters a finalized block whose parent snapshot is also missing (cascade from prior certificate-trusted blocks), it also falls back to `restore_persisted_snapshot` (line 592):

```rust
} else {
    // Parent snapshot is missing and the block's own snapshot is also
    // missing.  ...
    warn!(
        ?digest, ?parent_digest,
        "finalize_block: parent snapshot unavailable; restoring block as \
         trusted persisted snapshot to unblock finalization pipeline"
    );
    state.restore_persisted_snapshot(block).await;
}
```

This means that for chains of certificate-trusted blocks, neither the verification path nor the finalization path ever re-executes the block's transactions against the correct state.

## Code Reference

See the code blocks above. The key locations are:

- `crates/node/runner/src/app.rs:452-467` -- `is_catching_up()` determines whether catch-up mode is active
- `crates/node/runner/src/app.rs:552-573` -- Fallback 1: parent snapshot missing
- `crates/node/runner/src/app.rs:603-613` -- Fallback 2: execution failure
- `crates/node/runner/src/app.rs:628-637` -- Fallback 3: root computation failure
- `crates/node/runner/src/app.rs:653-664` -- Fallback 4: state root mismatch
- `crates/node/reporters/src/lib.rs:571-596` -- FinalizedReporter also uses empty-changeset restore
- `crates/node/ledger/src/lib.rs:372-389` -- `restore_persisted_snapshot()` creates snapshot with `QmdbChangeSet::default()`

## Impact

After a restart followed by rapid catch-up (which happens when the network produces blocks during the restart window), a chain of certificate-trusted blocks may never be re-executed. The node appears healthy -- advancing heights, participating in consensus, receiving finality certificates -- but its local QMDB state is missing the changes from those blocks. The divergence manifests in several ways:

1. **Invalid proposals**: When the node becomes leader and proposes a block, it computes a state root from its (stale) QMDB state. All other validators reject the proposal because their state roots are correct.

2. **Stale RPC responses**: RPC queries (e.g., `eth_getBalance`) return stale state data for accounts modified during the missed blocks.

3. **Finalization pipeline failure**: The finalization pipeline eventually detects a state root mismatch and may abort, stalling the node.

The catch-up threshold of 64 blocks (`CATCH_UP_THRESHOLD` at line 86) means the node must advance 64 full-execution verifications past its recovery point before catch-up ends. If all 64 blocks are certificate-trusted (common after a multi-second outage at 33 blocks/s), none of them are re-executed.

## Root Cause

The catch-up trust mechanism was designed for single-block gaps where the parent snapshot is temporarily unavailable. It correctly handles the common case where a recently restarted node misses a few blocks. However, when multiple consecutive blocks are certificate-trusted, the finalization pipeline's parent-snapshot lookup also misses, creating a cascade of empty-changeset snapshots with no re-execution path.

The key design flaw is that `restore_persisted_snapshot()` uses `QmdbChangeSet::default()` rather than queuing the block for later re-execution. Once a block is marked as "persisted" in the snapshot store, it is never revisited.

## Suggested Fix

1. **Queue for re-execution**: When a block is certificate-trusted but not fully re-executed, add it to a retry queue. After catch-up completes and parent snapshots become available, re-execute the queued blocks in order:

```rust
// In verify_block(), catch-up fallback:
if self.is_catching_up(block.height) {
    self.ledger.restore_persisted_snapshot(block).await;
    self.re_execution_queue.push(block.clone());  // Queue for later
    return true;
}

// After catch-up completes:
for block in self.re_execution_queue.drain(..) {
    self.re_execute_and_update_state(&block).await;
}
```

2. **Track unverified blocks**: Add a metric/counter for blocks that were certificate-trusted but never re-executed. Alert operators when this count is non-zero after catch-up completes.

3. **Force re-sync**: If any finalized block could not be re-executed by the reporter after catch-up ends, emit a critical error and trigger a state re-sync rather than silently continuing with stale QMDB state.

## Files to Modify

- `crates/node/runner/src/app.rs` -- Lines 552-664: Add re-execution queue to all four catch-up fallback paths
- `crates/node/reporters/src/lib.rs` -- Lines 571-596: Add re-execution logic to FinalizedReporter
- `crates/node/ledger/src/lib.rs` -- Lines 372-389: Consider alternative to empty changeset for restored snapshots

## Related Issues

- [009 -- GraduatedBlocker Catch-Up Flag Never Cleared](./009-graduated-blocker-never-cleared.md) -- Related catch-up recovery issue
- [005 -- Memory Exhaustion](./005-memory-exhaustion-devnet.md) -- OOM kills trigger restarts, which trigger catch-up
