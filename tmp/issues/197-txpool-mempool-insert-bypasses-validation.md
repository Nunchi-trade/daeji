# Mempool::insert() Bypasses All Validation and Computes Hash Differently

**Category**: bug -- txpool
**Severity**: critical

## Summary

The `Mempool::insert()` trait implementation provides a public insertion path into the transaction pool that skips all validation checks (chain ID, gas price, intrinsic gas, nonce, balance, sender limits, transaction size). Additionally, the `tx_to_ordered()` helper function used by this path computes the transaction hash differently from the validator's `recover_sender_and_hash()`, meaning the same transaction inserted via different paths will have different hashes in the pool's lookup maps.

## Problem

There are two independent code paths for inserting transactions into the pool:

1. **Validator path** (used by RPC and gossip handlers): `TransactionValidator::validate()` -> `TransactionPool::add()`. This path performs full validation and computes the hash via `keccak256(alloy_rlp::encode(envelope))`.

2. **Mempool trait path**: `Mempool::insert()` -> `tx_to_ordered()` -> `TransactionPool::add()`. This path only decodes RLP and recovers the sender -- no validation. It computes the hash via `keccak256(&tx.bytes)`.

The hash mismatch is the more critical immediate bug. For EIP-2718 typed transactions, `tx.bytes` is the 2718-encoded form (`type_byte || rlp_body`), while `alloy_rlp::encode(envelope)` wraps this in an outer RLP string. These produce different hashes for typed transactions.

## Code Reference

**Mempool::insert path** -- File: `crates/node/txpool/src/pool.rs`, lines 636-675

```rust
fn tx_to_ordered(tx: &Tx) -> Option<OrderedTransaction> {
    let envelope = TxEnvelope::decode_2718(&mut tx.bytes.as_ref()).ok()?;
    let sender = recover_sender_from_envelope(&envelope).ok()?;
    let hash = alloy_primitives::keccak256(&tx.bytes);  // <-- hash of raw 2718 bytes
    let nonce = envelope.nonce();
    let effective_gas_price = match &envelope {
        TxEnvelope::Legacy(tx) => tx.tx().gas_price,
        TxEnvelope::Eip2930(tx) => tx.tx().gas_price,
        TxEnvelope::Eip1559(tx) => tx.tx().max_fee_per_gas,
        TxEnvelope::Eip4844(tx) => tx.tx().tx().max_fee_per_gas,
        TxEnvelope::Eip7702(tx) => tx.tx().max_fee_per_gas,
    };

    Some(OrderedTransaction::new(
        hash,
        sender,
        nonce,
        effective_gas_price,
        current_timestamp(),
        envelope,
    ))
}

impl Mempool for TransactionPool {
    fn insert(&self, tx: Tx) -> bool {
        let Some(ordered) = tx_to_ordered(&tx) else {
            trace!("failed to decode transaction for mempool insert");
            self.record_rejection("decode_error");
            return false;
        };

        match self.add(ordered) {
            Ok(()) => true,
            Err(e) => {
                trace!(?e, "failed to insert transaction");
                self.record_rejection(&rejection_reason(&e));
                false
            }
        }
    }
    // ...
}
```

**Validator path** -- File: `crates/node/txpool/src/validator.rs`, lines 164-165

```rust
fn recover_sender_and_hash(envelope: &TxEnvelope) -> Result<(Address, B256), TxPoolError> {
    let hash = keccak256(alloy_rlp::encode(envelope));  // <-- hash of RLP-wrapped envelope
    // ...
}
```

## Impact

1. **Hash mismatch**: The same logical transaction inserted via `Mempool::insert()` and via the validator path will have different `hash` values in the pool's `by_hash` map. This means `pool.contains(hash)` and `pool.get(hash)` will return incorrect results depending on which insertion path was used.
2. **Duplicate bypass**: The `AlreadyExists` check in `add()` uses `by_hash`, so it will not detect duplicates across paths because the hashes differ.
3. **Unsafe API surface**: The `Mempool` trait is the public interface for transaction insertion. While current callers (gossip handler, RPC handler) validate before calling `insert()`, any new caller of `Mempool::insert()` would bypass all safety checks (chain ID, gas price, intrinsic gas, nonce, balance).
4. **Neither hash is the canonical Ethereum transaction hash**: The canonical Ethereum tx hash is `keccak256(encode_2718(envelope))` -- neither `keccak256(alloy_rlp::encode(envelope))` nor `keccak256(&tx.bytes)` is guaranteed to produce the canonical hash.

## Root Cause

Two independent code paths for transaction ingestion were developed without sharing a common hash computation or validation layer. The `tx_to_ordered()` function duplicates hash computation with a subtly different encoding method.

## Suggested Fix

1. **Unify hash computation**: Both paths should use the canonical Ethereum transaction hash: `keccak256(encode_2718(envelope))`, available via `envelope.tx_hash()` or `keccak256(envelope.encoded_2718())`.

2. **Enforce validation in insert**: Either make `Mempool::insert()` call the validator, or at minimum enforce a subset of critical checks (chain ID, intrinsic gas, balance).

**Before (pool.rs line 639):**
```rust
let hash = alloy_primitives::keccak256(&tx.bytes);
```

**After:**
```rust
let hash = *envelope.tx_hash();
```

**Before (validator.rs line 165):**
```rust
let hash = keccak256(alloy_rlp::encode(envelope));
```

**After:**
```rust
let hash = *envelope.tx_hash();
```

## Files to Modify

- `crates/node/txpool/src/pool.rs` -- Fix hash computation in `tx_to_ordered()` (line 639) and add validation to `Mempool::insert()` (lines 660-675)
- `crates/node/txpool/src/validator.rs` -- Fix hash computation in `recover_sender_and_hash()` (line 165)

## Related Issues

- `195-txpool-missing-eip2-signature-malleability-check.md` -- The `recover_sender_from_envelope()` called from `tx_to_ordered()` also lacks the EIP-2 low-s check

## Labels

bug, security, correctness, txpool
