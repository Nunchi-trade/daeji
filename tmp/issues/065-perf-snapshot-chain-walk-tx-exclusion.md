# Snapshot Chain Walk and BTreeSet for Transaction Exclusion Is O(depth * N)

**Category**: performance -- consensus
**Severity**: medium

**Labels**: `performance`, `consensus`

---

## Summary

When building a block proposal, Kora walks backward through all unpersisted ancestor snapshots to collect transaction IDs that should be excluded (already included in pending blocks). This produces a `BTreeSet<TxId>` rebuilt from scratch on every proposal by copying up to `64 * N` transaction IDs (where N is transactions per block). Additionally, `BTreeSet` provides O(log n) lookup when the ordering property is not needed -- a `HashSet` would provide O(1) lookups.

---

## Problem

The `collect_pending_tx_ids` method walks the snapshot ancestry chain from the current tip back to the last persisted snapshot, collecting all transaction IDs from unpersisted blocks into a `BTreeSet`. This set is then used to exclude already-included transactions during block building (via the `Mempool::build(max_txs, &excluded)` call).

The chain walk has two performance issues:

1. **Full rebuild on every proposal**: The exclusion set is rebuilt from scratch by walking up to `MAX_PROPOSAL_LAG = 64` ancestor snapshots. Each snapshot's `tx_ids` (`BTreeSet<TxId>`) is iterated and copied into a new `BTreeSet`.

2. **BTreeSet vs HashSet**: The exclusion set only needs membership testing (is this TX already included?). `BTreeSet` provides O(log n) lookup and O(n log n) construction, while `HashSet` provides O(1) lookup and O(n) construction. The sorted ordering of `BTreeSet` is not used for exclusion checking.

---

## Code Reference

**Chain walk** -- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:733-759`:
```rust
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
                return None;
            };
            excluded.extend(snapshot.tx_ids.iter().copied());  // Copies each TxId
            current = snapshot.parent;
        }

        Some(excluded)
    }
```

**Similar chain walk in merged_changes** -- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs:203-235`:
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
            chain.push(snapshot.changes.clone());  // Also clones changesets
            current = snapshot.parent;
        }
        // ...
    }
```

**The exclusion set is used in build** -- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/traits.rs:56-59`:
```rust
    fn build(&self, max_txs: usize, excluded: &BTreeSet<TxId>) -> Vec<Tx>;
```

---

## Impact

At 34 blocks/s with 10 validators and `MAX_PROPOSAL_LAG = 64`:

**Under current devnet load** (mostly empty blocks): The impact is negligible because `tx_ids` sets are small.

**Under production load** with N=100 transactions per block:
- 64 ancestor snapshots * 100 TxIds each = 6,400 TxIds (each TxId is 32 bytes = 200 KB of data)
- All 6,400 entries are inserted into a `BTreeSet` with O(log n) insertion = approximately 13 comparisons per insert
- Total: ~83,200 comparisons per proposal just to build the exclusion set
- Each comparison is a 32-byte hash comparison
- The `merged_changes` method performs a similar chain walk on the same data, compounding the cost

This chain walk happens on every proposal from the current leader, directly impacting block production latency.

---

## Root Cause

The exclusion set is rebuilt from scratch on every proposal by walking the ancestry chain. No incremental update mechanism exists. Additionally, `BTreeSet` was chosen (possibly for consistency with the `Mempool::build` API signature), but its sorted property is not needed for membership testing.

---

## Suggested Fix

**Option 1 (minimal change)**: Switch from `BTreeSet<TxId>` to `HashSet<TxId>` for O(1) membership testing:

```rust
// In collect_pending_tx_ids:
fn collect_pending_tx_ids(
    &self,
    snapshots: &InMemorySnapshotStore<OverlayState<QmdbState>>,
    from: ConsensusDigest,
) -> Option<HashSet<kora_consensus::TxId>> {
    let mut excluded = HashSet::new();
    // ... same walk logic ...
}
```

This also requires changing the `Mempool::build` trait to accept `HashSet` instead of `BTreeSet`:
```rust
fn build(&self, max_txs: usize, excluded: &HashSet<TxId>) -> Vec<Tx>;
```

**Option 2 (incremental approach)**: Maintain a running cumulative `HashSet<TxId>` in the `InMemorySnapshotStore` that is updated incrementally when snapshots are inserted or evicted. This replaces the O(depth * N) chain walk with an O(1) lookup of a pre-built set:

```rust
// In InMemorySnapshotStore, maintain:
cumulative_excluded: Arc<RwLock<HashSet<TxId>>>,

// When inserting a snapshot:
fn insert(&self, digest: Digest, snapshot: Snapshot<S>) {
    for tx_id in &snapshot.tx_ids {
        self.cumulative_excluded.write().insert(*tx_id);
    }
    self.snapshots.write().insert(digest, snapshot);
}

// When evicting a persisted snapshot:
fn evict(&self, digest: Digest) {
    if let Some(snapshot) = self.snapshots.write().remove(&digest) {
        for tx_id in &snapshot.tx_ids {
            self.cumulative_excluded.write().remove(tx_id);
        }
    }
}
```

Option 1 is a simpler change with moderate improvement. Option 2 eliminates the chain walk entirely for a larger improvement under load.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- `collect_pending_tx_ids()` method
- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/traits.rs` -- `Mempool::build` signature (if changing to `HashSet`)
- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs` -- `merged_changes()` for Option 2
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` -- `LedgerMempool::build` implementation

---

## Related Issues

- `068-perf-snapshot-cloning-deep-copies-changeset.md` -- snapshot `get()` clones perform deep copies, compounding the cost of chain walks
- `015-ledger-single-mutex-bottleneck.md` -- the snapshot store is accessed through the central mutex
- `064-perf-state-root-reacquires-mutex.md` -- another redundant operation in the proposal path
