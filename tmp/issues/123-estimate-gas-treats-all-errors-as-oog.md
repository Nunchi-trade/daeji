# `estimate_gas` Binary Search Treats All Errors as "Out of Gas" -- Masks Reverts and State Errors

**Category**: bug
**Severity**: medium
**Labels**: bug, correctness, executor, rpc

## Summary

The binary search loop in `estimate_gas` catches all errors from `simulate_call` with a blanket `Err(_)` pattern and treats them as "this gas limit is too low, try higher." This is incorrect for errors like contract reverts (which will fail at any gas limit), state database errors (transient failures), and invalid transaction errors. The result is that reverting calls return the block gas limit as their estimate instead of an error, and state errors produce silently wrong estimates.

## Problem

In `crates/node/executor/src/revm.rs`, the `estimate_gas` method (lines 271-303) performs a binary search to find the minimum gas at which a transaction succeeds. The initial check at line 282 confirms the call succeeds at the upper bound (block gas limit). Then the binary search loop at lines 288-301 narrows the range.

The problem is in the error handling at lines 297-299:

```rust
Err(_) => {
    lo = mid;
}
```

This catch-all treats every error type as "needs more gas." But `simulate_call` (lines 206-258) can return several distinct error types:

1. **`ExecutionError::Revert(output)`** (line 254) -- the contract's logic reverted with `revert()` or `require()`. This will happen at any gas limit because the revert is caused by business logic, not gas exhaustion.
2. **`ExecutionError::TxExecution("halt: OutOfGas")`** (line 256) -- the only error type that actually means "needs more gas."
3. **`ExecutionError::TxExecution("halt: ...")`** (line 256) -- other halts like `StackOverflow`, `InvalidOpcode`, etc., which are not gas-related.
4. **`ExecutionError::State(...)`** -- state database errors (I/O failures, corruption), which are transient and should be propagated.
5. **`ExecutionError::InvalidTx(...)`** -- the transaction itself is malformed.

Note that the initial upper-bound check at line 282 would catch a revert at the maximum gas limit. However, there are edge cases where a transaction succeeds at the block gas limit but reverts at lower gas limits due to gas-dependent behavior (e.g., a contract that checks `gasleft()` and reverts if insufficient). In such cases, the binary search would still waste all 25 iterations.

## Code Reference

**File**: `crates/node/executor/src/revm.rs`, lines 284-302

```rust
let mut lo = 21_000u64;
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
        Err(_) => {         // <-- catches ALL errors, treats all as "needs more gas"
            lo = mid;
        }
    }
}
Ok(best)
```

The `simulate_call` method returns these error variants (lines 249-258):

```rust
match result_and_state.result {
    ExecutionResult::Success { output, .. } => match output {
        Output::Call(bytes) => Ok(bytes),
        Output::Create(bytes, _) => Ok(bytes),
    },
    ExecutionResult::Revert { output, .. } => Err(ExecutionError::Revert(output)),
    ExecutionResult::Halt { reason, .. } => {
        Err(ExecutionError::TxExecution(format!("halt: {:?}", reason)))
    }
}
```

## Impact

- **Reverting calls get wrong estimates**: If a transaction's contract logic depends on the gas limit (e.g., uses `gasleft()`), and the call succeeds at the block gas limit but reverts at lower limits, the binary search will waste all 25 iterations and return a higher-than-necessary estimate. More commonly, if the initial upper-bound check passes but an intermediate gas value triggers a revert for gas-dependent reasons, the error is silently swallowed.
- **State errors produce wrong results**: If the state database has a transient I/O error during one binary search iteration, the search treats it as "needs more gas" and moves the lower bound up. The final estimate will be higher than the actual minimum, potentially causing the user's transaction to overpay for gas.
- **Wasted CPU**: For errors that will occur at any gas limit, all 25 iterations are performed unnecessarily. Each iteration performs a full EVM execution via `simulate_call`, which is expensive (state DB setup, REVM context creation, and execution).

## Root Cause

The error handling in the binary search loop uses a blanket `Err(_)` pattern without matching on the specific `ExecutionError` variant. The code does not distinguish between gas-related failures (which should adjust the search bounds) and non-gas failures (which should be propagated immediately).

## Suggested Fix

Match on the error variant and only treat gas-related errors as "needs more gas":

**Before:**
```rust
match self.simulate_call(state, params.clone(), context) {
    Ok(_) => {
        best = mid;
        hi = mid;
    }
    Err(_) => {
        lo = mid;
    }
}
```

**After:**
```rust
match self.simulate_call(state, params.clone(), context) {
    Ok(_) => {
        best = mid;
        hi = mid;
    }
    Err(ExecutionError::TxExecution(ref msg)) if msg.contains("OutOfGas") => {
        // Genuinely needs more gas -- raise lower bound
        lo = mid;
    }
    Err(ExecutionError::Revert(output)) => {
        // Contract reverted -- this will happen at any gas limit.
        // Return the revert error so the caller knows the call fails.
        return Err(ExecutionError::Revert(output));
    }
    Err(e) => {
        // State errors, invalid tx, other halts -- propagate immediately
        return Err(e);
    }
}
```

For a more robust solution, refactor `simulate_call` to return a structured enum that distinguishes "out of gas" from "revert" from "other halt" from "infrastructure error," rather than relying on string matching against error messages.

## Files to Modify

- `crates/node/executor/src/revm.rs` -- lines 292-300, refine error handling in `estimate_gas` binary search loop

## Related Issues

- `116-estimate-gas-lower-bound-wrong-for-create.md` -- another bug in the same `estimate_gas` method (hardcoded lower bound)
- `013-eth-call-blocks-async-runtime.md` -- `eth_call` uses the same `simulate_call` method and has its own issues
