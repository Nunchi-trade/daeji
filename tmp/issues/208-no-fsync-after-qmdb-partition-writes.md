# No fsync/flush Guarantee After QMDB Partition Writes

**Category**: bug -- storage, recovery
**Severity**: high

## Summary

Each QMDB partition's `write_batch()` calls `inner.apply_batch(merkleized)` followed by `inner.commit()`, but there is no explicit `fsync()` or `fdatasync()` call in the Kora storage layer. Whether committed data actually reaches disk depends entirely on the commonware-storage implementation (which may or may not fsync internally) and the OS page cache. On Linux with ext4 and default mount options, data can sit in the page cache for up to 30 seconds before being written to disk. A power failure during this window would lose committed blocks even though all three partitions completed successfully.

## Problem

Kora's QMDB backend has three partitions (accounts, storage, code), each implemented as a separate commonware-storage instance. When a block is finalized, each partition's `write_batch()` method writes changes to the underlying storage journal via:

1. `inner.apply_batch(merkleized)` -- applies the merkleized batch to the in-memory state
2. `inner.commit()` -- writes the batch to the journal on disk

However, neither the Kora storage layer nor (as far as can be determined) the commonware-storage `commit()` call explicitly invokes `fsync()` to ensure data reaches the physical disk. The OS may buffer the written data in the page cache indefinitely.

This is distinct from the non-atomic cross-partition write issue (issue #001): that issue is about consistency between partitions, while this issue is about durability of writes to any individual partition.

## Code Reference

File: `crates/storage/backend/src/accounts.rs`, lines 87-108 (AccountStore::write_batch)

```rust
impl QmdbBatchable for AccountStore {
    async fn write_batch<I>(&mut self, ops: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = (Self::Key, Option<Self::Value>)> + Send,
        I::IntoIter: Send,
    {
        let inner = self.inner.take()?;
        let mut batch = inner.new_batch();
        for (address, value) in ops {
            batch = batch.write(account_key(address), value.map(AccountValue));
        }
        let merkleized = batch
            .merkleize(&inner, None)
            .await
            .map_err(|e| BackendError::Storage(e.to_string()))?;
        let mut inner = inner;
        inner.apply_batch(merkleized).await.map_err(|e| BackendError::Storage(e.to_string()))?;
        inner.commit().await.map_err(|e| BackendError::Storage(e.to_string()))?;
        // ^^^ No fsync() after commit
        self.inner.restore(inner);
        Ok(())
    }
}
```

The same pattern appears in the other two partitions:

File: `crates/storage/backend/src/storage.rs`, lines 87-108 (StorageStore::write_batch):
```rust
inner.apply_batch(merkleized).await.map_err(|e| BackendError::Storage(e.to_string()))?;
inner.commit().await.map_err(|e| BackendError::Storage(e.to_string()))?;
// No fsync
```

File: `crates/storage/backend/src/code.rs`, lines 79-100 (CodeStore::write_batch):
```rust
inner.apply_batch(merkleized).await.map_err(|e| BackendError::Storage(e.to_string()))?;
inner.commit().await.map_err(|e| BackendError::Storage(e.to_string()))?;
// No fsync
```

## Impact

1. **Data loss on power failure**: If the host machine loses power (or the kernel panics) within the OS flush window (up to 30 seconds on Linux ext4 with default mount options), committed blocks that consensus considered finalized could be lost. The node would report success to consensus, but the data would not survive a power loss.
2. **Inconsistent state after crash**: If some partitions' data was flushed by the OS but others were not, the node could restart with an inconsistent state across partitions (e.g., accounts updated but storage slots not). The cross-partition commit sequence markers (issue #001) detect this inconsistency after the fact, but they cannot recover the lost data.
3. **Silent corruption**: The node has no way to know whether its data survived a power failure until it detects a partition root mismatch on restart. By that point, the data is irrecoverably lost.

On cloud deployments with battery-backed write caches (common on AWS EBS, GCP PD, Azure managed disks), the practical risk is lower because the storage controller handles durability. On bare-metal deployments (like the current Hetzner devnet), the risk is real.

## Root Cause

The Kora storage layer delegates all I/O to commonware-storage and does not add an explicit `fsync()` after commits. Whether commonware-storage itself calls `fsync()` internally is undocumented and unverified. The default assumption for any database is that `commit()` implies durability, but this is not guaranteed by file system semantics without explicit fsync.

## Suggested Fix

Three options with different performance/durability tradeoffs:

**Option A -- Explicit fsync after every commit (strongest durability, highest cost):**
```rust
inner.commit().await.map_err(|e| BackendError::Storage(e.to_string()))?;
// After commit, fsync the underlying file descriptor
// This requires commonware-storage to expose the file handle or provide a sync() method
```

At 34 blocks/second, fsync on every block would add significant latency (~1-5ms per fsync on SSD). This could reduce throughput.

**Option B -- Periodic fsync (compromise):**
Fsync every N blocks (e.g., every 32 or 64 blocks). This limits the window of potential data loss to ~1-2 seconds while having much less performance impact.

```rust
if block_height % 32 == 0 {
    inner.sync().await?;
}
```

**Option C -- Verify commonware-storage guarantees:**
Before implementing any fix, check whether commonware-storage's `commit()` already calls `fsync()` internally. If it does, this issue is a documentation gap rather than a bug. File an issue or check the commonware source code to confirm.

**Option D -- Configurable sync interval:**
Expose a `sync_interval` configuration parameter that operators can tune:
```toml
[storage]
fsync_interval = 32  # fsync every 32 blocks (0 = every block, -1 = never)
```

## Files to Modify

- `crates/storage/backend/src/accounts.rs` -- Add fsync after `commit()` in `write_batch()` (line 104)
- `crates/storage/backend/src/storage.rs` -- Add fsync after `commit()` in `write_batch()` (line 104)
- `crates/storage/backend/src/code.rs` -- Add fsync after `commit()` in `write_batch()` (line 96)
- Potentially `crates/storage/backend/src/config.rs` -- Add `fsync_interval` config parameter

## Related Issues

- `001-qmdb-non-atomic-cross-partition-writes.md` -- Non-atomic cross-partition writes; this issue extends that concern with durability guarantees
- `020-qmdb-persistence-blocks-finalization.md` -- QMDB persistence blocking finalization pipeline; fsync would add to this latency

## Labels

bug, reliability, storage, recovery
