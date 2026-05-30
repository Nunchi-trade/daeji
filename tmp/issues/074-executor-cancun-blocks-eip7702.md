# 074: SpecId::CANCUN Hardcoded -- No Upgrade Path to PRAGUE / EIP-7702

**Category:** executor / config
**Severity:** low

## Summary

The executor hardcodes `SpecId::CANCUN` as the active Ethereum hardfork with no mechanism to upgrade to newer forks at runtime or via configuration. EIP-7702 (set EOA account code for the duration of a transaction), introduced in the Prague hardfork, is fully implemented throughout the stack (decoding, validation, authorization list conversion) but is gated off at execution time because REVM requires `SpecId::PRAGUE` or later. The `spec_id` is not exposed in the node's TOML configuration, so operators cannot enable Prague without code changes.

## Problem

Kora is a minimal Ethereum-compatible execution client using REVM for EVM execution. The `ExecutionConfig` struct sets `SpecId::CANCUN` as a compile-time constant:

**File:** `crates/node/executor/src/config.rs`, lines 63-70

```rust
pub const fn new(chain_id: u64) -> Self {
    Self {
        chain_id,
        spec_id: SpecId::CANCUN,
        gas_limit_bounds: GasLimitBounds::DEFAULT,
        base_fee_params: BaseFeeParams::DEFAULT,
    }
}
```

A `with_spec_id()` builder method exists (lines 73-77) but is only used in tests:

```rust
#[must_use]
pub const fn with_spec_id(mut self, spec_id: SpecId) -> Self {
    self.spec_id = spec_id;
    self
}
```

The full EIP-7702 pipeline is already implemented across the stack:

- **Transaction decoding** (`crates/node/executor/src/revm.rs:588-606`): handles `TxEnvelope::Eip7702`, recovers signer, sets gas, value, data, access list, and authorization list
- **Authorization list conversion** (`crates/node/executor/src/revm.rs:636-669`): converts alloy authorization structs to REVM format with proper authority recovery
- **TxPool validation** (`crates/node/txpool/src/validator.rs:200,225`): `Eip7702` variant is handled in `effective_gas_price()` and `intrinsic_gas()`
- **RPC layer** (`crates/node/rpc/src/eth.rs:1344-1355,1358-1365,1377-1383`): `Eip7702` variant is handled in `signature_v()`, `transaction_type()`, `effective_gas_price()`, `max_fee_per_gas()`, and `max_priority_fee_per_gas()`

All of this code is live and correct, but REVM silently rejects EIP-7702 transactions when `spec_id < PRAGUE`.

The node's configuration schema (in `crates/node/config/`) does not include a `spec_id` or `hardfork` field, so there is no way to enable Prague without modifying source code.

## Code Reference

**File:** `crates/node/executor/src/config.rs:63-70`
```rust
pub const fn new(chain_id: u64) -> Self {
    Self {
        chain_id,
        spec_id: SpecId::CANCUN,     // <-- hardcoded, blocks EIP-7702
        gas_limit_bounds: GasLimitBounds::DEFAULT,
        base_fee_params: BaseFeeParams::DEFAULT,
    }
}
```

**File:** `crates/node/executor/src/config.rs:73-77`
```rust
#[must_use]
pub const fn with_spec_id(mut self, spec_id: SpecId) -> Self {
    self.spec_id = spec_id;
    self
}
```

## Impact

- **Account abstraction unavailable:** EIP-7702 enables smart-wallet UX patterns (session keys, social recovery, batched calls via delegation) that are increasingly standard on Ethereum mainnet and L2s. Kora cannot support these patterns.
- **Silent execution failures:** Wallets or SDKs that default to EIP-7702 (Type 4) transactions on Prague-era chains will pass Kora's txpool validation but fail during EVM execution with a generic error, making debugging difficult.
- **Low urgency for current devnet:** `SpecId::CANCUN` is sufficient for most current dApp needs. This is a forward-looking compatibility concern.

## Root Cause

The spec ID is hardcoded as a constant in `ExecutionConfig::new()` and there is no configuration plumbing to override it. The `with_spec_id()` builder method exists but is only used in tests. The node configuration schema does not include a `spec_id` or `hardfork` field.

## Suggested Fix

1. Add a `spec_id` field to the node's TOML configuration:
   ```toml
   [execution]
   spec_id = "PRAGUE"  # or "CANCUN" (default)
   ```

2. Parse and pass the spec ID through to `ExecutionConfig::with_spec_id()` in the runner's initialization path.

3. Default remains `CANCUN` for backward compatibility.

4. Add a startup log line when a non-default spec ID is active so operators are aware.

5. Document that switching to `PRAGUE` may require the EIP-4788 beacon root system contract at genesis (see issue 076).

## Files to Modify

- `crates/node/executor/src/config.rs` (lines 63-70) -- `ExecutionConfig::new()` hardcodes `SpecId::CANCUN`
- `crates/node/executor/src/config.rs` (lines 73-77) -- `with_spec_id()` builder (exists but unused in production)
- `crates/node/config/src/node.rs` -- config schema needs a `spec_id` field
- `crates/node/config/src/consensus.rs` -- may need spec_id plumbing

## Related Issues

- `076-executor-missing-eip4788-beacon-root.md` -- missing EIP-4788 beacon root system contract, required for full Prague compliance

## Labels

enhancement, executor, config
