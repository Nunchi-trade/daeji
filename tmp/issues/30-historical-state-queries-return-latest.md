# RPC: Historical state queries silently return latest state instead of erroring

**Severity:** Medium

## Summary

The `IndexedStateProvider` in Kora's RPC layer accepts an optional `BlockNumberOrTag` parameter on all account-state query methods (`eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt`) but completely ignores it. Every query returns the latest finalized state regardless of which block the caller specifies. A request like `eth_getBalance(addr, "0x5")` silently returns the balance at the head of the chain, not the balance at block 5, and no error is raised to inform the caller that the result is wrong.

This is a deviation from the Ethereum JSON-RPC specification. Any dApp, indexer, or toolchain that passes a historical block number and trusts the response will operate on incorrect data without knowing it.

## Background

### What is Kora?

Kora is an EVM-compatible blockchain built on a modular architecture with Commonware's Simplex consensus, REVM for EVM execution, and QMDB as the state database. It exposes an Ethereum-compatible JSON-RPC server so that standard Ethereum tooling (ethers.js, viem, Foundry, Hardhat, MetaMask) can interact with the chain.

### How RPC state queries work in Kora

The RPC server is structured in three layers:

1. **`EthApi` trait** (`crates/node/rpc/src/eth.rs`, lines 24-137) -- Defines the JSON-RPC method signatures using `jsonrpsee`'s `#[rpc]` proc macro. Methods like `get_balance`, `get_transaction_count`, `get_code`, and `get_storage_at` all accept an `Option<BlockNumberOrTag>` parameter, matching the Ethereum JSON-RPC spec.

2. **`EthApiImpl`** (`crates/node/rpc/src/eth.rs`, lines 192-420) -- Implements the `EthApi` trait. For state queries, it acquires a read lock on the state provider and delegates directly, passing the block parameter through untouched:

    ```rust
    // eth.rs, lines 261-268
    async fn get_balance(
        &self,
        address: Address,
        block: Option<BlockNumberOrTag>,
    ) -> RpcResult<U256> {
        let provider = self.state_provider.read().await;
        provider.balance(address, block).await.map_err(Into::into)
    }
    ```

3. **`IndexedStateProvider`** (`crates/node/rpc/src/indexed_provider.rs`, lines 62-207) -- The production implementation of the `StateProvider` trait. This is where the block parameter is dropped on the floor.

### The `StateProvider` trait contract

The `StateProvider` trait (`crates/node/rpc/src/state_provider.rs`, lines 19-94) defines the state-query interface. Its doc comments explicitly describe these as queries "at a given block":

```rust
/// Get the balance of an account at a given block.
async fn balance(
    &self,
    address: Address,
    block: Option<BlockNumberOrTag>,
) -> Result<U256, RpcError>;
```

The interface promises block-scoped state access. The implementation does not deliver it.

## The Code

**File:** `crates/node/rpc/src/indexed_provider.rs`, lines 64-110

All four account-state methods follow the same pattern -- the `block` parameter is underscore-prefixed to suppress the "unused variable" compiler warning:

```rust
async fn balance(
    &self,
    address: Address,
    _block: Option<BlockNumberOrTag>,  // <-- silently ignored
) -> Result<U256, RpcError> {
    self.state.balance(&address).await.map_err(state_error_to_rpc)
}

async fn nonce(
    &self,
    address: Address,
    _block: Option<BlockNumberOrTag>,  // <-- silently ignored
) -> Result<u64, RpcError> {
    self.state.nonce(&address).await.map_err(state_error_to_rpc)
}

async fn code(
    &self,
    address: Address,
    _block: Option<BlockNumberOrTag>,  // <-- silently ignored
) -> Result<Bytes, RpcError> {
    // ... delegates to self.state.code_hash / self.state.code
}

async fn storage(
    &self,
    address: Address,
    slot: U256,
    _block: Option<BlockNumberOrTag>,  // <-- silently ignored
) -> Result<U256, RpcError> {
    self.state.storage(&address, &slot).await.map_err(state_error_to_rpc)
}
```

Each method delegates directly to the `StateDbRead` trait on `self.state`. The `StateDbRead` trait (`crates/storage/traits/src/state.rs`, lines 13-48) provides only point-in-time access to the current state -- none of its methods accept a block number:

```rust
pub trait StateDbRead: Clone + Send + Sync + 'static {
    fn nonce(&self, address: &Address) -> impl Future<Output = Result<u64, StateDbError>> + Send;
    fn balance(&self, address: &Address) -> impl Future<Output = Result<U256, StateDbError>> + Send;
    fn code_hash(&self, address: &Address) -> impl Future<Output = Result<B256, StateDbError>> + Send;
    fn code(&self, code_hash: &B256) -> impl Future<Output = Result<Bytes, StateDbError>> + Send;
    fn storage(&self, address: &Address, slot: &U256) -> impl Future<Output = Result<U256, StateDbError>> + Send;
}
```

There is no concept of versioned state access anywhere in the storage layer. The underlying QMDB state database holds a single version of the world -- the latest finalized state.

### Block tag resolution is also misleading

The `IndexedStateProvider` does have a `resolve_block_number` method (lines 247-253) and a `resolve_tag` method (lines 255-262), which are used by `block_by_number`, `call`, `estimate_gas`, and `get_logs`. For block tags, the resolution is:

```rust
fn resolve_tag(&self, tag: BlockTag) -> Result<u64, RpcError> {
    match tag {
        BlockTag::Latest | BlockTag::Safe | BlockTag::Finalized | BlockTag::Pending => {
            Ok(self.index.head_block_number())
        }
        BlockTag::Earliest => Ok(0),
    }
}
```

`Pending`, `Safe`, `Finalized`, and `Latest` all resolve to the same value (`head_block_number`). This is defensible for a chain where finality is instant (Simplex consensus finalizes every block), but it means the caller cannot distinguish between these block stages. More critically, none of this resolution logic is even invoked for `balance`, `nonce`, `code`, or `storage` -- those methods ignore the block parameter entirely.

### Contrast with `eth_call` and `eth_estimateGas`

The `call` and `estimate_gas` methods (lines 145-163) do use the block parameter, but only to construct a `BlockContext` (gas limit, timestamp, base fee) for the REVM simulation:

```rust
async fn call(
    &self,
    request: CallRequest,
    block: Option<BlockNumberOrTag>,
) -> Result<Bytes, RpcError> {
    let block_ctx = self.block_context_for(block)?;
    let params = call_request_to_params(request);
    self.executor.simulate_call(&self.state, params, &block_ctx).map_err(execution_error_to_rpc)
}
```

The `block_context_for` method resolves the block tag to a block number, looks up the indexed block's header metadata, and constructs a `BlockContext`. But the actual state (`&self.state`) is still the latest finalized state -- the execution environment's block context says "pretend you are at block N" while the state it reads from is at the head. This is a subtler version of the same bug: `eth_call` at a historical block executes against current state with historical block metadata.

## What Breaks

### 1. DApps relying on historical balance snapshots

A governance dApp that calls `eth_getBalance(voter, snapshotBlock)` to determine voting power at a past snapshot block will get the voter's **current** balance instead. If the voter has since moved tokens, the governance vote tally will be wrong, and there is no indication that the result was fabricated.

### 2. Multicall and batched state snapshots

Contracts and front-ends that use Multicall patterns to snapshot state at a specific block (e.g., "give me all token balances as of block 1000") will receive current balances. The entire snapshot concept breaks down.

### 3. Block explorers and indexers

Block explorers like Etherscan display historical account state. When a user clicks on block 500 and views an address's balance at that block, the explorer queries `eth_getBalance(addr, "0x1F4")`. Kora returns the current balance, making the explorer display incorrect historical data.

### 4. Time-travel debugging

Developers who use `eth_call` with a historical block number to reproduce bugs or simulate transactions at a past state will get unpredictable results. The block context (gas limit, timestamp) will be historical, but the state (balances, storage, code) will be current. If a contract was upgraded or state has changed, the simulation will silently execute against the wrong state.

### 5. Ethereum toolchain compatibility

Standard Ethereum libraries (ethers.js, viem, web3.py) routinely pass block parameters in state queries. Many test suites and integration scripts hardcode block numbers. These will produce silently wrong results on Kora, making it appear to be compatible while subtly corrupting downstream logic.

## What Still Works

The following are unaffected:

- Queries with `block` set to `None`, `"latest"`, `"finalized"`, `"safe"`, or `"pending"` -- these all correctly map to the head state (which is also the only state available).
- `eth_getBlockByNumber`, `eth_getBlockByHash` -- block lookups use the indexed block data, not state.
- `eth_getTransactionByHash`, `eth_getTransactionReceipt` -- transaction and receipt lookups are correct.
- `eth_getLogs` -- log queries use the block index, which stores historical log data.
- `eth_sendRawTransaction` -- transaction submission is unrelated to state queries.
- Any dApp that only ever queries "latest" state (which is the majority of DeFi front-ends).

## Ethereum JSON-RPC Specification Reference

The [Ethereum JSON-RPC specification](https://ethereum.github.io/execution-apis/api-documentation/) defines the second parameter of `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, and `eth_getStorageAt` as a block identifier. When a specific block number is provided, the node must return the state at that block, or return an error if the state is unavailable (e.g., the node is not an archive node and has pruned that state).

Standard Ethereum nodes (Geth, Erigon, Nethermind, Besu) behave in one of two ways when a historical block number is requested:

- **Archive nodes:** Return the actual historical state.
- **Full nodes (pruned):** Return an error like `"missing trie node"` or `"state not available at block N"`.

Neither type silently returns current state. The silent fallback to latest state is unique to Kora and is the most dangerous behavior because it is undetectable by the caller.

## Proposed Fix (Minimum Viable)

The minimum viable fix is to detect when a specific historical block number is requested and return an explicit error instead of silently returning wrong data. This is honest behavior -- the caller knows their query cannot be satisfied and can handle the error appropriately.

### Files to modify

| File | Change |
|------|--------|
| `crates/node/rpc/src/indexed_provider.rs` | Add block parameter validation to `balance`, `nonce`, `code`, and `storage` |
| `crates/node/rpc/src/error.rs` | No changes needed -- `RpcError::NotImplemented` already exists and maps to `-32004 METHOD_NOT_SUPPORTED` |

### 1. Add a helper method to `IndexedStateProvider`

Add a method that checks if the requested block refers to current state or a historical block:

```rust
impl<S> IndexedStateProvider<S> {
    /// Reject requests for historical state that we cannot serve.
    ///
    /// Returns `Ok(())` if the block parameter is `None`, `Latest`, `Safe`,
    /// `Finalized`, or `Pending` (all of which map to head state).
    /// Returns `Err` if a specific block number is requested that differs
    /// from the current head.
    fn reject_historical_block(&self, block: &Option<BlockNumberOrTag>) -> Result<(), RpcError> {
        let head = self.index.head_block_number();
        match block {
            None => Ok(()),
            Some(BlockNumberOrTag::Latest) => Ok(()),
            Some(BlockNumberOrTag::Tag(
                BlockTag::Latest | BlockTag::Safe | BlockTag::Finalized | BlockTag::Pending,
            )) => Ok(()),
            Some(BlockNumberOrTag::Tag(BlockTag::Earliest)) => {
                if head == 0 {
                    Ok(())
                } else {
                    Err(RpcError::Internal(
                        "historical state queries are not supported; \
                         only latest state is available"
                            .to_string(),
                    ))
                }
            }
            Some(BlockNumberOrTag::Number(n)) => {
                if n.to::<u64>() == head {
                    Ok(())
                } else {
                    Err(RpcError::Internal(
                        "historical state queries are not supported; \
                         only latest state is available"
                            .to_string(),
                    ))
                }
            }
        }
    }
}
```

### 2. Add validation calls to each state-query method

Update each method to call `reject_historical_block` before delegating to the state database:

```rust
async fn balance(
    &self,
    address: Address,
    block: Option<BlockNumberOrTag>,
) -> Result<U256, RpcError> {
    self.reject_historical_block(&block)?;
    self.state.balance(&address).await.map_err(state_error_to_rpc)
}

async fn nonce(
    &self,
    address: Address,
    block: Option<BlockNumberOrTag>,
) -> Result<u64, RpcError> {
    self.reject_historical_block(&block)?;
    self.state.nonce(&address).await.map_err(state_error_to_rpc)
}

async fn code(
    &self,
    address: Address,
    block: Option<BlockNumberOrTag>,
) -> Result<Bytes, RpcError> {
    self.reject_historical_block(&block)?;
    let code_hash = match self.state.code_hash(&address).await {
        Ok(hash) => hash,
        Err(StateDbError::AccountNotFound(_)) => return Ok(Bytes::new()),
        Err(e) => return Err(state_error_to_rpc(e)),
    };
    if code_hash == B256::ZERO || code_hash == alloy_primitives::KECCAK256_EMPTY {
        return Ok(Bytes::new());
    }
    match self.state.code(&code_hash).await {
        Ok(bytes) => Ok(bytes),
        Err(StateDbError::CodeNotFound(_)) => Ok(Bytes::new()),
        Err(e) => Err(state_error_to_rpc(e)),
    }
}

async fn storage(
    &self,
    address: Address,
    slot: U256,
    block: Option<BlockNumberOrTag>,
) -> Result<U256, RpcError> {
    self.reject_historical_block(&block)?;
    self.state.storage(&address, &slot).await.map_err(state_error_to_rpc)
}
```

### 3. Consider the same treatment for `eth_call` and `eth_estimateGas`

The `call` and `estimate_gas` methods currently use the block parameter to construct a `BlockContext` but still execute against the latest state. This is arguably a separate but related bug. At minimum, these methods should also reject historical block numbers where the state would differ from head. However, since these methods do use the block context for gas limit and timestamp, rejecting them is a larger behavioral change. A warning log may be more appropriate here as an interim measure.

## Proposed Fix (Full -- Significant Effort)

For full Ethereum compatibility, Kora would need to support versioned state access, allowing queries at any historical (or at least recent) block number.

### Approach

1. **QMDB snapshots:** QMDB supports snapshotting. After each block is finalized, retain a state snapshot associated with that block number. This allows rewinding the state view to any retained block.

2. **Snapshot retention policy:** Maintain snapshots for the last N blocks (e.g., 256 or 1024). Older blocks would return the "historical state not available" error, similar to non-archive Ethereum nodes.

3. **Extend `StateDbRead`:** Add a `state_at(block_number: u64) -> Option<Self>` method that returns a state reader pinned to a specific block's snapshot, or `None` if the snapshot has been pruned.

4. **Wire through `IndexedStateProvider`:** When a historical block is requested, obtain a snapshot-pinned reader and delegate to it instead of the live state.

This is a significant architectural change that touches the storage layer, the state database traits, the provider implementation, and the block finalization pipeline. It should be tracked as a separate feature.

## Testing

### Unit tests for the minimum viable fix

```rust
#[tokio::test]
async fn test_balance_rejects_historical_block() {
    let index = Arc::new(BlockIndex::new());
    index.insert_block(create_test_block(10, B256::repeat_byte(10)), vec![], vec![]);
    let provider = IndexedStateProvider::with_chain_id(index, MockState, 1337);

    // Historical block number should error
    let result = provider.balance(Address::ZERO, Some(BlockNumberOrTag::Number(U64::from(5)))).await;
    assert!(result.is_err());

    // Current head block number should succeed
    let result = provider.balance(Address::ZERO, Some(BlockNumberOrTag::Number(U64::from(10)))).await;
    assert!(result.is_ok());

    // None (latest) should succeed
    let result = provider.balance(Address::ZERO, None).await;
    assert!(result.is_ok());

    // "latest" tag should succeed
    let result = provider
        .balance(Address::ZERO, Some(BlockNumberOrTag::Tag(BlockTag::Latest)))
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_nonce_rejects_historical_block() {
    // Same pattern as balance test
}

#[tokio::test]
async fn test_code_rejects_historical_block() {
    // Same pattern as balance test
}

#[tokio::test]
async fn test_storage_rejects_historical_block() {
    // Same pattern as balance test
}
```

### Integration tests

1. **ethers.js/viem with block parameter:** Submit a query like `provider.getBalance(addr, 5)` against a Kora node at block 10. Verify that the RPC response is an error with code `-32000` (or `-32004`) and a message indicating historical state is not supported.
2. **ethers.js/viem with "latest" tag:** Submit `provider.getBalance(addr, "latest")`. Verify that the response is a successful balance value.
3. **Hardhat/Foundry fork mode:** Attempt to use `--fork-block-number` against Kora. Verify that the tooling receives an explicit error rather than silently incorrect state.

## Verification Checklist

1. Call `eth_getBalance(addr, "0x5")` on a node at block 10 -- should return an RPC error, not a balance
2. Call `eth_getBalance(addr, "latest")` -- should return the correct current balance
3. Call `eth_getBalance(addr)` with no block parameter -- should return the correct current balance
4. Call `eth_getBalance(addr, "0xA")` where head is block 10 -- should return the correct current balance (matches head)
5. Call `eth_getTransactionCount(addr, "0x1")` -- should return an RPC error
6. Call `eth_getCode(addr, "0x1")` -- should return an RPC error
7. Call `eth_getStorageAt(addr, slot, "0x1")` -- should return an RPC error
8. Verify that `eth_call` and `eth_estimateGas` with `"latest"` still work correctly
9. Run `cargo test` across the workspace to confirm no regressions
10. Verify that standard dApp flows (token transfers, contract interactions using "latest") are unaffected
