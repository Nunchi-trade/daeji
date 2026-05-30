# No Block Gas Limit Validation at Transaction Ingress

**Category**: bug -- txpool
**Severity**: high

## Summary

The transaction validator checks that a transaction's gas limit is at least the intrinsic gas cost (lower bound) but never checks that it does not exceed the block gas limit (upper bound). A transaction requesting more gas than the block can provide will always fail during execution since it cannot fit in any block. Standard Ethereum clients (geth, reth, erigon) all reject such transactions at the pool level.

## Problem

The `validate()` function in `TransactionValidator` performs a lower-bound gas check (`gas_limit >= intrinsic_gas`) but has no upper-bound check (`gas_limit <= block_gas_limit`). Without this check, a user can submit a transaction with `gas_limit = u64::MAX`, which would:

1. Pass all pool validation checks
2. Be inserted into the pool and gossiped to peers
3. Never be includable in any block (the block gas limit is typically 30M gas)
4. Occupy pool capacity until it expires via TTL

The `TransactionValidator` struct does not have access to the current block gas limit -- it only stores `chain_id`, `state`, `config`, and `pool`.

## Code Reference

File: `crates/node/txpool/src/validator.rs`, lines 67-142 (the `validate` method)

```rust
pub async fn validate(&self, tx: Tx) -> Result<ValidatedTransaction, TxPoolError> {
    // ... size check, chain ID check, signature recovery ...

    let intrinsic_gas = intrinsic_gas(&envelope);
    let gas_limit = envelope.gas_limit();
    if gas_limit < intrinsic_gas {
        return Err(TxPoolError::IntrinsicGasTooLow {
            limit: gas_limit,
            intrinsic: intrinsic_gas,
        });
    }
    // Missing: if gas_limit > block_gas_limit { return Err(...); }

    // ... nonce check, balance check ...
}
```

The `TransactionValidator` struct (lines 44-50):

```rust
pub struct TransactionValidator<S> {
    chain_id: u64,
    state: S,
    config: PoolConfig,
    pool: Option<TransactionPool>,
    // No block_gas_limit field
}
```

The `PoolConfig` struct (file: `crates/node/txpool/src/config.rs`, lines 5-22) also does not contain a `block_gas_limit` field:

```rust
pub struct PoolConfig {
    pub max_pending_txs: usize,
    pub max_queued_txs: usize,
    pub max_txs_per_sender: usize,
    pub max_tx_size: usize,
    pub min_gas_price: u128,
    pub replacement_bump_percent: u8,
    pub pending_ttl_secs: u64,
    pub queued_ttl_secs: u64,
    // No block_gas_limit field
}
```

## Impact

1. **Pool pollution**: Non-includable transactions occupy pool capacity (up to 4,096 pending + 1,024 queued slots), displacing valid transactions that could actually be included in blocks.
2. **Bandwidth waste**: Non-includable transactions are gossiped to all peers via the P2P transaction gossip channel, wasting network bandwidth across the entire validator set.
3. **Block builder inefficiency**: The block builder's `build()` method must attempt to include these transactions, spend gas computing them, and then skip them when they exceed the block gas limit.
4. **DoS amplification**: An attacker can cheaply fill the pool with non-includable transactions because the balance check uses `max_tx_cost = gas_limit * max_fee_per_gas + value`, but with `gas_limit = u64::MAX` the balance check would reject (requires enormous balance). However, a moderately inflated gas limit (e.g., 2x the block gas limit) could pass the balance check while still being non-includable.

## Root Cause

The `TransactionValidator` does not have access to the current block gas limit. It was designed to validate gas against intrinsic cost (lower bound) but not against the block gas limit (upper bound). The gas limit is set at the application level in `RevmApplication` but is not threaded through to the pool validator.

## Suggested Fix

1. Add a `block_gas_limit` field to `PoolConfig`:

```rust
pub struct PoolConfig {
    // ... existing fields ...
    pub block_gas_limit: u64,
}
```

2. Add the upper-bound check in `validate()`:

```rust
if gas_limit > self.config.block_gas_limit {
    return Err(TxPoolError::GasLimitTooHigh {
        limit: gas_limit,
        max: self.config.block_gas_limit,
    });
}
```

3. Thread the block gas limit from the node configuration into `PoolConfig` when constructing the validator in `runner.rs`.

## Files to Modify

- `crates/node/txpool/src/config.rs` -- Add `block_gas_limit` field to `PoolConfig`
- `crates/node/txpool/src/validator.rs` -- Add gas limit upper-bound check in `validate()`
- `crates/node/txpool/src/error.rs` -- Add `GasLimitTooHigh` error variant
- `crates/node/runner/src/runner.rs` -- Pass block gas limit when constructing `PoolConfig` (lines 1116 and 1213 where `PoolConfig::default()` is used)

## Related Issues

- `204-txpool-pool-config-not-exposed-in-config-toml.md` -- `PoolConfig` is not configurable at all; the block gas limit should be added as part of the broader configuration exposure effort

## Labels

bug, correctness, txpool, config
