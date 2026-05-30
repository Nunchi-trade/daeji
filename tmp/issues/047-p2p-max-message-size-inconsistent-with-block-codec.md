# 047: MAX_MESSAGE_SIZE (1 MB) Is Inconsistent With Block Codec Limits (10K txs, 8 MB/tx)

**Category:** bug
**Severity:** medium

**Labels:** `bug`, `p2p`, `consensus`, `reliability`

## Summary

The P2P transport layer's `DEFAULT_MAX_MESSAGE_SIZE` is 1 MB (1,048,576 bytes), but the block codec configuration allows blocks that are far larger. The block codec permits up to 10,000 transactions per block and up to 8 MB per individual transaction. These limits are fundamentally inconsistent: a valid block that the consensus layer builds and approves may be too large for the transport layer to transmit to other validators, causing the block to be silently dropped and the view to nullify.

## Problem

The transport message size limit and the block codec limits are configured independently in different crates with no cross-validation.

Transport message size limit at `crates/network/transport/src/config.rs:12`:

```rust
/// Default maximum message size (1 MB).
pub const DEFAULT_MAX_MESSAGE_SIZE: u32 = 1024 * 1024;
```

Block codec limits at `crates/node/config/src/consensus.rs:19-22`:

```rust
/// Default maximum transactions decoded per block.
pub const DEFAULT_BLOCK_CODEC_MAX_TXS: usize = 10_000;

/// Default maximum bytes decoded per transaction in a block.
pub const DEFAULT_BLOCK_CODEC_MAX_TX_BYTES: usize = 8 * 1024 * 1024;
```

The `DEFAULT_MAX_MESSAGE_SIZE` of 1 MB is used when constructing both the `local` and `recommended` transport configs in `crates/network/transport/src/ext.rs:52-58` and `crates/network/transport/src/ext.rs:75-81`, and flows through to `crates/network/transport/src/config.rs:84-98` (for `recommended`) and `crates/network/transport/src/config.rs:103-125` (for `local`).

A fully-packed block could easily exceed 1 MB:
- A block with 10,000 transactions averaging just 100 bytes each = ~1 MB of transaction data alone
- Block header, state root, consensus metadata, and RLP encoding overhead add more
- A single large contract deployment transaction can be several hundred KB
- Even a modest block with 500 transactions at 250 bytes average = 125 KB of tx data, which with overhead approaches the limit under certain conditions

## Impact

If a proposing leader builds a block whose serialized size exceeds the transport's 1 MB message limit, the block cannot be disseminated to other validators over the P2P network. The block is silently dropped by the transport layer, and other validators never receive the proposal. This causes the view to nullify (the leader proposed but no one received the proposal), and the leader would repeatedly produce un-disseminable blocks until its slot passes in the round-robin schedule.

This is a **latent issue** -- it only manifests when blocks are large enough, which depends on transaction volume and transaction sizes. Under the current devnet load (mostly simple ETH transfers at ~110 bytes each), blocks are well under 1 MB. But under production load with contract deployments (which can be 10-100 KB each) and complex DeFi transactions with large calldata, this limit could be hit. At 30 blocks/s with 250M gas per block, a fully utilized block with typical DeFi transactions could exceed 1 MB.

## Root Cause

The transport message size limit and the block codec limits were configured independently in different crates (`kora_transport` vs. `kora_config`) without considering their interdependence. There is no startup validation that the transport can carry the maximum possible block.

## Suggested Fix

**Option 1 (recommended):** Increase `DEFAULT_MAX_MESSAGE_SIZE` to accommodate realistic maximum block sizes. A conservative value would be 16 MB:

```rust
/// Default maximum message size (16 MB).
/// Must be larger than the maximum possible serialized block size.
pub const DEFAULT_MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;
```

**Option 2:** Add a validation check during block construction that the serialized block size fits within the transport limit. The block builder should truncate the transaction list if adding more transactions would cause the serialized block to exceed the transport limit:

```rust
fn build_block(&self, txs: Vec<Tx>, max_transport_size: usize) -> Block {
    let mut block = Block::new(header);
    let mut serialized_size = header_size;
    for tx in txs {
        if serialized_size + tx.bytes.len() + ENCODING_OVERHEAD > max_transport_size {
            break; // Stop adding transactions
        }
        block.add_tx(tx);
        serialized_size += tx.bytes.len() + ENCODING_OVERHEAD;
    }
    block
}
```

**Option 3:** Add a startup check that validates the relationship between these limits and logs a warning if they are inconsistent:

```rust
if max_message_size < estimated_max_block_size(codec_config) {
    warn!(
        "Transport MAX_MESSAGE_SIZE ({}) is smaller than estimated max block size ({}). \
         Large blocks may be dropped.",
        max_message_size, estimated_max_block_size
    );
}
```

## Files to Modify

- `crates/network/transport/src/config.rs` -- `DEFAULT_MAX_MESSAGE_SIZE` at line 12
- `crates/node/config/src/consensus.rs` -- `DEFAULT_BLOCK_CODEC_MAX_TXS` at line 19 and `DEFAULT_BLOCK_CODEC_MAX_TX_BYTES` at line 22
- Optionally: `crates/node/runner/src/runner.rs` or startup validation -- add consistency check

## Related Issues

- `045-p2p-uniform-rate-quota-all-channels.md` -- Uniform rate quota combined with large message size allows bandwidth abuse
