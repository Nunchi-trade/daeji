# Intrinsic Gas Calculation Missing EIP-7702 Authorization List Cost

**Category**: bug -- txpool
**Severity**: critical

## Summary

The `intrinsic_gas()` function in the transaction validator does not account for the EIP-7702 authorization list gas cost when computing the intrinsic gas for EIP-7702 transactions. Per the EIP-7702 specification, each authorization entry costs 25,000 gas (`PER_EMPTY_ACCOUNT_COST`). The current code only adds the access list gas cost, which is a separate data structure. This allows EIP-7702 transactions with large authorization lists to pass the `IntrinsicGasTooLow` validation check despite having insufficient gas, causing them to always fail during EVM execution.

## Problem

EIP-7702 transactions contain two separate lists: an `access_list` (inherited from EIP-2930/1559) and an `authorization_list` (unique to EIP-7702). The `intrinsic_gas()` function correctly handles the access list gas cost for EIP-7702 transactions but completely ignores the authorization list.

Per the EIP-7702 specification, the intrinsic gas for an EIP-7702 transaction includes:
- Base gas (21,000)
- Calldata gas (4 per zero byte, 16 per non-zero byte)
- Access list gas (2,400 per address + 1,900 per storage key)
- **Authorization list gas: 25,000 per authorization entry** (missing)

## Code Reference

File: `crates/node/txpool/src/validator.rs`, lines 204-230

```rust
fn intrinsic_gas(envelope: &TxEnvelope) -> u64 {
    let mut gas = TX_BASE_GAS;

    let input = envelope.input();
    for byte in input.iter() {
        if *byte == 0 {
            gas += TX_DATA_ZERO_GAS;
        } else {
            gas += TX_DATA_NON_ZERO_GAS;
        }
    }

    if envelope.to().is_none() {
        gas += TX_CREATE_GAS;
    }

    let access_list_gas = match envelope {
        TxEnvelope::Legacy(_) => 0,
        TxEnvelope::Eip2930(tx) => access_list_gas_cost(&tx.tx().access_list),
        TxEnvelope::Eip1559(tx) => access_list_gas_cost(&tx.tx().access_list),
        TxEnvelope::Eip4844(tx) => access_list_gas_cost(&tx.tx().tx().access_list),
        TxEnvelope::Eip7702(tx) => access_list_gas_cost(&tx.tx().access_list),
        // ^^^ Only accounts for access list, NOT authorization list
    };
    gas += access_list_gas;

    gas
}
```

## Impact

1. **Block space waste**: EIP-7702 transactions with authorization lists and gas limits set between the (incorrectly computed) intrinsic gas and the true intrinsic gas will pass pool validation but fail during EVM execution. This wastes block space because failed transactions still consume block gas.
2. **DoS vector**: An attacker could submit many EIP-7702 transactions with large authorization lists (e.g., 10 entries = 250,000 gas undercount) but gas limits just above the incorrectly low intrinsic gas. These transactions would always fail during execution while consuming block builder time and block space.
3. **Revenue loss for validators**: Failed transactions reduce the block space available for valid, fee-paying transactions.

## Root Cause

The EIP-7702 branch in `intrinsic_gas()` was implemented to handle the access list cost (shared with EIP-2930 and EIP-1559) but the authorization list -- which is unique to EIP-7702 -- was not accounted for. This is likely an oversight from when EIP-7702 support was initially added.

## Suggested Fix

Add the authorization list gas cost to the EIP-7702 branch.

**Before:**
```rust
TxEnvelope::Eip7702(tx) => access_list_gas_cost(&tx.tx().access_list),
```

**After:**
```rust
TxEnvelope::Eip7702(tx) => {
    let al_gas = access_list_gas_cost(&tx.tx().access_list);
    let auth_gas = tx.tx().authorization_list.len() as u64 * 25_000; // PER_EMPTY_ACCOUNT_COST
    al_gas + auth_gas
}
```

## Files to Modify

- `crates/node/txpool/src/validator.rs` -- Add EIP-7702 authorization list gas cost on line 225

## Related Issues

- `198-txpool-intrinsic-gas-missing-eip3860-initcode-cost.md` -- Another missing component in the same `intrinsic_gas()` function (EIP-3860 initcode word cost)

## Labels

bug, correctness, txpool
