# 146: restore_persisted_snapshot Creates Snapshot with Empty Changeset Over Stale QMDB State

**Category**: bug
**Severity**: high
**Component**: consensus / ledger

## Summary

When a finalized block arrives during catch-up or after a restart, and neither the block's own snapshot nor its parent snapshot exists in memory, the `restore_persisted_snapshot()` method creates a synthetic snapshot with an empty changeset (`QmdbChangeSet::default()`) overlaid on the current QMDB base state. This snapshot is immediately marked as persisted and set as the head. However, no QMDB commit has occurred for this block's state transitions, so the overlay state (account balances, nonces, storage values) reflects the QMDB checkpoint -- not this block's actual execution results. If the node subsequently becomes the leader and builds a proposal on top of this snapshot, the proposal will execute transactions against stale state.

## Problem

The method `restore_persisted_snapshot()` in `crates/node/ledger/src/lib.rs` (line 373) is called when a finalized block's snapshot is missing from memory. This happens during catch-up (the node was behind and is receiving finalized blocks via the marshal) or after a restart (the in-memory snapshot store is empty).

The method creates a snapshot with:
- `state`: `OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default())` -- an overlay with NO state changes, meaning all state reads go to the QMDB base layer
- `state_root`: `block.state_root` -- the correct root from the block header (trusted via finality certificate)
- `changes`: `QmdbChangeSet::default()` -- empty changeset

This snapshot is then marked as persisted and set as the head. The consensus state root check will pass because `compute_root_from_store()` uses the parent's `state_root` field (from the block header) rather than actually querying QMDB. But the overlay state is stale:

- `state.nonce(addr)` returns the QMDB-committed nonce, not the nonce after this block
- `state.balance(addr)` returns the QMDB-committed balance, not the balance after this block
- `state.storage(addr, slot)` returns the QMDB-committed value, not this block's value

If the node then proposes a block as leader, it uses this stale state to validate transactions, potentially including transactions with nonces that have already been consumed or double-spending balances.

**File**: `crates/node/ledger/src/lib.rs` (lines 372-389)

## Code Reference

```rust
// crates/node/ledger/src/lib.rs:372-389
/// Restore a finalized block as an already-persisted snapshot over the current QMDB state.
pub async fn restore_persisted_snapshot(&self, block: &Block) {
    let mut inner = self.inner.lock().await;
    let digest = block.commitment();
    let state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
    //                                                  ^^^^^^^^^^^^^^^^^^^^^^^^
    //                          Empty changeset -- state reads fall through to QMDB base
    let snapshot = Snapshot::new(
        Some(block.parent()),
        state,
        block.state_root,  // Correct root from finality certificate
        QmdbChangeSet::default(),  // No changes recorded for this block
        tx_ids(&block.txs),
    );
    inner.snapshots.insert(digest, snapshot);
    inner.snapshots.mark_persisted(&[digest]);  // Marked as persisted immediately
    inner.head = digest;
    drop(inner);
    self.snapshot_notify.notify_waiters();
}
```

## Impact

**Scenario**: A 10-node network where node 5 restarts and catches up via the marshal.

1. Node 5 restarts. The in-memory snapshot store is empty.
2. The marshal delivers finalized blocks B100, B101, B102 via catch-up.
3. For each block, `restore_persisted_snapshot()` creates an empty-changeset snapshot over the QMDB state as of B99 (the last committed state before restart).
4. QMDB commits may or may not have happened for B100-B102 yet (depends on finalization timing).
5. Node 5 becomes the leader for view V. It calls `proposal_components()` which returns the overlay from the head snapshot (B102's empty overlay over QMDB-B99 state).
6. Node 5 builds a proposal using `build_proposal_txs()`, which queries nonces and balances from the stale overlay. It may include transactions with nonces already consumed in B100-B102, or transactions spending balances already depleted.
7. Other validators verify the proposal against the correct state and reject it, or the block is accepted with incorrect execution results.

**Mitigating factors**:
- The `MAX_PROPOSAL_LAG` guard limits how far ahead proposals can go relative to finalized height.
- During active catch-up, the node's leader turns may be skipped by consensus timeouts.
- The `acknowledge_checkpoint` mechanism ensures QMDB commits happen before the marshal GCs block data.
- The proposal verification on other nodes will detect state root mismatches and reject invalid proposals.

However, the core issue remains: the node builds proposals against stale state, which wastes network bandwidth and causes nullifications.

## Root Cause

`restore_persisted_snapshot()` creates a snapshot that is indistinguishable from a fully-executed snapshot. There is no flag to indicate that the snapshot was certificate-trusted (not fully executed), and no guard to prevent building proposals on top of such a snapshot.

## Suggested Fix

1. **Add a `verified` flag to `Snapshot`**: Mark snapshots as certificate-trusted vs. fully-verified.
2. **Skip proposal when parent is unverified**: In `propose()`, check if the parent snapshot is certificate-trusted and return `None` to skip the proposal.
3. **Alternatively**: Do not set `restore_persisted_snapshot` snapshots as the head -- only use them as placeholders for chain-walking termination, and wait for the actual execution to produce a verified snapshot.

```rust
// In propose():
let parent_snap = snapshots.get(&parent_digest)?;
if !parent_snap.is_verified {
    warn!("parent snapshot is certificate-trusted, skipping proposal");
    return None;
}
```

## Files to Modify

- `crates/node/ledger/src/lib.rs` -- modify `restore_persisted_snapshot()` to mark the snapshot as unverified
- `crates/node/consensus/src/traits.rs` -- add `verified: bool` field to `Snapshot`
- `crates/node/runner/src/app.rs` -- check `verified` flag before building proposals

## Related Issues

- [008-catch-up-silent-state-divergence.md](./008-catch-up-silent-state-divergence.md) -- related catch-up divergence issue via empty changesets; this issue covers the specific `restore_persisted_snapshot` path

## Labels

`bug`, `correctness`, `consensus`, `recovery`
