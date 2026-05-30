# eth_feeHistory Fetches Receipts Serially Per-Transaction Per-Block (N+1 Query Pattern)

## Category
performance -- rpc

## Severity
medium

## Summary
When `eth_feeHistory` is called with `reward_percentiles`, the implementation issues a separate `receipt_by_hash()` lookup for every transaction in every block in the requested range. This is a classic N+1 query anti-pattern: for a 1,024-block range with 50 transactions per block, it results in 51,200 sequential async receipt lookups, causing multi-second response times and holding a read lock on the state provider throughout.

## Problem
The `fee_history()` method iterates over each block in the requested range. For each block, when reward percentiles are requested, it calls `fetch_tx_gas_used()`, which issues a per-transaction receipt lookup:

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 653-669 (fee_history loop):
```rust
for block_number in oldest..oldest + count as u64 {
    let block = block_by_number_or_none(&*provider, block_number, reward.is_some()).await;
    // ...
    if let Some(block) = block {
        let gas_used = block.gas_used.to::<u64>();
        let gas_limit = block.gas_limit.to::<u64>();
        gas_used_ratio.push(block_gas_used_ratio(gas_used, gas_limit));

        if let (Some(percentiles), Some(rows)) = (&reward_percentiles, reward.as_mut()) {
            let tx_gas_used = fetch_tx_gas_used(&*provider, &block).await;
            rows.push(compute_reward_percentiles(&block, &tx_gas_used, percentiles));
        }
        // ...
    }
}
```

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 1173-1186 (per-tx receipt lookup):
```rust
async fn fetch_tx_gas_used<S: StateProvider>(provider: &S, block: &RpcBlock) -> Vec<u64> {
    let BlockTransactions::Full(txs) = &block.transactions else {
        return Vec::new();
    };
    let mut gas_used = Vec::with_capacity(txs.len());
    for tx in txs {
        let used = match provider.receipt_by_hash(tx.hash).await {
            Ok(Some(receipt)) => receipt.gas_used.to::<u64>(),
            _ => tx.gas.to::<u64>(),
        };
        gas_used.push(used);
    }
    gas_used
}
```

Each `provider.receipt_by_hash()` call acquires a read lock on the indexer's `receipts` HashMap, looks up a single entry, and returns. This is repeated for every transaction in every block, sequentially.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, line 668 (call site):
```rust
let tx_gas_used = fetch_tx_gas_used(&*provider, &block).await;
```

`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 1173-1186 (N+1 loop):
```rust
async fn fetch_tx_gas_used<S: StateProvider>(provider: &S, block: &RpcBlock) -> Vec<u64> {
    let BlockTransactions::Full(txs) = &block.transactions else {
        return Vec::new();
    };
    let mut gas_used = Vec::with_capacity(txs.len());
    for tx in txs {
        let used = match provider.receipt_by_hash(tx.hash).await {
            Ok(Some(receipt)) => receipt.gas_used.to::<u64>(),
            _ => tx.gas.to::<u64>(),
        };
        gas_used.push(used);
    }
    gas_used
}
```

## Impact
- **Latency:** A single `eth_feeHistory` call with `reward_percentiles` over 1,024 blocks (the standard `block_count` maximum in many clients) with an average of 50 transactions per block triggers 51,200 sequential receipt lookups. Each lookup acquires a read lock on the indexer's `receipts` RwLock. At the current devnet block rate (33 blocks/s), 1,024 blocks covers about 30 seconds of history.
- **Lock contention:** The state provider read lock is held for the entire duration of the outer loop (across all blocks), blocking any write-path callers (e.g., `update_state_provider()` which needs a write lock to update the provider with new blocks).
- **Cascading latency:** Because the fee_history call holds the state provider read lock for seconds, other RPC methods that need write access are starved, causing cascading delays for the entire RPC server.

## Root Cause
There is no batch receipt retrieval method on `StateProvider` or `BlockIndex`. The `IndexedStateProvider` can look up receipts only by individual transaction hash (`receipt_by_hash()`), forcing `fetch_tx_gas_used()` to issue one query per transaction.

## Suggested Fix
1. **Add a batch receipt method** to `BlockIndex` and `StateProvider`:

```rust
// In BlockIndex (crates/storage/indexer/src/store.rs):
pub fn get_receipts_for_block(&self, block_hash: &B256) -> Vec<IndexedReceipt> {
    self.receipts.read().values()
        .filter(|r| r.block_hash == *block_hash)
        .cloned()
        .collect()
}

// In StateProvider trait:
async fn receipts_by_block(&self, block_number: u64) -> Result<Vec<TransactionReceipt>, RpcError>;
```

2. **Rewrite `fetch_tx_gas_used()`** to use the batch method:

```rust
async fn fetch_tx_gas_used<S: StateProvider>(provider: &S, block: &RpcBlock) -> Vec<u64> {
    let receipts = provider.receipts_by_block(block.number.to::<u64>()).await.unwrap_or_default();
    receipts.iter().map(|r| r.gas_used.to::<u64>()).collect()
}
```

3. Alternatively, since the indexer already stores `gas_used` per receipt, the `IndexedReceipt` data can be returned directly without multiple HashMap lookups.

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs` -- rewrite `fetch_tx_gas_used()` to use batch receipt lookup
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/state_provider.rs` -- add `receipts_by_block()` to the `StateProvider` trait
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs` -- implement `receipts_by_block()` using the block index
- `/Users/will/dev/nunchi/daeji/crates/storage/indexer/src/store.rs` -- add `get_receipts_for_block()` to `BlockIndex`

## Related Issues
- `034-rpc-fee-history-holds-lock-across-loop.md` -- the fee_history outer loop holds the state provider lock for the entire block range; this issue adds the per-transaction receipt N+1 detail within that loop

## Labels
performance, rpc
