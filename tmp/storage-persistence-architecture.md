# Storage and Persistence Architecture

## What is Kora?

Kora is a blockchain node implementation built on top of the Commonware consensus framework. It executes Ethereum-compatible transactions using REVM (the Rust EVM implementation) and achieves consensus via the Simplex BFT protocol with BLS12-381 threshold signatures. Each validator in the network maintains a full copy of the blockchain state, which includes all account balances, nonces, contract storage slots, and deployed bytecode.

This document describes how Kora persists that state to disk, how it tracks pending state during block building, and how it recovers after a crash or restart.

---

## High-Level Architecture

```
+-----------------------------------------------------------+
|                   Block Execution (REVM)                   |
|            Produces a ChangeSet per block                  |
+-----------------------------------------------------------+
                          |
                          v
+-----------------------------------------------------------+
|             OverlayState<QmdbState>                        |
|   Layers uncommitted changes on top of persisted state    |
+-----------------------------------------------------------+
                          |
                          v
+-----------------------------------------------------------+
|            InMemorySnapshotStore                           |
|   Caches execution results indexed by block digest        |
|   Tracks: pending | persisting | persisted                |
+-----------------------------------------------------------+
                          |
            +-------------+-------------+
            |             |             |
            v             v             v
      +-----------+ +-----------+ +-----------+
      | Accounts  | | Storage   | |   Code    |
      | Partition | | Partition | | Partition |
      +-----------+ +-----------+ +-----------+
            |             |             |
            +-------------+-------------+
                          |
                          v
+-----------------------------------------------------------+
|          Commonware Storage Backend                        |
|   Journaled Merkle Trees (MMR) + append-only logs         |
+-----------------------------------------------------------+
                          |
                          v
+-----------------------------------------------------------+
|      Persistent Directory: {data_dir}/runtime             |
|      Override: KORA_RUNTIME_DIR env var (tmpfs in devnet)  |
+-----------------------------------------------------------+
```

---

## QMDB: The Authenticated Merkle Database

QMDB (Quantized Merkle Database) is Kora's state storage engine, built on top of Commonware's `commonware-storage` primitives. It provides three properties that a blockchain needs:

1. **Authenticated reads**: Every query can be verified against a Merkle root.
2. **Efficient writes**: Batch commits update all three partitions in a single pass.
3. **Deterministic roots**: All validators compute the same state root for the same state transitions.

### The Three Partitions

QMDB splits blockchain state into three independent partitions, each backed by its own journaled Merkle tree:

#### Accounts Partition

- **Key**: 20-byte Ethereum address
- **Value**: 80-byte fixed encoding (nonce: u64, balance: U256, code_hash: B256, generation: u64)
- **Generation field**: Increments when an account is selfdestructed and recreated; this invalidates old storage slots without needing to delete them from the storage partition.

Defined in: `crates/storage/backend/src/accounts.rs`

#### Storage Partition

- **Key**: 60 bytes = address (20) + generation (8) + slot (32)
- **Value**: 32-byte U256 storage value
- **Design note**: The composite key includes the account generation number so that recreation of a contract at the same address naturally invalidates all old storage without expensive deletion.

Defined in: `crates/storage/backend/src/storage.rs`

#### Code Partition

- **Key**: 32-byte keccak256 hash of the bytecode
- **Value**: Variable-length bytecode (max 24,576 bytes per EIP-170)
- **Content-addressed**: Multiple contracts with identical bytecode share a single code entry.

Defined in: `crates/storage/backend/src/code.rs`

### Underlying Storage Primitives

Each partition uses Commonware's `qmdb::any::VariableConfig` which combines:
- A **journaled Merkle tree** (MMR) for authenticated root computation
- A **contiguous variable journal** for the actual key-value data
- A **page cache** (16 KB pages, 1024 pages default) shared across partitions

Configuration is defined in `crates/storage/backend/src/config.rs` (`QmdbBackendConfig`) and instantiated in `crates/storage/backend/src/backend.rs` (`store_config()` function, lines 173-198).

---

## State Root Computation

The state root that appears in every Kora block is a composite hash of the three partition Merkle roots:

```rust
// crates/storage/qmdb/src/root.rs

const KORA_ROOT_NAMESPACE: &[u8] = b"_KORA_QMDB_ROOT";

fn compute(accounts_root: B256, storage_root: B256, code_root: B256) -> B256 {
    keccak256(
        b"_KORA_QMDB_ROOT" || accounts_root || storage_root || code_root
    )
}
```

This root is used when QMDB is fully committed (i.e., changes are written to the Merkle trees on disk).

### Transition Roots (Consensus Determinism)

During block building, validators need to agree on state roots *before* changes are committed to disk. The `StateRoot::transition()` function computes a deterministic root from a parent root and a `ChangeSet` using a different namespace:

```rust
// crates/storage/qmdb/src/root.rs

const KORA_TRANSITION_ROOT_NAMESPACE: &[u8] = b"_KORA_STATE_TRANSITION_ROOT";

fn transition(parent_root: B256, changes: &ChangeSet) -> B256 {
    if changes.is_empty() {
        return parent_root;
    }
    keccak256(
        b"_KORA_STATE_TRANSITION_ROOT" || parent_root || serialized(changes)
    )
}
```

Key property: if a block has no transactions (empty `ChangeSet`), its state root equals its parent's state root. This ensures empty blocks do not alter the chain's state root.

The serialization is deterministic: accounts are sorted by address (BTreeMap), and for each account the nonce, balance, code_hash, code presence flag, and sorted storage slots are included.

---

## Overlay States: Pending Changes During Block Building

During block execution, multiple blocks may be proposed in parallel (forking) before one is finalized. Kora handles this through `OverlayState<S>`, defined in `crates/storage/overlay/src/overlay.rs`.

An `OverlayState` wraps a base state (typically `QmdbState`, a handle to the committed QMDB stores) with an `Arc<ChangeSet>`:

```rust
pub struct OverlayState<S> {
    base: S,
    changes: Arc<ChangeSet>,
}
```

Reads check the overlay first. If the account/slot is found in `changes`, the overlay value is returned. Otherwise, the read falls through to the base `QmdbState` (which hits the on-disk Merkle tree).

Special behaviors:
- If an account is marked `selfdestructed` in the overlay, all storage reads return zero.
- If an account is marked `created` in the overlay but a slot is missing from `changes.storage`, zero is returned (not falling through to old base state).
- The `merge_changes()` method produces a new `ChangeSet` combining the overlay's existing changes with newer changes.

---

## The Snapshot System

### What is a Snapshot?

A `Snapshot<S>` (defined in `crates/node/consensus/src/traits.rs`) captures the full execution result of a single block:

```rust
pub struct Snapshot<S> {
    pub parent: Option<Digest>,      // Parent block digest
    pub state: S,                    // OverlayState at this point
    pub state_root: StateRoot,       // Computed state root
    pub changes: ChangeSet,          // State delta from this block alone
    pub tx_ids: BTreeSet<TxId>,      // Transactions included
}
```

### InMemorySnapshotStore

Defined in `crates/node/consensus/src/components/snapshot.rs`, this is an in-memory cache of all block snapshots. It tracks three sets:

```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,   // all snapshots
    persisted: Arc<RwLock<BTreeSet<Digest>>>,                // committed to QMDB
    persisting: Arc<RwLock<BTreeSet<Digest>>>,               // currently being committed
}
```

Operations:
- `insert(digest, snapshot)`: Stores a new snapshot.
- `get(digest)`: Retrieves a snapshot (cloned).
- `mark_persisted(digests)`: Marks digests as fully committed.
- `mark_persisting_chain(chain)` / `clear_persisting_chain(chain)`: Guards against concurrent persist attempts for the same chain.
- `changes_for_persist(digest)`: Walks from `digest` back to the last persisted ancestor, collecting all unpersisted changes. Returns the chain of digests and a merged `ChangeSet`.
- `merged_changes(parent, new_changes)`: Merges all unpersisted ancestor changes with a new set (used during block execution to compute speculative state roots).

### Persistence Flow

When a block is finalized by consensus, `LedgerView::persist_snapshot()` (in `crates/node/ledger/src/lib.rs`, line 293) is called:

1. Acquire lock, call `changes_for_persist(digest)` to get the chain of unpersisted ancestors and their merged changes.
2. Check `can_persist_chain()` to ensure no concurrent persist is in progress for this chain.
3. Mark the chain as "persisting" (optimistic lock).
4. Release the lock and call `qmdb.commit_changes(merged_changes)` -- this writes to the Commonware Merkle trees on disk.
5. Re-acquire lock, clear the "persisting" marks.
6. On success: replace the tip snapshot's overlay with a fresh `OverlayState::new(qmdb.state(), ChangeSet::default())` (discarding the now-committed changes from memory) and mark the chain as persisted.
7. On failure: propagate the error; the chain remains unpersisted for a future retry.

---

## The Ledger Layer

`LedgerView` (in `crates/node/ledger/src/lib.rs`) ties together the mempool, snapshot store, seed tracker, and QMDB backend under a single `Arc<Mutex<LedgerState>>`:

```rust
struct LedgerState {
    mempool: InMemoryMempool,
    snapshots: InMemorySnapshotStore<OverlayState<QmdbState>>,
    seeds: InMemorySeedTracker,
    qmdb: QmdbLedger,
}
```

The `LedgerService` wraps `LedgerView` with event publishing (for observers that log or react to ledger events).

### Initialization

`LedgerView::init_with_config_and_genesis()` (line 111):

1. Opens the QMDB backend (`CommonwareBackend::open()`), which creates/opens the three partitions.
2. Optionally applies the genesis allocation (first run only).
3. Computes the genesis root from QMDB.
4. Creates a genesis block (height 0, zero parent, zero prevrandao, empty txs).
5. Creates and inserts a genesis snapshot with an empty overlay.
6. Marks the genesis snapshot as persisted.

---

## KORA_RUNTIME_DIR and Commonware's Journal

### How It Works

The `runtime_storage_directory()` function in `crates/node/runner/src/runner.rs` (line 74) determines where Commonware stores all its data:

```rust
pub fn runtime_storage_directory(data_dir: &Path) -> PathBuf {
    match std::env::var_os("KORA_RUNTIME_DIR") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => data_dir.join("runtime"),
    }
}
```

The resolved path is passed to `cw_tokio::Runner::new(Config::default().with_storage_directory(runtime_dir))` when starting the Commonware runtime. All Commonware storage primitives (journals, Merkle trees, archives) use this directory.

### Partition Names on Disk

Each partition creates multiple subdirectories under the runtime directory:
- `{prefix}-accounts-log` -- journal for account data
- `{prefix}-accounts-mmr` -- Merkle mountain range for account roots
- `{prefix}-accounts-mmr-meta` -- metadata for the MMR
- `{prefix}-storage-log`, `{prefix}-storage-mmr`, `{prefix}-storage-mmr-meta` -- same for storage
- `{prefix}-code-log`, `{prefix}-code-mmr`, `{prefix}-code-mmr-meta` -- same for code
- `{prefix}-finalized-blocks` -- archive of finalized block data
- `{prefix}-finalizations-by-height` -- archive of finalization certificates

The default prefix is `"kora"` (constant `PARTITION_PREFIX` in runner.rs line 56), with QMDB using `"kora-qmdb"` (runner.rs line 371).

### Docker Devnet: tmpfs

In the Docker devnet configuration (`docker/compose/devnet.yaml`), each validator mounts a 1 GB tmpfs at `/runtime`:

```yaml
tmpfs:
  - /runtime:size=1g,mode=1777
environment:
  - KORA_RUNTIME_DIR=/runtime
```

This avoids Docker-volume fsync latency and greatly improves consensus throughput in development. The tradeoff is that all Commonware-managed state (journals, Merkle trees, block archives) is lost on container restart.

---

## Recovery Flow: What Happens on Restart

When a validator restarts, the recovery is handled by `recover_finalized_state()` in `crates/node/runner/src/runner.rs` (lines 170-228):

### Step 1: Detect Existing History

```rust
let has_finalized_history = finalized_blocks.last_index().is_some();
```

If no finalized blocks exist in the archive, the node starts fresh from genesis (applying the genesis allocation to QMDB).

### Step 2: Restore VRF Seeds

```rust
for height in start..=end {
    if let Some(finalization) = finalizations_by_height.get(ArchiveId::Index(height)).await? {
        ledger.set_seed(finalization.proposal.payload, seed_hash(finalization.seed())).await;
    }
}
```

The VRF seed cache (`InMemorySeedTracker`) must be populated because subsequent consensus rounds need the seed from the most recent finalized block to compute `prevrandao`.

### Step 3: Restore Block Index

Each finalized block from the archive is fed to the block indexer (if RPC is enabled) so that historical block queries work immediately after restart.

### Step 4: Set Ledger Head

```rust
if let Some(head) = head {
    ledger.restore_persisted_snapshot(&head).await;
}
```

`restore_persisted_snapshot()` creates a new snapshot for the head block with a clean overlay (empty `ChangeSet`) pointing at the current QMDB state, and marks it as persisted. This tells the snapshot store that everything up to and including this block has already been committed to QMDB.

### What QMDB Itself Restores

Because QMDB is built on journaled Merkle trees, Commonware handles its own recovery automatically when the stores are opened. The journals replay uncommitted entries and the MMR reconstructs its state from the journal. Kora does not need to explicitly replay transactions -- it only needs to restore the metadata (seeds, block index, snapshot markers).

### Important Caveat

A recent revert (commit `6462dad`) removed a `restore_persisted_digest` feature that attempted to restore from the finalization archive alone (without block data). This caused state inconsistencies because the snapshot store needs the actual block data to correctly reconstruct parent pointers.

---

## Known Issues and Limitations

### 1. InMemorySnapshotStore Has No Eviction

**Problem**: The `InMemorySnapshotStore` is a `BTreeMap<Digest, Snapshot<S>>` that grows without bound. There is no eviction policy, no maximum size, and no background pruning of old snapshots.

**When it matters**: If consensus stalls (e.g., not enough validators are online to finalize), the node continues building proposals and accumulating snapshots. Each snapshot contains a full `OverlayState` and `ChangeSet`.

**Impact**: Memory usage grows proportionally to the number of unfinalized views. At the default block production rate, a sustained stall could accumulate thousands of snapshots before the process is OOM-killed.

**Mitigant**: In practice, finalization keeps up and persistence replaces snapshot overlays with clean references to QMDB. The issue only manifests during extended liveness failures.

### 2. No Compaction of Old Snapshots

**Problem**: After a snapshot is marked "persisted", its overlay is replaced with a clean QMDB reference, but the `BTreeMap` entry itself is never removed. Over time, the persisted set and the snapshot map grow indefinitely.

**Impact**: Slow memory leak proportional to chain height. Each persisted snapshot is small (empty overlay), but the keys and metadata still consume memory.

### 3. tmpfs for Runtime Means State Lost on Container Restart

**Problem**: The Docker devnet uses tmpfs for `KORA_RUNTIME_DIR`. All Commonware journals, Merkle trees, and archives reside in memory-backed filesystems. When a container restarts, all state is gone.

**Impact**: Validators must re-sync from genesis on every restart in devnet mode. This is acceptable for development but would be catastrophic in production.

**Production mitigation**: Do not set `KORA_RUNTIME_DIR` to a tmpfs path. The default (`{data_dir}/runtime`) uses persistent disk storage.

### 4. Non-Atomic Cross-Partition Writes

**Problem**: When `commit_changes()` writes to QMDB, the three partitions (accounts, storage, code) are written sequentially. If the process crashes between partition writes, one partition may be ahead of the others.

**Mitigant**: Commonware's journal-based storage provides crash recovery at the individual partition level. However, cross-partition consistency after a partial commit has not been formally verified.

### 5. No Garbage Collection for Dead Storage

**Problem**: When a contract selfdestructs, its storage slots (keyed by the old generation number) remain in the storage partition forever. New storage uses a bumped generation, making old entries unreachable but not deleted.

**Impact**: Disk usage grows monotonically, even if most contract state has been destroyed.

---

## Configuration Reference

| Parameter | Default Value | Location |
|-----------|--------------|----------|
| Page size | 16 KB | `crates/storage/backend/src/config.rs` |
| Page cache size | 1,024 pages | `crates/storage/backend/src/config.rs` |
| Items per blob (MMR) | 128 | `crates/storage/backend/src/backend.rs` |
| Write buffer | 1 MB | `crates/storage/backend/src/backend.rs` |
| Max code size | 24,576 bytes | `crates/storage/backend/src/backend.rs` |
| Partition prefix | `"kora"` | `crates/node/runner/src/runner.rs` |
| QMDB partition prefix | `"kora-qmdb"` | `crates/node/runner/src/runner.rs` |
| Runtime dir env var | `KORA_RUNTIME_DIR` | `crates/node/runner/src/runner.rs` |
| Default runtime dir | `{data_dir}/runtime` | `crates/node/runner/src/runner.rs` |
| Docker tmpfs size | 1 GB | `docker/compose/devnet.yaml` |

---

## Crate Map

| Crate | Path | Responsibility |
|-------|------|----------------|
| `kora-qmdb` | `crates/storage/qmdb/` | Core types: `ChangeSet`, `AccountUpdate`, `StateRoot`, `QmdbStore` traits |
| `kora-backend` | `crates/storage/backend/` | Commonware integration: opens partitions, computes roots from MMR digests |
| `kora-handlers` | `crates/storage/handlers/` | Thread-safe `QmdbHandle` with `RwLock`, REVM `Database` adapter |
| `kora-overlay` | `crates/storage/overlay/` | `OverlayState<S>`: ephemeral change layer over any `StateDbRead` |
| `kora-traits` | `crates/storage/traits/` | `StateDbRead`, `StateDbWrite`, `StateDb` trait definitions |
| `kora-qmdb-ledger` | `crates/storage/qmdb-ledger/` | `QmdbLedger`: high-level adapter combining backend + handlers + root computation |
| `kora-ledger` | `crates/node/ledger/` | `LedgerView` / `LedgerService`: snapshot management, mempool, persistence orchestration |
| `kora-consensus` | `crates/node/consensus/` | `InMemorySnapshotStore`, `Snapshot`, `SnapshotStore` trait, `InMemoryMempool` |
| `kora-runner` | `crates/node/runner/` | `ProductionRunner`: wires everything together, recovery, KORA_RUNTIME_DIR resolution |

---

## Code References

- State root computation: `crates/storage/qmdb/src/root.rs` (lines 1-62)
- ChangeSet and AccountUpdate: `crates/storage/qmdb/src/changes.rs` (lines 1-100)
- Backend with three partitions: `crates/storage/backend/src/backend.rs` (lines 1-258)
- Backend configuration: `crates/storage/backend/src/config.rs` (lines 1-51)
- OverlayState implementation: `crates/storage/overlay/src/overlay.rs` (lines 1-165)
- InMemorySnapshotStore: `crates/node/consensus/src/components/snapshot.rs` (lines 1-166)
- Snapshot struct: `crates/node/consensus/src/traits.rs` (lines 18-43)
- LedgerView (init, persist, restore): `crates/node/ledger/src/lib.rs` (lines 52-341)
- QmdbLedger (commit, root): `crates/storage/qmdb-ledger/src/ledger.rs` (lines 1-118)
- Runtime directory resolution: `crates/node/runner/src/runner.rs` (lines 68-83)
- Recovery flow: `crates/node/runner/src/runner.rs` (lines 170-228)
- Production runner startup: `crates/node/runner/src/runner.rs` (lines 329-450)
