# 077: 10 EIP-4844 Blob Transaction Handling Gaps

**Category:** executor / correctness
**Severity:** low

## Summary

Kora targets `SpecId::CANCUN` and has partial scaffolding for EIP-4844 (proto-danksharding), but the implementation is incomplete across every layer. Blob transactions (Type 3) can be decoded and pass through the txpool and executor without crashing, but blob gas economics, KZG commitment verification, sidecar storage, and several RPC fields are entirely missing. Since no blob transactions are submitted to Kora today, this is not causing runtime failures, but it blocks Cancun spec compliance and any blob-dependent use case.

## Problem

Kora is a minimal Ethereum-compatible execution client. It declares `SpecId::CANCUN`, which includes EIP-4844 (proto-danksharding). While basic blob transaction decoding and type dispatch work, 10 specific gaps were identified across the txpool, executor, and RPC layers.

### What works today

- **Blob tx decoding:** `TxEnvelope::decode_2718()` successfully decodes Type 3 transactions
- **BLOBHASH opcode:** REVM with `SpecId::CANCUN` enables the opcode, and `blob_versioned_hashes` are propagated into `TxEnv` (in `crates/node/executor/src/revm.rs:585-586`)
- **Basic type dispatch:** Txpool validator, executor, and RPC all have `Eip4844` match arms for standard fields

### 10 gaps identified

**Gap 1: `max_tx_cost()` omits blob gas from balance check**

**File:** `crates/node/txpool/src/validator.rs`, lines 241-247

```rust
fn max_tx_cost(envelope: &TxEnvelope) -> U256 {
    let gas_limit = U256::from(envelope.gas_limit());
    let max_fee = U256::from(effective_gas_price(envelope));
    let value = envelope.value();

    gas_limit * max_fee + value
}
```

For blob transactions, the correct cost is: `gas_limit * max_fee + value + blob_count * BLOB_GAS_PER_BLOB * max_fee_per_blob_gas`. The blob gas component is missing, so a sender could submit a blob transaction without having enough balance to cover blob fees.

**Gap 2: No blob-specific validation in txpool**

**File:** `crates/node/txpool/src/validator.rs`, lines 67-142

The validator's `validate()` method handles `Eip4844` in its match arms for `effective_gas_price` and `intrinsic_gas`, but performs no blob-specific validation: no blob count limits (max 6 per tx), no version byte check (must be 0x01), no blob fee floor check, and no prohibition of contract creation with blob transactions.

**Gap 3: `blob_base_fee` never populated**

**File:** `crates/node/executor/src/context.rs`, lines 23, 37, 44-47

The `BlockContext` struct has a `blob_base_fee: Option<u128>` field (line 23) and a `with_blob_base_fee()` builder (lines 44-47), but neither the runner nor the app ever calls `with_blob_base_fee()`:

**File:** `crates/node/runner/src/runner.rs`, lines 662-663
```rust
BlockContext::new(header, B256::ZERO, block.prevrandao)
    .with_recent_block_hashes(recent_hashes)
    // .with_blob_base_fee() never called
```

**File:** `crates/node/runner/src/app.rs`, line 250
```rust
BlockContext::new(header, B256::ZERO, prevrandao)
// .with_blob_base_fee() never called
```

There is no `calc_excess_blob_gas()` or `fake_exponential()` implementation anywhere in the codebase.

**Gap 4: `blob_excess_gas_and_price` conditionally set in REVM `BlockEnv` but never triggered**

**File:** `crates/node/executor/src/revm.rs`, lines 399-404

```rust
if let Some(blob_base_fee) = context.blob_base_fee {
    blk.blob_excess_gas_and_price = Some(BlobExcessGasAndPrice {
        excess_blob_gas: 0,
        blob_gasprice: blob_base_fee,
    });
}
```

Since `blob_base_fee` is always `None` (Gap 3), this branch is never taken. The `BLOBBASEFEE` opcode returns 0, and blob fee deduction may be skipped.

**Gap 5: No KZG commitment verification** -- No `c_kzg` crate dependency, no trusted setup, no `verify_blob_kzg_proof`. The point evaluation precompile at address `0x0a` cannot function.

**Gap 6: No blob sidecar storage** -- No storage, propagation, retrieval API, or pruning. Blob data is silently discarded after decoding.

**Gap 7: `eth_blobBaseFee` RPC method not implemented** -- Not present in the `EthApi` trait (`crates/node/rpc/src/eth.rs:51-188`).

**Gap 8: Block header missing blob fields**

**File:** `crates/node/rpc/src/types.rs`, lines 62-111

The `RpcBlock` struct does not include `excess_blob_gas`, `blob_gas_used`, or `parent_beacon_block_root` fields. These fields are required by the Cancun specification.

**Gap 9: `MAX_BLOB_GAS_PER_BLOCK` (786,432) not enforced**

**File:** `crates/node/executor/src/revm.rs`, lines 410-428

The block gas limit enforcement only tracks `cumulative_gas` for execution gas. There is no tracking of cumulative blob gas per block, so the 786,432 blob gas limit is not enforced.

**Gap 10: Blob data silently discarded** -- The `RpcTransaction` struct (`crates/node/rpc/src/types.rs:132-177`) does not include `max_fee_per_blob_gas` or `blob_versioned_hashes` fields. No sidecar propagation exists.

## Code Reference

**File:** `crates/node/txpool/src/validator.rs:241-247`
```rust
fn max_tx_cost(envelope: &TxEnvelope) -> U256 {
    let gas_limit = U256::from(envelope.gas_limit());
    let max_fee = U256::from(effective_gas_price(envelope));
    let value = envelope.value();

    gas_limit * max_fee + value
}
```

**File:** `crates/node/executor/src/context.rs:22-23`
```rust
/// Blob base fee for Cancun+ (EIP-4844).
pub blob_base_fee: Option<u128>,
```

## Impact

No blob transactions are submitted to Kora today, so these gaps are not causing runtime failures. However, they block:

- Serving as an L1 for rollups relying on blob data availability (e.g., optimistic or ZK rollups posting data to Kora)
- Full Cancun spec compliance (tools and indexers may flag missing header fields)
- Any blob-dependent smart contract use case (EIP-4844-aware contracts)

## Root Cause

EIP-4844 was partially scaffolded during the Cancun upgrade but never fully implemented. The blob gas economics, KZG verification, sidecar infrastructure, and RPC surface were left as stubs or omitted entirely.

## Suggested Fix

### Phase 1: Correct economic accounting (Gaps 1, 3, 4, 9)
- Implement `calc_excess_blob_gas()` and `fake_exponential()` for blob base fee derivation
- Populate `blob_base_fee` in `BlockContext` at both construction sites (runner.rs and app.rs)
- Set `blob_excess_gas_and_price` in REVM `BlockEnv` (the code exists, it just needs the input)
- Track cumulative blob gas and enforce `MAX_BLOB_GAS_PER_BLOCK`
- Fix `max_tx_cost()` to include the blob gas component
- Set `excess_blob_gas` and `blob_gas_used` to `Some(0)` in block headers

### Phase 2: Validation and RPC completeness (Gaps 2, 7, 8, 10)
- Add blob-specific txpool validation (count limits, version byte, fee floor, no contract creation)
- Implement `eth_blobBaseFee` RPC method
- Add blob fields to `RpcBlock`, `RpcTransactionReceipt`, and `RpcTransaction`

### Phase 3: KZG verification (Gap 5)
- Integrate `c_kzg` crate with Ethereum trusted setup
- Verify KZG commitments against versioned hashes
- Enable point evaluation precompile at `0x0a`

### Phase 4: Sidecar infrastructure (Gaps 6, 10)
- Design blob sidecar storage with configurable retention
- Add sidecar propagation in P2P layer
- Implement blob retrieval API
- Add blob pruning after retention period

## Files to Modify

- `crates/node/txpool/src/validator.rs` -- blob gas in `max_tx_cost()`, blob-specific validation
- `crates/node/executor/src/context.rs` -- `blob_base_fee` population
- `crates/node/executor/src/revm.rs` -- `blob_excess_gas_and_price` in `BlockEnv`, cumulative blob gas tracking
- `crates/node/runner/src/runner.rs` (lines 653-664) -- `BlockContext` construction with blob base fee
- `crates/node/runner/src/app.rs` (lines 234-251) -- `BlockContext` construction with blob base fee
- `crates/node/rpc/src/eth.rs` -- `eth_blobBaseFee` method
- `crates/node/rpc/src/types.rs` (lines 62-111, 132-177, 182-214) -- blob fields in `RpcBlock`, `RpcTransaction`, `RpcTransactionReceipt`

## Related Issues

- `076-executor-missing-eip4788-beacon-root.md` -- another missing Cancun component (EIP-4788)
- `078-rpc-missing-standard-methods.md` -- `eth_blobBaseFee` is among the missing RPC methods

## Labels

enhancement, correctness, executor, rpc, txpool
