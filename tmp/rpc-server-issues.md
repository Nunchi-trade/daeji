# Kora RPC Server: Issues and Missing Features

## What is Kora?

Kora is a high-performance blockchain built on the Commonware framework. It runs a BFT consensus protocol among a fixed validator set and exposes an Ethereum-compatible JSON-RPC interface so that standard Ethereum tooling (wallets, block explorers, SDKs like ethers.js and viem) can interact with the chain.

The RPC server lives in `crates/node/rpc/` and is built on:
- **jsonrpsee** for JSON-RPC method dispatch and transport
- **axum** for the HTTP health/status endpoints
- **Tower** for middleware (CORS, concurrency limiting)

---

## RPC Namespaces and Methods

### `eth_*` Namespace (Ethereum JSON-RPC)

| Method | Status | Notes |
|--------|--------|-------|
| `eth_chainId` | Implemented | Returns configured chain ID |
| `eth_blockNumber` | Implemented | From block index or atomic counter |
| `eth_getBalance` | Implemented | Latest state only (block param ignored) |
| `eth_getTransactionCount` | Implemented | Latest state only (block param ignored) |
| `eth_getCode` | Implemented | Latest state only; returns `0x` for missing accounts |
| `eth_getStorageAt` | Implemented | Latest state only (block param ignored) |
| `eth_sendRawTransaction` | Implemented | Decodes, validates, forwards to mempool |
| `eth_call` | Implemented | Uses revm executor against live state |
| `eth_estimateGas` | Implemented | Uses revm executor |
| `eth_getBlockByNumber` | Implemented | Full and hash-only modes |
| `eth_getBlockByHash` | Implemented | Full and hash-only modes |
| `eth_getTransactionByHash` | Implemented | Checks index, then pending cache |
| `eth_getTransactionReceipt` | Implemented | From block index |
| `eth_gasPrice` | Stub | Returns hardcoded 1 gwei |
| `eth_maxPriorityFeePerGas` | Stub | Returns hardcoded 1 gwei |
| `eth_feeHistory` | Stub | Returns uniform base fee, no real data |
| `eth_accounts` | Implemented | Always returns empty (non-custodial node) |
| `eth_protocolVersion` | Implemented | Returns `"0x44"` |
| `eth_syncing` | Stub | Always returns `false` |
| `eth_getLogs` | Implemented | With address and topic filtering |
| `eth_subscribe` | NOT IMPLEMENTED | No WebSocket support |
| `eth_unsubscribe` | NOT IMPLEMENTED | No WebSocket support |

### `net_*` Namespace

| Method | Status | Notes |
|--------|--------|-------|
| `net_version` | Implemented | Returns chain ID as string |
| `net_listening` | Stub | Always returns `true` |
| `net_peerCount` | Bug | See Issue 6 / separate document |

### `web3_*` Namespace

| Method | Status | Notes |
|--------|--------|-------|
| `web3_clientVersion` | Implemented | Returns `"kora/{version}"` |
| `web3_sha3` | Implemented | Returns keccak256 hash |

### `kora_*` Namespace (Custom)

| Method | Status | Notes |
|--------|--------|-------|
| `kora_nodeStatus` | Implemented | Returns validator state, view, counters |

### Not Implemented

| Method | Category |
|--------|----------|
| `txpool_status` | Mempool introspection |
| `txpool_content` | Mempool introspection |
| `txpool_inspect` | Mempool introspection |
| `eth_pendingTransactions` | Mempool introspection |
| `debug_traceTransaction` | Debugging |
| `debug_traceBlockByNumber` | Debugging |

---

## Issue 1: Pending Transaction Memory Leak

**Severity:** Medium
**File:** `crates/node/rpc/src/eth.rs`, line 197

### Problem

The `EthApiImpl` struct contains a `pending_txs` field:

```rust
pending_txs: Arc<RwLock<HashMap<B256, RpcTransaction>>>,
```

When `eth_sendRawTransaction` is called (line 307), the decoded transaction is unconditionally inserted into this map:

```rust
self.pending_txs.write().await.insert(tx_hash, pending_tx);
```

Entries are only removed when `eth_getTransactionByHash` is called AND the transaction has been indexed into a finalized block (lines 351-352):

```rust
if indexed.is_some() {
    self.pending_txs.write().await.remove(&hash);
    return Ok(indexed);
}
```

### Why This is a Problem

There is no TTL, no size cap, and no background eviction. Transactions are removed only via explicit lookup. If a transaction:
- Is submitted but never included in a block (e.g., nonce too high, gas too low)
- Is included but nobody ever queries it by hash

...then the entry remains in the HashMap forever.

### Impact

On a long-running node receiving many transactions (especially from automated systems that submit and forget), memory usage grows unbounded. Each `RpcTransaction` is approximately 400-500 bytes, so 1 million leaked entries would consume ~500MB.

### Proposed Fix

Add a bounded cache with time-based eviction:

```rust
// Option A: Periodic cleanup task
// Spawn a background task that sweeps entries older than N minutes.

// Option B: Bounded HashMap with LRU eviction
// Replace HashMap with an LRU cache capped at e.g. 10,000 entries.

// Option C: Remove on finalization
// When a block is finalized, remove all pending_txs that appear in it.
```

---

## Issue 2: Rate Limiting Configured but NOT Wired

**Severity:** Medium
**File:** `crates/node/rpc/src/config.rs`, lines 130-150

### Problem

`RateLimitConfig` is defined with sensible defaults:

```rust
pub struct RateLimitConfig {
    /// Maximum requests per second per client.
    pub requests_per_second: u64,
    /// Burst size for rate limiting.
    pub burst_size: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self { requests_per_second: 100, burst_size: 200 }
    }
}
```

The `RpcServerConfig` struct stores this config:

```rust
pub struct RpcServerConfig {
    // ...
    pub rate_limit: RateLimitConfig,
    // ...
}
```

And there is even a builder method:

```rust
pub const fn with_rate_limit(mut self, requests_per_second: u64) -> Self {
    self.rate_limit.requests_per_second = requests_per_second;
    self
}
```

However, in `crates/node/rpc/src/server.rs`, the `start()` method (line 202) **never reads `rate_limit` from the config**. The `from_config` constructor (line 185) copies `config.cors` and `config.max_connections` but ignores `config.rate_limit` entirely. No Tower `RateLimitLayer` is constructed anywhere in the server code.

### What IS Enforced

The `max_connections` limit IS wired correctly:
- For the JSON-RPC server: `Server::builder().max_connections(max_connections)` (line 239)
- For the HTTP server: `ConcurrencyLimitLayer::new(max_connections as usize)` (line 219)

This caps concurrent connections but does NOT limit request rate per connection.

### Impact

A single client holding one connection can issue unlimited JSON-RPC requests per second. Under adversarial conditions, this can:
- Saturate the RPC server's CPU
- Create lock contention on `state_provider` (held under `RwLock`)
- Degrade consensus performance on validators running public RPC

### Proposed Fix

Wire the existing config into a Tower rate-limiting layer:

```rust
use tower::limit::RateLimitLayer;
use std::time::Duration;

// In server.start():
let rate_limit_layer = RateLimitLayer::new(
    config.rate_limit.requests_per_second as u64,
    Duration::from_secs(1),
);
```

Or use jsonrpsee's built-in per-connection rate limiting if available in the version used.

---

## Issue 3: No WebSocket/Subscription Support

**Severity:** Medium
**File:** `crates/node/rpc/src/server.rs`

### Problem

The JSON-RPC server is HTTP-only. There is no WebSocket transport configured, and the `eth_subscribe` / `eth_unsubscribe` methods are not defined in the `EthApi` trait (`crates/node/rpc/src/eth.rs`).

jsonrpsee supports WebSocket subscriptions natively, but the server is built with `Server::builder()` without enabling the WS transport or registering any subscription methods.

### Impact

- Applications cannot receive real-time notifications for new blocks, pending transactions, or log events
- Must poll `eth_blockNumber` or `eth_getBlockByNumber` to detect new state
- Incompatible with tools that require `eth_subscribe` (some indexers, The Graph, etc.)
- Higher latency for event detection and increased request load from polling

### Proposed Fix

1. Enable WebSocket transport in the jsonrpsee server builder
2. Implement `eth_subscribe` with at least `newHeads` and `logs` subscription types
3. Add a broadcast channel that the block finalizer publishes to

---

## Issue 4: No Mempool Introspection

**Severity:** Low
**Files:** `crates/node/rpc/src/eth.rs`, `crates/node/rpc/src/kora.rs`

### Problem

There are no RPC methods to inspect the mempool/transaction pool state. The standard Geth `txpool_*` namespace is entirely absent:
- `txpool_status` - pending/queued transaction counts
- `txpool_content` - full pending transaction list grouped by sender
- `txpool_inspect` - summary of pending transactions

The internal `pending_txs` HashMap in `EthApiImpl` is a shallow cache for submitted transactions; it is not the actual mempool.

### Impact

When diagnosing chain stalls or transaction inclusion failures:
- Cannot determine if transactions are stuck in the mempool
- Cannot identify nonce gaps or poisoned accounts
- Must add ad-hoc logging or inspect internal state via debugger
- Operators have no visibility into pending transaction queue depth

### Proposed Fix

Expose a read handle to the mempool (the `LedgerService` or equivalent) and implement at minimum `txpool_status`:

```rust
#[rpc(server, namespace = "txpool")]
pub trait TxPoolApi {
    #[method(name = "status")]
    async fn status(&self) -> RpcResult<TxPoolStatus>;
}
```

---

## Issue 5: Historical State Queries Not Supported

**Severity:** Low
**File:** `crates/node/rpc/src/indexed_provider.rs`, lines 64-110

### Problem

The `IndexedStateProvider` implementation accepts an optional `BlockNumberOrTag` parameter on state-query methods but ignores it entirely:

```rust
async fn balance(
    &self,
    address: Address,
    _block: Option<BlockNumberOrTag>,  // <-- underscore-prefixed, unused
) -> Result<U256, RpcError> {
    self.state.balance(&address).await.map_err(state_error_to_rpc)
}
```

The same pattern applies to `nonce`, `code`, and `storage`. The underlying `StateDbRead` trait has no concept of versioned state -- it always returns the latest finalized value.

Block tags `Pending`, `Safe`, `Finalized`, `Latest` all resolve to the same thing (`head_block_number`), which is correct. But requesting state at a specific historical block number (e.g., `eth_getBalance(addr, "0x5")`) silently returns the latest state instead of an error or the historical value.

### Impact

- DApps relying on historical state get silently incorrect results
- Multicall contracts that snapshot state at a specific block will see current state
- Deviation from Ethereum JSON-RPC spec (should return state at that block or error)

### Proposed Fix

Either:
1. Return an explicit error when a specific historical block number is requested (honest approach)
2. Implement archive-node-style versioned state access (significant effort)

Minimum viable fix: detect non-latest block requests and return `RpcError::NotImplemented`.

---

## Issue 6: Hardcoded Gas Price and Fee History

**Severity:** Low
**File:** `crates/node/rpc/src/eth.rs`, lines 366-401

### Problem

Gas price and fee history return static values regardless of actual chain conditions:

```rust
async fn gas_price(&self) -> RpcResult<U256> {
    Ok(U256::from(1_000_000_000u64))  // Always 1 gwei
}

async fn max_priority_fee_per_gas(&self) -> RpcResult<U256> {
    Ok(U256::from(1_000_000_000u64))  // Always 1 gwei
}
```

`eth_feeHistory` constructs a response with uniform base fees and zero gas usage ratios:

```rust
Ok(FeeHistory {
    base_fee_per_gas: vec![base_fee; count + 1],  // Same value repeated
    gas_used_ratio: vec![0.0; count],              // Always 0%
    oldest_block: U64::from(oldest),
    reward: reward_percentiles.map(|percentiles| {
        vec![vec![U256::from(1_000_000_000u64); percentiles.len()]; count]
    }),
})
```

### Impact

- Fee estimation tools (MetaMask, ethers.js) get static values that do not reflect demand
- Currently a non-issue because Kora does not enforce minimum gas prices
- Will become a problem if/when dynamic fee mechanisms are introduced

### Proposed Fix

Track actual gas usage per block in the indexer and compute fee history from real data. For `eth_gasPrice`, return the moving average of recent blocks' effective gas prices.

---

## Error Handling

The RPC server has well-structured error handling defined in `crates/node/rpc/src/error.rs`:

| Error | JSON-RPC Code | Description |
|-------|---------------|-------------|
| `BlockNotFound` | -32001 | Block does not exist in index |
| `TransactionNotFound` | -32001 | Transaction not in index or pending cache |
| `AccountNotFound` | -32001 | Account has no state (may return zero instead) |
| `InvalidBlockNumber` | -32602 | Malformed block number parameter |
| `InvalidTransaction` | -32602 | Failed to decode or verify transaction |
| `ExecutionFailed` | -32015 | EVM revert or out-of-gas during call/estimate |
| `StateError` | -32603 | Internal state database failure |
| `Internal` | -32603 | Catch-all internal error |
| `NotImplemented` | -32004 | Method exists but is not supported |

These follow Ethereum JSON-RPC error code conventions correctly.

---

## Architecture Diagram

```
                 Client (wallet / dapp / CLI)
                         |
                         v
        ┌────────────────────────────────────┐
        │  jsonrpsee Server (port 8545)      │
        │  max_connections: 100              │
        │  (rate limiting: NOT active)       │
        └────────────────┬───────────────────┘
                         |
         ┌───────────────┼───────────────────┐
         |               |                   |
         v               v                   v
    ┌─────────┐    ┌──────────┐       ┌───────────┐
    │ EthApi  │    │ NetApi   │       │ Web3Api   │
    │ Impl    │    │ Impl     │       │ Impl      │
    └────┬────┘    └──────────┘       └───────────┘
         |
    ┌────┴───────────────────────────────────────┐
    │  IndexedStateProvider                       │
    │  ┌─────────────────┐  ┌─────────────────┐  │
    │  │ BlockIndex      │  │ StateDbRead     │  │
    │  │ (blocks, txs,   │  │ (QMDB - live   │  │
    │  │  receipts, logs)│  │  account state) │  │
    │  └─────────────────┘  └─────────────────┘  │
    └────────────────────────────────────────────┘
         |
         v (eth_sendRawTransaction only)
    ┌──────────────────────┐
    │  TxSubmitCallback    │
    │  → Mempool/Validator │
    └──────────────────────┘
```

Additionally, a separate **HTTP server** (port 8546 by default) exposes:
- `GET /status` — returns `NodeStatus` JSON (chain_id, view, counters, peer_count)
- `GET /health` — returns `"ok"` with 200 status

---

## Relevant Source Files

| File | Purpose |
|------|---------|
| `crates/node/rpc/src/lib.rs` | Module declarations and public exports |
| `crates/node/rpc/src/server.rs` | HTTP + JSON-RPC server initialization and lifecycle |
| `crates/node/rpc/src/eth.rs` | `EthApi`, `NetApi`, `Web3Api` trait definitions and implementations |
| `crates/node/rpc/src/kora.rs` | `KoraApi` trait and implementation (node status) |
| `crates/node/rpc/src/config.rs` | `RpcServerConfig`, `CorsConfig`, `RateLimitConfig` |
| `crates/node/rpc/src/error.rs` | `RpcError` enum and JSON-RPC error code mapping |
| `crates/node/rpc/src/state.rs` | `NodeState` / `NodeStatus` for consensus metadata |
| `crates/node/rpc/src/state_provider.rs` | `StateProvider` trait (abstract state access) |
| `crates/node/rpc/src/indexed_provider.rs` | Production `StateProvider` combining index + QMDB |
| `crates/node/runner/src/runner.rs` | Runner that wires RPC server with mempool and state |
| `crates/node/reporters/src/lib.rs` | `NodeStateReporter` that updates RPC state from consensus |
