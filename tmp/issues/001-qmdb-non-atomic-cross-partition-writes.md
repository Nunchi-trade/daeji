# 001: Non-Atomic Cross-Partition QMDB Writes Can Permanently Brick Node

**Category:** bug / storage
**Severity:** critical
**Labels:** bug, reliability, correctness, storage, recovery

---

## Summary

Kora's QMDB storage engine writes state changes to three independent partitions (accounts, storage, code) sequentially. If the process crashes between any two of these writes, the database enters a permanently inconsistent state. The system detects this inconsistency on restart but provides no recovery mechanism, leaving the node bricked and requiring full reprovisioning from scratch.

## Problem

The `QmdbStore::apply_batches()` method at `crates/storage/qmdb/src/store.rs:334-366` writes to three partitions one after another in sequence. Before writing, a monotonically increasing commit sequence number (`commit_seq + 1`) is injected into each partition's batch as a sentinel key. After all three writes succeed, the in-memory `commit_seq` is advanced.

If a crash occurs between any two partition writes (e.g., after accounts is written but before storage or code), the sentinel values will differ across partitions. On the next startup, the `read_partition_commit_seqs()` method (line 390) reads these sentinels back and `PartitionCommitSeqs::is_consistent()` (line 84) detects the mismatch. The backend's `verify_partition_consistency()` at `crates/storage/backend/src/backend.rs:142-160` then returns `BackendError::InconsistentPartitions`, which causes the node to abort startup. There is no automated recovery path -- the node is permanently bricked.

The three sentinel keys used for this detection are:
- `COMMIT_SEQ_ACCOUNT_KEY` (line 17): `0xFFFF...FFFE` address in accounts partition
- `COMMIT_SEQ_STORAGE_KEY` (line 26): address `0xFFFF...FFFE` with generation `u64::MAX` and slot `U256::MAX` in storage partition
- `COMMIT_SEQ_CODE_KEY` (line 32): `0xFFFF...FFFE` code hash in code partition

## Code Reference

**Sequential partition writes in `apply_batches()` -- `crates/storage/qmdb/src/store.rs:334-366`:**

```rust
pub async fn apply_batches(&mut self, batches: StoreBatches) -> Result<(), QmdbError> {
    let next_seq = self.commit_seq.saturating_add(1);
    let stores = self.stores_mut()?;

    // Inject commit sequence markers into each partition batch.
    let mut account_ops = batches.accounts;
    account_ops.push((COMMIT_SEQ_ACCOUNT_KEY, Some(encode_commit_seq_account(next_seq))));

    let mut storage_ops = batches.storage;
    storage_ops.push((COMMIT_SEQ_STORAGE_KEY, Some(U256::from(next_seq))));

    let mut code_ops = batches.code;
    code_ops.push((COMMIT_SEQ_CODE_KEY, Some(encode_commit_seq_code(next_seq))));

    stores
        .accounts
        .write_batch(account_ops)
        .await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;

    // CRASH WINDOW: If process dies here, accounts has seq N+1 but storage/code still have seq N

    stores
        .storage
        .write_batch(storage_ops)
        .await
        .map_err(|e| QmdbError::Storage(e.to_string()))?;

    // CRASH WINDOW: accounts and storage have seq N+1 but code still has seq N

    stores.code.write_batch(code_ops).await.map_err(|e| QmdbError::Storage(e.to_string()))?;

    // All three partitions committed successfully; advance the sequence.
    self.commit_seq = next_seq;

    Ok(())
}
```

**Consistency check with no recovery -- `crates/storage/backend/src/backend.rs:142-160`:**

```rust
pub async fn verify_partition_consistency(&self) -> Result<PartitionCommitSeqs, BackendError> {
    let seqs = read_partition_commit_seqs(&self.accounts, &self.storage, &self.code).await?;

    if let Some(msg) = seqs.inconsistency_message() {
        error!(
            accounts_seq = ?seqs.accounts,
            storage_seq = ?seqs.storage,
            code_seq = ?seqs.code,
            "QMDB partition consistency check FAILED"
        );
        return Err(BackendError::InconsistentPartitions(msg));
    }

    info!(
        commit_seq = ?seqs.accounts.unwrap_or(0),
        "QMDB partition consistency check passed"
    );
    Ok(seqs)
}
```

**`is_consistent()` check -- `crates/storage/qmdb/src/store.rs:84-93`:**

```rust
pub const fn is_consistent(&self) -> bool {
    match (self.accounts, self.storage, self.code) {
        // No markers at all -- pre-fix node, skip check.
        (None, None, None) => true,
        // All present and matching.
        (Some(a), Some(s), Some(c)) => a == s && s == c,
        // Mixed presence means inconsistency (or very first commit was partial).
        _ => false,
    }
}
```

## Impact

At 33 blocks/second with QMDB persistence every 256 blocks (configurable via `KORA_CHECKPOINT_INTERVAL` at `crates/node/runner/src/runner.rs:79`), the commit window occurs roughly every 7.7 seconds. While the crash window itself is narrow (the duration of three sequential I/O operations), over weeks of continuous operation the cumulative probability becomes non-trivial, especially during:

- **OOM kills**: 8 of 10 devnet nodes are at 100% of their 4 GB memory limit.
- **Power failures**: Bare-metal deployments (Hetzner) have no UPS protection.
- **Docker stop**: The default grace period is 5 seconds, which may not be enough for in-flight writes to complete.

A bricked node requires a full state re-sync from genesis or a backup restore. At 470K+ blocks, this is increasingly expensive and time-consuming.

## Root Cause

The three QMDB partitions are independent commonware-storage instances. There is no write-ahead log (WAL) or two-phase commit protocol to make the cross-partition write appear atomic. The commit sequence markers provide crash **detection** but not crash **recovery**.

## Suggested Fix

**Option 1 -- Write-Ahead Log (WAL)** (recommended short-term fix):

Before writing to any partition, serialize all three partition batches to a single WAL file and `fsync` it. Then apply the batches. On recovery, replay any incomplete WAL entries to bring all partitions to the same state:

```rust
// Pseudocode for apply_batches with WAL
async fn apply_batches(&mut self, batches: StoreBatches) -> Result<(), QmdbError> {
    // 1. Write all three batches to a single WAL file and fsync
    let wal_entry = WalEntry::new(next_seq, &account_ops, &storage_ops, &code_ops);
    wal_entry.write_and_fsync(&self.wal_path)?;

    // 2. Apply to each partition (idempotent replay is safe)
    self.apply_accounts(account_ops).await?;
    self.apply_storage(storage_ops).await?;
    self.apply_code(code_ops).await?;

    // 3. Remove WAL entry after all writes succeed
    wal_entry.remove()?;
    self.commit_seq = next_seq;
    Ok(())
}
```

**Option 2 -- Recovery on mismatch** (immediate mitigation):

When `is_consistent()` returns false, identify which partition(s) are behind and replay the missing sentinel writes to bring all partitions to the same sequence number.

**Option 3 -- Single-partition design** (long-term):

Combine all three data types into a single commonware-storage partition with a composite key scheme, making each commit inherently atomic.

## Files to Modify

- `crates/storage/qmdb/src/store.rs` -- `apply_batches()` (lines 334-366): Add WAL write before partition writes
- `crates/storage/qmdb/src/store.rs` -- Add WAL replay logic for crash recovery
- `crates/storage/backend/src/backend.rs` -- `verify_partition_consistency()` (lines 142-160): Add recovery path instead of returning fatal error

## Related Issues

- [010 -- block_fees HashMap Grows Without Bound](./010-block-fees-hashmap-unbounded.md) -- Another storage-related concern
- [005 -- Memory Exhaustion](./005-memory-exhaustion-devnet.md) -- OOM kills trigger the crash window
