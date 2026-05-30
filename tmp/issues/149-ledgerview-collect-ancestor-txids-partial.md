# 149: LedgerView::collect_ancestor_tx_ids Silently Returns Partial Excluded Set on Snapshot Gap

**Category**: bug
**Severity**: medium
**Component**: consensus

## Summary

The `LedgerView::collect_ancestor_tx_ids()` method silently breaks out of its ancestor chain walk when a snapshot is not found, returning a partial set of excluded transaction IDs. This contrasts with the production code path in `RevmApplication::collect_pending_tx_ids()` which correctly returns `None` on a gap, causing the proposal to be skipped. If `LedgerView::build_proposal_txs()` is ever used in a production code path, the partial excluded set could lead to duplicate transaction inclusion in proposed blocks. Additionally, the doc comment on this method is stale and inaccurate.

## Problem

In `crates/node/consensus/src/ledger.rs`, the `collect_ancestor_tx_ids()` method walks the snapshot chain from a parent digest back to the last persisted ancestor, collecting transaction IDs that should be excluded from new proposals. When a snapshot is missing from the chain (due to eviction or catch-up), the method silently `break`s and returns whatever partial set it has collected so far:

```rust
// crates/node/consensus/src/ledger.rs:104-121
fn collect_ancestor_tx_ids(&self, _parent: Option<Digest>) -> BTreeSet<TxId> {
    let mut excluded = BTreeSet::new();
    let mut current = _parent;

    while let Some(digest) = current {
        if self.snapshots.is_persisted(&digest) {
            break;
        }

        let Some(snapshot) = self.snapshots.get(&digest) else {
            break;  // <-- silently returns partial excluded set
        };
        excluded.extend(snapshot.tx_ids.iter().copied());
        current = snapshot.parent;
    }

    excluded
}
```

The production proposal path in `RevmApplication::collect_pending_tx_ids()` (file `crates/node/runner/src/app.rs`, lines 733-759) handles the same scenario correctly:

```rust
let Some(snapshot) = snapshots.get(&digest) else {
    warn!(
        ?digest,
        collected_so_far = excluded.len(),
        "snapshot chain gap during tx exclusion collection -- \
         refusing to build block to prevent duplicate transactions"
    );
    return None;  // <-- correctly signals "do not build a block"
};
```

The doc comment on `collect_ancestor_tx_ids()` (lines 99-103) still says "Currently returns an empty set since `Snapshot<S>` does not contain transaction data" -- this is no longer true, as the method now walks the chain and collects `tx_ids`.

**File**: `crates/node/consensus/src/ledger.rs` (lines 99-121)

## Code Reference

```rust
// crates/node/consensus/src/ledger.rs:99-121
/// Collect transaction IDs from unpersisted ancestor blocks.
///
/// Currently returns an empty set since `Snapshot<S>` does not contain
/// transaction data. Transaction deduplication relies on the mempool's
/// prune mechanism after finalization.
fn collect_ancestor_tx_ids(&self, _parent: Option<Digest>) -> BTreeSet<TxId> {
    let mut excluded = BTreeSet::new();
    let mut current = _parent;

    while let Some(digest) = current {
        if self.snapshots.is_persisted(&digest) {
            break;
        }

        let Some(snapshot) = self.snapshots.get(&digest) else {
            break;  // BUG: returns partial set instead of signaling error
        };
        excluded.extend(snapshot.tx_ids.iter().copied());
        current = snapshot.parent;
    }

    excluded
}
```

Compare with the correct production implementation:

```rust
// crates/node/runner/src/app.rs:733-759
fn collect_pending_tx_ids(
    &self,
    snapshots: &InMemorySnapshotStore<OverlayState<QmdbState>>,
    from: ConsensusDigest,
) -> Option<BTreeSet<kora_consensus::TxId>> {
    let mut excluded = BTreeSet::new();
    let mut current = Some(from);

    while let Some(digest) = current {
        if snapshots.is_persisted(&digest) {
            break;
        }
        let Some(snapshot) = snapshots.get(&digest) else {
            warn!(
                ?digest,
                collected_so_far = excluded.len(),
                "snapshot chain gap during tx exclusion collection -- \
                 refusing to build block to prevent duplicate transactions"
            );
            return None;  // CORRECT: signal "do not build"
        };
        excluded.extend(snapshot.tx_ids.iter().copied());
        current = snapshot.parent;
    }

    Some(excluded)
}
```

## Impact

**Current risk**: Low. The `build_proposal_txs()` method that calls `collect_ancestor_tx_ids()` is only used through `LedgerView`, which is primarily used in tests and the consensus crate's generic interface. The production proposal path in `RevmApplication` uses its own correctly-implemented `collect_pending_tx_ids()`.

**Latent risk**: If `LedgerView::build_proposal_txs()` is ever used in a production code path (e.g., a secondary validator mode, a testing harness with real consensus, or a refactoring that consolidates the two paths), the partial excluded set would allow duplicate transactions to be included in proposals. The stale doc comment increases the risk by misleading developers about the method's behavior.

**Specific failure mode**: If snapshots B1 -> B2 -> B3 exist but B2 is evicted, calling `collect_ancestor_tx_ids(Some(B3))` would collect tx IDs from B3 only (missing B2's transactions). A proposal built on B3 could then include transactions already included in B2.

## Root Cause

The method was updated from its original "always return empty set" implementation to actually walk the ancestor chain, but the error handling was not updated to match the production code path. The `break` on missing snapshot was likely a placeholder that was never revisited.

## Suggested Fix

1. Change the return type to `Option<BTreeSet<TxId>>` and return `None` on gap:

```rust
fn collect_ancestor_tx_ids(&self, parent: Option<Digest>) -> Option<BTreeSet<TxId>> {
    let mut excluded = BTreeSet::new();
    let mut current = parent;

    while let Some(digest) = current {
        if self.snapshots.is_persisted(&digest) {
            break;
        }

        let snapshot = self.snapshots.get(&digest)?;  // Returns None on gap
        excluded.extend(snapshot.tx_ids.iter().copied());
        current = snapshot.parent;
    }

    Some(excluded)
}
```

2. Update `build_proposal_txs()` to handle `None`:

```rust
pub fn build_proposal_txs(&self, parent: Option<Digest>, max_txs: usize) -> Vec<Tx> {
    let excluded = match self.collect_ancestor_tx_ids(parent) {
        Some(ids) => ids,
        None => return Vec::new(),  // Don't build if excluded set is incomplete
    };
    self.mempool.build(max_txs, &excluded)
}
```

3. Fix the stale doc comment to accurately describe the method's behavior.

## Files to Modify

- `crates/node/consensus/src/ledger.rs` -- fix `collect_ancestor_tx_ids()` return type and gap handling; fix stale doc comment

## Related Issues

- [008-catch-up-silent-state-divergence.md](./008-catch-up-silent-state-divergence.md) -- related catch-up issue, different code path

## Labels

`bug`, `correctness`, `consensus`
