# Intrinsic Gas Calculation Missing EIP-3860 Initcode Word Cost and Size Limit

**Category**: bug -- txpool
**Severity**: high

## Summary

For contract creation transactions (`to == None`), the `intrinsic_gas()` function adds the `TX_CREATE_GAS` (32,000 gas) surcharge but does not add the EIP-3860 (Shanghai) per-word initcode cost of `2 * ceil(len(initcode) / 32)`. Additionally, the validator does not enforce the EIP-3860 maximum initcode size of 49,152 bytes. This allows contract creation transactions with large initcode to pass pool validation with insufficient gas, causing them to fail during EVM execution.

## Problem

EIP-3860 (activated in the Shanghai upgrade) imposes two requirements on contract creation transactions:

1. **Initcode word cost**: Each 32-byte word of initcode costs 2 gas, computed as `INITCODE_WORD_COST * ceil(len(initcode) / 32)` where `INITCODE_WORD_COST = 2`.
2. **Maximum initcode size**: Initcode must not exceed 49,152 bytes (`MAX_INITCODE_SIZE = 2 * MAX_CODE_SIZE`).

Neither requirement is enforced by the transaction validator. The `intrinsic_gas()` function only adds the base `TX_CREATE_GAS` (32,000) for creation transactions.

For a contract deployment with 49,152 bytes of initcode, the missing cost is `2 * ceil(49152/32) = 3,072 gas`. While not enormous, this gap means the pool will accept transactions that the EVM will reject, wasting block space.

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
        // Missing: EIP-3860 initcode word cost
        // Missing: EIP-3860 max initcode size check
    }

    let access_list_gas = match envelope {
        TxEnvelope::Legacy(_) => 0,
        TxEnvelope::Eip2930(tx) => access_list_gas_cost(&tx.tx().access_list),
        TxEnvelope::Eip1559(tx) => access_list_gas_cost(&tx.tx().access_list),
        TxEnvelope::Eip4844(tx) => access_list_gas_cost(&tx.tx().tx().access_list),
        TxEnvelope::Eip7702(tx) => access_list_gas_cost(&tx.tx().access_list),
    };
    gas += access_list_gas;

    gas
}
```

The `validate()` function also does not check initcode size (lines 67-142). The `max_tx_size` config (128 KB) provides a coarser limit, but the EIP-3860 limit of 49,152 bytes is more restrictive and semantically correct for initcode specifically.

## Impact

1. **Intrinsic gas underestimated**: Contract creation transactions with large initcode pass the `IntrinsicGasTooLow` validation but fail during EVM execution. For maximum-size initcode (49,152 bytes), the undercount is 3,072 gas.
2. **Block space waste**: Failed contract creations consume block gas without producing useful state changes, reducing space available for valid transactions.
3. **Missing size limit**: While the `max_tx_size` config (128 KB) provides a coarser limit, it allows initcode up to ~128 KB, which exceeds the EIP-3860 limit of 49,152 bytes by over 2.5x. Transactions with oversized initcode will always fail in the EVM.
4. **Spec non-compliance**: Post-Shanghai, all Ethereum execution clients enforce EIP-3860. Not enforcing it at the pool level is a deviation from the specification.

## Root Cause

The `intrinsic_gas()` function was implemented with pre-Shanghai logic that only accounts for `TX_CREATE_GAS` without the EIP-3860 per-word initcode cost. The EIP-3860 size limit check was also not added to the `validate()` function.

## Suggested Fix

Add the initcode word cost and size check:

**Before (line 216-218):**
```rust
if envelope.to().is_none() {
    gas += TX_CREATE_GAS;
}
```

**After:**
```rust
if envelope.to().is_none() {
    gas += TX_CREATE_GAS;
    // EIP-3860: initcode word cost
    let initcode_len = envelope.input().len() as u64;
    gas += 2 * ((initcode_len + 31) / 32);
}
```

Additionally, add a size check in `validate()` before computing intrinsic gas:

```rust
// EIP-3860: max initcode size
if envelope.to().is_none() && envelope.input().len() > 49_152 {
    return Err(TxPoolError::OversizedData); // or a new InitcodeTooLarge error variant
}
```

## Files to Modify

- `crates/node/txpool/src/validator.rs` -- Add EIP-3860 initcode word cost in `intrinsic_gas()` (line 216-218) and max initcode size check in `validate()` (around line 93)
- `crates/node/txpool/src/error.rs` -- Potentially add an `InitcodeTooLarge` error variant

## Related Issues

- `196-txpool-intrinsic-gas-missing-eip7702-auth-list-cost.md` -- Another missing component in the same `intrinsic_gas()` function

## Labels

bug, correctness, txpool
