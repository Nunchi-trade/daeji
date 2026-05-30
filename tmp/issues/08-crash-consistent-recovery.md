# Storage: No crash-consistent recovery or state validation on startup

**Severity:** High
**Component:** `crates/node/runner/src/runner.rs`, `crates/node/ledger/src/lib.rs`, `crates/storage/qmdb-ledger/src/ledger.rs`
**Affects:** All Kora validator nodes that restart after a crash or unclean shutdown

---

## Summary

Kora is a blockchain validator node that maintains two independent persistence layers: **QMDB** (a custom key-value state database for account/storage/code state) and **Commonware archives** (append-only logs of finalized blocks and finalization certificates). When a Kora node restarts after a crash, the recovery procedure in `recover_finalized_state()` (`crates/node/runner/src/runner.rs`, lines 170-228) reads the finalization archive to find the last finalized block, then creates a minimal snapshot pointing to the current QMDB state. It does **not** verify that QMDB's state root matches the archived block's state root, nor does it replay any blocks to bring QMDB up to date if it lagged behind the archive at crash time.

A crash during `commit_changes()` can leave QMDB in a partially-applied state. The node will restart, blindly trust QMDB's current state, and begin producing blocks with an incorrect state root. This leads to **silent state divergence** from other validators, which may not be detected until a state root mismatch causes the node to be slashed or excluded from consensus.

---

## Background: Kora's Persistence Architecture

Kora has three distinct layers of state:

### 1. QMDB (durable, on-disk)
- Location: `crates/storage/qmdb-ledger/src/ledger.rs`, `crates/storage/qmdb/src/store.rs`, `crates/storage/handlers/src/qmdb.rs`
- Stores: account state (nonce, balance, code_hash, generation), contract storage slots, contract code
- Three separate Commonware-backed partitions: accounts, storage, code
- State root: computed as `StateRoot::compute(accounts_root, storage_root, code_root)` in `crates/storage/qmdb/src/root.rs` (lines 16-23) -- a keccak256 hash of `b"_KORA_QMDB_ROOT"` concatenated with the three partition Merkle roots
- Commits are done via `QmdbLedger::commit_changes()` (lines 95-110 of `crates/storage/qmdb-ledger/src/ledger.rs`):

```rust
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

The `commit_changes()` method on `QmdbStore` (in `crates/storage/qmdb/src/store.rs`, lines 224-230) builds batches and applies them:

```rust
pub async fn commit_changes(&mut self, changes: ChangeSet) -> Result<(), QmdbError> {
    if changes.is_empty() {
        return Ok(());
    }
    let batches = self.build_batches(&changes).await?;
    self.apply_batches(batches).await
}
```

The `apply_batches()` method (lines 195-217) writes to three separate stores sequentially:

```rust
pub async fn apply_batches(&mut self, batches: StoreBatches) -> Result<(), QmdbError> {
    let stores = self.stores_mut()?;
    stores.accounts.write_batch(batches.accounts).await...?;
    stores.storage.write_batch(batches.storage).await...?;
    stores.code.write_batch(batches.code).await...?;
    Ok(())
}
```

**This is not atomic.** A crash between `accounts.write_batch()` and `storage.write_batch()` leaves QMDB in a state where account data has been updated but storage slots have not. There is no write-ahead log or transaction journal wrapping the three writes.

Each individual partition is backed by Commonware's journaled Merkle tree storage (`commonware-storage`), which provides crash recovery at the **individual partition level** -- if a single `write_batch()` is interrupted, the partition's journal can replay uncommitted entries on next startup. However, **cross-partition consistency** after a partial commit (e.g., accounts written but storage not) is NOT guaranteed by Commonware. The three partitions have independent journals and there is no two-phase commit protocol coordinating them.

### 2. Commonware Archives (durable, on-disk)
- Finalized blocks archive: `{partition_prefix}-finalized-blocks`
- Finalization certificates archive: `{partition_prefix}-finalizations-by-height`
- Managed by Commonware's `Archive` trait, which provides append-only storage with `get()` by index or key
- Blocks are archived by the Marshal layer (Commonware's block delivery pipeline) independently of QMDB commits

### 3. In-Memory Snapshots (volatile, lost on crash)
- `InMemorySnapshotStore<OverlayState<QmdbState>>` -- all snapshots are lost on crash
- `InMemorySeedTracker` -- all seed-to-digest mappings are lost on crash
- `InMemoryMempool` -- all pending transactions are lost on crash

**The critical invariant:** After finalization, the archive records block N as finalized, and QMDB should contain the state resulting from executing blocks 0 through N. If QMDB is behind (e.g., it only committed through block N-2), the node has a consistency gap.

**Docker devnet caveat:** On the devnet, `KORA_RUNTIME_DIR` is set to `/runtime` which is a tmpfs mount (`docker/compose/devnet.yaml`). All Commonware-managed storage -- including QMDB partitions, the finalized blocks archive, finalization certificates archive, and consensus journals -- resides on this tmpfs because they are all initialized through the Commonware runtime context that uses this directory. A container restart wipes ALL state, making recovery irrelevant (the node starts from genesis). This means the crash-recovery scenarios described below only apply to production deployments where `KORA_RUNTIME_DIR` points to persistent storage (the default is `{data_dir}/runtime`).

**Production implication:** In a production deployment with persistent storage, the three partition writes within `apply_batches()` are each independently journaled by Commonware's storage layer. If a single `write_batch()` is interrupted mid-write, that partition's journal can recover on restart. However, a crash between partition writes (e.g., accounts committed, storage not) leaves the database in a state where one partition is ahead of the others, and there is no cross-partition recovery mechanism.

---

## Current Recovery Mechanism

On startup, `ProductionRunner::run()` in `crates/node/runner/src/runner.rs` (lines 329-578) calls `recover_finalized_state()` (lines 170-228):

```rust
async fn recover_finalized_state<FB, FC>(
    ledger: &LedgerService,
    block_index: Option<&Arc<BlockIndex>>,
    finalized_blocks: &FB,
    finalizations_by_height: &FC,
    provider: &RevmContextProvider,
) -> anyhow::Result<()>
where
    FB: Archive<Key = ConsensusDigest, Value = Block>,
    FC: Archive<Key = ConsensusDigest, Value = CertArchive>,
{
    let block_ranges: Vec<_> = finalized_blocks.ranges().collect();
    let finalization_ranges: Vec<_> = finalizations_by_height.ranges().collect();

    // Step 1: Replay all seeds from finalization archive
    for (start, end) in finalization_ranges {
        for height in start..=end {
            if let Some(finalization) = finalizations_by_height
                .get(ArchiveId::Index(height)).await...?
            {
                ledger.set_seed(
                    finalization.proposal.payload,
                    seed_hash(finalization.seed())
                ).await;
            }
        }
    }

    // Step 2: Iterate all archived blocks, keep the last one
    let mut recovered = 0u64;
    let mut head = None;
    for (start, end) in block_ranges {
        for height in start..=end {
            let Some(block) = finalized_blocks
                .get(ArchiveId::Index(height)).await...?
            else { continue };

            if let Some(index) = block_index {
                index_recovered_block(index, &block, provider);
            }
            head = Some(block);
            recovered += 1;
        }
    }

    // Step 3: Create a minimal snapshot for the last archived block
    if let Some(head) = head {
        ledger.restore_persisted_snapshot(&head).await;
        info!(height = head.height, blocks = recovered,
              "recovered finalized ledger head from archive");
    }

    Ok(())
}
```

The `restore_persisted_snapshot()` method in `crates/node/ledger/src/lib.rs` (lines 239-252) creates a snapshot with an **empty ChangeSet** pointing to the current QMDB state:

```rust
pub async fn restore_persisted_snapshot(&self, block: &Block) {
    let inner = self.inner.lock().await;
    let digest = block.commitment();
    let state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
    let snapshot = Snapshot::new(
        Some(block.parent()),
        state,
        block.state_root,         // Uses the ARCHIVED block's state root
        QmdbChangeSet::default(), // Empty changes -- trusts QMDB is correct
        tx_ids(&block.txs),
    );
    inner.snapshots.insert(digest, snapshot);
    inner.snapshots.mark_persisted(&[digest]);
}
```

**What this does NOT do:**
1. It does NOT compute the current QMDB root and compare it to `block.state_root`
2. It does NOT verify that QMDB actually contains the state described by the archived block
3. It does NOT replay any blocks from the archive through execution to bring QMDB up to date
4. It does NOT check whether QMDB's committed height matches the archive's finalized height
5. It blindly sets `state_root` to the archived block's value, even if QMDB's actual root is different

---

## The Reverted Commit

Commit `e17a728` ("feat(ledger): add restore persisted digest functionality") attempted to address a related problem -- when finalization certificates exist in the archive but finalized blocks do not (a scenario that can occur if the blocks archive was corrupted or truncated). It added a `restore_persisted_digest()` method that created a snapshot from the finalization digest and marked it as persisted.

This commit was **reverted** in `6462dad` with the message "Revert feat(ledger): add restore persisted digest functionality". The revert was correct: blindly marking a digest as persisted without validating that QMDB actually contains the corresponding state is dangerous. The reverted code would have created a "persisted" snapshot with `parent: None` and an empty `tx_ids` set, which would break `merged_changes()` traversal for any subsequent block that tried to walk back through the persisted chain.

However, the revert did not solve the underlying problem -- it just removed an incorrect fix. The core issue remains: **there is no mechanism to validate QMDB state against the archive on startup**.

---

## What Can Go Wrong

### Scenario 1: Crash During commit_changes()

1. Block N is finalized. The `FinalizedReporter` calls `persist_snapshot(digest)`.
2. `persist_snapshot()` calls `qmdb.commit_changes(changes)` (line 307 of `crates/node/ledger/src/lib.rs`).
3. Inside `QmdbStore::apply_batches()`, `accounts.write_batch()` succeeds but the process crashes before `storage.write_batch()` completes.
4. QMDB now has updated account nonces/balances for block N but stale storage slots.
5. On restart, `recover_finalized_state()` finds block N in the archive and calls `restore_persisted_snapshot(&block_N)`.
6. The snapshot is created with `state_root = block_N.state_root` but QMDB's actual state root is different (because storage was not fully committed).
7. The node begins participating in consensus. When it proposes or verifies block N+1, it executes transactions against the corrupted QMDB state, producing a different state root than honest validators.
8. **Silent state divergence.** The node may continue producing blocks that pass local verification but fail verification on other validators.

### Scenario 2: Archive Ahead of QMDB

1. Block N is finalized by consensus. The Marshal layer archives block N and its finalization certificate.
2. The `FinalizedReporter` begins persisting block N to QMDB but the process crashes before `commit_changes()` is called (e.g., during the execution verification step in `handle_finalized_update()`, lines 133-147 of `crates/node/reporters/src/lib.rs`).
3. The archive says block N is finalized, but QMDB only contains state through block N-1 (or earlier).
4. On restart, `restore_persisted_snapshot()` creates a snapshot for block N pointing to QMDB state that only reflects block N-1.
5. The node treats QMDB's N-1 state as if it were N's state. Any account touched by block N's transactions will have stale values.

### Scenario 3: Multiple Blocks Behind

1. The node falls behind during a period of rapid finalization (e.g., block delivery is backed up).
2. The archive records blocks N, N+1, N+2 as finalized, but QMDB has only committed through block N-2.
3. The node crashes.
4. On restart, `restore_persisted_snapshot()` is called with the last archived block (N+2), creating a snapshot pointing to QMDB state from block N-2.
5. The node is now 4 blocks behind in QMDB state but believes it is at block N+2.

### Consequence: Undetectable Until Too Late

The `FinalizedReporter` does perform a state root check when re-executing finalized blocks (lines 160-169 of `crates/node/reporters/src/lib.rs`):

```rust
if state_root != block.state_root {
    warn!(?digest, expected = ?block.state_root, computed = ?state_root,
          "state root mismatch for finalized block");
    ack.acknowledge();
    return;
}
```

However, this check only runs inside the `if !snapshot_exists || block_index.is_some()` block (line 124 of `reporters/src/lib.rs`). After `restore_persisted_snapshot()` creates a snapshot and marks it as persisted, `snapshot_exists` returns `true` for that digest. If RPC is disabled (`block_index.is_none()`), the entire re-execution block is skipped and the verification path is never reached. The state root mismatch goes undetected.

Note: if RPC **is** enabled (`block_index.is_some()`), re-execution does occur for indexing purposes and the state root check does run. But RPC is optional, and a validator running without RPC (a reasonable production configuration for non-public validators) will have no verification at all.

---

## Additional Impact: FinalizedReporter Replay Cost

When a finalized block arrives and no cached snapshot exists (`!snapshot_exists` path in `handle_finalized_update()`, lines 124-186 of `crates/node/reporters/src/lib.rs`), the reporter must:

1. Find the parent snapshot
2. Re-execute all transactions in the block
3. Compute the state root
4. Verify it matches
5. Insert a new snapshot

Without snapshot caching during recovery, every finalized block that arrives during catch-up requires full re-execution. The current recovery procedure in `recover_finalized_state()` only creates a snapshot for the **last** archived block, not for intermediate blocks. This means that if the node restarts and consensus delivers a batch of finalized blocks, each one triggers a full re-execution chain starting from the recovery point.

---

## Proposed Fixes

### Fix 1: Persist Last-Committed Digest Alongside QMDB

Add a small metadata record that is written atomically with each QMDB commit, recording the digest of the block whose changes were just committed:

```rust
// In QmdbLedger (crates/storage/qmdb-ledger/src/ledger.rs):
pub async fn commit_changes_with_digest(
    &self,
    changes: QmdbChangeSet,
    digest: ConsensusDigest,
) -> Result<StateRoot, Error> {
    let _storage_access = self.handle.storage_access().await;
    let mut store = self.handle.write().await;
    store.commit_changes(changes).await...?;
    // Write digest to a dedicated metadata partition
    store.set_metadata(b"last_committed_digest", &digest.as_ref()).await...?;
    let root = ...; // compute root as before
    Ok(StateRoot(root))
}
```

This requires the metadata write to be part of the same commit batch or to use a separate journal file that is fsynced before returning.

### Important: Transition Roots vs QMDB Roots

Kora uses two different state root computations (see `crates/storage/qmdb/src/root.rs`):

1. **Transition roots** (`StateRoot::transition()`, line 26): A deterministic keccak256 hash of `b"_KORA_STATE_TRANSITION_ROOT"` + parent_root + serialized_changes. Used during consensus for agreement before QMDB commits. This is what appears in block headers (`block.state_root`).

2. **QMDB roots** (`StateRoot::compute()`, line 16): A keccak256 hash of `b"_KORA_QMDB_ROOT"` + the three partition Merkle roots. Computed after QMDB commits via `QmdbLedger::commit_changes()` (line 104 of `crates/storage/qmdb-ledger/src/ledger.rs`).

**These two roots are NOT the same value** for the same state. Block headers contain transition roots, but QMDB stores Merkle roots. The current `recover_finalized_state()` uses `block.state_root` (a transition root) in the restored snapshot, not the QMDB Merkle root. This means a recovery fix that compares `qmdb.root()` to `block.state_root` would need to account for this difference -- they will never match by construction. Instead, the fix should either:
- Store the transition root alongside the QMDB commit as metadata
- Or re-derive the transition root from the committed state and compare

### Fix 2: Validate QMDB State on Startup Using Last-Committed Digest

Because `block.state_root` contains a transition root (not a QMDB Merkle root), direct comparison between `qmdb.root()` and `block.state_root` is not possible. Instead, use the last-committed digest (from Fix 1) to determine whether QMDB is up to date:

```rust
async fn recover_finalized_state(...) -> anyhow::Result<()> {
    // ... existing seed and block iteration ...

    if let Some(head) = head {
        // NEW: Check if QMDB is behind the archive
        let last_committed = ledger.last_committed_digest().await;
        let head_digest = head.commitment();

        if last_committed != Some(head_digest) {
            let committed_height = match last_committed {
                Some(d) => find_height_of_digest(&finalized_blocks, &d).await?,
                None => 0, // QMDB has no commit marker; replay from genesis
            };
            warn!(
                archived_height = head.height,
                qmdb_committed_height = committed_height,
                "QMDB is behind archive; replaying missing blocks"
            );
            // Replay blocks from committed_height+1 through head
            replay_blocks(
                ledger, &finalized_blocks, committed_height + 1,
                head.height, provider
            ).await?;
        }

        ledger.restore_persisted_snapshot(&head).await;
        info!(height = head.height, blocks = recovered,
              "recovered finalized ledger head from archive");
    }

    Ok(())
}
```

### Fix 3: Replay Missing Blocks From Archive

When a mismatch is detected, replay the missing blocks through full execution:

```rust
async fn replay_blocks(
    ledger: &LedgerService,
    finalized_blocks: &impl Archive<Key = ConsensusDigest, Value = Block>,
    from_height: u64,
    to_height: u64,
    provider: &RevmContextProvider,
) -> anyhow::Result<()> {
    let executor = RevmExecutor::new(chain_id);

    for height in from_height..=to_height {
        let block = finalized_blocks
            .get(ArchiveId::Index(height)).await?
            .context("missing archived block during replay")?;

        let parent_digest = block.parent();
        let parent_snapshot = ledger.parent_snapshot(parent_digest).await
            .context("missing parent snapshot during replay")?;

        let block_context = provider.context(&block);
        let outcome = executor.execute(
            &parent_snapshot.state, &block_context, &block.tx_bytes()
        )?;

        let state_root = ledger.compute_root_from_store(
            parent_digest, outcome.changes.clone()
        ).await?;

        anyhow::ensure!(
            state_root == block.state_root,
            "state root mismatch during replay at height {height}"
        );

        // Insert snapshot and persist
        ledger.insert_snapshot(
            block.commitment(), parent_digest,
            /* new overlay state */, state_root,
            outcome.changes, &block.txs,
        ).await;
        ledger.persist_snapshot(block.commitment()).await?;
    }

    Ok(())
}
```

### Fix 4: Add Post-Recovery Verification Step

After recovery completes and before the node joins consensus, add a final verification step. Since QMDB Merkle roots and block transition roots are in different namespaces and cannot be directly compared, the verification should:

1. Read the last-committed digest from QMDB metadata (from Fix 1)
2. Confirm it matches the archive head digest
3. Optionally re-execute the last block to verify the transition root matches

```rust
// After recover_finalized_state() returns:
let last_committed = ledger.last_committed_digest().await;
let head_digest = head.commitment();

anyhow::ensure!(
    last_committed == Some(head_digest),
    "post-recovery digest mismatch: QMDB last committed={last_committed:?}, \
     archive head={head_digest:?}. QMDB may be corrupted or behind."
);

// Optional: re-execute head block and verify transition root
let parent = ledger.parent_snapshot(head.parent()).await
    .context("missing parent snapshot after recovery")?;
let outcome = executor.execute(&parent.state, &provider.context(&head), &head.tx_bytes())?;
let computed_root = ledger.compute_root_from_store(head.parent(), outcome.changes).await?;
anyhow::ensure!(
    computed_root == head.state_root,
    "post-recovery execution mismatch at height {}: computed={computed_root:?}, \
     expected={:?}", head.height, head.state_root
);

info!("post-recovery state validation passed");
```

This should be a hard failure that prevents the node from starting if validation fails, since participating in consensus with incorrect state would cause the node to produce invalid blocks.

---

## Pseudocode for Safe Recovery

```
fn safe_recover(ledger, archives, executor, provider):
    // 1. Load archive metadata
    let archived_head = archives.blocks.last()
    let archived_certs = archives.finalizations.last()

    if archived_head is None:
        return  // Fresh node, no recovery needed

    // 2. Replay all seeds (needed for prevrandao)
    for cert in archives.finalizations.iter():
        ledger.set_seed(cert.digest, hash(cert.seed))

    // 3. Read QMDB's last-committed digest (from Fix 1 metadata)
    let qmdb_digest = ledger.last_committed_digest()  // NEW metadata

    // 4. Find divergence point
    let replay_start = if qmdb_digest is Some(d):
        // Find the height of the last committed block
        archives.blocks.height_of(d) + 1
    else:
        // QMDB has no commit marker; must replay from genesis
        0

    // NOTE: We cannot compare qmdb.root() to block.state_root directly
    // because blocks use transition roots (StateRoot::transition) while
    // QMDB stores Merkle roots (StateRoot::compute). These are in
    // different hash namespaces and will never match for non-genesis blocks.

    // 5. If last-committed digest exists, verify QMDB integrity
    //    by re-executing the last committed block and checking the
    //    transition root matches the archived block's state_root
    if replay_start > 0:
        let last_block = archives.blocks.get(replay_start - 1)
        let parent_block = archives.blocks.get(replay_start - 2)  // or genesis
        // Re-execute to verify transition root
        let outcome = executor.execute(parent_state, last_block)
        let root = compute_transition_root(parent_root, outcome.changes)
        if root != last_block.state_root:
            ERROR("QMDB state is corrupted at height {replay_start - 1}")
            // Option A: Wipe QMDB and replay from genesis
            // Option B: Binary search for last valid height
            replay_start = binary_search_valid_height(archives, ledger)

    // 6. Replay missing blocks
    for height in replay_start..=archived_head.height:
        let block = archives.blocks.get(height)
        let parent = ledger.parent_snapshot(block.parent())
        let outcome = executor.execute(parent.state, block)
        let root = ledger.compute_root(block.parent(), outcome.changes)
        ASSERT(root == block.state_root,
               "replay mismatch at height {height}")
        ledger.insert_snapshot(block.digest(), ...)
        ledger.persist_snapshot(block.digest())

    // 7. Final validation: verify last committed digest matches archive head
    let final_digest = ledger.last_committed_digest()
    ASSERT(final_digest == Some(archived_head.digest()),
           "post-recovery commit marker mismatch")

    // 8. Mark recovery complete
    ledger.restore_persisted_snapshot(archived_head)
    log("recovery complete at height {}", archived_head.height)
```

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/runner/src/runner.rs` | Update `recover_finalized_state()` (lines 170-228) to read last-committed digest, compare to archive head, and replay missing blocks if QMDB is behind |
| `crates/node/ledger/src/lib.rs` | Add `last_committed_digest()` method to `LedgerView`/`LedgerService`; update `persist_snapshot()` to record the committed digest in QMDB metadata |
| `crates/storage/qmdb-ledger/src/ledger.rs` | Optionally add `commit_changes_with_digest()` method that persists the last-committed block digest alongside the QMDB commit (Fix 1) |
| `crates/storage/qmdb/src/store.rs` | (Read-only reference) Understand `apply_batches()` non-atomicity at lines 195-217 |
| `crates/node/reporters/src/lib.rs` | (Read-only reference) Understand the state root check at lines 160-169 of `handle_finalized_update()` and why it can be bypassed when `snapshot_exists && !block_index.is_some()` |

## Verification Steps

After implementing the fix, verify correctness with these manual checks:

1. **Pre-fix: Confirm no state validation exists today:**
   ```bash
   grep -n "qmdb_root\|state_root.*match\|state_root.*mismatch" crates/node/runner/src/runner.rs
   # Should show no matches in recover_finalized_state()
   ```

2. **After implementing, run all existing tests:**
   ```bash
   cargo test -p kora-runner
   cargo test -p kora-ledger
   cargo test -p kora-qmdb-ledger
   ```

3. **Manual integration test:** Start a devnet with persistent storage (set `KORA_RUNTIME_DIR` to a disk path, not tmpfs). Produce 100 blocks, then `kill -9` a validator. Restart it and verify via logs:
   - The node logs the QMDB root it found on startup
   - The node compares it against the archive head's state root
   - If they match: recovery completes without replay
   - If they differ: recovery replays missing blocks and logs the replay

---

## Testing Plan

### 1. Simulate crash mid-persist, verify recovery replays missing blocks

```
Test: crash_during_commit_changes_recovers_correctly
  1. Initialize a 1-validator devnet with QMDB and archives
  2. Produce 10 blocks with transfers, persisting each
  3. Produce block 11 but intercept commit_changes() to:
     a. Write accounts batch successfully
     b. Panic before storage batch completes
  4. Restart the node with safe recovery
  5. Verify: recovery detects last-committed digest does not match block 11's digest
  6. Verify: recovery replays block 11 from archive
  7. Verify: post-recovery last-committed digest matches block 11's digest
  8. Verify: node can produce block 12 with correct state
```

### 2. Simulate archive ahead of QMDB

```
Test: archive_ahead_of_qmdb_triggers_replay
  1. Produce 10 blocks, persist all to QMDB
  2. Produce blocks 11-15, archive them but skip QMDB commits
  3. Restart with safe recovery
  4. Verify: recovery detects last committed = block 10, archive head = block 15
  5. Verify: blocks 11-15 are replayed and committed to QMDB
  6. Verify: final last-committed digest matches block 15's digest
```

### 3. Verify state root validation prevents startup with corrupted QMDB

```
Test: corrupted_qmdb_prevents_startup
  1. Produce 10 blocks, persist all
  2. Manually corrupt QMDB (e.g., modify an account balance)
  3. Restart with safe recovery
  4. Verify: post-recovery validation detects root mismatch
  5. Verify: node refuses to start (returns error, does not join consensus)
```

### 4. Verify clean restart does not trigger unnecessary replay

```
Test: clean_restart_skips_replay
  1. Produce 10 blocks, persist all, shut down cleanly
  2. Restart with safe recovery
  3. Verify: last-committed digest matches archive head digest, no replay occurs
  4. Verify: recovery completes in O(1) time (reads last-committed digest + last archived block only)
```

### 5. Verify recovery with empty archive

```
Test: fresh_node_no_archive_no_replay
  1. Start a fresh node with no archive data
  2. Verify: recovery is a no-op
  3. Verify: genesis snapshot is created normally
  4. Verify: node can produce block 1
```
