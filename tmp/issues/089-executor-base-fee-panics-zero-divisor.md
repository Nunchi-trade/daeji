# calculate_base_fee() Panics on Division by Zero if BaseFeeParams Fields Are Zero

**Category**: Executor / Reliability
**Severity**: High

## Summary

The `calculate_base_fee()` function performs unguarded division by `elasticity_multiplier` and `max_change_denominator` from `BaseFeeParams`. If either field is zero, the function panics with a division-by-zero error in the block production hot path. Since this function is called on every block via the executor, a misconfiguration would crash every validator simultaneously and cause a crash-loop that cannot recover without fixing the config. While the current hardcoded defaults are safe (elasticity=2, denominator=8), the `BaseFeeParams` struct has public fields with no validation, so zero values can be constructed by any code.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`, the `calculate_base_fee()` function at lines 331-358 performs three divisions that panic on zero:

```rust
// crates/node/executor/src/revm.rs:331-358
/// Calculate the expected base fee for the next block (EIP-1559).
pub fn calculate_base_fee(
    parent_base_fee: u64,
    parent_gas_used: u64,
    parent_gas_limit: u64,
    params: &crate::BaseFeeParams,
) -> u64 {
    let parent_gas_target = parent_gas_limit / params.elasticity_multiplier;  // PANICS if elasticity_multiplier == 0

    if parent_gas_used == parent_gas_target {
        return parent_base_fee;
    }

    if parent_gas_used > parent_gas_target {
        let gas_used_delta = parent_gas_used - parent_gas_target;
        let base_fee_delta = (parent_base_fee as u128).saturating_mul(gas_used_delta as u128)
            / (parent_gas_target as u128)           // PANICS if parent_gas_target == 0 (which it is when elasticity == 0)
            / (params.max_change_denominator as u128);  // PANICS if max_change_denominator == 0
        let base_fee_delta = base_fee_delta.max(1) as u64;
        parent_base_fee.saturating_add(base_fee_delta)
    } else {
        let gas_used_delta = parent_gas_target - parent_gas_used;
        let base_fee_delta = (parent_base_fee as u128).saturating_mul(gas_used_delta as u128)
            / (parent_gas_target as u128)           // PANICS if parent_gas_target == 0
            / (params.max_change_denominator as u128);  // PANICS if max_change_denominator == 0
        parent_base_fee.saturating_sub(base_fee_delta as u64)
    }
}
```

The `BaseFeeParams` struct in `/Users/will/dev/nunchi/daeji/crates/node/executor/src/config.rs` has public fields with no constructor validation:

```rust
// crates/node/executor/src/config.rs:28-46
/// EIP-1559 base fee calculation parameters.
#[derive(Clone, Debug)]
pub struct BaseFeeParams {
    /// Elasticity multiplier (default: 2).
    pub elasticity_multiplier: u64,
    /// Base fee max change denominator (default: 8).
    pub max_change_denominator: u64,
}

impl BaseFeeParams {
    /// Default base fee parameters.
    pub const DEFAULT: Self = Self { elasticity_multiplier: 2, max_change_denominator: 8 };
}

impl Default for BaseFeeParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}
```

Because both fields are `pub`, any code can construct `BaseFeeParams { elasticity_multiplier: 0, max_change_denominator: 0 }` without going through a validated constructor.

The division-by-zero paths:

1. **Line 338**: `parent_gas_limit / params.elasticity_multiplier` -- panics when `elasticity_multiplier == 0`
2. **Lines 347, 354**: `/ (parent_gas_target as u128)` -- panics when `parent_gas_target == 0`, which happens when `elasticity_multiplier == 0` (causing `parent_gas_limit / 0` to panic first, or if `parent_gas_limit == 0` too, `parent_gas_target` would be 0)
3. **Lines 348, 355**: `/ (params.max_change_denominator as u128)` -- panics when `max_change_denominator == 0`

## Impact

- **Node crash**: A zero `elasticity_multiplier` or `max_change_denominator` causes a division-by-zero panic in the block production hot path. `calculate_base_fee()` is called from `validate_base_fee()` (line 143) which is called from `validate_header()` (line 95) during both proposal building and verification. A panic here crashes the node.
- **Network-wide failure**: Since all validators use the same chain parameters (loaded from `ExecutionConfig`), a misconfiguration deployed via config update would crash every node simultaneously.
- **Unrecoverable crash-loop**: The node would crash-loop on every restart because the panic occurs during block validation, before any state is written. The same block would be re-attempted on each restart, triggering the same panic.
- **Current risk is low**: With the hardcoded `const DEFAULT` values (`elasticity_multiplier: 2`, `max_change_denominator: 8`), this cannot happen unless someone explicitly constructs bad params. However, as the configuration system evolves (e.g., config file overrides, governance-driven parameter changes, CLI flags), the risk of invalid values reaching this code increases.

## Root Cause

The `BaseFeeParams` struct uses public fields (`pub elasticity_multiplier`, `pub max_change_denominator`) with no constructor validation. The `calculate_base_fee()` function assumes both values are non-zero without defensive checks or `debug_assert!` guards. Rust's integer division panics on division by zero in both debug and release builds.

## Suggested Fix

**Option A (recommended): Validate at construction**

Add a validated constructor to `BaseFeeParams` and add `debug_assert!` in `calculate_base_fee()`:

```rust
// crates/node/executor/src/config.rs:
impl BaseFeeParams {
    /// Create validated base fee parameters.
    ///
    /// # Panics
    /// Panics if either parameter is zero.
    pub fn new(elasticity_multiplier: u64, max_change_denominator: u64) -> Self {
        assert!(elasticity_multiplier > 0, "elasticity_multiplier must be > 0");
        assert!(max_change_denominator > 0, "max_change_denominator must be > 0");
        Self { elasticity_multiplier, max_change_denominator }
    }

    pub const DEFAULT: Self = Self { elasticity_multiplier: 2, max_change_denominator: 8 };
}
```

This keeps the `pub` fields for backward compatibility but provides a safe constructor. The `const DEFAULT` remains valid since the values are non-zero.

**Option B: Defensive checks in calculate_base_fee()**

```rust
// crates/node/executor/src/revm.rs:
pub fn calculate_base_fee(
    parent_base_fee: u64,
    parent_gas_used: u64,
    parent_gas_limit: u64,
    params: &crate::BaseFeeParams,
) -> u64 {
    debug_assert!(params.elasticity_multiplier > 0, "elasticity_multiplier must be non-zero");
    debug_assert!(params.max_change_denominator > 0, "max_change_denominator must be non-zero");

    // Defensive: clamp to 1 to prevent panic in release builds
    let elasticity = params.elasticity_multiplier.max(1);
    let denominator = params.max_change_denominator.max(1);
    let parent_gas_target = parent_gas_limit / elasticity;
    // ... rest uses `denominator` instead of params.max_change_denominator
}
```

**Option C: Startup validation**

Validate `BaseFeeParams` during node startup in `ExecutionConfig::new()` or in the runner initialization, returning an error before entering the consensus loop.

**Recommendation**: Combine Options A and B. Add `BaseFeeParams::new()` for safe construction, and add `debug_assert!` guards in `calculate_base_fee()` as defense-in-depth. This way, even if someone bypasses the constructor (via direct field construction), the `debug_assert!` will catch it in test/debug builds.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/config.rs` (lines 28-46) -- add validated constructor to `BaseFeeParams`
- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` (lines 331-358) -- add `debug_assert!` guards in `calculate_base_fee()`

## Related Issues

- `088-executor-gas-limit-naming-misleading.md` -- another `validate_*` method in the same file with similar clarity concerns

## Labels

bug, reliability, executor, config
