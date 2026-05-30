# 005: Memory Exhaustion -- 8 of 10 Nodes at 100% of 4 GB Limit

**Category:** bug / deployment
**Severity:** critical
**Labels:** bug, reliability, performance, docker, shutdown, recovery, metrics

---

## Summary

On the live 10-node devnet at 65.21.232.29, 8 of 10 validator containers are at or within 5 MB of their 4 GB Docker memory limit. Node 9 has experienced 14 OOM-triggered restarts. Multiple unbounded in-memory data structures grow without limit, and the Docker memory allocation is insufficient for the node's baseline footprint plus this growth. The `restart: unless-stopped` Docker policy masks the problem by silently restarting containers, but each restart loses all runtime state and requires a full catch-up from genesis.

## Problem

Multiple unbounded in-memory data structures compound with insufficient Docker resource allocation to cause systematic memory exhaustion across the cluster.

**Contributing factors:**

1. **`block_fees` HashMap** (`crates/node/runner/src/app.rs:119`): Stores `(gas_used, base_fee)` per block, inserted via `record_block_fees()` at line 230-232, but **never pruned**. The comment on lines 114-118 claims "the map is bounded by the number of unfinalized blocks" but this is incorrect -- no code removes entries on finalization. At 33 blocks/s with ~80 bytes per entry (32-byte digest key + 16-byte value tuple + HashMap overhead), this grows ~5.5 MB/hour or ~132 MB/day.

2. **`InMemorySeedTracker` BTreeMap** (`crates/node/consensus/src/components/seed.rs:12-14`): A `BTreeMap<Digest, B256>` that stores consensus seed values per block digest. Insertions happen via `SeedTracker::insert()` at line 43-45 but entries are never removed. At 33 blocks/s with ~64 bytes per entry, this grows ~200 MB/day.

3. **Snapshot store**: The `InMemorySnapshotStore` retains up to 256 snapshots, each containing a full `ChangeSet` with all account, storage, and code modifications for that block. Under heavy transaction load, each changeset can be several megabytes.

4. **Tokio thread oversubscription**: `docker/scripts/entrypoint.sh:22` sets `TOKIO_WORKER_THREADS=8` by default, but each container is limited to 1.2 CPU cores. The 8 idle thread stacks waste ~64 MB of memory (8 MB per stack).

5. **Recovery memory spike**: `recover_finalized_state()` at `crates/node/runner/src/runner.rs:288-334` loads all archived blocks into a `BTreeMap<u64, Block>` (line 319: `recovered_blocks.insert(height, block)`) during restart, causing a memory spike at the worst possible time.

## Code Reference

**Unbounded `block_fees` HashMap -- `crates/node/runner/src/app.rs:114-119`:**

```rust
    /// Per-block `(gas_used, base_fee_per_gas)` cache, keyed by consensus
    /// digest.  Populated when a block is built or verified so that the
    /// *next* block can compute its EIP-1559 base fee from the parent's
    /// gas usage.  Entries are small (32 + 16 bytes) and the map is bounded
    /// by the number of unfinalized blocks.  // <-- INCORRECT: never pruned
    block_fees: Arc<RwLock<HashMap<ConsensusDigest, (u64, u64)>>>,
```

**Never-pruned insert -- `crates/node/runner/src/app.rs:230-232`:**

```rust
fn record_block_fees(&self, digest: ConsensusDigest, gas_used: u64, base_fee: u64) {
    self.block_fees.write().insert(digest, (gas_used, base_fee));
}
```

**Unbounded `InMemorySeedTracker` -- `crates/node/consensus/src/components/seed.rs:12-14`:**

```rust
pub struct InMemorySeedTracker {
    inner: Arc<RwLock<BTreeMap<Digest, B256>>>,
}
```

**Insert without bound -- `crates/node/consensus/src/components/seed.rs:43-45`:**

```rust
fn insert(&self, digest: Digest, seed: B256) {
    self.inner.write().insert(digest, seed);
}
```

**Recovery loads all archived blocks into memory -- `crates/node/runner/src/runner.rs:318-334`:**

```rust
let mut recovered_blocks = BTreeMap::new();
for (start, end) in block_ranges {
    for height in start..=end {
        let Some(block) = finalized_blocks
            .get(ArchiveId::Index(height))
            .await
            .with_context(|| format!("load finalized block at height {height}"))?
        else {
            continue;
        };

        index_recovered_block(block_index, &block, provider);
        recovered_blocks.insert(height, block);
        recovered += 1;
    }
}
```

**Tokio thread oversubscription -- `docker/scripts/entrypoint.sh:22-23`:**

```bash
export TOKIO_WORKER_THREADS="${TOKIO_WORKER_THREADS:-8}"
export RAYON_NUM_THREADS="${RAYON_NUM_THREADS:-2}"
```

## Impact

- **OOM kills at any time**: 8/10 nodes at the memory ceiling means any transient allocation spike triggers a kill by the Docker cgroup OOM killer.
- **Restart cascades**: Each OOM restart requires catching up from the last persisted checkpoint (every 256 blocks). During catch-up, `recover_finalized_state()` loads all archived blocks into a `BTreeMap`, consuming even more memory and creating a potential crash-restart loop. Node 9's 14 restarts demonstrate this pattern.
- **Quorum risk**: Loss of 2+ nodes puts the network near the quorum boundary (7 of 10 needed for consensus), risking liveness for the entire chain.
- **Silent degradation**: The `restart: unless-stopped` Docker policy hides the problem from operators. Without monitoring, nodes may be in a perpetual restart cycle.

## Root Cause

Multiple factors compound:
1. Several in-memory caches grow without bound (`block_fees`, `InMemorySeedTracker`).
2. The Docker memory limit (4 GB) is too low for the baseline memory footprint plus unbounded growth.
3. `TOKIO_WORKER_THREADS=8` creates 8 threads on a 1.2-CPU cgroup, wasting memory on idle thread stacks.
4. `recover_finalized_state()` loads all archived blocks into a `BTreeMap` during restart, spiking memory at the worst possible time.

## Suggested Fix

**Immediate (operational):**
1. Increase Docker memory limit to 6-8 GB per node in the compose file.
2. Set `TOKIO_WORKER_THREADS=2` to match the 1.2-CPU cgroup allocation.
3. Set `RAYON_NUM_THREADS=1` to match the actual strategy parameter.

**Short-term (code fixes):**
1. Fix `block_fees` HashMap leak -- prune entries on finalization (see issue 010).
2. Fix `InMemorySeedTracker` leak -- replace `BTreeMap` with a bounded LRU cache or prune entries older than the finalization horizon.
3. Limit `recover_finalized_state()` to streaming archived blocks or only loading the tail (last N blocks) rather than the entire archive.
4. Add a `kora_process_memory_bytes` gauge metric for monitoring.

**Medium-term:**
1. Profile memory usage with `jemalloc` stats to identify the dominant allocators.
2. Add memory budgets to the snapshot store and overlay caches.

## Files to Modify

- `docker/compose/devnet.yaml` -- Increase memory limits per node
- `docker/scripts/entrypoint.sh:22-23` -- Reduce `TOKIO_WORKER_THREADS` default
- `crates/node/runner/src/app.rs:119, 230-232` -- Add pruning to `block_fees` (see issue 010)
- `crates/node/consensus/src/components/seed.rs:12-46` -- Add bounds to `InMemorySeedTracker`
- `crates/node/runner/src/runner.rs:288-358` -- Stream or limit archived blocks during recovery

## Related Issues

- [010 -- block_fees HashMap Grows Without Bound](./010-block-fees-hashmap-unbounded.md) -- Detailed analysis of one of the contributing memory leaks
- [001 -- Non-Atomic Cross-Partition QMDB Writes](./001-qmdb-non-atomic-cross-partition-writes.md) -- OOM kills trigger the crash window for partition inconsistency
