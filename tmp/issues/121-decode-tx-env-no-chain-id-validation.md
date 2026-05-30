# `decode_tx_env` Does Not Validate Transaction Chain ID Against Node's Configured Chain ID

**Category**: bug
**Severity**: high
**Labels**: bug, security, correctness, executor

## Summary

The `decode_tx_env` function in the REVM executor accepts a `chain_id` parameter but never uses it (the parameter is prefixed with an underscore: `_chain_id`). This means the executor does not validate that a transaction's chain ID matches the node's configured chain. Pre-EIP-155 legacy transactions (which have no chain ID at all) are accepted unconditionally, enabling cross-chain replay attacks between different Kora networks or between Kora and Ethereum mainnet.

## Problem

In `crates/node/executor/src/revm.rs`, the `decode_tx_env` function at line 505 has the signature:

```rust
fn decode_tx_env(tx_bytes: &Bytes, _chain_id: u64) -> Result<revm::context::TxEnv, ExecutionError> {
```

The `_chain_id` parameter (note the underscore prefix, which in Rust indicates the value is intentionally unused) is never referenced in the function body. The function decodes the transaction envelope and passes whatever chain ID is present in the transaction directly to the REVM `TxEnv` builder, without comparing it to the node's expected chain ID.

This affects all five transaction types:

1. **Legacy (line 517-531)**: `tx.chain_id` is an `Option<u64>`. For pre-EIP-155 legacy transactions, this is `None`, and no chain ID validation happens at all. The transaction is accepted on any chain.
2. **EIP-2930 (line 533-548)**: `tx.chain_id` is set as `Some(tx.chain_id)` without validation.
3. **EIP-1559 (line 550-566)**: Same pattern.
4. **EIP-4844 (line 568-586)**: Same pattern.
5. **EIP-7702 (line 588-605)**: Same pattern.

While REVM's internal `cfg.chain_id` check may reject typed transactions (EIP-2930+) with mismatched chain IDs during execution, this relies on REVM's internal validation rather than explicit checking at the decode stage. More critically, pre-EIP-155 legacy transactions bypass this entirely because they carry no chain ID to compare against.

## Code Reference

**File**: `crates/node/executor/src/revm.rs`, line 505

```rust
fn decode_tx_env(tx_bytes: &Bytes, _chain_id: u64) -> Result<revm::context::TxEnv, ExecutionError> {
    use alloy_consensus::TxEnvelope;
    use alloy_eips::eip2718::Decodable2718 as _;

    let envelope = TxEnvelope::decode_2718(&mut tx_bytes.as_ref())
        .map_err(|e| ExecutionError::TxDecode(format!("{}", e)))?;

    let mut builder = revm::context::TxEnv::builder();

    match &envelope {
        TxEnvelope::Legacy(signed) => {
            let tx = signed.tx();
            let caller = signed.recover_signer().map_err(|e| {
                ExecutionError::TxDecode(format!("failed to recover signer: {}", e))
            })?;

            builder = builder
                .caller(caller)
                .gas_limit(tx.gas_limit)
                .gas_price(tx.gas_price)
                .value(tx.value)
                .data(tx.input.clone())
                .nonce(tx.nonce)
                .chain_id(tx.chain_id)     // <-- passes tx's chain_id directly, never validated
                .kind(convert_tx_kind(tx.to));
        }
        // ... other branches follow the same pattern ...
```

The executor's type definition at line 361 shows that transactions arrive as raw `Bytes`:

```rust
impl<S: StateDb> BlockExecutor<S> for RevmExecutor {
    type Tx = Bytes;
```

## Impact

- **Cross-chain replay attacks**: An attacker can take a signed pre-EIP-155 legacy transaction from one Kora network (e.g., a testnet) and submit it to a different Kora network (e.g., mainnet). If the sender has balances on both chains, the transaction will execute on both, allowing the attacker to drain funds.
- **Ethereum mainnet replay**: If Kora uses the same chain ID as Ethereum mainnet (which is the default -- see issue `051-config-default-chain-id-collides-mainnet.md`), legacy transactions from Ethereum mainnet could be replayed on Kora and vice versa.
- **Silent acceptance**: The function was clearly designed to accept a `chain_id` parameter for validation purposes (it is passed in at every call site), but the validation was never implemented. This is likely an oversight rather than intentional behavior.

## Root Cause

The `chain_id` parameter is passed to `decode_tx_env` at the call site but never used in the function body. The underscore prefix `_chain_id` confirms the Rust compiler was warned about the unused parameter and the warning was suppressed rather than the parameter being used. The validation logic that should compare the transaction's chain ID against the node's configured chain ID was never written.

## Suggested Fix

Validate the transaction's chain ID against the node's expected chain ID, and reject pre-EIP-155 legacy transactions that lack chain ID protection:

**Before:**
```rust
fn decode_tx_env(tx_bytes: &Bytes, _chain_id: u64) -> Result<revm::context::TxEnv, ExecutionError> {
```

**After:**
```rust
fn decode_tx_env(tx_bytes: &Bytes, chain_id: u64) -> Result<revm::context::TxEnv, ExecutionError> {
    // ... decode envelope ...

    // After decoding, validate chain_id for all transaction types
    let tx_chain_id = match &envelope {
        TxEnvelope::Legacy(signed) => signed.tx().chain_id,
        TxEnvelope::Eip2930(signed) => Some(signed.tx().chain_id),
        TxEnvelope::Eip1559(signed) => Some(signed.tx().chain_id),
        TxEnvelope::Eip4844(signed) => Some(signed.tx().tx().chain_id),
        TxEnvelope::Eip7702(signed) => Some(signed.tx().chain_id),
    };

    match tx_chain_id {
        Some(id) if id != chain_id => {
            return Err(ExecutionError::InvalidTx(format!(
                "chain_id mismatch: expected {}, got {}", chain_id, id
            )));
        }
        None => {
            // Reject pre-EIP-155 legacy transactions (no replay protection)
            return Err(ExecutionError::InvalidTx(
                "pre-EIP-155 transactions without chain_id are not accepted".to_string()
            ));
        }
        _ => {} // chain_id matches
    }

    // ... rest of function ...
}
```

If backward compatibility with pre-EIP-155 transactions is required (e.g., for replaying historical Ethereum transactions), add a configuration flag to allow them explicitly, but default to rejecting them.

## Files to Modify

- `crates/node/executor/src/revm.rs` -- line 505, add chain ID validation to `decode_tx_env`

## Related Issues

- `051-config-default-chain-id-collides-mainnet.md` -- Kora's default chain ID matches Ethereum mainnet, which compounds this replay attack risk
- `117-redundant-ecdsa-recovery-per-tx.md` -- another issue in the same `decode_tx_env` function (redundant signature recovery)
- `195-txpool-missing-eip2-signature-malleability-check.md` -- related transaction validation gap in the transaction pool
