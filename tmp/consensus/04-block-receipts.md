# 04 — `eth_getBlockReceipts`

## Problem

`eth_getBlockReceipts` returns `-32601: Method not found`. This method is
needed by the explorer to render transaction detail panels and is part of the
standard Ethereum JSON-RPC API (EIP-1474 extension).

## Root Cause

The method is simply not defined in the `EthApi` trait or implemented anywhere.
The infrastructure to support it already exists: `BlockIndex` stores receipts
per-block, and `IndexedStateProvider` already has `receipt_by_hash()` which
returns individual receipts.

## Design

### Add to the `EthApi` Trait

Add `eth_getBlockReceipts` to the existing `EthApi` trait in `eth.rs`:

```rust
/// Returns all receipts for a given block.
#[method(name = "getBlockReceipts")]
async fn get_block_receipts(
    &self,
    block: BlockNumberOrTag,
) -> RpcResult<Option<Vec<RpcTransactionReceipt>>>;
```

### Add to `StateProvider` Trait

```rust
async fn block_receipts(
    &self,
    block: BlockNumberOrTag,
) -> Result<Option<Vec<RpcTransactionReceipt>>, RpcError>;
```

### Implement in `IndexedStateProvider`

The implementation uses the existing `BlockIndex` infrastructure:

```rust
async fn block_receipts(
    &self,
    block: BlockNumberOrTag,
) -> Result<Option<Vec<RpcTransactionReceipt>>, RpcError> {
    let block_num = self.resolve_block_number(&block)?;
    let indexed_block = match self.index.get_block_by_number(block_num) {
        Some(b) => b,
        None => return Ok(None),
    };

    let receipts: Vec<RpcTransactionReceipt> = indexed_block
        .transaction_hashes
        .iter()
        .filter_map(|tx_hash| self.index.get_receipt(tx_hash))
        .map(indexed_receipt_to_rpc)
        .collect();

    Ok(Some(receipts))
}
```

### Check: Does `BlockIndex` Store Receipts?

From `crates/storage/indexer/src/lib.rs`, `BlockIndex::insert_block()` takes
`Vec<IndexedReceipt>` and stores them. Receipts are looked up by transaction
hash via `get_receipt()`. To get all receipts for a block, we iterate the
block's `transaction_hashes` and look up each receipt.

If `BlockIndex` doesn't have a `get_receipts_for_block()` method, the
per-hash iteration approach above works and avoids changing the indexer. If
performance becomes a concern (blocks with many txs), a bulk method can be
added later.

### `NoopStateProvider` Implementation

The `NoopStateProvider` should return a sensible default:

```rust
async fn block_receipts(&self, _block: BlockNumberOrTag) -> Result<Option<Vec<RpcTransactionReceipt>>, RpcError> {
    Ok(None)
}
```

## Files to Change

| File | Change |
|------|--------|
| `crates/node/rpc/src/eth.rs` | Add `get_block_receipts` to `EthApi` trait and `EthApiImpl` |
| `crates/node/rpc/src/state_provider.rs` | Add `block_receipts()` to `StateProvider` trait + `NoopStateProvider` |
| `crates/node/rpc/src/indexed_provider.rs` | Implement `block_receipts()` using `BlockIndex` |

## Tests

1. **Unit test**: Insert a block with 2 transactions and their receipts into
   `BlockIndex`. Call `block_receipts(block_number)`. Verify both receipts
   are returned with correct fields.

2. **Unit test**: Call `block_receipts` for a non-existent block number.
   Verify `None` is returned (not an error).

3. **Unit test**: Call `block_receipts` for a block with zero transactions.
   Verify `Some(vec![])` is returned (empty array, not null).

## Verification

```bash
curl -s -X POST $RPC_URL \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockReceipts","params":["latest"],"id":1}' \
  | jq '.result'
# Expected: [] (empty array for blocks with no txs), not an error
```
