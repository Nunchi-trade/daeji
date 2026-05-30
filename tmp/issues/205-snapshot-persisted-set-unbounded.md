# Persisted-Set BTreeSet Grows Without Bound -- ~86 MB/Day Memory Leak

**Category**: bug -- consensus
**Severity**: high

## Summary

The `InMemorySnapshotStore` maintains a `persisted: Arc<RwLock<BTreeSet<Digest>>>` set that tracks which block digests have been committed to the persistent store (QMDB). This set is intentionally never pruned -- per the design comment, persisted markers are kept even after snapshot data is evicted so that ancestor chain-walking algorithms can terminate correctly at persisted boundaries. Every finalized block adds a 32-byte `Digest` to this set and it is never removed, creating an unbounded memory leak that accumulates approximately 86 MB per day at 34 blocks/second.

## Problem

The `InMemorySnapshotStore` has two distinct data structures for persisted snapshots:

1. `snapshots: BTreeMap<Digest, Snapshot<S>>` -- The actual snapshot data (state overlay, change set, tx IDs). This is bounded: `evict_persisted()` removes entries when the count exceeds `max_persisted_retained` (default: 256).

2. `persisted: BTreeSet<Digest>` -- A marker set indicating which digests have been committed to QMDB. This is **unbounded**: entries are added in `mark_persisted()` but never removed.

The design comment at lines 131-135 explains why:

> The `persisted` marker is intentionally **kept** for evicted digests so that ancestor chain-walking (`merged_changes`, `changes_for_persist`, `collect_pending_tx_ids`) still terminates correctly at persisted boundaries.

At 34 blocks/second, the BTreeSet grows by approximately 34 * 32 bytes/second = 1,088 bytes/second. Over time:
- ~86 MB/day
- ~2.5 GB/month
- ~30 GB/year

The BTreeSet overhead (node pointers, balance metadata) roughly doubles the per-entry cost, so real memory usage is approximately 2x the raw digest storage.

## Code Reference

File: `crates/node/consensus/src/components/snapshot.rs`

**The persisted set (line 35):**
```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,
    persisted: Arc<RwLock<BTreeSet<Digest>>>,  // <-- unbounded, never pruned
    persisting: Arc<RwLock<BTreeSet<Digest>>>,
    persisted_order: Arc<RwLock<VecDeque<Digest>>>,
    max_persisted_retained: usize,
}
```

**mark_persisted() adds entries (lines 193-201):**
```rust
fn mark_persisted(&self, digests: &[Digest]) {
    let mut persisted = self.persisted.write();
    let mut order = self.persisted_order.write();
    for digest in digests {
        if persisted.insert(*digest) {
            order.push_back(*digest);
        }
    }
}
```

**evict_persisted() removes snapshot data but NOT the persisted marker (lines 138-171):**
```rust
pub fn evict_persisted(&self) -> usize {
    // ...
    while order.len() > self.max_persisted_retained {
        let Some(oldest) = order.pop_front() else { break; };
        if persisted.contains(&oldest) && snapshots.remove(&oldest).is_some() {
            // ^^^ removes from `snapshots`, but `persisted` entry is kept
            evicted += 1;
        }
    }
    // ...
}
```

**merged_changes() uses persisted for chain-walk termination (lines 203-235):**
```rust
fn merged_changes(&self, parent: Digest, new_changes: ChangeSet) -> Result<ChangeSet, ConsensusError> {
    let persisted = self.persisted.read();
    // Walk back through ancestor snapshots
    while let Some(digest) = current {
        if persisted.contains(&digest) {
            break;  // <-- this is why the set is kept
        }
        // ...
    }
    // ...
}
```

## Impact

Slow memory leak that will eventually contribute to node OOM on long-running deployments. After a month of continuous operation, the persisted set alone consumes approximately 2.5 GB of memory. Combined with other known memory leaks in the same codebase (seed tracker, block fees HashMap, persisted_order VecDeque), the aggregate memory pressure can force an OOM restart.

On the current devnet deployment (64 GB RAM), this specific leak alone would not cause OOM for over a year, but combined with other leaks and normal memory usage, it could contribute to OOM within weeks to months.

## Root Cause

The persisted marker set was designed as an append-only structure to support ancestor chain-walking termination. The chain-walking algorithms (`merged_changes`, `changes_for_persist`) need to know when they have reached a persisted snapshot so they can stop walking. Without the marker, they would walk back to genesis or fail with `SnapshotNotFound`.

The design choice to keep all markers indefinitely was the simplest correct approach, but it creates an unbounded memory growth pattern.

## Suggested Fix

Maintain a high-water mark of the oldest retained snapshot's height. Once all snapshots below that height have been evicted, the chain walk can terminate on height comparison rather than set membership, allowing old persisted markers to be pruned.

**Option A**: Prune persisted markers alongside snapshot eviction. When `evict_persisted()` removes a snapshot, also remove its persisted marker. This requires adjusting the chain-walk algorithms to terminate when a snapshot is not found (rather than when it is marked persisted), since evicted snapshots would no longer have markers.

**Option B**: Track the minimum persisted height. After eviction, any digest whose height is below the oldest retained snapshot can safely have its persisted marker removed, because the chain walk will never reach that digest (it would hit a retained persisted snapshot first).

```rust
pub fn evict_persisted(&self) -> usize {
    let mut persisted = self.persisted.write();  // now mutable
    let mut snapshots = self.snapshots.write();
    let mut order = self.persisted_order.write();

    let mut evicted = 0;
    while order.len() > self.max_persisted_retained {
        let Some(oldest) = order.pop_front() else { break; };
        if persisted.contains(&oldest) && snapshots.remove(&oldest).is_some() {
            persisted.remove(&oldest);  // <-- also remove the marker
            evicted += 1;
        }
    }
    evicted
}
```

If Option A is used, the chain-walk termination in `merged_changes()` and `changes_for_persist()` must be updated to handle the case where neither the snapshot nor a persisted marker exists (interpret as "already persisted and evicted").

## Files to Modify

- `crates/node/consensus/src/components/snapshot.rs` -- Modify `evict_persisted()` to also remove persisted markers, and update `merged_changes()` / `changes_for_persist()` termination logic

## Related Issues

- `207-persisted-order-vecdeque-unbounded.md` -- Companion issue in the same component (VecDeque capacity never shrinks)
- `016-seed-tracker-unbounded.md` -- Same unbounded growth pattern in the seed tracker
- `010-block-fees-hashmap-unbounded.md` -- Same unbounded growth pattern in the block fees cache

## Labels

bug, reliability, consensus, performance
