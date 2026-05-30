# Gas Limit Delta Validation Variable Naming Is Misleading (Spec-Correct Code Reads Like Off-by-One Bug)

**Category**: Executor / Documentation
**Severity**: Low

## Summary

In the `validate_gas_limit()` method, the variable `max_delta` and the error message "exceeds maximum delta" suggest the intent is to allow changes up to and including `max_delta`. However, the comparison uses `>=` (greater-than-or-equal), which rejects the value `max_delta` itself. The code is correct per the Ethereum specification (Yellow Paper Section 4.3.4: `diff` must be strictly less than `parent_gas_limit / 1024`), but the naming creates a readability trap that could lead a future contributor to "fix" the comparison to `>`, introducing an actual spec violation.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`, the `validate_gas_limit()` method at lines 102-134 validates that the gas limit change between consecutive blocks is within bounds:

```rust
// crates/node/executor/src/revm.rs:102-134
fn validate_gas_limit(
    &self,
    gas_limit: u64,
    parent_gas_limit: u64,
) -> Result<(), ExecutionError> {
    let bounds = &self.config.gas_limit_bounds;

    if gas_limit < bounds.min {
        return Err(ExecutionError::BlockValidation(format!(
            "gas limit {} below minimum {}",
            gas_limit, bounds.min
        )));
    }

    if gas_limit > bounds.max {
        return Err(ExecutionError::BlockValidation(format!(
            "gas limit {} above maximum {}",
            gas_limit, bounds.max
        )));
    }

    let max_delta = parent_gas_limit / bounds.max_delta_divisor;
    let diff = gas_limit.abs_diff(parent_gas_limit);

    if diff >= max_delta {
        return Err(ExecutionError::BlockValidation(format!(
            "gas limit change {} exceeds maximum delta {}",
            diff, max_delta
        )));
    }

    Ok(())
}
```

The `max_delta_divisor` is configured in the `GasLimitBounds` struct:

```rust
// crates/node/executor/src/config.rs:5-20
pub struct GasLimitBounds {
    /// Minimum gas limit.
    pub min: u64,
    /// Maximum gas limit.
    pub max: u64,
    /// Maximum change from parent (denominator for delta calculation).
    /// Gas limit can change by at most parent_gas_limit / max_delta_divisor.
    pub max_delta_divisor: u64,
}

impl GasLimitBounds {
    /// Default gas limit bounds.
    pub const DEFAULT: Self = Self { min: 5000, max: u64::MAX, max_delta_divisor: 1024 };
}
```

The naming creates a specific readability problem:

- `max_delta` reads as "the maximum allowed value for the delta"
- `diff >= max_delta` reads as "the diff is greater than or equal to the maximum allowed"
- This reads like an off-by-one bug: if `max_delta` is the maximum *allowed* value, shouldn't it be `diff > max_delta`?

But the code is actually correct per the Ethereum specification. The Ethereum Yellow Paper (Section 4.3.4) and go-ethereum's `VerifyGaslimit` function both require `diff` to be *strictly less than* `parent_gas_limit / 1024`. The variable named `max_delta` is actually the *exclusive upper bound* (the smallest *disallowed* value), not the *inclusive maximum* (the largest *allowed* value).

## Impact

- **No runtime impact**: The current code is correct per Ethereum specification. This is purely a readability and maintainability issue.
- **Spec compliance risk**: A well-intentioned refactor by a developer who reads `max_delta` as "maximum allowed" could change the comparison from `>=` to `>`, introducing an off-by-one error that violates the Ethereum specification. This would cause Kora to accept blocks that other Ethereum clients reject (or vice versa), breaking cross-client compatibility.
- **Documentation comment misleading**: The `GasLimitBounds` doc comment says "Gas limit can change by at most parent_gas_limit / max_delta_divisor" which implies the value itself is allowed, contradicting the `>=` check.

## Root Cause

The variable was named `max_delta` (suggesting an inclusive maximum) when it represents an exclusive bound (the smallest disallowed value). The error message "exceeds maximum delta" reinforces the ambiguity by using "exceeds" rather than "not below."

## Suggested Fix

**Option A (rename variable)** -- Rename to clarify the exclusive bound semantics:

```rust
// BEFORE:
let max_delta = parent_gas_limit / bounds.max_delta_divisor;
let diff = gas_limit.abs_diff(parent_gas_limit);
if diff >= max_delta {
    return Err(ExecutionError::BlockValidation(format!(
        "gas limit change {} exceeds maximum delta {}",
        diff, max_delta
    )));
}

// AFTER:
let delta_bound = parent_gas_limit / bounds.max_delta_divisor;
let diff = gas_limit.abs_diff(parent_gas_limit);
if diff >= delta_bound {
    return Err(ExecutionError::BlockValidation(format!(
        "gas limit change {} not below bound {} (must be strictly less)",
        diff, delta_bound
    )));
}
```

**Option B (add spec comment)** -- Add a comment clarifying the Ethereum spec requirement:

```rust
// Per Ethereum Yellow Paper (Section 4.3.4) and go-ethereum's VerifyGaslimit:
// diff must be STRICTLY LESS THAN parent_gas_limit / 1024.
// `max_delta` is the exclusive upper bound, not the maximum allowed value.
let max_delta = parent_gas_limit / bounds.max_delta_divisor;
let diff = gas_limit.abs_diff(parent_gas_limit);
if diff >= max_delta {
```

Also update the doc comment on `max_delta_divisor` in the config:

```rust
// BEFORE (crates/node/executor/src/config.rs:13):
/// Maximum change from parent (denominator for delta calculation).
/// Gas limit can change by at most parent_gas_limit / max_delta_divisor.

// AFTER:
/// Denominator for the gas limit delta bound.
/// Gas limit change must be strictly less than parent_gas_limit / max_delta_divisor.
```

**Recommendation**: Option A is preferred as it makes the semantics self-documenting without relying on comments. Option B is acceptable as a minimal change.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` (lines 123-131) -- `validate_gas_limit()` variable name and error message
- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/config.rs` (lines 12-14) -- `GasLimitBounds` doc comment for `max_delta_divisor`

## Related Issues

- None currently identified.

## Labels

documentation, executor, good first issue
