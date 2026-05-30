# max_tx_cost() Missing EIP-4844 Blob Gas Cost in Balance Check

**Category**: bug -- txpool
**Severity**: high

## Summary

The `max_tx_cost()` function computes the maximum cost of a transaction as `gas_limit * max_fee_per_gas + value`, which is correct for all transaction types except EIP-4844 blob transactions. For blob transactions, the true maximum cost also includes blob gas: `max_fee_per_blob_gas * num_blobs * GAS_PER_BLOB`. This missing component means the balance check in the validator can pass for blob transactions whose sender does not have enough ETH to cover the total cost, allowing them into the pool where they will fail during execution.

## Problem

EIP-4844 introduces a separate fee market for blob data. A blob transaction's total cost is:

```
total_cost = gas_limit * max_fee_per_gas + value + max_fee_per_blob_gas * total_blob_gas
```

where `total_blob_gas = num_blobs * GAS_PER_BLOB = num_blobs * 131,072`.

The `max_tx_cost()` function only computes `gas_limit * max_fee_per_gas + value`, omitting the blob gas cost entirely. This value is used in the balance check on line 128 of `validate()`.

## Code Reference

File: `crates/node/txpool/src/validator.rs`, lines 241-247

```rust
fn max_tx_cost(envelope: &TxEnvelope) -> U256 {
    let gas_limit = U256::from(envelope.gas_limit());
    let max_fee = U256::from(effective_gas_price(envelope));
    let value = envelope.value();

    gas_limit * max_fee + value
    // Missing for EIP-4844: + max_fee_per_blob_gas * total_blob_gas
}
```

The balance check that uses this function (lines 122-130):

```rust
let max_cost = max_tx_cost(&envelope);
let balance = self
    .state
    .balance(&sender)
    .await
    .map_err(|e| TxPoolError::StateError(e.to_string()))?;
if balance < max_cost {
    return Err(TxPoolError::InsufficientBalance { need: max_cost, have: balance });
}
```

## Impact

1. **Balance check bypass**: A sender with insufficient balance for the total blob transaction cost (execution gas + blob gas + value) could pass the balance check because the blob gas cost is omitted. For example, a blob transaction with 6 blobs at `max_fee_per_blob_gas = 1 gwei` adds `6 * 131,072 * 1e9 = ~786 billion wei` (~0.000786 ETH) of blob gas cost that is not checked.
2. **Failed execution**: Blob transactions that pass the incomplete balance check will fail during EVM execution when the full cost (including blob gas) is deducted, wasting block space and computation.
3. **Pool pollution**: Senders with insufficient balance can fill the pool with blob transactions that will always fail, displacing valid transactions.

## Root Cause

The `max_tx_cost()` function was implemented as a generic cost calculation across all transaction types without special handling for EIP-4844's separate blob gas fee market. EIP-4844 blob transactions have a dual fee structure: regular execution gas (shared with all tx types) and blob gas (unique to EIP-4844).

## Suggested Fix

Add blob gas cost for EIP-4844 transactions:

**Before:**
```rust
fn max_tx_cost(envelope: &TxEnvelope) -> U256 {
    let gas_limit = U256::from(envelope.gas_limit());
    let max_fee = U256::from(effective_gas_price(envelope));
    let value = envelope.value();

    gas_limit * max_fee + value
}
```

**After:**
```rust
fn max_tx_cost(envelope: &TxEnvelope) -> U256 {
    let gas_limit = U256::from(envelope.gas_limit());
    let max_fee = U256::from(effective_gas_price(envelope));
    let value = envelope.value();
    let mut cost = gas_limit * max_fee + value;

    // EIP-4844: add blob gas cost
    if let TxEnvelope::Eip4844(tx) = envelope {
        let blob_tx = tx.tx().tx();
        let num_blobs = blob_tx.blob_versioned_hashes.len() as u64;
        let total_blob_gas = U256::from(num_blobs * 131_072); // GAS_PER_BLOB
        let max_blob_fee = U256::from(blob_tx.max_fee_per_blob_gas);
        cost += total_blob_gas * max_blob_fee;
    }

    cost
}
```

## Files to Modify

- `crates/node/txpool/src/validator.rs` -- Add EIP-4844 blob gas cost to `max_tx_cost()` (lines 241-247)

## Related Issues

- None

## Labels

bug, correctness, txpool
