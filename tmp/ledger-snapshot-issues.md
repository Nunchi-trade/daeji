# Ledger and Snapshot System: Architecture and Issues

## What is Kora?

Kora is a minimal, high-performance EVM execution client built in Rust. It combines:
- **Commonware Simplex** for BLS12-381 threshold consensus (a DAG-based BFT protocol)
- **REVM** for EVM-compatible transaction execution
- **QMDB** (Quantized Merkle Database) from Commonware for authenticated state storage

Kora runs as a 4-validator network using BLS threshold signatures. Validators propose blocks, verify them through REVM execution, reach consensus via simplex, and persist finalized state to QMDB. The observed throughput on devnet is approximately 59-148 blocks/sec depending on configuration and nullification rate (typically ~108 blocks/sec with 26% nullification).

---

## Ledger Architecture

### High-Level Flow

```
Transaction Submission
        |
        v
    Mempool (InMemoryMempool)
        |
        v
    Block Proposal (RevmApplication::build_block)
        |
        v
    EVM Execution (RevmExecutor) -> produces ChangeSet
        |
        v
    Snapshot Creation (OverlayState + ChangeSet cached per digest)
        |
        v
    Consensus (simplex notarize/finalize)
        |
        v
    Finalization (FinalizedReporter::handle_finalized_update)
        |
        v
    Persistence (persist_snapshot -> commit_changes to QMDB)
        |
        v
    Mempool Pruning
```

### QMDB: The Authenticated Key-Value Store

QMDB is an authenticated key-value store from Commonware that uses **journaled Merkle trees**. It provides:
- Efficient batch writes with atomic Merkle root updates
- SHA-256 based authentication (each partition has its own Merkle root)
- zstd compression for on-disk storage
- Journal-based persistence via `commonware-storage`

Data is stored in the directory specified by `KORA_RUNTIME_DIR` (defaults to `data_dir/runtime`).

### Three Partitions

| Partition | Key | Value | Size |
|-----------|-----|-------|------|
| **Accounts** | 20-byte Ethereum address | 80 bytes: nonce(8) + balance(32) + code_hash(32) + generation(8) | Fixed 80B per entry |
| **Storage** | 60 bytes: address(20) + generation(8) + slot(32) | 32-byte U256 value | Fixed 32B per entry |
| **Code** | 32-byte code hash (B256) | Variable-length bytecode (max 24,576 bytes) | Up to 24KB per contract |

The **generation** field in accounts is incremented on selfdestruct/recreate, which invalidates old storage slots without requiring deletion scans.

### State Root Computation

The state root is a keccak256 hash computed from the three partition roots:

```rust
// From crates/storage/qmdb/src/root.rs
fn compute(accounts_root: B256, storage_root: B256, code_root: B256) -> B256 {
    keccak256(b"_KORA_QMDB_ROOT" || accounts_root || storage_root || code_root)
}
```

However, during consensus (before finalization), Kora uses a **deterministic transition root** that does not require reading the actual Merkle tree:

```rust
fn transition(parent_root: B256, changes: &ChangeSet) -> B256 {
    if changes.is_empty() { return parent_root; }
    keccak256(b"_KORA_STATE_TRANSITION_ROOT" || parent_root || serialized_changes)
}
```

This allows validators to agree on state roots without persisting intermediate state. The transition root is deterministic given the same parent root and change set, enabling agreement before QMDB commits.

**Important distinction**: The `StateRoot::transition()` function produces consensus roots that differ from the actual Merkle roots produced by `commit_changes()`. The consensus root is used for block headers and agreement; the Merkle root reflects actual on-disk authenticated state.

---

## The Snapshot System

### What Snapshots Are

A snapshot (`Snapshot<S>`) is a point-in-time capture of execution state at a specific block. It contains:

```rust
pub struct Snapshot<S> {
    pub parent: Option<Digest>,       // Parent block digest
    pub state: S,                     // OverlayState<QmdbState> -- layered state view
    pub state_root: StateRoot,        // Computed consensus state root
    pub changes: ChangeSet,           // Pending state changes (account updates + storage)
    pub tx_ids: BTreeSet<TxId>,       // Transaction IDs included in this block
}
```

In practice, `S = OverlayState<QmdbState>`, which layers a `ChangeSet` (in-memory overlay) on top of the persisted QMDB state handle (read-through cache).

### InMemorySnapshotStore

The `InMemorySnapshotStore` holds all snapshots in a `BTreeMap<Digest, Snapshot<S>>` protected by `parking_lot::RwLock`:

```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,  // ALL snapshots
    persisted: Arc<RwLock<BTreeSet<Digest>>>,               // Which are committed to QMDB
    persisting: Arc<RwLock<BTreeSet<Digest>>>,              // In-flight persistence guard
}
```

Key operations:
- `insert(digest, snapshot)` -- stores a new snapshot (no eviction)
- `get(digest)` -- returns a clone of the snapshot
- `mark_persisted(digests)` -- marks digests as committed to QMDB
- `changes_for_persist(digest)` -- walks the parent chain collecting unpersisted `ChangeSet`s, merges them oldest-first
- `merged_changes(parent, new)` -- same walk but includes new changes (used for speculative root computation)

### When Snapshots Are Created

Snapshots are created at three points:

1. **Genesis** (line 136 of ledger/src/lib.rs): A genesis snapshot with empty changes and the base QMDB state.

2. **Block verification** (app.rs:220-232): When `verify_block()` succeeds, a snapshot is inserted with the merged overlay state and the block's change set.

3. **Finalization replay** (reporters/src/lib.rs:171-186): If a finalized block has no cached snapshot (e.g., after restart), the `FinalizedReporter` re-executes the block and inserts the snapshot.

4. **Recovery** (runner.rs:219): `restore_persisted_snapshot()` creates a minimal snapshot for recovered blocks.

### When Snapshots Are Cleaned Up

**After persistence** (ledger/src/lib.rs lines 312-327): When `persist_snapshot()` succeeds, the **tip** snapshot is replaced with a compacted version that has an empty `ChangeSet` and a fresh `OverlayState` pointing to the current QMDB state. This was added in commit `c8116de` ("fix ledger snapshot chain compaction").

```rust
// After successful commit to QMDB:
if let Some(tip) = chain.last() {
    let compact_state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
    inner.snapshots.insert(*tip, Snapshot::new(
        snapshot.parent,
        compact_state,
        snapshot.state_root,
        QmdbChangeSet::default(),  // Changes cleared -- they're in QMDB now
        snapshot.tx_ids,
    ));
}
inner.snapshots.mark_persisted(&chain);
```

**However**: The old snapshot entries for intermediate chain ancestors are **never removed** from the `BTreeMap`. They remain as entries with their original data. Only the tip gets compacted. The `persisted` set grows, and the `snapshots` map grows, without bound.

---

## Identified Issues

### Issue 1: No Snapshot Eviction -- Memory Grows Linearly with Chain Height

**Root Cause**: `InMemorySnapshotStore` has no `remove()`, `evict()`, or `retain()` method. Every snapshot ever inserted remains in the `BTreeMap` forever. The `persisted` set also grows unboundedly.

**Evidence**: Searching the entire `components/snapshot.rs` file reveals no removal operations. The only modification to existing entries is the compaction in `persist_snapshot()` which replaces the tip but does not remove ancestors.

**Impact**: Each snapshot entry persists in RAM. Even after compaction clears the `ChangeSet` of the tip, the intermediate blocks in the chain (which are marked persisted but not compacted) retain their full `ChangeSet`, `OverlayState`, and `tx_ids`.

### Issue 2: No Chain Compaction for Intermediate Ancestors

**Root Cause**: The compaction logic in `persist_snapshot()` only compacts the **last** element of the persisted chain (`chain.last()`). All intermediate ancestors in the chain retain their original `ChangeSet` and overlay data.

**Code** (ledger/src/lib.rs):
```rust
if let Some(tip) = chain.last()  // Only compacts the tip!
    && let Some(snapshot) = inner.snapshots.get(tip)
{
    // ... compact only this one snapshot
}
```

If a chain `[genesis -> A -> B -> C]` is persisted, only `C` gets compacted. `A` and `B` retain their full `ChangeSet` data indefinitely.

### Issue 3: InMemorySnapshotStore is Unbounded -- OOM on Long-Running Nodes

**Root Cause**: No maximum capacity, no LRU eviction, no periodic cleanup task.

**Also unbounded**: `InMemorySeedTracker` stores a `BTreeMap<Digest, B256>` (32 + 32 = 64 bytes per entry) without any cleanup. At 108 blocks/sec, this alone grows at ~6.75 KB/sec or ~580 MB/day.

**The `persisted` set**: Also a `BTreeSet<Digest>` that grows without bound (32 bytes per entry).

### Issue 4: State Access During Block Building -- Overlay Safety Under Concurrent Access

**Architecture**: The `OverlayState<QmdbState>` uses `Arc<ChangeSet>` for the overlay and a cloned `QmdbState` handle for the base. The `LedgerView` mutex (`futures::lock::Mutex`) protects the overall `LedgerState`, but snapshot access clones the snapshot and drops the lock:

```rust
pub async fn query_balance(&self, digest: ConsensusDigest, address: Address) -> Option<U256> {
    let snapshot = {
        let inner = self.inner.lock().await;
        inner.snapshots.get(&digest)  // Clones the snapshot
    }?;
    snapshot.state.balance(&address).await.ok()  // Reads from clone without lock
}
```

**Risk**: The `QmdbState` handle (which wraps `QmdbHandle`) contains interior mutability (`RwLock`-guarded store access). During persistence, `commit_changes()` acquires a write lock on the store (`self.handle.write().await`) and mutates the Merkle tree. Concurrent reads from cloned `QmdbState` handles may see partially-applied state if the read path does not properly coordinate with the write path.

**Mitigating factor**: The `QmdbHandle` uses `tokio::sync::RwLock`, so reads and writes are properly serialized at the handle level. The `storage_access()` guard in `commit_changes()` provides additional coordination. This appears safe, but the architecture makes it non-obvious and depends on the correctness of the underlying commonware-storage implementation.

### Issue 5: No Archive Mode -- Cannot Query Historical State

**Current behavior**: Once a snapshot's `ChangeSet` is committed to QMDB and compacted, the QMDB state reflects only the latest committed state. There is no mechanism to query state at an arbitrary historical block height.

**Impact**:
- RPC methods like `eth_getBalance` at a historical block number are not supported.
- No `eth_call` at historical blocks.
- The `query_balance(digest)` method works only for snapshots still in memory (non-evicted, non-compacted), which is a racing window.

### Issue 6: Recovery -- State Lost on Restart and Replay Requirements

**Current recovery mechanism** (runner.rs lines 170-228):

1. On startup, the runner reads the finalization archive to rebuild seeds.
2. It iterates through all archived finalized blocks and calls `restore_persisted_snapshot()` for the last one.
3. `restore_persisted_snapshot()` creates a minimal snapshot with empty `ChangeSet` pointing to the current QMDB state.

**Problem**: The recovery process relies on the finalization archive (`commonware-storage Archive`) being intact. It does NOT re-execute blocks or verify that QMDB state matches the archived block's state root. If QMDB was partially written (crash during `commit_changes()`), the state is silently inconsistent.

**Additional problem**: The `restore_persisted_digest` feature (commit `e17a728`) was implemented and then **reverted** (commit `6462dad`) because it was unsafe. The revert message indicates the approach of marking arbitrary digests as "persisted" without validating QMDB state was problematic.

**Replay cost**: Without snapshot caching, every finalized block delivered to the `FinalizedReporter` must be re-executed (the `!snapshot_exists` path in `handle_finalized_update`). This adds execution latency to the finalization pipeline.

### Issue 7: Digest/State Root Persistence -- Reverted Commit Indicates Fragility

**Background**: Commit `e17a728` ("add restore persisted digest functionality") attempted to:
- Add a `restore_persisted_digest` method to mark a finalized digest as persisted
- Use it during startup to recover the last finalized head from archives

This was **reverted** in `6462dad` because marking a digest as persisted without ensuring QMDB actually contains that state is dangerous. If the QMDB state lags behind (e.g., crash before persistence completed), the system would believe state is persisted when it is not, leading to:
- Lost state transitions
- Incorrect `changes_for_persist()` walks (skipping blocks that need re-application)
- Silent state divergence from other validators

The current approach (`restore_persisted_snapshot`) is safer but still does not validate consistency.

---

## Memory Growth Model

### Per-Snapshot Memory Estimate

Each `Snapshot<OverlayState<QmdbState>>` contains:

| Component | Typical Size | Notes |
|-----------|-------------|-------|
| `parent: Option<Digest>` | 33 bytes | 1 discriminant + 32 bytes |
| `state_root: StateRoot` | 32 bytes | B256 |
| `tx_ids: BTreeSet<TxId>` | ~48 + 32*N bytes | N = txs in block |
| `changes: ChangeSet` | Variable | See below |
| `state: OverlayState<QmdbState>` | ~200 bytes | Arc + handle clones |
| BTreeMap overhead | ~64 bytes | Node pointers |

**ChangeSet per account touched**:
- `AccountUpdate`: 1+1+8+32+32 + Option<Vec<u8>> + BTreeMap<U256,U256> = ~106 bytes base + storage
- Typical transfer touching 2 accounts (sender+receiver): ~250 bytes
- Contract call touching 5 accounts + 20 storage slots: ~2 KB

**Conservative estimate for empty blocks** (no transactions):
- ~400 bytes per snapshot (mostly overhead from Arc, BTreeMap nodes, empty collections)

**Estimate for blocks with 10 transfers**:
- ~3-5 KB per snapshot (2 accounts touched per transfer, each with nonce+balance update)

**Estimate for blocks with 100 contract interactions**:
- ~50-100 KB per snapshot

### Growth Rate Calculation

| Scenario | Blocks/sec | Bytes/snapshot | Growth/sec | Growth/hour | Growth/day |
|----------|-----------|----------------|-----------|-------------|-----------|
| Empty blocks (idle) | 108 | 400 B | 43 KB/s | 155 MB/hr | 3.7 GB/day |
| Light load (10 tx/block) | 108 | 4 KB | 432 KB/s | 1.5 GB/hr | 36 GB/day |
| Heavy load (100 tx/block) | 108 | 50 KB | 5.4 MB/s | 19.4 GB/hr | 466 GB/day |

**Note**: After the compaction fix (`c8116de`), the tip snapshot's `ChangeSet` is cleared after persistence. However, ALL snapshots remain in the map. The base overhead per snapshot entry (even with empty ChangeSet) is still ~400 bytes. Additionally, intermediate ancestors in chains are NOT compacted.

### Also Growing Without Bound

- `InMemorySeedTracker`: 64 bytes/entry * 108 entries/sec = 6.75 KB/s = 583 MB/day
- `persisted` BTreeSet: 32 bytes/entry * 108 entries/sec = 3.37 KB/s = 291 MB/day
- `persisting` BTreeSet: Transient, should stay small

**Total minimum growth rate (empty blocks)**: ~10 KB/s or ~864 MB/day just from map entry overhead.

---

## Prometheus Metric to Watch

The key metric for detecting memory growth is:

```
runtime_process_rss
```

This is the Resident Set Size reported by the Commonware runtime's metrics system. The metrics endpoint is exposed at the configured `metrics_addr` (typically port 9090) in OpenMetrics format.

Additional useful queries:
```promql
# Rate of RSS growth (bytes/sec)
rate(runtime_process_rss[5m])

# Hours until 8GB OOM (assuming 2GB baseline)
(8589934592 - runtime_process_rss) / (rate(runtime_process_rss[1h]) > 0) / 3600
```

---

## Impact on Long-Running Validators

| Load | Time to +4GB RSS growth | Time to OOM (8GB container) |
|------|-------------------------|---------------------------|
| Idle (empty blocks) | ~4.6 days | ~5-6 days |
| Light load | ~2.7 hours | ~4-5 hours |
| Heavy load | ~12 minutes | ~20-25 minutes |

On the current devnet configuration (4 validators, Docker containers), validators running under sustained load will OOM within hours. Even idle chains will exhibit monotonically increasing memory, reaching critical levels within a week.

**Observed behavior** (from `tmp/production-failure-timeline.md`): The chain stalls after sustained operation, consistent with memory pressure causing performance degradation before eventual OOM.

---

## Proposed Fixes

### Fix 1: Snapshot Eviction Policy

**Approach**: Add a bounded eviction strategy to `InMemorySnapshotStore`.

```rust
impl<S> InMemorySnapshotStore<S> {
    /// Remove all snapshots older than `keep_recent` blocks from the tip.
    pub fn evict_before(&self, tip: &Digest, keep_recent: usize) {
        let snapshots = self.snapshots.write();
        let persisted = self.persisted.read();
        // Walk from tip, keep `keep_recent` entries
        // Remove everything else that is persisted
    }
}
```

Call `evict_before()` after each successful `persist_snapshot()`. Only evict snapshots that are marked `persisted` (safe to remove because their state is in QMDB).

**Keep**: The last N persisted snapshots (N=2-3) for parent lookup during the next block's verification.

### Fix 2: Full Chain Compaction

**Approach**: Compact ALL entries in the persisted chain, not just the tip.

```rust
// In persist_snapshot(), after successful commit:
for digest in &chain {
    if let Some(snapshot) = inner.snapshots.get(digest) {
        let compact_state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
        inner.snapshots.insert(*digest, Snapshot::new(
            snapshot.parent,
            compact_state,
            snapshot.state_root,
            QmdbChangeSet::default(),
            snapshot.tx_ids.clone(),
        ));
    }
}
```

This ensures intermediate ancestors do not retain large `ChangeSet` allocations.

### Fix 3: Bounded Store with Automatic Eviction

**Approach**: Replace `BTreeMap` with a bounded structure.

Option A: Fixed-capacity ring buffer (keeps last N snapshots by insertion order).
Option B: LRU cache with configurable max entries (e.g., `lru::LruCache`).
Option C: Time-based TTL (evict snapshots older than T seconds if persisted).

Recommended: **Option A with N=256** (covers 2-3 seconds of blocks at 108 blocks/sec, enough for consensus pipeline depth).

Also bound `InMemorySeedTracker` with the same approach.

### Fix 4: Audit Concurrent State Access

**Approach**:
1. Document the concurrency contract for `QmdbHandle` reads during `commit_changes()`.
2. Add integration tests that perform concurrent reads and writes.
3. Consider adding a read-snapshot mechanism: before `commit_changes()`, clone the current read state so that in-flight queries see a consistent view.

### Fix 5: Historical State Queries (Archive Mode)

**Approach** (longer term):
1. Store per-block `ChangeSet` in the finalized block archive.
2. To query historical state, load the nearest snapshot and replay change sets forward.
3. Alternatively, maintain a separate "archive" QMDB instance that keeps all historical Merkle tree versions.

This is a feature enhancement, not a bug fix. Prioritize after stability.

### Fix 6: Crash-Consistent Recovery

**Approach**:
1. Persist the last-committed digest alongside the QMDB commit (as metadata in the journal).
2. On startup, compare the archived finalized head with the last-committed QMDB digest.
3. If they differ, replay the missing blocks from the archive through execution and re-commit.
4. Add a state root verification step: after recovery, compute the QMDB root and compare with the expected block state root.

```rust
// Pseudocode for safe recovery
let qmdb_root = qmdb.root().await?;
let archived_head_root = last_archived_block.state_root;
if qmdb_root != archived_head_root {
    // Replay from last matching point
    replay_from_archive(qmdb, finalized_blocks, qmdb_root).await?;
}
```

### Fix 7: Persist Digest Safely

**Approach**: Instead of the reverted `restore_persisted_digest` (which blindly trusted archive data), implement a verified restore:

1. Compute QMDB's actual root on startup.
2. Find the archived block whose `state_root` matches the QMDB root (may require walking backward from the tip).
3. Mark only that block (and its ancestors) as persisted.
4. Replay and persist any blocks between that point and the archive tip.

This ensures the `persisted` set accurately reflects QMDB's actual state.

---

## Priority Ordering

| Priority | Issue | Effort | Impact |
|----------|-------|--------|--------|
| P0 | #1 + #3: Snapshot eviction | Medium | Prevents OOM on all deployments |
| P0 | #2: Full chain compaction | Small | Reduces memory by 30-50% for multi-block chains |
| P1 | #6: Crash-consistent recovery | Medium | Prevents silent state corruption |
| P1 | #7: Safe digest persistence | Medium | Required for reliable restarts |
| P2 | #4: Concurrent access audit | Small | Confidence, not a known bug |
| P3 | #5: Archive mode | Large | Feature, not stability |

---

## Key Source Files

| File | Purpose |
|------|---------|
| `crates/node/ledger/src/lib.rs` | `LedgerView` and `LedgerService` -- main ledger orchestration |
| `crates/node/consensus/src/components/snapshot.rs` | `InMemorySnapshotStore` implementation |
| `crates/node/consensus/src/traits.rs` | `Snapshot` struct and `SnapshotStore` trait |
| `crates/node/consensus/src/components/seed.rs` | `InMemorySeedTracker` (also unbounded) |
| `crates/node/reporters/src/lib.rs` | `FinalizedReporter` -- finalization callback, triggers persistence |
| `crates/node/runner/src/app.rs` | `RevmApplication` -- block proposal and verification |
| `crates/node/runner/src/runner.rs` | `ProductionRunner` -- node startup and recovery |
| `crates/storage/overlay/src/overlay.rs` | `OverlayState<S>` -- change layering |
| `crates/storage/qmdb/src/root.rs` | `StateRoot::transition()` and `StateRoot::compute()` |
| `crates/storage/qmdb/src/changes.rs` | `ChangeSet` and `AccountUpdate` definitions |
| `crates/storage/qmdb/src/store.rs` | `QmdbStore` -- partition writes and batching |
| `crates/storage/qmdb-ledger/src/ledger.rs` | `QmdbLedger` -- high-level QMDB operations |
| `crates/storage/backend/src/backend.rs` | `CommonwareBackend` -- Merkle tree + storage initialization |
