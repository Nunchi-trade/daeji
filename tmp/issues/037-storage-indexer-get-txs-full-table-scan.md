# Indexer get_transactions_for_block Is O(total_txs) Full Table Scan

**Category**: Performance -- Storage
**Severity**: Medium
**Labels**: `performance`, `storage`, `rpc`

## Summary

The `BlockIndex::get_transactions_for_block()` method retrieves transactions for a block by scanning the entire `transactions` HashMap and filtering by `block_hash`. This is a full table scan that grows linearly with the total number of indexed transactions across all blocks, even though the `IndexedBlock` struct already stores a `transaction_hashes` list that could be used for targeted O(txs_in_block) lookups.

## Problem

Kora is an EVM execution client that maintains an in-memory block index (`BlockIndex` in `crates/storage/indexer/src/store.rs`) for serving RPC queries. The index retains up to 10,000 blocks (about 5 minutes of history at 34 blocks/s) and stores all transactions in a flat `HashMap<B256, IndexedTransaction>` keyed by transaction hash.

The `get_transactions_for_block()` method is called when `eth_getBlockByHash` or `eth_getBlockByNumber` is invoked with `full_transactions=true`. It iterates over every entry in the `transactions` map, comparing each transaction's `block_hash` field against the requested hash. This is O(total_txs) where `total_txs` is the number of all indexed transactions across all retained blocks.

The `IndexedBlock` struct already stores a `transaction_hashes: Vec<B256>` field (visible in the test helper at line 340), but `get_transactions_for_block()` does not use it. The efficient approach would be to look up the block first, get its `transaction_hashes` list, and then look up each transaction by hash from the `transactions` map.

## Code Reference

`crates/storage/indexer/src/store.rs:193-204`:
```rust
/// Gets all indexed transactions for a block in transaction-index order.
pub fn get_transactions_for_block(&self, block_hash: &B256) -> Vec<IndexedTransaction> {
    let mut txs = self
        .transactions
        .read()
        .values()
        .filter(|tx| tx.block_hash == *block_hash)  // O(total_txs) filter
        .cloned()
        .collect::<Vec<_>>();
    txs.sort_by_key(|tx| tx.index);
    txs
}
```

The `IndexedBlock` struct has a `transaction_hashes` field (from test helper at `crates/storage/indexer/src/store.rs:340`):
```rust
fn create_test_block(number: u64, hash: B256) -> IndexedBlock {
    IndexedBlock {
        hash,
        number,
        // ...
        transaction_hashes: vec![],  // This field exists but is unused by get_transactions_for_block
    }
}
```

The `blocks_by_hash` map is available for looking up the block first (`crates/storage/indexer/src/store.rs:20`):
```rust
blocks_by_hash: RwLock<HashMap<B256, IndexedBlock>>,
```

## Impact

With the default retention of 10,000 blocks and an average of 100 transactions per block under load, the `transactions` map holds 1,000,000 entries. Every `eth_getBlockByNumber` or `eth_getBlockByHash` call with `full_transactions=true` scans and filters all of them, even though only approximately 100 belong to the requested block. This is a 10,000x overhead compared to the optimal approach.

Block explorers and indexing tools frequently request full blocks. Under sustained RPC load with multiple concurrent full-block requests, the CPU cost of these scans becomes significant.

Concrete scenario: A block explorer catching up on the latest 100 blocks issues 100 `eth_getBlockByNumber` calls with `full_transactions=true`. Each call scans 1,000,000 entries, resulting in 100,000,000 comparisons total, when only 10,000 transactions (100 per block) are actually needed.

## Root Cause

The indexer stores transactions in a flat `HashMap<B256, IndexedTransaction>` keyed by transaction hash, with no secondary index by block hash. The `IndexedBlock` struct already stores `transaction_hashes: Vec<B256>` (a list of transaction hashes in insertion order), but `get_transactions_for_block` does not use it.

## Suggested Fix

Use the block's `transaction_hashes` list for targeted lookups instead of scanning the full table:

**Before:**
```rust
pub fn get_transactions_for_block(&self, block_hash: &B256) -> Vec<IndexedTransaction> {
    let mut txs = self
        .transactions
        .read()
        .values()
        .filter(|tx| tx.block_hash == *block_hash)
        .cloned()
        .collect::<Vec<_>>();
    txs.sort_by_key(|tx| tx.index);
    txs
}
```

**After:**
```rust
pub fn get_transactions_for_block(&self, block_hash: &B256) -> Vec<IndexedTransaction> {
    let blocks = self.blocks_by_hash.read();
    let Some(block) = blocks.get(block_hash) else {
        return Vec::new();
    };
    let txs_map = self.transactions.read();
    block
        .transaction_hashes
        .iter()
        .filter_map(|hash| txs_map.get(hash).cloned())
        .collect()
    // Already in order since transaction_hashes preserves insertion order
}
```

This changes the complexity from O(total_txs) to O(txs_in_block), which is typically a 1,000-10,000x improvement. The sort is also eliminated since `transaction_hashes` preserves the original insertion order.

## Files to Modify

- `crates/storage/indexer/src/store.rs` -- `get_transactions_for_block()` method (lines 193-204)

## Related Issues

None.
