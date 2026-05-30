# Overlay ChangeSet Fully Cloned on Every commit and compute_root

**Category**: Performance -- Storage
**Severity**: Medium
**Labels**: `performance`, `storage`, `executor`

## Summary

Both `OverlayState::commit()` and `OverlayState::compute_root()` perform a full deep clone of the overlay's `ChangeSet` (stored behind an `Arc`) before merging it with incoming changes. The `ChangeSet` contains a `BTreeMap<Address, AccountUpdate>` where each `AccountUpdate` can hold large storage maps and contract bytecode. Every block execution triggers at least one clone via `compute_root()` and one via `commit()`, creating significant allocation churn at 34 blocks/s.

## Problem

Kora is an EVM execution client that layers pending state changes on top of a persistent state database (QMDB). The `OverlayState<S>` struct in `crates/storage/overlay/src/overlay.rs` stores its change set behind an `Arc<ChangeSet>` for cheap reference counting. However, both `commit()` and `compute_root()` need to merge the overlay's existing changes with new incoming changes, which requires a mutable copy. The current implementation achieves this by deep-cloning the entire `Arc` contents via `(*overlay).clone()`.

The `ChangeSet` is defined in `crates/storage/qmdb/src/changes.rs:8-12` as:
```rust
pub struct ChangeSet {
    pub accounts: BTreeMap<Address, AccountUpdate>,
}
```

Each `AccountUpdate` can contain:
- A `BTreeMap<U256, U256>` for storage changes (up to thousands of entries for a contract with many storage modifications)
- An `Option<Vec<u8>>` for contract bytecode (up to 24KB per EIP-170)

Cloning the full `ChangeSet` copies all of this data, including deep-copying every `BTreeMap` node and every `Vec<u8>`.

## Code Reference

`crates/storage/overlay/src/overlay.rs:139-165`:
```rust
impl<S: StateDbWrite> StateDbWrite for OverlayState<S> {
    fn commit(
        &self,
        changes: ChangeSet,
    ) -> impl std::future::Future<Output = Result<B256, StateDbError>> + Send {
        let base = self.base.clone();
        let overlay = Arc::clone(&self.changes);
        async move {
            let mut merged = (*overlay).clone();  // FULL deep clone of entire BTreeMap
            merged.merge(changes);
            base.commit(merged).await
        }
    }

    fn compute_root(
        &self,
        changes: &ChangeSet,
    ) -> impl std::future::Future<Output = Result<B256, StateDbError>> + Send {
        let base = self.base.clone();
        let overlay = Arc::clone(&self.changes);
        let changes = changes.clone();  // Clone incoming changes too
        async move {
            let mut merged = (*overlay).clone();  // FULL deep clone again
            merged.merge(changes);
            base.compute_root(&merged).await
        }
    }

    fn merge_changes(&self, older: ChangeSet, newer: ChangeSet) -> ChangeSet {
        self.base.merge_changes(older, newer)
    }
}
```

Additionally, the `merge_changes` method on `OverlayState` (lines 34-38) also clones:
```rust
pub fn merge_changes(&self, newer: ChangeSet) -> ChangeSet {
    let mut merged = (*self.changes).clone();
    merged.merge(newer);
    merged
}
```

The `ChangeSet::merge()` method in `crates/storage/qmdb/src/changes.rs:32-40`:
```rust
pub fn merge(&mut self, other: Self) {
    for (address, update) in other.accounts {
        if let Some(existing) = self.accounts.get_mut(&address) {
            existing.merge(update);
        } else {
            self.accounts.insert(address, update);
        }
    }
}
```

## Impact

Every block execution triggers at least one `compute_root()` call (when the proposer computes the state root) and one `commit()` call (to persist the state). Each call performs a full deep clone of the overlay's change set. At 34 blocks per second, this creates 68+ deep clones per second.

For a block that touches 200 accounts, each with a few storage changes, the `ChangeSet` clone copies approximately 200 `AccountUpdate` structs, each containing nested `BTreeMap` nodes. While each individual clone is not catastrophically expensive, the cumulative allocation churn puts pressure on the global allocator, increases fragmentation, and contributes to latency variance in the execution pipeline.

Concrete scenario: A block containing a batch DEX arbitrage transaction touches 50 contracts, modifying 500 storage slots total. The resulting `ChangeSet` contains 50 `AccountUpdate` entries with a combined 500 `BTreeMap` entries. Cloning this twice per block (once for `compute_root`, once for `commit`) at 34 blocks/s results in 68 clones/s, creating significant allocator traffic.

## Root Cause

The `ChangeSet` is stored in an `Arc` (shared reference), but the `merge` operation requires a mutable copy. The current approach clones the entire `Arc` contents to produce a mutable copy for merging, even though the original `Arc` is often not needed after the merge.

## Suggested Fix

**Option 1** (simplest): Use `Arc::try_unwrap()` to avoid the clone when there is only one reference:

```rust
fn commit(
    &self,
    changes: ChangeSet,
) -> impl std::future::Future<Output = Result<B256, StateDbError>> + Send {
    let base = self.base.clone();
    let overlay = Arc::clone(&self.changes);
    async move {
        let mut merged = Arc::try_unwrap(overlay).unwrap_or_else(|arc| (*arc).clone());
        merged.merge(changes);
        base.commit(merged).await
    }
}
```

**Option 2**: Use a copy-on-write data structure (e.g., `im::OrdMap` from the `im` crate) for the change set's `accounts` field. Cloning an `im::OrdMap` is O(1) since it shares structure.

**Option 3**: Restructure the `merge` operation to accept two immutable references and produce a new `ChangeSet` without modifying either input, enabling shared access without cloning. This avoids mutation but still requires building a new `BTreeMap`:

```rust
fn merge_two(base: &ChangeSet, newer: &ChangeSet) -> ChangeSet {
    let mut merged = base.clone();
    for (address, update) in &newer.accounts {
        // ... merge logic ...
    }
    merged
}
```

This does not eliminate the clone but centralizes it.

## Files to Modify

- `crates/storage/overlay/src/overlay.rs` -- `commit()` (lines 140-151), `compute_root()` (lines 153-165), and `merge_changes()` (lines 34-38): use `Arc::try_unwrap()` or restructure to avoid cloning
- `crates/storage/qmdb/src/changes.rs` -- Optionally switch `BTreeMap` to `im::OrdMap` for O(1) cloning

## Related Issues

- [036-storage-overlay-code-lookup-linear-scan.md](036-storage-overlay-code-lookup-linear-scan.md) -- Adding a secondary index (code_by_hash) to `ChangeSet` would increase the clone cost. Both issues should be considered together when redesigning the `ChangeSet` structure.
