# 078: 30+ Missing Standard Ethereum JSON-RPC Methods

**Category:** rpc
**Severity:** medium

## Summary

Kora implements 31 Ethereum JSON-RPC methods (22 fully, 6 partially for live state only, 3 as constant stubs). Over 30 standard methods across `eth_*`, `net_*`, `web3_*`, `debug_*`, and `trace_*` namespaces are entirely missing, returning `-32601: method not found`. Several of the missing methods are trivial to implement because the data already exists in the block index, while others (debug/trace) require significant infrastructure.

## Problem

Kora is a minimal Ethereum-compatible execution client. Its RPC layer provides enough methods for basic transaction submission and state queries, but many standard methods expected by developer tooling, block explorers, and indexers are absent.

### Currently implemented methods

**File:** `crates/node/rpc/src/eth.rs`, lines 51-188 (EthApi trait definition)

**Fully implemented (22):**
`eth_chainId`, `eth_blockNumber`, `eth_sendRawTransaction`, `eth_getBlockByNumber`, `eth_getBlockByHash`, `eth_getTransactionByHash`, `eth_getTransactionReceipt`, `eth_gasPrice`, `eth_maxPriorityFeePerGas`, `eth_feeHistory`, `eth_getLogs`, `eth_newFilter`, `eth_newBlockFilter`, `eth_newPendingTransactionFilter`, `eth_getFilterChanges`, `eth_getFilterLogs`, `eth_uninstallFilter`, `net_version`, `net_listening`, `net_peerCount`, `web3_clientVersion`, `web3_sha3`

**Partial -- live state only (6):**
`eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt`, `eth_call`, `eth_estimateGas`

These methods reject historical block parameters because QMDB has no historical state (see issue 075 for the race condition this causes with explicit block numbers).

**Constant stubs (3):**
`eth_accounts` (returns `[]`), `eth_protocolVersion` (returns `"0x44"`), `eth_syncing` (returns `false`)

### Missing methods by priority

**High priority (breaks common tooling):**
- `eth_getBlockTransactionCountByHash` -- trivial, data exists in `BlockIndex`
- `eth_getBlockTransactionCountByNumber` -- trivial, data exists in `BlockIndex`
- `eth_getTransactionByBlockHashAndIndex` -- trivial, data exists in `BlockIndex`
- `eth_getTransactionByBlockNumberAndIndex` -- trivial, data exists in `BlockIndex`
- `eth_createAccessList` -- medium effort, requires EVM dry-run with access list collection

**Medium priority (developer tooling):**
- `debug_traceTransaction` -- high effort, requires EVM tracing infrastructure
- `debug_traceCall` -- high effort, requires EVM tracing infrastructure
- `eth_blobBaseFee` -- low effort, stub returning `0` (see issue 077)

**Low priority (deprecated or trivial stubs):**
- `eth_coinbase` (return `Address::ZERO`), `eth_mining` (return `false`), `eth_hashrate` (return `0x0`)
- Uncle methods: `eth_getUncleCountByBlockHash`, `eth_getUncleCountByBlockNumber`, `eth_getUncleByBlockHashAndIndex`, `eth_getUncleByBlockNumberAndIndex` (all return `null` or `0x0` post-merge)
- `eth_sign`, `eth_signTransaction`, `eth_sendTransaction` (wallet-only methods, should return error)
- `eth_getProof` -- architecturally impossible without MPT (see issue 072)

**Missing namespaces:**
`debug_*` and `trace_*` are entirely absent and require EVM tracing infrastructure.

## Code Reference

**File:** `crates/node/rpc/src/eth.rs:51-188` (EthApi trait - only these methods are defined)
```rust
#[rpc(server, namespace = "eth")]
pub trait EthApi {
    #[method(name = "chainId")]
    async fn chain_id(&self) -> RpcResult<U64>;

    #[method(name = "blockNumber")]
    async fn block_number(&self) -> RpcResult<U64>;

    // ... 29 more methods defined ...

    #[method(name = "uninstallFilter")]
    async fn uninstall_filter(&self, filter_id: U256) -> RpcResult<bool>;
}
```

The `BlockIndex` store (`crates/storage/indexer/src/store.rs`) contains all the data needed for the high-priority lookup methods:

```rust
pub struct BlockIndex {
    blocks_by_hash: RwLock<HashMap<B256, IndexedBlock>>,
    blocks_by_number: RwLock<HashMap<u64, B256>>,
    transactions: RwLock<HashMap<B256, IndexedTransaction>>,
    receipts: RwLock<HashMap<B256, IndexedReceipt>>,
    logs_by_block: RwLock<HashMap<B256, Vec<IndexedLog>>>,
    head_block: AtomicU64,
}
```

## Impact

| Tool | Compatible? | Blocking gaps |
|------|-------------|---------------|
| MetaMask | Yes | Core methods present |
| ethers.js / viem | Basic use only | `eth_createAccessList` missing |
| Hardhat | Mostly | `debug_traceTransaction` missing |
| Foundry (cast/forge) | Mostly | `trace_*`, `debug_*` missing for fork testing |
| The Graph | Partial | `eth_getBlockTransactionCountByNumber` missing |
| Block explorers | Partial | `eth_getTransactionByBlockNumberAndIndex`, `trace_*` missing |

## Root Cause

Initial RPC implementation focused on core transaction submission and state query methods. Block/transaction lookup by index, deprecated stubs, and tracing namespaces were not prioritized.

## Suggested Fix

### Phase 1: High-priority block/transaction lookup methods

All data exists in `BlockIndex`. Add trait methods to `EthApi` and implement in `EthApiImpl` by delegating to `StateProvider` / `BlockIndex`:

```rust
// Add to EthApi trait:
#[method(name = "getBlockTransactionCountByHash")]
async fn get_block_transaction_count_by_hash(&self, hash: B256) -> RpcResult<Option<U64>>;

#[method(name = "getBlockTransactionCountByNumber")]
async fn get_block_transaction_count_by_number(&self, block: BlockNumberOrTag) -> RpcResult<Option<U64>>;

#[method(name = "getTransactionByBlockHashAndIndex")]
async fn get_transaction_by_block_hash_and_index(&self, hash: B256, index: U64) -> RpcResult<Option<RpcTransaction>>;

#[method(name = "getTransactionByBlockNumberAndIndex")]
async fn get_transaction_by_block_number_and_index(&self, block: BlockNumberOrTag, index: U64) -> RpcResult<Option<RpcTransaction>>;
```

### Phase 2: Deprecated stubs

Return spec-correct constants for `eth_coinbase`, `eth_mining`, `eth_hashrate`, and uncle methods.

### Phase 3: `debug_*` / `trace_*` namespaces

Defer to a future milestone -- requires EVM execution tracing infrastructure (step-by-step opcode logging, state diff tracking, call trace collection).

## Files to Modify

- `crates/node/rpc/src/eth.rs` -- `EthApi` trait definitions and `EthApiImpl` implementations
- `crates/node/rpc/src/state_provider.rs` -- `StateProvider` trait (add new method signatures)
- `crates/node/rpc/src/indexed_provider.rs` -- `IndexedStateProvider` implementation via `BlockIndex`
- `crates/storage/indexer/src/store.rs` -- `BlockIndex` may need new accessor methods

## Related Issues

- `072-storage-state-root-not-mpt.md` -- `eth_getProof` is architecturally impossible without MPT
- `073-rpc-missing-websocket-subscriptions.md` -- missing WebSocket subscription kinds
- `075-rpc-state-queries-race-with-head.md` -- explicit block number race condition affects existing partial methods
- `077-executor-eip4844-blob-handling-gaps.md` -- includes `eth_blobBaseFee`

## Labels

enhancement, rpc, good first issue
