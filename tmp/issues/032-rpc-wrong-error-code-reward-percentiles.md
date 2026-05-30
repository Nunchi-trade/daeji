# validate_reward_percentiles Returns Wrong RPC Error Variant

**Category**: Bug -- RPC
**Severity**: Low
**Labels**: `bug`, `rpc`, `correctness`, `good first issue`

## Summary

The `validate_reward_percentiles()` function returns `RpcError::InvalidTransaction` when percentile values are invalid, but this is a parameter validation error for the `eth_feeHistory` method, not a transaction error. The wrong variant produces a misleading error message (`"invalid transaction: ..."` instead of `"invalid params: ..."`) that confuses clients. The semantically correct variant `RpcError::InvalidParams` exists and should be used instead.

## Problem

Kora is an EVM execution client that serves Ethereum-compatible JSON-RPC. The `eth_feeHistory` method accepts an optional `reward_percentiles` array parameter. Before performing any work, the handler validates the percentiles via `validate_reward_percentiles()` in `crates/node/rpc/src/eth.rs:1150-1168`.

This function uses `RpcError::InvalidTransaction` for its error returns, but percentile validation has nothing to do with transactions -- it is validating RPC method parameters. The `RpcError` enum (defined in `crates/node/rpc/src/error.rs:39-92`) has a dedicated `InvalidParams` variant with the display format `"invalid params: {0}"`, which is the correct choice for parameter validation errors.

Both `InvalidTransaction` and `InvalidParams` currently map to the same numeric error code (`INVALID_PARAMS` = `-32602`) in the `From<RpcError>` implementation (`crates/node/rpc/src/error.rs:107,109`), so the numeric code is correct today. However, the error *message* differs: clients see `"invalid transaction: reward percentiles must be in [0, 100]"` instead of the expected `"invalid params: reward percentiles must be in [0, 100]"`.

## Code Reference

`crates/node/rpc/src/eth.rs:1148-1168`:
```rust
/// Validates that `reward_percentiles` values are in `[0, 100]` and
/// monotonically non-decreasing, per the Ethereum JSON-RPC specification.
fn validate_reward_percentiles(percentiles: &[f64]) -> RpcResult<()> {
    for p in percentiles {
        if !p.is_finite() || *p < 0.0 || *p > 100.0 {
            return Err(RpcError::InvalidTransaction(        // WRONG: not a tx error
                "reward percentiles must be in [0, 100]".to_string(),
            )
            .into());
        }
    }
    for w in percentiles.windows(2) {
        if w[0] > w[1] {
            return Err(RpcError::InvalidTransaction(        // WRONG: same issue
                "reward percentiles must be monotonically non-decreasing".to_string(),
            )
            .into());
        }
    }
    Ok(())
}
```

The error variant definitions in `crates/node/rpc/src/error.rs:61-63,86-87`:
```rust
/// Invalid transaction.
#[error("invalid transaction: {0}")]
InvalidTransaction(String),

// ...

/// Invalid method parameters.
#[error("invalid params: {0}")]
InvalidParams(String),
```

Both map to the same code in `crates/node/rpc/src/error.rs:106-109`:
```rust
RpcError::InvalidTransaction(_) => (codes::INVALID_PARAMS, other.to_string()),
// ...
RpcError::InvalidParams(_) => (codes::INVALID_PARAMS, other.to_string()),
```

## Impact

Clients and developer tools calling `eth_feeHistory` with invalid percentiles receive an error message that says `"invalid transaction: ..."` -- which is confusing because the call has nothing to do with transactions. Automated retry logic or error categorization that keys on the error message string (e.g., matching `"invalid transaction"`) will misclassify this error as a transaction failure. Additionally, if the error code mapping is ever corrected to use `TRANSACTION_REJECTED` (`-32003`) for `InvalidTransaction`, this would change the numeric error code and break clients that rely on it.

## Root Cause

The wrong `RpcError` variant was chosen during initial implementation, likely by copy-paste from a different function that validates transaction parameters. `RpcError::InvalidParams` exists and produces the correct message, but `RpcError::InvalidTransaction` was used instead.

## Suggested Fix

Replace `RpcError::InvalidTransaction` with `RpcError::InvalidParams` in both error returns:

**Before:**
```rust
return Err(RpcError::InvalidTransaction(
    "reward percentiles must be in [0, 100]".to_string(),
).into());
```

**After:**
```rust
return Err(RpcError::InvalidParams(
    "reward percentiles must be in [0, 100]".to_string(),
).into());
```

And the same for the monotonicity check:

**Before:**
```rust
return Err(RpcError::InvalidTransaction(
    "reward percentiles must be monotonically non-decreasing".to_string(),
).into());
```

**After:**
```rust
return Err(RpcError::InvalidParams(
    "reward percentiles must be monotonically non-decreasing".to_string(),
).into());
```

## Files to Modify

- `crates/node/rpc/src/eth.rs` -- `validate_reward_percentiles()` function (lines 1150-1168): change both `RpcError::InvalidTransaction` to `RpcError::InvalidParams`

## Related Issues

None.
