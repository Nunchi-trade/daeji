# 02 — Account Default Behavior

## Problem

`eth_getBalance` and `eth_getTransactionCount` return error `-32001: account
not found` for addresses that don't exist in state. The Ethereum JSON-RPC spec
says these should return `0x0` (zero balance, zero nonce).

Meanwhile, `eth_getCode` already handles this correctly — it catches
`StateDbError::AccountNotFound` and returns `0x` (empty bytes). This
inconsistency means the fix for balance/nonce should follow the same pattern
already established by `eth_getCode`.

## Root Cause

`IndexedStateProvider::balance()` and `nonce()` delegate directly to
`self.state.balance()` / `self.state.nonce()` and map **all** errors through
`state_error_to_rpc`, which converts `AccountNotFound` into an RPC error.

```rust
// indexed_provider.rs:64-70 — current (broken)
async fn balance(&self, address: Address, _block: Option<BlockNumberOrTag>) -> Result<U256, RpcError> {
    self.state.balance(&address).await.map_err(state_error_to_rpc)
}
```

Compare with `eth_getCode` which already does the right thing:

```rust
// indexed_provider.rs:80-101 — current (correct)
async fn code(&self, address: Address, _block: Option<BlockNumberOrTag>) -> Result<Bytes, RpcError> {
    let code_hash = match self.state.code_hash(&address).await {
        Ok(hash) => hash,
        Err(StateDbError::AccountNotFound(_)) => return Ok(Bytes::new()), // ← handles it
        Err(e) => return Err(state_error_to_rpc(e)),
    };
    // ...
}
```

## Design

Apply the same `AccountNotFound → default value` pattern to `balance()`,
`nonce()`, and `storage()`. This is not a band-aid — it's the correct Ethereum
semantics: non-existent accounts are indistinguishable from accounts with
zero balance, zero nonce, empty code, and zero storage.

### Changes to `IndexedStateProvider`

```rust
async fn balance(&self, address: Address, _block: Option<BlockNumberOrTag>) -> Result<U256, RpcError> {
    match self.state.balance(&address).await {
        Ok(b) => Ok(b),
        Err(StateDbError::AccountNotFound(_)) => Ok(U256::ZERO),
        Err(e) => Err(state_error_to_rpc(e)),
    }
}

async fn nonce(&self, address: Address, _block: Option<BlockNumberOrTag>) -> Result<u64, RpcError> {
    match self.state.nonce(&address).await {
        Ok(n) => Ok(n),
        Err(StateDbError::AccountNotFound(_)) => Ok(0),
        Err(e) => Err(state_error_to_rpc(e)),
    }
}

async fn storage(&self, address: Address, slot: U256, _block: Option<BlockNumberOrTag>) -> Result<U256, RpcError> {
    match self.state.storage(&address, &slot).await {
        Ok(v) => Ok(v),
        Err(StateDbError::AccountNotFound(_)) => Ok(U256::ZERO),
        Err(e) => Err(state_error_to_rpc(e)),
    }
}
```

### Why Not Change the State Layer

One might consider making `StateDbRead::balance()` itself return `Ok(0)` for
missing accounts. This is the wrong layer — the state database correctly
reports that the account doesn't exist, which is useful information for other
callers (e.g., the executor's `AccountNotFound` check during EVM execution).
The RPC layer is where the Ethereum JSON-RPC spec's "return zero" semantics
should be applied.

## Files to Change

| File | Change |
|------|--------|
| `crates/node/rpc/src/indexed_provider.rs` | `balance()`, `nonce()`, `storage()` — catch `AccountNotFound`, return zero |

## Tests

The existing test infrastructure already has `MissingAccountState` (line 422 of
`indexed_provider.rs`). Add tests that exercise it:

```rust
#[tokio::test]
async fn test_balance_missing_account_returns_zero() {
    let index = Arc::new(BlockIndex::new());
    let provider = IndexedStateProvider::with_chain_id(index, MissingAccountState, 1337);
    let balance = provider.balance(Address::ZERO, None).await.unwrap();
    assert_eq!(balance, U256::ZERO);
}

#[tokio::test]
async fn test_nonce_missing_account_returns_zero() {
    let index = Arc::new(BlockIndex::new());
    let provider = IndexedStateProvider::with_chain_id(index, MissingAccountState, 1337);
    let nonce = provider.nonce(Address::ZERO, None).await.unwrap();
    assert_eq!(nonce, 0);
}

#[tokio::test]
async fn test_storage_missing_account_returns_zero() {
    let index = Arc::new(BlockIndex::new());
    let provider = IndexedStateProvider::with_chain_id(index, MissingAccountState, 1337);
    let value = provider.storage(Address::ZERO, U256::ZERO, None).await.unwrap();
    assert_eq!(value, U256::ZERO);
}
```

## Verification

```bash
curl -s -X POST $RPC_URL \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","latest"],"id":1}' \
  | jq '.result'
# Expected: "0x0"  (not an error object)
```
