# Redundant ECDSA Signature Recovery on Every Transaction During Block Execution

**Category**: performance
**Severity**: medium
**Labels**: performance, executor, enhancement

## Summary

The `decode_tx_env` function in the REVM executor calls `recover_signer()` for every transaction during block execution. ECDSA signature recovery is computationally expensive (~350us per call). By the time a transaction reaches the executor, the sender has already been recovered and validated by the transaction pool. This work is repeated three times per finalized block (propose, verify, finalize), wasting significant CPU time.

## Problem

In Kora's block execution pipeline, transactions arrive at the executor as raw `Bytes` (the `BlockExecutor::Tx` type is set to `Bytes` at line 361 of `crates/node/executor/src/revm.rs`). The `decode_tx_env` function (line 505) must decode these raw bytes and recover the signer address from the ECDSA signature for each transaction.

However, the sender address has already been recovered during transaction pool validation when the transaction was first received. Since the executor only receives raw bytes, this pre-recovered sender is lost and must be recomputed.

The function handles all five Ethereum transaction types (Legacy, EIP-2930, EIP-1559, EIP-4844, EIP-7702), and every branch calls `signed.recover_signer()`:

- Line 519: Legacy transactions
- Line 535: EIP-2930 transactions
- Line 552: EIP-1559 transactions
- Line 570: EIP-4844 transactions
- Line 590: EIP-7702 transactions

Each block execution triggers this recovery for every transaction, and Kora's architecture executes each block three times (propose, verify, finalize) as described in issue `014-triple-block-execution.md`.

## Code Reference

**File**: `crates/node/executor/src/revm.rs`, lines 361 and 505-607

The executor type definition:
```rust
impl<S: StateDb> BlockExecutor<S> for RevmExecutor {
    type Tx = Bytes;    // <-- raw bytes, no pre-recovered sender
```

The decode function (showing one of five identical patterns):
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
            let caller = signed.recover_signer().map_err(|e| {   // <-- expensive ECDSA recovery
                ExecutionError::TxDecode(format!("failed to recover signer: {}", e))
            })?;

            builder = builder
                .caller(caller)
                .gas_limit(tx.gas_limit)
                // ... rest of fields ...
        }
        // EIP-2930, EIP-1559, EIP-4844, EIP-7702 all follow the same pattern
        // with recover_signer() at lines 535, 552, 570, 590
```

## Impact

- **CPU overhead**: At 350us per ECDSA recovery and 100 transactions per block, this adds ~35ms of pure CPU time per block execution. With triple execution (propose, verify, finalize), that becomes ~105ms of wasted ECDSA computation per finalized block.
- **Throughput reduction**: On a 10-node devnet producing ~33 blocks/s, this represents ~3.5 seconds of wasted CPU per second of chain operation (105ms x 33). As transaction counts increase, this becomes a throughput bottleneck.
- **Scalability**: The cost scales linearly with both the number of transactions per block and the number of execution phases. Any future optimization that reduces triple execution to double execution would still leave redundant recovery in two phases.

## Root Cause

The `BlockExecutor` trait defines its associated type `Tx` as `Bytes`, which carries only the raw RLP-encoded transaction without any metadata. The pre-recovered sender address from transaction pool validation is not propagated to the executor. This forces the executor to re-derive the sender from the signature every time `decode_tx_env` is called.

## Suggested Fix

Change the executor's transaction type to carry the pre-recovered sender address alongside the raw transaction bytes, eliminating the need for repeated ECDSA recovery.

**Option 1: New wrapper type**

```rust
/// A transaction with a pre-recovered sender address.
pub struct ValidatedTx {
    pub sender: Address,
    pub raw: Bytes,
}

impl<S: StateDb> BlockExecutor<S> for RevmExecutor {
    type Tx = ValidatedTx;  // <-- carries sender
    // ...
}
```

**Option 2: Change `decode_tx_env` signature**

```rust
fn decode_tx_env(
    tx_bytes: &Bytes,
    sender: Address,      // <-- pre-recovered sender passed in
    chain_id: u64,
) -> Result<revm::context::TxEnv, ExecutionError> {
    // Decode the transaction envelope for fields, but use the provided sender
    // instead of calling recover_signer()
}
```

Both options require updating the consensus/marshal layer to pass the sender address through to the executor. The transaction pool already has the sender available as `Tx::sender` when it validates incoming transactions.

## Files to Modify

- `crates/node/executor/src/revm.rs` -- change `type Tx = Bytes` and update `decode_tx_env`
- `crates/node/executor/src/lib.rs` -- update `BlockExecutor` trait if needed
- `crates/node/runner/src/runner.rs` -- update call sites that pass transactions to the executor
- `crates/network/marshal/src/actor.rs` -- propagate sender through the consensus pipeline

## Related Issues

- `014-triple-block-execution.md` -- the broader triple execution issue; this ECDSA recovery cost compounds with the triple execution overhead
- `121-decode-tx-env-no-chain-id-validation.md` -- another issue in the same `decode_tx_env` function (unused `_chain_id` parameter)
