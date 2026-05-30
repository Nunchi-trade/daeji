# Snapshot Cloning Deep-Copies ChangeSet and tx_ids on Every Read

**Category**: performance -- consensus
**Severity**: medium

**Labels**: `performance`, `consensus`, `storage`

---

## Summary

Every `InMemorySnapshotStore::get()` call clones the entire `Snapshot<S>` structure, which triggers deep copies of the `changes` field (a `ChangeSet` containing `BTreeMap<Address, AccountUpdate>` with nested storage maps) and the `tx_ids` field (a `BTreeSet<TxId>`). These fields are immutable after insertion but are stored as owned values, so every clone is a full deep copy. Wrapping them in `Arc` would make cloning an O(1) atomic reference count bump instead of an O(state_size) copy.

---

## Problem

The `InMemorySnapshotStore` stores snapshots in a `BTreeMap<Digest, Snapshot<S>>` behind an `Arc<RwLock>`. The `get()` method returns a clone of the snapshot. A `Snapshot<S>` contains:

- `parent: Option<Digest>` -- cheap (Copy)
- `state: S` (where S = `OverlayState<QmdbState>`) -- uses `Arc<ChangeSet>` internally, so clone is cheap (Arc bump)
- `state_root: StateRoot` -- cheap (Copy, it's a `B256` wrapper)
- `changes: ChangeSet` -- **expensive**: `BTreeMap<Address, AccountUpdate>` where each `AccountUpdate` contains a `BTreeMap<U256, U256>` of storage slots, plus optional `Vec<u8>` code bytes
- `tx_ids: BTreeSet<TxId>` -- **expensive**: full deep copy of all 32-byte transaction IDs

The `get()` method is called frequently in hot paths:

1. In `build_block` via `wait_for_snapshot` (1 call per proposal)
2. In `verify_block` via `parent_snapshot` (1 call per verification)
3. In `collect_pending_tx_ids` -- walks up to 64 ancestors, cloning each (up to 64 calls per proposal)
4. In `merged_changes` -- walks ancestors for changeset merging (up to 64 calls)
5. In `changes_for_persist` -- walks ancestors for persistence (up to 64 calls)
6. In `finalize_block` via `parent_snapshot` (1 call per finalization)

---

## Code Reference

**Snapshot struct definition** -- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/traits.rs:19-31`:
```rust
#[derive(Clone, Debug)]
pub struct Snapshot<S> {
    /// Parent block digest.
    pub parent: Option<Digest>,
    /// State database at this point.
    pub state: S,
    /// Computed state root.
    pub state_root: StateRoot,
    /// Pending state changes not yet persisted.
    pub changes: ChangeSet,          // <-- owned, deep-cloned
    /// Transaction IDs included in this snapshot's block.
    pub tx_ids: BTreeSet<TxId>,      // <-- owned, deep-cloned
}
```

**The get() clone** -- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs:181-183`:
```rust
impl<S: StateDb> SnapshotStore<S> for InMemorySnapshotStore<S> {
    fn get(&self, digest: &Digest) -> Option<Snapshot<S>> {
        self.snapshots.read().get(digest).cloned()  // Deep clone of entire snapshot
    }
```

**Chain walk that clones each snapshot** -- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs:203-225`:
```rust
    fn merged_changes(
        &self,
        parent: Digest,
        new_changes: ChangeSet,
    ) -> Result<ChangeSet, ConsensusError> {
        let snapshots = self.snapshots.read();
        let persisted = self.persisted.read();

        let mut chain = Vec::new();
        let mut current = Some(parent);

        while let Some(digest) = current {
            if persisted.contains(&digest) {
                break;
            }
            let snapshot =
                snapshots.get(&digest).ok_or(ConsensusError::SnapshotNotFound(digest))?;
            chain.push(snapshot.changes.clone());  // Deep clone of ChangeSet
            current = snapshot.parent;
        }
```

Note: `merged_changes` accesses snapshots directly from the `snapshots` map (not via the `get()` trait method), so it clones only `changes` rather than the full snapshot. But `collect_pending_tx_ids` in `app.rs` calls `snapshots.get(&digest)` which goes through the `SnapshotStore::get` trait and clones the full snapshot just to read `tx_ids` and `parent`.

---

## Impact

Under load with 100 transactions per block and non-trivial contract interactions:

- Each `ChangeSet` contains a `BTreeMap` with potentially hundreds of account updates, each with nested storage `BTreeMap`s. A single clone copies the entire tree structure.
- The `collect_pending_tx_ids` chain walk clones up to 64 full snapshots per proposal (including their `ChangeSet` and `tx_ids`) even though it only reads the `tx_ids` and `parent` fields.
- At 34 blocks/s, this creates significant allocation pressure from the deep copies, consuming CPU on memory allocation and putting pressure on the global allocator (especially under jemalloc).
- The deep copies also increase cache pollution, as freshly allocated memory for clones displaces hot data from CPU caches.

For empty blocks (current devnet load), the `ChangeSet` is empty and `tx_ids` is empty, so the impact is negligible. The issue becomes significant under production transaction load.

---

## Root Cause

The `Snapshot` struct stores `changes` and `tx_ids` as owned values rather than shared references. The `#[derive(Clone)]` on `Snapshot` triggers deep copies for these standard library collections because they do not implement cheap `Clone` (they are `BTreeMap` and `BTreeSet`, not `Arc`-wrapped). The design prioritizes simplicity over performance.

---

## Suggested Fix

Wrap `changes` and `tx_ids` in `Arc` within the `Snapshot` struct so that cloning is an atomic reference count bump rather than a deep copy:

**Before** (in `traits.rs`):
```rust
pub struct Snapshot<S> {
    pub parent: Option<Digest>,
    pub state: S,
    pub state_root: StateRoot,
    pub changes: ChangeSet,
    pub tx_ids: BTreeSet<TxId>,
}
```

**After**:
```rust
pub struct Snapshot<S> {
    pub parent: Option<Digest>,
    pub state: S,
    pub state_root: StateRoot,
    pub changes: Arc<ChangeSet>,
    pub tx_ids: Arc<BTreeSet<TxId>>,
}
```

Since these fields are immutable once the snapshot is created and inserted into the store, `Arc` sharing is safe and correct. Callers that need to modify a `ChangeSet` (like `merged_changes`) already create their own mutable copy, so this change is transparent to them.

The `Snapshot::new` constructor would wrap the values in `Arc`:
```rust
pub fn new(
    parent: Option<Digest>,
    state: S,
    state_root: StateRoot,
    changes: ChangeSet,
    tx_ids: BTreeSet<TxId>,
) -> Self {
    Self {
        parent,
        state,
        state_root,
        changes: Arc::new(changes),
        tx_ids: Arc::new(tx_ids),
    }
}
```

This changes snapshot cloning from O(state_size) to O(1) and eliminates all allocation overhead from `get()` calls.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/traits.rs` -- change `Snapshot` fields to `Arc<ChangeSet>` and `Arc<BTreeSet<TxId>>`
- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs` -- update `merged_changes`, `changes_for_persist` to deref `Arc`
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- update `collect_pending_tx_ids` and `verify_block` snapshot usage
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` -- update `finalize_block` snapshot usage
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` -- update snapshot insertion calls

---

## Related Issues

- `065-perf-snapshot-chain-walk-tx-exclusion.md` -- chain walk clones compound the cost of this issue
- `015-ledger-single-mutex-bottleneck.md` -- snapshot access contention through the central mutex
- `064-perf-state-root-reacquires-mutex.md` -- another unnecessary operation on each snapshot access
