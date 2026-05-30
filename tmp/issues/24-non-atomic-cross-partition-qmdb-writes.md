# Storage: Non-atomic cross-partition QMDB writes can leave state inconsistent after crash

**Severity:** Medium (P2)
**Component:** `crates/storage/qmdb/src/store.rs`, `crates/storage/backend/src/backend.rs`, `crates/storage/qmdb-ledger/src/ledger.rs`
**Affects:** All Kora validator nodes using persistent storage (not tmpfs devnet)

---

## Summary

Kora is an EVM-compatible blockchain built on the Commonware consensus framework. It persists all on-chain state -- account balances, contract storage slots, and contract bytecode -- in QMDB, a custom key-value state database. QMDB splits state into three independent partitions, each backed by its own Commonware journaled Merkle tree. When a block is finalized and its state changes are committed, the `commit_changes()` method in `QmdbStore` (`crates/storage/qmdb/src/store.rs`, lines 224-230) writes to each partition sequentially via `apply_batches()` (lines 195-217). The three `write_batch()` calls are not wrapped in any cross-partition transaction or write-ahead log.

If the process crashes between partition writes -- for example, after the accounts partition is updated but before the storage partition write completes -- the database is left in a state where one partition reflects the new block's changes while the others still reflect the previous block. Commonware's journal-based storage provides crash recovery at the individual partition level (if a single `write_batch()` is interrupted mid-write, the partition's journal can replay uncommitted entries on restart), but there is no mechanism guaranteeing that all three partitions commit together as an atomic unit. The state root computed from mismatched partitions would differ from the expected value, and the current recovery flow (`recover_finalized_state()` in `crates/node/runner/src/runner.rs`, lines 170-228) does not detect this condition.

---

## Background: QMDB's Three-Partition Architecture

QMDB stores all EVM state across three separate Commonware-backed partitions. Each partition has its own key space, value encoding, and independent journaled Merkle tree:

### 1. Accounts Partition

- **Key:** 20-byte Ethereum address (`Address`)
- **Value:** 80-byte fixed encoding: nonce (8 bytes) + balance (32 bytes) + code_hash (32 bytes) + generation (8 bytes)
- **Encoding:** Defined in `crates/storage/qmdb/src/encoding.rs`, lines 41-72 (`AccountEncoding`)

```rust
// crates/storage/qmdb/src/encoding.rs:52-58
pub fn encode(nonce: u64, balance: U256, code_hash: B256, generation: u64) -> [u8; 80] {
    let mut buf = [0u8; 80];
    buf[0..8].copy_from_slice(&nonce.to_be_bytes());
    buf[8..40].copy_from_slice(&balance.to_be_bytes::<32>());
    buf[40..72].copy_from_slice(code_hash.as_slice());
    buf[72..80].copy_from_slice(&generation.to_be_bytes());
    buf
}
```

### 2. Storage Partition

- **Key:** 60-byte composite key: address (20 bytes) + generation (8 bytes) + slot (32 bytes)
- **Value:** 32-byte `U256` storage value
- **Encoding:** Defined in `crates/storage/qmdb/src/encoding.rs`, lines 6-39 (`StorageKey`)

The `generation` field in the storage key is critical: it is incremented when an account is recreated (via `CREATE2` to the same address) or selfdestructed, which invalidates all previous storage entries without needing to delete them individually.

### 3. Code Partition

- **Key:** 32-byte keccak256 hash of the bytecode (`B256`)
- **Value:** Variable-length contract bytecode (up to 24,576 bytes per EIP-170)
- **Max size:** Enforced by `CODE_MAX_BYTES` constant in `crates/storage/backend/src/backend.rs`, line 21

### Partition Independence

Each partition is opened as an independent Commonware storage instance in `crates/storage/backend/src/backend.rs`, lines 200-230 (`open_stores()`):

```rust
// crates/storage/backend/src/backend.rs:200-230
async fn open_stores(context: Context, config: &QmdbBackendConfig) -> Result<Stores, BackendError> {
    let page_cache = CacheRef::from_pooler(&context, config.page_size, config.page_cache_size);

    let accounts = AccountStore::init(
        context.with_label("accounts"),
        store_config(&config.partition_prefix, "accounts", page_cache.clone(), ()),
    ).await.map_err(|e| BackendError::Storage(e.to_string()))?;

    let storage = StorageStore::init(
        context.with_label("storage"),
        store_config(&config.partition_prefix, "storage", page_cache.clone(), ()),
    ).await.map_err(|e| BackendError::Storage(e.to_string()))?;

    let code = CodeStore::init(
        context.with_label("code"),
        store_config(&config.partition_prefix, "code", page_cache, (RangeCfg::new(0..=CODE_MAX_BYTES), ())),
    ).await.map_err(|e| BackendError::Storage(e.to_string()))?;

    Ok(Stores { accounts, storage, code })
}
```

Each partition gets its own journal partition name (e.g., `{prefix}-accounts-log`, `{prefix}-storage-log`, `{prefix}-code-log`), its own Merkle tree partition (e.g., `{prefix}-accounts-mmr`, `{prefix}-storage-mmr`, `{prefix}-code-mmr`), and its own Merkle metadata partition (e.g., `{prefix}-accounts-mmr-meta`, `{prefix}-storage-mmr-meta`, `{prefix}-code-mmr-meta`). These are nine independent Commonware storage partitions with no shared transaction coordination.

---

## The Non-Atomic Write Path

### Step 1: Finalization triggers persistence

When a block is finalized, the `FinalizedReporter` calls `persist_snapshot()` on the ledger (`crates/node/ledger/src/lib.rs`, line 307), which in turn calls `qmdb.commit_changes(changes)`:

```rust
// crates/node/ledger/src/lib.rs:293-307
pub async fn persist_snapshot(&self, digest: ConsensusDigest) -> LedgerResult<bool> {
    let (changes, qmdb, chain) = {
        let inner = self.inner.lock().await;
        let (chain, changes) = inner.snapshots.changes_for_persist(digest)?;
        // ...
        (changes, inner.qmdb.clone(), chain)
    };

    let result = qmdb.commit_changes(changes).await;
    // ...
}
```

### Step 2: QmdbLedger delegates to QmdbStore

The `QmdbLedger::commit_changes()` method (`crates/storage/qmdb-ledger/src/ledger.rs`, lines 95-110) acquires a write lock and calls through to the underlying store:

```rust
// crates/storage/qmdb-ledger/src/ledger.rs:95-110
pub async fn commit_changes(&self, changes: QmdbChangeSet) -> Result<StateRoot, Error> {
    let _storage_access = self.handle.storage_access().await;
    let mut store = self.handle.write().await;
    store
        .commit_changes(changes)
        .await
        .map_err(|e| kora_traits::StateDbError::Storage(e.to_string()))?;
    let stores =
        store.stores().map_err(|e| kora_traits::StateDbError::Storage(e.to_string()))?;
    let root = QmdbStateRoot::compute(
        B256::from_slice(stores.accounts.root()?.as_ref()),
        B256::from_slice(stores.storage.root()?.as_ref()),
        B256::from_slice(stores.code.root()?.as_ref()),
    );
    Ok(StateRoot(root))
}
```

### Step 3: QmdbStore writes partitions sequentially

The `QmdbStore::commit_changes()` method (`crates/storage/qmdb/src/store.rs`, lines 224-230) builds batches and applies them:

```rust
// crates/storage/qmdb/src/store.rs:224-230
pub async fn commit_changes(&mut self, changes: ChangeSet) -> Result<(), QmdbError> {
    if changes.is_empty() {
        return Ok(());
    }
    let batches = self.build_batches(&changes).await?;
    self.apply_batches(batches).await
}
```

The `apply_batches()` method (`crates/storage/qmdb/src/store.rs`, lines 195-217) is where the non-atomic write occurs:

```rust
// crates/storage/qmdb/src/store.rs:195-217
pub async fn apply_batches(&mut self, batches: StoreBatches) -> Result<(), QmdbError> {
    let stores = self.stores_mut()?;

    stores
        .accounts
        .write_batch(batches.accounts)       // <-- Write 1: accounts partition
        .await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;

    stores
        .storage
        .write_batch(batches.storage)        // <-- Write 2: storage partition
        .await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;

    stores
        .code
        .write_batch(batches.code)           // <-- Write 3: code partition
        .await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;

    Ok(())
}
```

The three `write_batch()` calls execute sequentially. Each one is a separate operation against an independent Commonware journal. There is no wrapping transaction, no two-phase commit, and no write-ahead log that coordinates the three writes.

---

## Failure Scenarios

### Scenario 1: Crash After Accounts, Before Storage

This is the most likely partial-commit scenario because the accounts partition is written first and the storage partition (which typically has more entries per block due to contract storage updates) takes longer.

1. Block N is finalized. The `FinalizedReporter` calls `persist_snapshot(digest_N)`.
2. `persist_snapshot()` merges the change set from unpersisted ancestors and calls `qmdb.commit_changes(merged_changes)`.
3. Inside `apply_batches()`:
   - `accounts.write_batch()` completes successfully. The accounts partition now reflects block N's state: updated nonces, balances, code hashes, and generation counters.
   - The process is killed (SIGKILL, OOM, hardware failure) before `storage.write_batch()` begins.
4. The storage partition still reflects block N-1's state. Any storage slot changes from block N's transactions (contract state updates, mapping writes, etc.) are missing.
5. The code partition also still reflects block N-1's state. If block N deployed new contracts, their bytecode is missing.

**State after crash:**

| Partition | State |
|-----------|-------|
| Accounts | Block N (nonces, balances updated) |
| Storage | Block N-1 (stale contract storage) |
| Code | Block N-1 (missing newly deployed contracts) |

**On restart:**

The `recover_finalized_state()` function in `crates/node/runner/src/runner.rs` (lines 170-228) reads the finalization archive, finds block N as the last finalized block, and calls `restore_persisted_snapshot(&block_N)`. This creates a snapshot with `state_root = block_N.state_root` pointing to the current QMDB state. It does NOT verify that QMDB's actual partition roots are consistent with each other or with the archived block's expected state.

The node begins participating in consensus with a corrupted state. When it executes transactions for block N+1:

- Account lookups return updated nonces/balances (from block N).
- Storage lookups return stale values (from block N-1).
- A contract that wrote to storage in block N and reads that storage in block N+1 will see the pre-write value.
- The resulting state root will differ from what honest validators compute, causing **silent state divergence**.

### Scenario 2: Crash After Accounts and Storage, Before Code

1. Block N deploys a new contract via a `CREATE` or `CREATE2` transaction.
2. `accounts.write_batch()` succeeds: the new account entry is written with the contract's `code_hash` and `generation = 0`.
3. `storage.write_batch()` succeeds: the contract's constructor-initialized storage slots are written.
4. The process crashes before `code.write_batch()` completes.
5. On restart, the accounts partition has an account record pointing to a `code_hash` for which no corresponding bytecode exists in the code partition.
6. Any subsequent transaction that calls this contract will attempt to load bytecode by `code_hash`, get `None`, and fail with a `CodeNotFound` error.

### Scenario 3: Error (Not Crash) After Partial Write

Even without a process crash, if `storage.write_batch()` or `code.write_batch()` returns an error (e.g., disk full, I/O error), the function propagates the error via `?`. But the accounts partition has already been written. The caller (`persist_snapshot()` in `crates/node/ledger/src/lib.rs`, line 307) sees the error and handles it at line 310, but the accounts partition write cannot be rolled back.

```rust
// crates/node/ledger/src/lib.rs:307-325
let result = qmdb.commit_changes(changes).await;
let inner = self.inner.lock().await;
inner.snapshots.clear_persisting_chain(&chain);
match result {
    Ok(_) => {
        // ... success path: update snapshot with new state ...
    }
    Err(e) => {
        // Error path: log and return error
        // But accounts partition changes are already committed!
        Err(LedgerError::StateDb(e))
    }
}
```

---

## Why Individual Partition Crash Recovery Is Not Sufficient

Each Commonware partition uses a journaled storage mechanism. If `write_batch()` is interrupted mid-write within a single partition, the journal can replay or discard the incomplete write on next startup, restoring that partition to a consistent state. This provides **intra-partition** crash recovery.

However, cross-partition consistency requires that all three partitions either commit together or none of them commit. The journals are independent:

- The accounts journal knows nothing about the storage journal's state.
- The storage journal knows nothing about the code journal's state.
- There is no shared sequence number, epoch counter, or commit marker that ties the three journals together.

After a crash, each partition independently recovers to its own last consistent state. If accounts committed successfully before the crash but storage did not, the accounts partition recovers to the post-commit state while the storage partition recovers to the pre-commit state. Both partitions are individually consistent, but they are mutually inconsistent -- they reflect different logical points in the block history.

---

## State Root Implications

QMDB's state root is computed from the three partition Merkle roots (`crates/storage/qmdb/src/root.rs`, lines 16-23):

```rust
// crates/storage/qmdb/src/root.rs:16-23
pub fn compute(accounts_root: B256, storage_root: B256, code_root: B256) -> B256 {
    let mut buf = Vec::with_capacity(KORA_ROOT_NAMESPACE.len() + 96);
    buf.extend_from_slice(KORA_ROOT_NAMESPACE);
    buf.extend_from_slice(accounts_root.as_slice());
    buf.extend_from_slice(storage_root.as_slice());
    buf.extend_from_slice(code_root.as_slice());
    keccak256(buf)
}
```

If the accounts partition is at block N but the storage and code partitions are at block N-1, the computed state root is a hash of mismatched inputs. This root does not correspond to any valid historical state -- it is not the root for block N (because storage/code are stale) and it is not the root for block N-1 (because accounts are ahead).

Additionally, Kora uses two distinct root computations (see `crates/storage/qmdb/src/root.rs`):

1. **Transition roots** (`StateRoot::transition()`, line 26): Used during consensus. A keccak256 of `b"_KORA_STATE_TRANSITION_ROOT"` + parent_root + serialized_changes. This is what appears in block headers as `block.state_root`.

2. **QMDB Merkle roots** (`StateRoot::compute()`, line 16): Used after QMDB commits. A keccak256 of `b"_KORA_QMDB_ROOT"` + the three partition roots.

These two values are in different hash namespaces and are never equal for the same block. The current recovery code (`restore_persisted_snapshot()`) uses the archived block's transition root, not the QMDB Merkle root, so there is no way to detect the mismatch by simple comparison. A cross-partition consistency check would need to either re-derive the expected QMDB root independently or verify partition roots against a stored reference.

---

## Relationship to Issue #08

Issue #08 ("No crash-consistent recovery or state validation on startup") addresses the broader problem of recovery validation, including the archive-vs-QMDB consistency gap. This issue focuses specifically on the non-atomicity of the three-partition write within `apply_batches()`, which is the mechanism by which QMDB can enter an internally inconsistent state in the first place.

Issue #08's proposed fixes (persisting a last-committed digest, validating on startup, replaying missing blocks) would detect and recover from the consequences of this bug. But they do not prevent the inconsistency from occurring. The fixes proposed here aim to make the commit itself atomic, so that cross-partition inconsistency cannot arise regardless of when a crash occurs.

---

## Proposed Fixes

### Fix 1: Cross-Partition Commit Journal

Add a lightweight write-ahead log that records the intent to write all three partitions before any individual partition write begins. The commit protocol becomes:

1. **Prepare:** Serialize the complete `StoreBatches` (or a compact manifest of it) to a dedicated WAL partition.
2. **Execute:** Write each partition sequentially as today.
3. **Finalize:** Mark the WAL entry as complete.

On recovery, check the WAL:
- If a WAL entry exists but is not marked complete, the commit was interrupted. Roll forward by replaying the remaining partition writes from the WAL data.
- If no incomplete WAL entry exists, all partitions are consistent.

```rust
// Proposed change to crates/storage/qmdb/src/store.rs
pub async fn apply_batches(&mut self, batches: StoreBatches) -> Result<(), QmdbError> {
    let stores = self.stores_mut()?;

    // 1. Write intent to WAL before any partition write
    let wal_entry = self.wal.prepare(&batches).await?;

    // 2. Execute partition writes (same as today)
    stores.accounts.write_batch(batches.accounts).await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;
    stores.storage.write_batch(batches.storage).await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;
    stores.code.write_batch(batches.code).await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;

    // 3. Mark WAL entry as complete
    self.wal.finalize(wal_entry).await?;

    Ok(())
}
```

This approach has the advantage of being transparent to the rest of the system: the `commit_changes()` API does not change, and recovery is handled internally by the storage layer.

**Trade-off:** The WAL doubles the write volume for each commit (once to WAL, once to partitions). For blocks with large storage change sets (e.g., a Uniswap swap touching dozens of storage slots), this could be significant.

### Fix 2: Deterministic Partition Ordering with Commit Sequence Number

A lighter-weight alternative: store a monotonically increasing commit sequence number in each partition's metadata after each `write_batch()`. On recovery, read the sequence number from each partition. If they do not all match, the partitions are inconsistent.

```rust
// After each write_batch, append a sequence marker:
stores.accounts.write_batch(batches.accounts).await?;
stores.accounts.set_commit_seq(seq).await?;

stores.storage.write_batch(batches.storage).await?;
stores.storage.set_commit_seq(seq).await?;

stores.code.write_batch(batches.code).await?;
stores.code.set_commit_seq(seq).await?;
```

On recovery:

```rust
let acct_seq = stores.accounts.get_commit_seq().await?;
let stor_seq = stores.storage.get_commit_seq().await?;
let code_seq = stores.code.get_commit_seq().await?;

if acct_seq != stor_seq || stor_seq != code_seq {
    // Partial commit detected. The partition with the highest seq is
    // ahead. Roll back the ahead partition(s) or roll forward the
    // behind partition(s) using archived block data.
    recover_partial_commit(acct_seq, stor_seq, code_seq, archives).await?;
}
```

**Trade-off:** Detection is cheap, but recovery still requires replaying from the archive (the sequence number alone does not tell you what data to write). This approach is useful when combined with issue #08's block replay mechanism.

### Fix 3: Startup Partition Consistency Validation

Regardless of which write-time fix is chosen, add a startup check that verifies cross-partition consistency before the node joins consensus. This is a defense-in-depth measure:

```rust
// Proposed addition to crates/node/runner/src/runner.rs, inside run() before engine.start()
async fn validate_partition_consistency(
    backend: &CommonwareBackend,
) -> anyhow::Result<()> {
    let accounts_root = backend.accounts().root()?;
    let storage_root = backend.storage().root()?;
    let code_root = backend.code().root()?;

    let computed_root = state_root_from_roots(accounts_root, storage_root, code_root);

    // Compare against the last known good root (from metadata or archive)
    let expected_root = backend.last_committed_root().await?;

    if let Some(expected) = expected_root {
        anyhow::ensure!(
            computed_root == expected,
            "QMDB partition consistency check failed: \
             computed root {computed_root:?} != expected {expected:?}. \
             Partitions may be at different commit heights."
        );
    }

    info!("QMDB partition consistency check passed");
    Ok(())
}
```

This validation should be a hard failure that prevents the node from starting. Participating in consensus with inconsistent partition state would cause the node to produce invalid blocks and diverge from the network.

### Fix 4: Atomic Batch Across Partitions (Commonware Upstream)

The most robust fix would be to extend Commonware's storage layer with a multi-partition atomic commit primitive. This would allow `apply_batches()` to submit all three partition writes as a single atomic operation at the storage layer, with the journal guaranteeing all-or-nothing semantics across partitions.

This requires changes to the Commonware upstream dependency and is the highest-effort option, but it would eliminate the problem at its root.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/storage/qmdb/src/store.rs` | Modify `apply_batches()` (lines 195-217) to add WAL or commit sequence tracking around the three `write_batch()` calls |
| `crates/storage/backend/src/backend.rs` | Add partition consistency validation method to `CommonwareBackend`; add recovery logic for partial commits |
| `crates/storage/qmdb-ledger/src/ledger.rs` | Update `commit_changes()` (lines 95-110) to integrate with WAL or commit sequence; add `last_committed_root()` metadata query |
| `crates/node/runner/src/runner.rs` | Add partition consistency check in the startup path before `engine.start()` (after `open_stores()` and before `recover_finalized_state()` or after it, depending on the recovery strategy) |

---

## Testing

### 1. Crash injection: kill process mid-commit

```
Test: crash_between_partition_writes_detected_on_recovery
  1. Initialize a 1-validator node with persistent storage (KORA_RUNTIME_DIR on disk, not tmpfs)
  2. Produce 10 blocks with contract deployments and storage writes, persisting each
  3. Produce block 11 with a transaction that modifies accounts, storage, and deploys code
  4. Intercept apply_batches() to:
     a. Allow accounts.write_batch() to complete
     b. Panic / kill before storage.write_batch() begins
  5. Restart the node
  6. Verify: startup partition consistency check detects the mismatch
  7. Verify: node refuses to start OR triggers recovery replay
  8. After recovery: verify computed state root matches expected root for block 11
```

### 2. Property test: all crash points produce recoverable state

```
Test: any_crash_point_in_apply_batches_is_recoverable
  1. For each of the 5 possible crash points in apply_batches():
     - Before accounts.write_batch()
     - After accounts.write_batch(), before storage.write_batch()
     - After storage.write_batch(), before code.write_batch()
     - After code.write_batch() (all committed, WAL not finalized)
     - After WAL finalized (clean commit)
  2. Simulate crash at that point
  3. Restart and run recovery
  4. Verify: all three partitions are at the same logical block height
  5. Verify: computed state root is valid (matches either block N or block N-1)
```

### 3. Integration test: state root consistency after crash recovery

```
Test: state_root_consistent_after_partial_commit_recovery
  1. Produce 20 blocks with diverse transactions (transfers, contract deployments, storage writes)
  2. On block 15, simulate a crash after accounts write completes but before storage write
  3. Restart with recovery
  4. Produce blocks 16-20
  5. Compare state roots with a reference node that did not crash
  6. Verify: all state roots match exactly
```

### 4. Error path test: I/O error on second partition does not corrupt state

```
Test: io_error_on_storage_write_batch_does_not_leave_accounts_ahead
  1. Produce 5 blocks, persist all
  2. Inject an I/O error on storage.write_batch() for block 6
  3. Verify: commit_changes() returns an error
  4. Verify: accounts partition was NOT advanced (if WAL is implemented) OR
     the inconsistency is detected and flagged (if sequence number approach)
  5. Retry commit_changes() with block 6
  6. Verify: all three partitions are consistent and state root is correct
```

### 5. Clean restart does not trigger false positive

```
Test: clean_shutdown_passes_partition_consistency_check
  1. Produce 50 blocks with clean shutdowns between every 10 blocks
  2. On each restart, verify: partition consistency check passes immediately
  3. Verify: no unnecessary recovery or replay is triggered
  4. Verify: startup time is not degraded by the consistency check
```

---

## Verification Steps

After implementing the fix, verify correctness with these manual checks:

1. **Confirm non-atomicity exists today:**
   Review `crates/storage/qmdb/src/store.rs` lines 195-217 and verify there is no transaction wrapping, no WAL, and no commit coordination between the three `write_batch()` calls.

2. **Run existing tests:**
   ```bash
   cargo test -p kora-qmdb
   cargo test -p kora-backend
   cargo test -p kora-qmdb-ledger
   cargo test -p kora-runner
   ```

3. **Manual devnet test:** Start a devnet with persistent storage (`KORA_RUNTIME_DIR` pointing to a disk path). Produce 100 blocks under load, then `kill -9` a validator mid-commit. Restart and verify via logs:
   - The partition consistency check runs on startup
   - If partitions are inconsistent: the check detects the mismatch, recovery runs, and the node starts with consistent state
   - If partitions are consistent: the check passes quickly and the node starts normally

4. **Verify state root agreement:** After crash recovery, confirm that the recovered node produces the same state root as other validators for the next finalized block. Disagreement indicates the recovery did not fully restore consistency.
