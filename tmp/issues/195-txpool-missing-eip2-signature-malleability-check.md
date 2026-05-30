# Missing EIP-2 Low-S Signature Malleability Check in Transaction Validation

**Category**: bug -- txpool
**Severity**: critical

## Summary

The transaction validator in Kora's transaction pool accepts ECDSA signatures with high `s` values, violating the EIP-2 (Homestead) requirement that `s` must be in the lower half of the secp256k1 curve order. This allows an observer to take any valid pending transaction and produce a second valid transaction with a different hash but the same sender and nonce, polluting the mempool and breaking hash-based transaction identity.

## Problem

The `recover_sender_and_hash()` function in the transaction validator recovers the ECDSA signer from `(r, s, v)` but never checks that `s <= secp256k1_n / 2`. Per EIP-2, post-Homestead Ethereum requires the `s` value to be in the lower half of the secp256k1 curve order to prevent transaction malleability.

The `k256` crate's `Signature::from_slice()` only verifies that the signature bytes are a valid encoding -- it does **not** reject high-s values by default.

Without this check, an attacker can take any valid transaction in the mempool, compute `s' = secp256k1_n - s` and flip `v`, creating a second valid transaction with a different hash but the same sender and nonce. Both would pass validation and enter the pool because they have different hashes (so `AlreadyExists` does not fire), and the `NonceAlreadyInPool` check only fires when a pool reference is attached to the validator.

## Code Reference

File: `crates/node/txpool/src/validator.rs`, lines 164-191

```rust
fn recover_sender_and_hash(envelope: &TxEnvelope) -> Result<(Address, B256), TxPoolError> {
    let hash = keccak256(alloy_rlp::encode(envelope));

    let signature = envelope.signature();
    let r_be = B256::from_slice(&signature.r().to_be_bytes::<32>());
    let s_be = B256::from_slice(&signature.s().to_be_bytes::<32>());
    let mut sig_bytes = [0u8; 64];
    sig_bytes[..32].copy_from_slice(r_be.as_slice());
    sig_bytes[32..].copy_from_slice(s_be.as_slice());

    let sig = Signature::from_slice(&sig_bytes).map_err(|_| TxPoolError::InvalidSignature)?;
    // ^^^ No check that s is in the lower half of the curve order (EIP-2)

    let v = signature.v();
    let v_val: u64 = if v { 1 } else { 0 };
    let recovery_id =
        RecoveryId::try_from(v_val as u8).map_err(|_| TxPoolError::InvalidSignature)?;

    let signing_hash = envelope.signature_hash();
    let verifying_key =
        VerifyingKey::recover_from_prehash(signing_hash.as_slice(), &sig, recovery_id)
            .map_err(|_| TxPoolError::InvalidSignature)?;

    let pubkey = verifying_key.to_encoded_point(false);
    let pubkey_bytes = pubkey.as_bytes();
    let pubkey_hash = Keccak256::digest(&pubkey_bytes[1..]);
    let sender = Address::from_slice(&pubkey_hash[12..]);

    Ok((sender, hash))
}
```

## Impact

1. **Transaction malleability**: An observer can mutate any pending transaction's signature to produce a second valid entry in the pool with a different hash. This is a fundamental Ethereum consensus violation.
2. **Pool pollution**: Each malleable variant doubles pool entries for a single logical transaction, wasting pool capacity.
3. **Hash identity broken**: Any code relying on `tx.hash` being canonical for a given sender+nonce pair will behave incorrectly. This includes RPC methods like `eth_getTransactionByHash` and mempool event subscribers.
4. **Duplicate inclusion risk**: Both malleable variants could be presented to the block builder. While the EVM executor would reject the second one (same nonce), it still wastes block space and computation.

## Root Cause

The signature recovery path validates format correctness of the signature (valid `r` and `s` scalar values) but does not enforce the EIP-2 low-s requirement. The `k256` crate's `Signature::from_slice()` intentionally does not enforce this -- it is the caller's responsibility.

## Suggested Fix

After constructing the `k256::ecdsa::Signature`, check `sig.normalize_s().is_some()`. This method returns `Some(normalized_sig)` if `s` was in the upper half and needed normalization, meaning the original signature violates EIP-2.

**Before:**
```rust
let sig = Signature::from_slice(&sig_bytes).map_err(|_| TxPoolError::InvalidSignature)?;
```

**After:**
```rust
let sig = Signature::from_slice(&sig_bytes).map_err(|_| TxPoolError::InvalidSignature)?;
if sig.normalize_s().is_some() {
    return Err(TxPoolError::InvalidSignature); // EIP-2: reject high-s value
}
```

## Files to Modify

- `crates/node/txpool/src/validator.rs` -- Add low-s check after `Signature::from_slice()` on line 174

## Related Issues

- `197-txpool-mempool-insert-bypasses-validation.md` -- The `Mempool::insert()` path also performs signature recovery (via `recover_sender_from_envelope`) without a low-s check, so both paths need fixing.

## Labels

bug, security, correctness, txpool
