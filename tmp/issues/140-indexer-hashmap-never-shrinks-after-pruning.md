# 140: Indexer HashMaps Never Shrink After Pruning -- Memory Fragmentation

**Category**: performance
**Severity**: medium
**Component**: storage / indexer

## Summary

The `BlockIndex` in-memory indexer uses five `HashMap`s to store blocks, transactions, receipts, and logs. The `prune_before()` method removes old entries from these maps but never calls `shrink_to_fit()`, so the maps retain their peak-capacity bucket arrays indefinitely. On a long-running node, this means steady-state memory usage is roughly double what the retained data window actually requires.

## Problem

Kora's in-memory block index (`BlockIndex`) retains the last 10,000 blocks of data in five separate `HashMap`s. The `prune_before()` method (line 121) iterates through these maps and removes entries older than a given block number. However, Rust's `HashMap::remove()` does not shrink the underlying allocation. After a large pruning pass, the hash maps keep their peak bucket arrays allocated.

For example, if the node has indexed 100,000 transactions over time and then prunes down to 10,000, each `HashMap` still holds a power-of-2 bucket array sized for 100,000+ entries (131,072 buckets minimum). This wastes at least 128 KB of pointer storage per map, plus the overhead of the hash table metadata. With five maps, this adds up.

The problem is compounded because the pruned entries (`IndexedBlock`, `IndexedTransaction`, `IndexedReceipt`) contain heap-allocated fields like `Bytes`, `U256`, and `Vec<B256>`. While the `Vec`/`Bytes` data itself is freed when entries are removed, the bucket array holding the now-empty slots remains allocated.

**File**: `crates/storage/indexer/src/store.rs`

## Code Reference

```rust
// crates/storage/indexer/src/store.rs:121-174
pub fn prune_before(&self, min_block_number: u64) {
    // Phase 1: collect block numbers, hashes, and tx hashes to prune
    // under short-lived read locks.
    let hashes_to_remove: Vec<(u64, B256)> = {
        let by_number = self.blocks_by_number.read();
        by_number
            .iter()
            .filter(|(num, _)| **num < min_block_number)
            .map(|(num, hash)| (*num, *hash))
            .collect()
    };

    if hashes_to_remove.is_empty() {
        return;
    }

    let tx_hashes: Vec<B256> = { /* ... */ };

    // Phase 2: remove block-level entries under write locks.
    {
        let mut by_number = self.blocks_by_number.write();
        let mut by_hash = self.blocks_by_hash.write();
        let mut logs = self.logs_by_block.write();
        for &(num, hash) in &hashes_to_remove {
            by_number.remove(&num);
            by_hash.remove(&hash);
            logs.remove(&hash);
        }
        // BUG: No shrink_to_fit() called on any map after bulk removal
    }

    // Phase 3: remove transaction-level entries under write locks.
    {
        let mut txs = self.transactions.write();
        let mut rcpts = self.receipts.write();
        for h in &tx_hashes {
            txs.remove(h);
            rcpts.remove(h);
        }
        // BUG: No shrink_to_fit() called on any map after bulk removal
    }

    debug!(
        min_block_number,
        pruned_blocks = hashes_to_remove.len(),
        pruned_txs = tx_hashes.len(),
        "pruned old index entries",
    );
}
```

## Impact

On a long-running Kora node producing ~33 blocks/second, the indexer retains 10,000 blocks (about 5 minutes of history). Over time, the HashMap bucket arrays grow to accommodate the peak number of entries ever stored, and never release that memory. This results in:

1. **Steady-state memory overhead**: Approximately 2x the memory needed for the retained window, wasted on empty hash table buckets.
2. **Memory-constrained environments**: On devnet nodes with 4 GB RAM limits, this unnecessary allocation reduces the headroom available for QMDB caches and execution.
3. **Compounding effect**: Each of the five maps independently retains its peak allocation, so the waste multiplies across `blocks_by_hash`, `blocks_by_number`, `transactions`, `receipts`, and `logs_by_block`.

## Root Cause

Rust's `HashMap::remove()` and `retain()` do not shrink the underlying bucket array. The `prune_before()` method removes entries but never calls `shrink_to_fit()` to release the excess capacity back to the allocator.

## Suggested Fix

Add `shrink_to_fit()` calls after each bulk removal phase:

```rust
// After Phase 2:
{
    let mut by_number = self.blocks_by_number.write();
    let mut by_hash = self.blocks_by_hash.write();
    let mut logs = self.logs_by_block.write();
    for &(num, hash) in &hashes_to_remove {
        by_number.remove(&num);
        by_hash.remove(&hash);
        logs.remove(&hash);
    }
    by_number.shrink_to_fit();
    by_hash.shrink_to_fit();
    logs.shrink_to_fit();
}

// After Phase 3:
{
    let mut txs = self.transactions.write();
    let mut rcpts = self.receipts.write();
    for h in &tx_hashes {
        txs.remove(h);
        rcpts.remove(h);
    }
    txs.shrink_to_fit();
    rcpts.shrink_to_fit();
}
```

Alternatively, pre-allocate with `HashMap::with_capacity(MAX_RETAINED_BLOCKS)` in `BlockIndex::new()` to avoid repeated grow/shrink cycles, and only call `shrink_to_fit()` when the map's capacity exceeds 2x its length.

## Files to Modify

- `crates/storage/indexer/src/store.rs` -- add `shrink_to_fit()` calls in `prune_before()` (lines 146-166)

## Related Issues

- [010-block-fees-hashmap-unbounded.md](./010-block-fees-hashmap-unbounded.md) -- similar unbounded HashMap growth pattern in a different component
- [030-stale-storage-slots-unbounded-disk.md](./030-stale-storage-slots-unbounded-disk.md) -- related unbounded growth issue in storage

## Labels

`performance`, `storage`, `good first issue`
