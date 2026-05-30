# No WebSocket Subscription Support in JSON-RPC Server

**Severity:** Medium
**Component:** `kora-rpc`
**Labels:** enhancement, rpc, ethereum-compatibility

## Summary

The Kora JSON-RPC server accepts WebSocket connections (jsonrpsee 0.24 enables both HTTP and WS transport by default), but the `eth_subscribe` / `eth_unsubscribe` methods are entirely absent. This means clients can open a WebSocket connection but have no way to receive real-time push notifications for new blocks, log events, or pending transactions. Any application that needs to react to on-chain state changes must resort to polling.

## Current Behavior

### WebSocket transport is enabled but unused

In `crates/node/rpc/src/server.rs`, both `RpcServer::start()` (line 238) and `JsonRpcServer::start()` (line 391) build the jsonrpsee server with `Server::builder()` and call `.build(addr)`. jsonrpsee 0.24 enables both HTTP and WebSocket transport by default, so the server already accepts WebSocket connections. However, since no subscription methods are registered, clients that connect over WebSocket can only issue standard request/response RPC calls -- they cannot subscribe to real-time events.

### No subscription methods registered

In `crates/node/rpc/src/eth.rs`, the `EthApi` trait (lines 24-137) defines 20 standard Ethereum methods (`eth_chainId`, `eth_blockNumber`, `eth_getBalance`, etc.) but does not include `eth_subscribe` or `eth_unsubscribe`. These methods are not listed, not implemented, and not stubbed. There is no `EthSubscriptionApi` or `EthPubSubApi` trait anywhere in the crate.

A search for "subscribe" across the entire `crates/node/rpc/` directory returns zero results.

### Configuration placeholder exists but is unused

The node configuration crate (`crates/node/config/src/rpc.rs`) already defines a `ws_addr` field with a default of `"0.0.0.0:8546"`:

```rust
// crates/node/config/src/rpc.rs, lines 18-20
#[serde(default = "default_ws_addr")]
pub ws_addr: String,
```

However, this address is never read by the RPC server. The `RpcServerConfig` struct in `crates/node/rpc/src/config.rs` (the runtime config used by the actual server) has no `ws_addr` field at all. The node-level config value is silently ignored during server construction.

### No event broadcast infrastructure

The `FinalizedReporter` in `crates/node/reporters/src/lib.rs` (lines 363-424) processes finalized blocks by persisting snapshots and indexing block/transaction/receipt data, but it does not publish events to any broadcast channel. There is no `tokio::sync::broadcast` channel, no event bus, and no notification mechanism that a subscription handler could tap into.

## Impact

### Applications cannot receive real-time notifications

Without `eth_subscribe`, applications have no way to be notified when:
- A new block is finalized (`newHeads`)
- A log matching specific topics is emitted (`logs`)
- A new transaction enters the mempool (`newPendingTransactions`)

### Polling is the only option

Clients must poll `eth_blockNumber` or `eth_getBlockByNumber` in a loop to detect new state. This introduces:
- **Higher latency**: Detection delay equals the polling interval. A 1-second poll interval means up to 1 second of unnecessary lag. Tighter intervals increase load without eliminating the gap entirely.
- **Increased request volume**: Every connected client generates continuous HTTP requests regardless of whether anything has changed, placing unnecessary load on the server and network.
- **Wasted bandwidth**: Every poll response carries full HTTP headers and connection overhead, even when the response is "nothing changed."

### Incompatibility with ecosystem tooling

Several widely-used tools and services require or strongly prefer `eth_subscribe`:
- **The Graph**: Protocol indexers use `eth_subscribe` with `newHeads` to track the chain tip and trigger subgraph indexing. Without it, The Graph cannot index Kora without custom polling adapters.
- **Blockchain indexers** (e.g., Ponder, Goldsky, Envio): Many indexing frameworks rely on WebSocket subscriptions for real-time event streaming.
- **MetaMask and browser wallets**: MetaMask prefers a WebSocket connection with `eth_subscribe` for real-time balance and transaction status updates. Without subscription support, MetaMask falls back to HTTP polling, resulting in delayed balance updates and a degraded user experience.
- **ethers.js / viem**: Both libraries have first-class support for `provider.on("block", ...)` and `contract.on("Transfer", ...)` which use `eth_subscribe` under the hood. Without it, developers must use manual polling wrappers.

### DApp developer friction

Developers building on Kora will find that standard patterns from other EVM chains do not work:
```javascript
// This fails silently or errors out on Kora today
const provider = new ethers.WebSocketProvider("ws://localhost:8546");
provider.on("block", (blockNumber) => {
    console.log("New block:", blockNumber);
});
```

Developers must instead write polling loops, adding boilerplate and potential bugs (missed blocks if the poll interval is too wide, duplicate processing if the interval is too narrow).

## Root Cause

The gap exists because the RPC server was built with request/response functionality in mind. The jsonrpsee dependency (version 0.24, declared in `crates/node/rpc/Cargo.toml`) already accepts WebSocket connections by default and fully supports subscription methods natively, but subscriptions have not been implemented:

1. **Server transport**: `Server::builder().build()` creates a listener that accepts both HTTP and WebSocket connections by default. WS transport is already enabled; what is missing is subscription method handlers.
2. **Subscription methods**: jsonrpsee provides the `#[subscription]` proc macro attribute for defining subscription RPC methods, but no subscription trait or handler has been written.
3. **Event source**: The `FinalizedReporter` is the natural place where new-block events originate, but it has no mechanism to fan out notifications to connected WebSocket clients.

## Proposed Fix

### 1. No transport changes needed -- WebSocket is already enabled

jsonrpsee 0.24's `Server::builder().build()` accepts both HTTP and WebSocket connections by default. The existing server in `crates/node/rpc/src/server.rs` already handles WS connections. No changes to the server builder are needed to enable WebSocket transport.

Optionally, a dedicated WS-only server could be bound to the `ws_addr` from the config to provide cleaner separation, allowing operators to expose HTTP and WS on different ports or interfaces.

### 2. Wire the `ws_addr` config value through to the RPC server

Bridge the gap between `kora-config`'s `RpcConfig.ws_addr` and `kora-rpc`'s `RpcServerConfig`. Add a `ws_addr` field to `RpcServerConfig` in `crates/node/rpc/src/config.rs` and plumb it through the runner so the configured address is actually used.

### 3. Add a broadcast channel for finalized block events

In `crates/node/reporters/src/lib.rs`, add a `tokio::sync::broadcast::Sender` to `FinalizedReporter`. After a block is successfully persisted and indexed, publish a notification containing the block header (and optionally receipts/logs) to this channel:

```rust
pub struct FinalizedReporter<E, P> {
    // ... existing fields ...
    block_tx: Option<tokio::sync::broadcast::Sender<FinalizedBlockEvent>>,
}

pub struct FinalizedBlockEvent {
    pub block: IndexedBlock,
    pub receipts: Vec<IndexedReceipt>,
}
```

This broadcast channel can serve multiple subscribers with minimal overhead since `tokio::sync::broadcast` is designed for exactly this fan-out pattern.

### 4. Implement `eth_subscribe` and `eth_unsubscribe`

Create a new file `crates/node/rpc/src/subscription.rs` with an `EthSubscriptionApi` trait using jsonrpsee's `#[subscription]` macro:

```rust
#[rpc(server, namespace = "eth")]
pub trait EthSubscriptionApi {
    #[subscription(name = "subscribe" => "subscription", unsubscribe = "unsubscribe", item = serde_json::Value)]
    async fn subscribe(&self, kind: SubscriptionKind, params: Option<LogFilter>) -> SubscriptionResult;
}
```

Subscription types to implement:
- **`newHeads`** (required): Push a block header object each time a new block is finalized. This is the most widely used subscription type and unblocks The Graph, indexers, and wallet integrations.
- **`logs`** (required): Push log objects matching a filter (address, topics) each time a relevant log appears in a finalized block. This is essential for DApps tracking contract events.
- **`newPendingTransactions`** (optional): Push transaction hashes when new transactions enter the mempool. Less critical but useful for MEV and monitoring tooling.

### 5. Wire subscription handlers to the broadcast channel

The subscription handler receives a `broadcast::Receiver` from the `FinalizedReporter`'s channel and filters/transforms events based on the subscription type:
- For `newHeads`: Forward every `FinalizedBlockEvent` as a JSON block header
- For `logs`: Filter each block's receipts against the subscriber's log filter (address + topics) and forward matching logs

### Files to modify

| File | Change |
|------|--------|
| `crates/node/rpc/src/server.rs` | Merge the subscription API module into the RPC module (WS transport is already enabled by default) |
| `crates/node/rpc/src/eth.rs` | Add `EthSubscriptionApi` trait with `subscribe`/`unsubscribe` methods (or place in a new file) |
| `crates/node/rpc/src/config.rs` | Add `ws_addr: SocketAddr` field to `RpcServerConfig` |
| `crates/node/rpc/src/lib.rs` | Export the new subscription module and types |
| `crates/node/rpc/Cargo.toml` | Potentially no changes needed; jsonrpsee 0.24 with `"server"` feature already includes WS support |
| `crates/node/reporters/src/lib.rs` | Add `broadcast::Sender<FinalizedBlockEvent>` to `FinalizedReporter`; publish after successful block persistence |
| `crates/node/runner/src/runner.rs` | Wire the broadcast channel from `FinalizedReporter` to the RPC subscription handler |
| `crates/node/config/src/rpc.rs` | No changes needed; `ws_addr` field already exists |
| **New:** `crates/node/rpc/src/subscription.rs` | Subscription management: handler implementation, per-client subscription tracking, log filtering |

## Testing

### Unit tests

- **`newHeads` subscription**: Create a subscription handler with a mock broadcast channel. Send a `FinalizedBlockEvent` through the channel. Verify the subscriber receives a JSON object matching the expected block header format.
- **`logs` subscription with filter**: Create a `logs` subscription with a specific address and topic filter. Send a block event containing multiple receipts with mixed logs. Verify that only logs matching the filter are delivered, and non-matching logs are suppressed.
- **Multiple subscription types**: Subscribe to both `newHeads` and `logs` on the same connection. Send a block event. Verify both subscriptions receive their respective notifications.
- **Unsubscribe**: Subscribe, verify the subscription ID is returned, call `eth_unsubscribe` with that ID, send another block event, verify no further notifications are received.

### Integration tests

- **WebSocket connection lifecycle**: Connect to `ws_addr`, send a JSON-RPC `eth_subscribe` request, receive the subscription ID, receive at least one notification, send `eth_unsubscribe`, verify clean disconnection.
- **Multiple concurrent subscriptions**: Open multiple WebSocket connections, each with different subscription filters. Finalize a block. Verify each connection receives only the notifications matching its filter.
- **HTTP still works alongside WS**: After enabling WS, verify that HTTP JSON-RPC requests (`eth_blockNumber`, `eth_chainId`, etc.) continue to function correctly on the HTTP endpoint.
- **Connection drop handling**: Subscribe, then abruptly close the WebSocket connection. Verify the server cleans up the subscription without leaking resources or panicking.
- **Backpressure / slow consumer**: Subscribe and intentionally delay reading from the WebSocket. Verify the server either buffers up to a configured limit and then drops the subscription, or handles the slow consumer gracefully without blocking the broadcast channel for other subscribers.

## References

- [Ethereum JSON-RPC Specification: eth_subscribe](https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_subscribe)
- [jsonrpsee subscription documentation](https://docs.rs/jsonrpsee/latest/jsonrpsee/server/index.html)
- [EIP-758: Subscriptions and Filters](https://eips.ethereum.org/EIPS/eip-758)
