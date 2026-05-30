# gas_oracle_cache and state_provider Nested Lock Ordering Risk

**Category**: Bug -- RPC
**Severity**: Medium
**Labels**: `bug`, `rpc`, `reliability`

## Summary

The `recent_fee_estimate()` method acquires nested `tokio::sync::RwLock` guards in a specific order: first `state_provider.read()`, then `gas_oracle_cache.read()`, then potentially `gas_oracle_cache.write()` -- all while still holding the `state_provider` read lock. While no reverse ordering currently exists in the codebase, this nested lock acquisition is fragile and creates a latent deadlock risk if future code acquires these locks in the opposite order.

## Problem

Kora is an EVM execution client that serves Ethereum-compatible JSON-RPC. The `EthApiImpl` struct holds two `tokio::sync::RwLock`-protected fields that are accessed together in the `recent_fee_estimate()` method:

1. `state_provider: Arc<RwLock<S>>` -- wraps the state provider used for block/receipt queries
2. `gas_oracle_cache: Arc<RwLock<Option<CachedGasOracleEstimate>>>` -- caches the most recent gas price estimate

The `recent_fee_estimate()` method acquires the `state_provider` read lock first, then acquires either the `gas_oracle_cache` read lock (to check the cache) or write lock (to update the cache) -- all while the `state_provider` read lock is still held. This creates a lock ordering dependency: `state_provider` must always be acquired before `gas_oracle_cache`.

The lock ordering invariant is not documented or enforced. If any future code path acquires `gas_oracle_cache` first and then `state_provider`, a deadlock will occur. With `tokio::sync::RwLock`, such a deadlock would silently hang the async runtime without any error message.

## Code Reference

`crates/node/rpc/src/eth.rs:423-439`:
```rust
async fn recent_fee_estimate(&self) -> RpcResult<GasOracleEstimate> {
    let provider = self.state_provider.read().await;  // Lock 1: state_provider read
    let head = provider
        .block_number()
        .await
        .unwrap_or_else(|_| self.block_height.load(std::sync::atomic::Ordering::Relaxed));

    if let Some(cached) = *self.gas_oracle_cache.read().await  // Lock 2: gas_oracle_cache read
        && cached.head == head
    {
        return Ok(cached.estimate);
    }

    let estimate = estimate_recent_fees(&*provider, head, self.gas_oracle_config).await;
    *self.gas_oracle_cache.write().await = Some(CachedGasOracleEstimate { head, estimate });
    // Lock 2 (write) acquired and released here ^
    Ok(estimate)
    // Lock 1 released here
}
```

The field definitions at `crates/node/rpc/src/eth.rs:286,294`:
```rust
state_provider: Arc<RwLock<S>>,
// ...
gas_oracle_cache: Arc<RwLock<Option<CachedGasOracleEstimate>>>,
```

This method is called by `gas_price()` and `max_priority_fee_per_gas()` (`crates/node/rpc/src/eth.rs:617-622`):
```rust
async fn gas_price(&self) -> RpcResult<U256> {
    Ok(self.recent_fee_estimate().await?.gas_price)
}

async fn max_priority_fee_per_gas(&self) -> RpcResult<U256> {
    Ok(self.recent_fee_estimate().await?.priority_fee)
}
```

## Impact

There is no immediate deadlock risk with the current codebase, as no other code path acquires these locks in the reverse order. However, the lock ordering invariant is implicit and undocumented. A future refactoring that accesses the gas oracle cache while also needing the state provider could inadvertently create a deadlock that only manifests under concurrent load -- a notoriously difficult bug to reproduce and diagnose.

Additionally, holding the `state_provider` read lock across the `gas_oracle_cache.write()` call means that the state_provider read lock is held for the entire duration of `estimate_recent_fees()`, which iterates over multiple blocks to compute a fee estimate. This unnecessarily blocks any code that needs the state_provider write lock.

## Root Cause

The method holds the `state_provider` read lock longer than necessary. The state_provider is only needed to get the `head` block number and to compute the fee estimate. The gas_oracle_cache operations (read and write) do not require the state_provider to be locked.

## Suggested Fix

Release the `state_provider` lock before acquiring the `gas_oracle_cache` write lock:

```rust
async fn recent_fee_estimate(&self) -> RpcResult<GasOracleEstimate> {
    let provider = self.state_provider.read().await;
    let head = provider
        .block_number()
        .await
        .unwrap_or_else(|_| self.block_height.load(std::sync::atomic::Ordering::Relaxed));

    if let Some(cached) = *self.gas_oracle_cache.read().await
        && cached.head == head
    {
        return Ok(cached.estimate);
    }

    let estimate = estimate_recent_fees(&*provider, head, self.gas_oracle_config).await;
    drop(provider);  // Release state_provider BEFORE writing gas_oracle_cache
    *self.gas_oracle_cache.write().await = Some(CachedGasOracleEstimate { head, estimate });
    Ok(estimate)
}
```

This eliminates the nested lock acquisition entirely and reduces the duration the state_provider read lock is held.

## Files to Modify

- `crates/node/rpc/src/eth.rs` -- `recent_fee_estimate()` method (lines 423-439): add `drop(provider)` before the `gas_oracle_cache.write()` call

## Related Issues

- [034-rpc-fee-history-holds-lock-across-loop.md](034-rpc-fee-history-holds-lock-across-loop.md) -- Another RPC method (`fee_history`) that holds the `state_provider` read lock for an extended duration.
