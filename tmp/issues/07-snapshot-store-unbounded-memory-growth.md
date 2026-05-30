# Storage: InMemorySnapshotStore grows without bound, causing OOM on long-running nodes

**Severity:** Critical (P0)
**Component:** `crates/node/consensus/src/components/snapshot.rs`, `crates/node/consensus/src/components/seed.rs`
**Affects:** All long-running Kora validator nodes

---

## Summary

Kora is a blockchain validator node built on the [Commonware](https://github.com/commonwarexyz/monorepo) consensus framework. It uses a Simplex BFT consensus engine to finalize blocks, executes EVM transactions via REVM, and persists state to QMDB (a custom key-value state database). The node keeps in-memory snapshots of execution state at each block height so that speculative forks can be replayed and state roots can be verified before finalization.

The `InMemorySnapshotStore` in `crates/node/consensus/src/components/snapshot.rs` never removes snapshots after they are persisted. Every block that is proposed, verified, or finalized inserts a new entry into an unbounded `BTreeMap<Digest, Snapshot<S>>`, and no code path ever calls `remove()`, `retain()`, or any eviction method on this map. The same problem exists for the `persisted` and `persisting` `BTreeSet<Digest>` fields, as well as the entirely separate `InMemorySeedTracker` in `crates/node/consensus/src/components/seed.rs`. Memory grows linearly with chain height, and a production node will inevitably OOM.

---

## Background: What Snapshots Are

In Kora's architecture, a **snapshot** is a point-in-time capture of execution state at a particular block. Snapshots serve two purposes:

1. **Speculative execution:** When a validator proposes or verifies a block, it needs the parent block's state to execute transactions against. Snapshots provide this without requiring a full QMDB read for every block.
2. **Fork replay:** Because Simplex consensus can have multiple candidate blocks at the same height (before finalization), the node may need to replay execution along different fork branches. Snapshots let it walk back to a common ancestor and replay forward.

Each snapshot is defined in `crates/node/consensus/src/traits.rs` (lines 18-44):

```rust
/// A snapshot of execution state at a specific block.
#[derive(Clone, Debug)]
pub struct Snapshot<S> {
    /// Parent block digest.
    pub parent: Option<Digest>,
    /// State database at this point.
    pub state: S,
    /// Computed state root.
    pub state_root: StateRoot,
    /// Pending state changes not yet persisted.
    pub changes: ChangeSet,
    /// Transaction IDs included in this snapshot's block.
    pub tx_ids: BTreeSet<TxId>,
}
```

In production, `S` is `OverlayState<QmdbState>` (defined in `crates/storage/overlay/src/overlay.rs`), which holds an `Arc<ChangeSet>` overlay on top of a cloneable QMDB handle. The `ChangeSet` (defined in `crates/storage/qmdb/src/changes.rs`) contains a `BTreeMap<Address, AccountUpdate>` where each `AccountUpdate` includes nonce, balance, code_hash, optional code bytes, and a `BTreeMap<U256, U256>` of storage slot changes.

The `Digest` type is `commonware_cryptography::sha256::Digest`, which is a 32-byte hash (aliased as `ConsensusDigest` in `crates/node/domain/src/aliases.rs`).

---

## The Unbounded Data Structures

### InMemorySnapshotStore (`crates/node/consensus/src/components/snapshot.rs`)

```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,   // NEVER pruned
    persisted: Arc<RwLock<BTreeSet<Digest>>>,                // NEVER pruned
    persisting: Arc<RwLock<BTreeSet<Digest>>>,               // Cleared per-chain, but persisted is not
}
```

The `SnapshotStore` trait (lines 77-103 of `crates/node/consensus/src/traits.rs`) defines `get()`, `insert()`, `is_persisted()`, `mark_persisted()`, `merged_changes()`, and `changes_for_persist()`. There is **no** `remove()`, `evict()`, `retain()`, or `prune()` method anywhere in the trait or its implementation. The `InMemorySnapshotStore` implementation (lines 78-166 of `snapshot.rs`) likewise has no removal path -- `can_persist_chain()`, `mark_persisting_chain()`, and `clear_persisting_chain()` manage the `persisting` guard set, but nothing touches the `snapshots` map or `persisted` set in a removal capacity.

Entries are inserted via:
- `LedgerView::insert_snapshot()` (`crates/node/ledger/src/lib.rs`, lines 218-230) -- called during block proposal/verification
- `LedgerView::restore_persisted_snapshot()` (lines 239-252) -- called during recovery
- `LedgerView::cache_snapshot()` (lines 233-236) -- called by FinalizedReporter
- `FinalizedReporter::handle_finalized_update()` (lines 171-186 of `crates/node/reporters/src/lib.rs`) -- inserts a snapshot when re-executing a finalized block for which no cached snapshot exists

Entries are never removed. The `snapshots` BTreeMap grows by one entry per block forever.

### InMemorySeedTracker (`crates/node/consensus/src/components/seed.rs`)

```rust
pub struct InMemorySeedTracker {
    inner: Arc<RwLock<BTreeMap<Digest, B256>>>,  // NEVER pruned
}
```

The `SeedTracker` trait defines only `get()` and `insert()`. No removal method exists. A new seed is inserted for every notarization and finalization event via `seed_report_inner()` in `crates/node/reporters/src/lib.rs` (lines 40-63), called from `SeedReporter::report()` (lines 91-103).

---

## Partial Compaction in persist_snapshot() Is Insufficient

When a block is finalized, `FinalizedReporter` (in `crates/node/reporters/src/lib.rs`, lines 204-207) calls `persist_snapshot(digest)`. This eventually reaches `LedgerView::persist_snapshot()` in `crates/node/ledger/src/lib.rs` (lines 293-333).

The persist logic does compact the **tip** snapshot after a successful QMDB commit (lines 312-327):

```rust
if let Some(tip) = chain.last()
    && let Some(snapshot) = inner.snapshots.get(tip)
{
    let compact_state =
        OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
    inner.snapshots.insert(
        *tip,
        Snapshot::new(
            snapshot.parent,
            compact_state,
            snapshot.state_root,
            QmdbChangeSet::default(),
            snapshot.tx_ids,
        ),
    );
}
inner.snapshots.mark_persisted(&chain);
```

This replaces the tip snapshot's `ChangeSet` and `OverlayState` with empty versions (pointing to the fresh QMDB state). However:

1. **Only the tip is compacted.** All intermediate ancestor snapshots in the `chain` vector remain in the map with their full `ChangeSet` and `OverlayState` data intact. They are marked as persisted but never removed.
2. **The `persisted` BTreeSet itself grows without bound.** After marking, the digest remains in `persisted` forever.
3. **Old snapshots from prior heights are never removed.** Even if they will never be accessed again (because the chain has moved far past them), they remain in the map.

---

## Memory Growth Model

### Per-Snapshot Memory Overhead

Each `Snapshot<OverlayState<QmdbState>>` contains:

| Component | Empty block | Light load (2-5 transfers) | Heavy load (contracts) |
|---|---|---|---|
| `Digest` key (BTreeMap overhead) | ~96 bytes | ~96 bytes | ~96 bytes |
| `parent: Option<Digest>` | 33 bytes | 33 bytes | 33 bytes |
| `state_root: StateRoot` | 32 bytes | 32 bytes | 32 bytes |
| `state: OverlayState` (Arc + ChangeSet) | ~80 bytes | ~500-2,000 bytes | ~10,000-50,000 bytes |
| `changes: ChangeSet` | ~48 bytes | ~500-2,000 bytes | ~10,000-50,000 bytes |
| `tx_ids: BTreeSet<TxId>` | ~24 bytes | ~200-500 bytes | ~200-500 bytes |
| **Total per snapshot** | **~400 bytes** | **~3,000-5,000 bytes** | **~50,000-100,000 bytes** |

### Per-Seed Entry

Each `InMemorySeedTracker` entry is a `(Digest, B256)` pair in a BTreeMap:
- Key: 32 bytes (Digest) + BTreeMap node overhead (~32 bytes)
- Value: 32 bytes (B256)
- **Total per entry: ~64 bytes** (conservative estimate with BTreeMap node overhead)

### Per-Persisted Entry

Each entry in the `persisted` BTreeSet:
- Key: 32 bytes (Digest) + BTreeMap node overhead
- **Total per entry: ~32 bytes** (conservative estimate)

### Growth Rates at 108 blocks/sec

Kora achieves approximately 59-148 blocks per second depending on configuration and nullification rate. The typical observed rate on the 4-validator devnet is ~108 blocks/sec with a 26% nullification rate. Empty blocks are produced rapidly during idle periods; the effective rate depends on nullification behavior, but the architecture must handle sustained throughput.

Using the typical observed rate of 108 blocks/sec (one block per ~9.3ms view):

**Snapshot store (`snapshots` BTreeMap):**

| Scenario | Per-snapshot | Blocks/sec | MB/hour | GB/day |
|---|---|---|---|---|
| Empty blocks (idle chain) | ~400 bytes | 108 | ~155 MB | ~3.7 GB |
| Light transfer load | ~4,000 bytes | 108 | ~1,555 MB | ~36 GB |
| Heavy contract load | ~75,000 bytes | 108 | ~29,160 MB | ~700 GB |

**Seed tracker (`InMemorySeedTracker`):**
- 64 bytes/entry * 108 entries/sec = ~6,912 bytes/sec = ~24.3 MB/hour = ~583 MB/day

**Persisted set (`persisted` BTreeSet):**
- 32 bytes/entry * 108 entries/sec = ~3,456 bytes/sec = ~12.1 MB/hour = ~291 MB/day

### Time to OOM

Assuming a production node with 32 GB RAM and ~24 GB available for application use:

| Scenario | Combined growth rate | Growth/hour | Time to OOM |
|---|---|---|---|
| Empty blocks (idle) | ~4.6 GB/day | ~191 MB/hr | ~5-6 days |
| Light transfer load | ~38 GB/day | ~1,591 MB/hr | ~15 hours |
| Heavy contract load | ~701 GB/day | ~29,196 MB/hr | ~49 minutes |

Note: "Combined growth rate" includes snapshot store, seed tracker (~583 MB/day), and persisted set (~291 MB/day). These rates assume all snapshots retain their full data. In practice, the tip snapshot of a persisted chain is compacted (ChangeSet cleared), but all intermediate ancestors and all non-tip entries retain their original sizes.

---

## Impact

- **Production deployment is blocked.** Any Kora node running for more than a few hours under load will OOM and crash.
- **Idle chains are also affected.** Even with zero transactions, the node will OOM within a week.
- **Recovery makes it worse.** On restart, `recover_finalized_state()` in `crates/node/runner/src/runner.rs` (lines 170-228) iterates all archived blocks and calls `restore_persisted_snapshot()` for the last one, but seed entries are re-inserted for every finalization in the archive. If the node previously ran for days, recovery re-populates the seed tracker with all historical seeds.
- **No monitoring exists for this.** The Commonware runtime exposes a `runtime_process_rss` metric via the `/metrics` endpoint (configured in `runner.rs` lines 444-477), but there are no alerts or dashboards tracking snapshot store cardinality or seed tracker size.
- **No workaround exists.** The only way to reclaim memory is to restart the node. On the Docker devnet, the runtime directory is on tmpfs (`KORA_RUNTIME_DIR=/runtime`, configured in `docker/compose/devnet.yaml`), so a restart loses all Commonware archive data (finalized blocks, finalization certificates, consensus journals). The node starts from genesis with no recovery possible. Even on production deployments with persistent volumes, a restart triggers `recover_finalized_state()` which re-inserts all historical seeds from the archive (see Issue #08), partially re-polluting memory.

---

## Prometheus Monitoring

To observe this issue before it causes OOM, monitor:

```promql
# RSS growth rate - should be near-zero for a healthy node
rate(runtime_process_rss[5m])

# If custom metrics are added:
kora_snapshot_store_size
kora_seed_tracker_size
kora_persisted_set_size
```

---

## Proposed Fixes

### Fix 1: Snapshot Eviction Policy (Minimum Viable)

Add an `evict_before()` method to `SnapshotStore` that removes all persisted snapshots older than N blocks from the current tip:

```rust
// In SnapshotStore trait (crates/node/consensus/src/traits.rs):
fn evict_persisted_except(&self, keep: &[Digest]);

// In InMemorySnapshotStore:
fn evict_persisted_except(&self, keep: &[Digest]) {
    let keep_set: BTreeSet<Digest> = keep.iter().copied().collect();
    let mut snapshots = self.snapshots.write();
    let mut persisted = self.persisted.write();

    let to_remove: Vec<Digest> = persisted
        .iter()
        .filter(|d| !keep_set.contains(d))
        .copied()
        .collect();

    for digest in &to_remove {
        snapshots.remove(digest);
        persisted.remove(digest);
    }
}
```

Call this from `persist_snapshot()` after marking the new chain as persisted, keeping only the most recent persisted digest.

### Fix 2: Full Chain Compaction

Currently `persist_snapshot()` only compacts the tip snapshot (lines 312-327 of `crates/node/ledger/src/lib.rs`). It should compact ALL entries in the persisted chain, not just the tip. However, this alone does not solve the growth problem -- it only reduces per-entry size. Entries still accumulate.

### Fix 3: Bounded Store with Ring Buffer or LRU (Recommended)

Replace the `BTreeMap<Digest, Snapshot<S>>` with a bounded data structure:

```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,
    persisted: Arc<RwLock<BTreeSet<Digest>>>,
    persisting: Arc<RwLock<BTreeSet<Digest>>>,
    max_snapshots: usize,  // e.g., 256
}
```

When inserting a new snapshot, if the store exceeds `max_snapshots`, remove the oldest persisted entries. A recommended bound is N=256, which gives ~25 seconds of history at 108 blocks/sec -- more than enough for fork replay since the `activity_timeout` is 256 views and the `skip_timeout` is 32 views.

### Fix 4: Bound InMemorySeedTracker

Apply the same bounded approach to `InMemorySeedTracker`. Seeds are only needed for building the `prevrandao` field of the next block, which references the parent digest's seed. A bound of N=256 entries is sufficient.

```rust
pub struct InMemorySeedTracker {
    inner: Arc<RwLock<BTreeMap<Digest, B256>>>,
    max_entries: usize,
}
```

Since `BTreeMap` does not have efficient oldest-entry eviction, consider switching to an `IndexMap` or maintaining a separate `VecDeque<Digest>` as an insertion-order eviction queue.

---

## Priority

**P0 -- blocks production deployment.** This bug guarantees that every Kora node will eventually crash. The time to crash depends on load but is measured in hours under realistic conditions. No workaround exists short of periodically restarting nodes (which loses all in-memory state and forces expensive archive replay on recovery).

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/consensus/src/traits.rs` | Add `evict_persisted_except()` (or similar removal method) to `SnapshotStore` trait; optionally add `evict()` to `SeedTracker` trait |
| `crates/node/consensus/src/components/snapshot.rs` | Implement eviction in `InMemorySnapshotStore`; add `max_snapshots` bound |
| `crates/node/consensus/src/components/seed.rs` | Implement eviction in `InMemorySeedTracker`; add `max_entries` bound |
| `crates/node/ledger/src/lib.rs` | Call eviction after `persist_snapshot()` succeeds (around line 328); compact ALL chain entries, not just the tip (lines 312-327) |
| `crates/node/reporters/src/lib.rs` | (No changes required, but verify `handle_finalized_update` works correctly after eviction) |

## Verification Steps

After implementing the fix, verify correctness with these manual checks:

1. **Confirm no removal methods exist today** (pre-fix baseline):
   ```bash
   grep -n "remove\|evict\|retain\|prune\|pop\|clear" crates/node/consensus/src/components/snapshot.rs
   # Should show only `clear_persisting_chain` (which only clears the `persisting` set, not `snapshots` or `persisted`)
   ```

2. **After implementing the fix**, run the existing tests to confirm no regressions:
   ```bash
   cargo test -p kora-consensus
   cargo test -p kora-ledger
   ```

3. **Verify RSS stability under load**: Start a single-node devnet, run the load generator for 5 minutes, and query `runtime_process_rss` via the metrics endpoint at 1-minute intervals. RSS should plateau after the snapshot window fills, not grow linearly.

---

## Testing Plan

1. **Unit test: Eviction correctness.** Insert N+1 snapshots into a bounded store with max_snapshots=N. Verify that the oldest persisted snapshot is evicted and that unpersisted snapshots are never evicted.

2. **Unit test: Seed tracker bound.** Insert N+1 seeds into a bounded tracker. Verify the oldest is evicted and the newest N are retained.

3. **Integration test: Memory stability under sustained load.** Run a single-node devnet for 10,000 blocks using the load generator (`bin/loadgen`). Sample RSS at block 100, 1000, 5000, and 10000. Verify that RSS growth is sublinear (bounded by the snapshot window, not by total chain height).

4. **Integration test: Recovery with bounded store.** Run a node for 1000 blocks, kill it, restart it, and verify that `recover_finalized_state()` completes successfully and the node can produce new blocks. The seed tracker should only contain seeds from the recovery window, not all historical seeds.

5. **Regression test: merged_changes still works.** After eviction, calling `merged_changes()` with a parent that has been evicted should return `ConsensusError::SnapshotNotFound`. Verify this is handled gracefully by the consensus layer (it should trigger a re-fetch from the archive, not a panic).

6. **Stress test: Monitor RSS via Prometheus.** Deploy with the `runtime_process_rss` metric and verify that RSS stabilizes after the snapshot window is full, rather than growing linearly.
