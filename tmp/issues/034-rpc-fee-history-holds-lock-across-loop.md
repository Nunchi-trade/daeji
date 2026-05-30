# fee_history Holds state_provider Read Lock Across Entire 1024-Block Loop

**Category**: Performance -- RPC
**Severity**: Medium
**Labels**: `performance`, `rpc`

## Summary

The `eth_feeHistory` RPC handler acquires the `state_provider` read lock at the start of the function and holds it for the entire loop, which can iterate over up to 1,024 blocks. Inside the loop, for each block with reward percentiles enabled, it performs per-transaction receipt lookups. This holds the read lock for hundreds of milliseconds, preventing any code that needs a write lock on the state provider from proceeding.

## Problem

Kora is an EVM execution client that serves Ethereum-compatible JSON-RPC. The `state_provider` field in `EthApiImpl` is wrapped in a `tokio::sync::RwLock` (`crates/node/rpc/src/eth.rs:286`):

```rust
state_provider: Arc<RwLock<S>>,
```

The `fee_history` method acquires a read lock at the top and holds it for the entire function scope, including a loop that can iterate up to 1,024 blocks. Inside the loop, it calls `block_by_number_or_none()` and, when reward percentiles are requested, `fetch_tx_gas_used()` which issues one `receipt_by_hash` call per transaction in the block.

While `tokio::sync::RwLock` read guards are shared (multiple readers can proceed concurrently), holding one for the duration of this loop prevents any code path that needs a write guard from proceeding. Since the `state_provider` is wrapped in `Arc<RwLock<S>>`, any external code that needs to replace or update the state provider must acquire a write lock and will be blocked until all read guards are dropped.

## Code Reference

`crates/node/rpc/src/eth.rs:625-689`:
```rust
async fn fee_history(
    &self,
    block_count: U64,
    newest_block: BlockNumberOrTag,
    reward_percentiles: Option<Vec<f64>>,
) -> RpcResult<FeeHistory> {
    // Validate percentile values before doing any work.
    if let Some(percentiles) = &reward_percentiles {
        validate_reward_percentiles(percentiles)?;
    }

    let provider = self.state_provider.read().await;  // Lock acquired here
    let head = provider
        .block_number()
        .await
        .unwrap_or_else(|_| self.block_height.load(std::sync::atomic::Ordering::Relaxed));
    let newest = resolve_fee_history_newest(newest_block, head);
    let requested = block_count.to::<u64>().min(1024);  // Up to 1024 blocks
    let count = requested.min(newest.saturating_add(1)) as usize;
    let oldest = newest.saturating_add(1).saturating_sub(count as u64);

    // ... setup vectors ...

    for block_number in oldest..oldest + count as u64 {
        let block = block_by_number_or_none(&*provider, block_number, reward.is_some()).await;
        // ...
        if let (Some(percentiles), Some(rows)) = (&reward_percentiles, reward.as_mut()) {
            let tx_gas_used = fetch_tx_gas_used(&*provider, &block).await;
            // fetch_tx_gas_used calls receipt_by_hash() per transaction
            rows.push(compute_reward_percentiles(&block, &tx_gas_used, percentiles));
        }
        // ...
    }
    // Lock released when provider goes out of scope
    // ...
}
```

The `fetch_tx_gas_used` helper issues per-transaction receipt lookups:

`crates/node/rpc/src/eth.rs:1173-1186`:
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

A single `eth_feeHistory` call with `block_count=1024` and `reward_percentiles` enabled holds the state_provider read lock while performing up to 1,024 block lookups and potentially thousands of receipt lookups (one per transaction per block). With blocks averaging even 10 transactions, that is 10,240 receipt lookups performed serially while the read lock is held.

Multiple concurrent fee_history calls compound the problem. Block explorers, wallets, and gas estimation services frequently call `eth_feeHistory`, so this is a commonly triggered code path.

Concrete scenario: A DeFi frontend calls `eth_feeHistory(1024, "latest", [25, 50, 75])` to display gas price trends. The query runs for hundreds of milliseconds. During this time, if the runner needs to swap the state provider (e.g., after a state database compaction), it cannot acquire the write lock and is blocked until the query completes.

## Root Cause

The method acquires the read lock once at the top and uses it for all block fetches and receipt lookups within the loop, rather than releasing and re-acquiring per block or per batch. This was a straightforward implementation that guarantees a consistent view across all blocks, but the cost is prolonged lock holding.

## Suggested Fix

Release and re-acquire the read lock per block:

```rust
for block_number in oldest..oldest + count as u64 {
    let provider = self.state_provider.read().await;
    let block = block_by_number_or_none(&*provider, block_number, reward.is_some()).await;
    // ... collect data from block ...
    if let (Some(percentiles), Some(rows)) = (&reward_percentiles, reward.as_mut()) {
        let tx_gas_used = fetch_tx_gas_used(&*provider, &block).await;
        rows.push(compute_reward_percentiles(&block, &tx_gas_used, percentiles));
    }
    // ... update last_base_fee, last_gas_used, last_gas_limit ...
    drop(provider);  // Release lock before next iteration
}
```

This allows state provider updates to proceed between block lookups, at the cost of a slightly inconsistent view across the range (which is acceptable for fee history, since it is advisory data used for gas estimation).

## Files to Modify

- `crates/node/rpc/src/eth.rs` -- `fee_history()` method (lines 625-689): restructure to release the read lock between iterations

## Related Issues

- [035-rpc-gas-oracle-nested-lock-ordering.md](035-rpc-gas-oracle-nested-lock-ordering.md) -- Another RPC method (`recent_fee_estimate`) that holds the `state_provider` read lock longer than necessary, creating a nested lock ordering concern.
