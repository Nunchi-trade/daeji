# Transaction Pool Re-encodes Transaction on Every ID Lookup, Risking Hash Mismatch

**Category**: Transaction Pool / Correctness + Performance
**Severity**: Medium

## Summary

The `ordered_tx_id()` function in the transaction pool reconstructs a `Tx` from a decoded `TxEnvelope` by re-encoding it via `encode_2718()`, then computes the `TxId` as `keccak256(self.encode())` (which applies an additional codec length prefix). If re-encoding produces bytes different from the original raw transaction (e.g., due to non-canonical RLP encoding), the resulting `TxId` will not match the one computed at insertion time, causing failures in pruning and exclusion checks. Additionally, this re-encoding + hashing is performed on every call in hot paths, creating unnecessary allocation and CPU pressure.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/pool.rs`, the `ordered_to_tx()` and `ordered_tx_id()` functions at lines 604-612 convert an `OrderedTransaction` back to a `Tx` by re-encoding the decoded `TxEnvelope`:

```rust
// crates/node/txpool/src/pool.rs:604-612
fn ordered_to_tx(tx: &OrderedTransaction) -> Tx {
    let mut raw = Vec::new();
    tx.envelope.encode_2718(&mut raw);
    Tx::new(Bytes::from(raw))
}

fn ordered_tx_id(tx: &OrderedTransaction) -> TxId {
    ordered_to_tx(tx).id()
}
```

The `Tx::id()` method (in `crates/node/domain/src/tx.rs:27-29`) computes:

```rust
// crates/node/domain/src/tx.rs:27-29
pub fn id(&self) -> TxId {
    TxId(keccak256(self.encode()))
}
```

Where `self.encode()` uses the `commonware_codec::Write` trait to serialize the `bytes` field with a length prefix. So `TxId = keccak256(length_prefix + raw_tx_bytes)`.

**Correctness risk**: The `TxId` depends on `encode_2718()` producing byte-identical output to the original raw bytes that were used at insertion time. If the original bytes contain non-canonical RLP encoding (e.g., non-minimal length prefixes, leading zero padding), the re-encoding via `encode_2718()` will produce the canonical form, which differs from the original. In that case:

1. The `TxId` computed at pool insertion (via `tx_to_ordered()` at line 636, which calls `alloy_primitives::keccak256(&tx.bytes)` for the hash but constructs the `OrderedTransaction` without preserving the raw bytes) will differ from the `TxId` computed by `ordered_tx_id()`.
2. `prune()` (line 717) will fail to find the transaction in `by_id` because the ID does not match.
3. `build()`'s exclusion check via `next_candidate()` (line 45) will miss already-included transactions, potentially allowing double-inclusion.

**Performance overhead**: `ordered_tx_id()` is called in hot paths:
- `next_candidate()` (line 45) calls it for each candidate to check against the `excluded` set.
- `next_candidate()` is called from `build()` (line 701) in the inner selection loop.
- With `max_txs=1000` and thousands of candidates at 34 blocks/second, this performs thousands of allocations + encodings + hashes per block.

## Code Reference

The `OrderedTransaction` struct stores the decoded `TxEnvelope` and extracted fields, but does not store the original raw bytes or pre-computed `TxId`:

```rust
// crates/node/txpool/src/ordering.rs:10-23
pub struct OrderedTransaction {
    /// Transaction hash.
    pub hash: B256,
    /// Sender address recovered from signature.
    pub sender: Address,
    /// Transaction nonce.
    pub nonce: u64,
    /// Effective gas price for ordering.
    pub effective_gas_price: u128,
    /// Timestamp when transaction was received.
    pub timestamp: u64,
    /// The decoded transaction envelope.
    pub envelope: TxEnvelope,
}
```

The `next_candidate()` function calls `ordered_tx_id()` on the hot path:

```rust
// crates/node/txpool/src/pool.rs:34-55
impl BuildSenderState {
    fn next_candidate(&mut self, excluded: &BTreeSet<TxId>) -> Option<OrderedTransaction> {
        while let Some(tx) = self.txs.get(self.index) {
            if tx.nonce < self.expected_nonce {
                self.index += 1;
                continue;
            }

            if tx.nonce > self.expected_nonce {
                return None;
            }

            if excluded.contains(&ordered_tx_id(tx)) {  // <-- hot path re-encode + hash
                self.expected_nonce = tx.nonce.saturating_add(1);
                self.index += 1;
                continue;
            }

            return Some(tx.clone());
        }

        None
    }
    // ...
}
```

## Impact

- **Correctness**: Transactions with non-canonical EIP-2718 encodings may become unprunable from the pool, causing unbounded memory growth. In the worst case, a transaction could be double-included in a block if `next_candidate()` fails to match it against the `excluded` set due to a hash mismatch, which would cause consensus divergence between validators.
- **Performance**: At 34 blocks/s with up to 1000 candidates per block, this creates approximately 34,000 allocate + encode + hash operations per second. Each operation allocates a `Vec<u8>`, encodes the full transaction envelope (~200 bytes typical), wraps it in `Bytes`, applies codec encoding, then computes `keccak256`. This creates allocation pressure and cache pollution on the consensus-critical hot path.

## Root Cause

`OrderedTransaction` stores the decoded `TxEnvelope` and extracted metadata (sender, nonce, gas price) but does not store the original `TxId` or raw bytes. This forces reconstruction from the decoded envelope whenever an ID is needed. The `TxId` computation path goes through `Tx::id()` which applies a codec length prefix before hashing, adding another layer of encoding.

## Suggested Fix

Store the pre-computed `TxId` and original raw bytes in `OrderedTransaction` at insertion time:

```rust
// BEFORE (crates/node/txpool/src/ordering.rs):
pub struct OrderedTransaction {
    pub hash: B256,
    pub sender: Address,
    pub nonce: u64,
    pub effective_gas_price: u128,
    pub timestamp: u64,
    pub envelope: TxEnvelope,
}

// AFTER:
pub struct OrderedTransaction {
    pub hash: B256,
    pub sender: Address,
    pub nonce: u64,
    pub effective_gas_price: u128,
    pub timestamp: u64,
    pub envelope: TxEnvelope,
    pub tx_id: TxId,      // cached at insertion time from original bytes
    pub raw_bytes: Bytes,  // original bytes for Tx reconstruction
}
```

Then replace the hot-path functions with simple field accesses:

```rust
// BEFORE (crates/node/txpool/src/pool.rs:604-612):
fn ordered_to_tx(tx: &OrderedTransaction) -> Tx {
    let mut raw = Vec::new();
    tx.envelope.encode_2718(&mut raw);
    Tx::new(Bytes::from(raw))
}

fn ordered_tx_id(tx: &OrderedTransaction) -> TxId {
    ordered_to_tx(tx).id()
}

// AFTER:
fn ordered_to_tx(tx: &OrderedTransaction) -> Tx {
    Tx::new(tx.raw_bytes.clone())  // zero-copy via Bytes::clone() (Arc increment)
}

fn ordered_tx_id(tx: &OrderedTransaction) -> TxId {
    tx.tx_id  // simple field access, no allocation or hashing
}
```

Update the insertion path to populate the new fields:

```rust
// In tx_to_ordered() (crates/node/txpool/src/pool.rs:636-657):
fn tx_to_ordered(tx: &Tx) -> Option<OrderedTransaction> {
    let raw_bytes = tx.bytes.clone();
    let tx_id = tx.id();  // compute once from original bytes
    let envelope = TxEnvelope::decode_2718(&mut tx.bytes.as_ref()).ok()?;
    // ... existing sender recovery and field extraction ...
    Some(OrderedTransaction::new(
        hash, sender, nonce, effective_gas_price, current_timestamp(), envelope,
        tx_id, raw_bytes,
    ))
}
```

Trade-off: ~64 bytes additional memory per pooled transaction (32-byte `TxId` + ~32 bytes `Bytes` handle), negligible compared to the `TxEnvelope` itself.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/ordering.rs` -- add `tx_id: TxId` and `raw_bytes: Bytes` fields to `OrderedTransaction`
- `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/pool.rs` (lines 604-612) -- replace `ordered_to_tx()` and `ordered_tx_id()` with field accesses
- `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/pool.rs` (lines 636-657) -- update `tx_to_ordered()` to populate new fields at insertion time

## Related Issues

- `084-txpool-build-quadratic-scaling.md` -- the `build()` method calls `ordered_tx_id()` in its inner loop; fixing this issue reduces the constant factor of that quadratic cost

## Labels

bug, performance, correctness, txpool
