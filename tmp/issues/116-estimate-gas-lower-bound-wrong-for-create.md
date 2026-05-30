# `estimate_gas` Binary Search Lower Bound Hardcoded to 21,000 -- Incorrect for Contract Creation Transactions

**Category**: bug
**Severity**: low
**Labels**: bug, correctness, performance, executor, good first issue

## Summary

The `estimate_gas` method in the REVM executor hardcodes its binary search lower bound to `21_000`, which is the intrinsic gas cost for a simple ETH transfer. Contract creation (CREATE) transactions have a higher intrinsic cost (at minimum 53,000 per EIP-3860, plus 200 per byte of initcode), so the lower bound is too low for these transactions. This wastes binary search iterations exploring a gas range that can never produce a successful execution for CREATE calls.

## Problem

In Kora's REVM executor (`crates/node/executor/src/revm.rs`), the `estimate_gas` method performs a binary search between a lower bound and the block gas limit to find the minimum gas required for a transaction to succeed. The lower bound is unconditionally set to `21_000` at line 284, regardless of the transaction type.

For contract creation transactions (where `params.to` is `None`), the minimum intrinsic gas is 53,000 (32,000 for CREATE + 21,000 base). The binary search therefore wastes iterations probing gas values in the range [21,000, 53,000], where execution will always fail for CREATE transactions. Each wasted iteration performs a full EVM execution via `simulate_call`, which is expensive.

The `CallParams` struct (defined at line 176 of the same file) has a `to: Option<Address>` field where `None` indicates contract creation.

## Code Reference

**File**: `crates/node/executor/src/revm.rs`, lines 271-303

```rust
pub fn estimate_gas<S: kora_traits::StateDbRead>(
    &self,
    state: &S,
    mut params: CallParams,
    context: &BlockContext,
) -> Result<u64, ExecutionError> {
    let upper =
        params.gas_limit.unwrap_or(context.header.gas_limit).min(context.header.gas_limit);

    // Confirm the call succeeds at the upper bound.
    params.gas_limit = Some(upper);
    self.simulate_call(state, params.clone(), context)?;

    let mut lo = 21_000u64;       // <-- hardcoded, wrong for CREATE txs
    let mut hi = upper;
    let mut best = upper;
    let mut iters = 0u32;
    while lo + 1 < hi && iters < 25 {
        iters += 1;
        let mid = lo + (hi - lo) / 2;
        params.gas_limit = Some(mid);
        match self.simulate_call(state, params.clone(), context) {
            Ok(_) => {
                best = mid;
                hi = mid;
            }
            Err(_) => {
                lo = mid;
            }
        }
    }
    Ok(best)
}
```

## Impact

- **Performance**: For every `eth_estimateGas` call on a contract creation transaction, approximately 1-2 extra binary search iterations are wasted. Each iteration performs a full EVM execution via `simulate_call`, which involves setting up the REVM context, building a state database adapter, and running the EVM. On a loaded node serving many estimate requests, this adds unnecessary CPU overhead.
- **Correctness**: The final result is still correct because the binary search converges to the right answer regardless of the starting lower bound. However, the inefficiency is avoidable.

## Root Cause

The lower bound is hardcoded to the ETH transfer intrinsic gas cost (21,000) without accounting for the higher intrinsic cost of contract creation transactions (53,000+). The `params.to` field, which distinguishes CREATE from CALL transactions, is not consulted when setting the lower bound.

## Suggested Fix

Compute the intrinsic gas lower bound based on the transaction type:

**Before:**
```rust
let mut lo = 21_000u64;
```

**After:**
```rust
// Contract creation (CREATE) has a higher intrinsic gas cost than simple transfers.
// 53_000 = 21_000 (base) + 32_000 (CREATE cost per EIP-3860).
let mut lo = if params.to.is_none() { 53_000u64 } else { 21_000u64 };
```

For an even more accurate lower bound, add the initcode cost (200 gas per byte of `params.data` for CREATE transactions), though the simple two-branch check provides the majority of the benefit.

## Files to Modify

- `crates/node/executor/src/revm.rs` -- line 284, adjust the lower bound in `estimate_gas`

## Related Issues

- `123-estimate-gas-treats-all-errors-as-oog.md` -- another bug in the same `estimate_gas` binary search loop (error handling)
