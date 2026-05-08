# 06 — HDC Missing RPC Methods

## Problem

Two HDC operations are referenced in docs but not available via RPC:

1. **`hdc_permute`** — The `permute()` function exists in `kora-hdc` core
   (`vector.rs:105-127`) and is used internally by `TrigramEncoder`, but is not
   exposed through the RPC chain.
2. **`hdc_cosineSimilarity`** — Does not exist anywhere. The codebase has
   `similarity()` which is normalized Hamming distance (`1 - hamming/D`), not
   cosine similarity. For binary hypervectors, cosine similarity and normalized
   Hamming distance are equivalent, so the existing `hdc_similarity` method
   already serves this purpose.

## Design

### Add `hdc_permute` to the RPC Chain

**Layer 1: `kora-hdc-chain` (`crates/hdc/chain/src/rpc.rs`)**

Add `permute_rpc` to `HdcApi`:

```rust
/// `hdc_permute` -- cyclic left-rotation of a vector by `n` positions.
pub fn permute_rpc(&self, v: &[u8], n: usize) -> Result<Vec<u8>, HdcRpcError> {
    let vec = parse_vector(v)?;
    let result = kora_hdc::permute(&vec, n);
    Ok(serialize(&result).to_vec())
}
```

**Layer 2: `kora-rpc` (`crates/node/rpc/src/hdc.rs`)**

Add to the trait:

```rust
/// Cyclic left-rotation of an HDC vector by `n` bit positions.
#[method(name = "permute")]
async fn permute(&self, vector: Bytes, n: u32) -> RpcResult<Bytes>;
```

Add to the impl:

```rust
async fn permute(&self, vector: Bytes, n: u32) -> RpcResult<Bytes> {
    let v = vector.to_vec();
    let inner = self.inner.clone();
    let result = tokio::task::spawn_blocking(move || inner.permute_rpc(&v, n as usize))
        .await
        .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
        .map_err(map_hdc_err)?;
    Ok(Bytes::from(result))
}
```

### Do NOT Add `hdc_cosineSimilarity`

For binary hypervectors (elements in {0, 1}), cosine similarity reduces to:

```
cos(a, b) = (D - hamming(a,b)) / D = 1 - hamming(a,b)/D = similarity(a, b)
```

The existing `hdc_similarity` method already computes this value. Adding a
separate `hdc_cosineSimilarity` that returns the identical result would be
misleading — it suggests a different computation when there isn't one.

If callers expect the name `cosineSimilarity`, the right approach is
documentation, not a duplicate method. The explorer impl docs should note that
`hdc_similarity` IS the cosine similarity for binary vectors.

## Files to Change

| File | Change |
|------|--------|
| `crates/hdc/chain/src/rpc.rs` | Add `permute_rpc()` method to `HdcApi` |
| `crates/node/rpc/src/hdc.rs` | Add `permute` to `HdcRpcApi` trait and `HdcApiImpl` |

## Tests

1. **Unit test in `crates/hdc/chain/src/rpc.rs`**: Encode a vector, permute by
   0, verify result equals original. Permute by D, verify result equals
   original. Permute by 1, verify result differs from original.

2. **Unit test in `crates/node/rpc/src/hdc.rs`**:
   ```rust
   #[tokio::test]
   async fn permute_zero_is_identity() {
       let api = make_api();
       let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
       let result = api.permute(v.clone(), 0).await.unwrap();
       assert_eq!(result, v);
   }

   #[tokio::test]
   async fn permute_produces_correct_length() {
       let api = make_api();
       let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
       let result = api.permute(v, 7).await.unwrap();
       assert_eq!(result.len(), kora_hdc::BYTES);
   }

   #[tokio::test]
   async fn permute_rejects_wrong_length() {
       let api = make_api();
       let short = Bytes::from(vec![0u8; 100]);
       assert!(api.permute(short, 1).await.is_err());
   }
   ```

## Verification

```bash
# Encode, then permute
VEC=$(curl -s -X POST $RPC_URL \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"hdc_encode","params":["hello"],"id":1}' \
  | jq -r '.result')

# Permute by 0 should return same vector
curl -s -X POST $RPC_URL \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_permute\",\"params\":[\"$VEC\", 0],\"id\":2}" \
  | jq -r '.result'
# Should equal $VEC

# Permute by 1 should return different vector
curl -s -X POST $RPC_URL \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_permute\",\"params\":[\"$VEC\", 1],\"id\":3}" \
  | jq -r '.result'
# Should differ from $VEC
```
