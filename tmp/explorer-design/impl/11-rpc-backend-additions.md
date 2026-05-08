# 11 -- RPC Backend Additions (Rust)

This is the sole Rust-implementation document. Every other explorer implementation document targets TypeScript/frontend code. This document covers all changes to the `kora-rpc`, `kora-reporters`, `kora-indexer`, and `kora-runner` crates needed to support the explorer's real-time features.

---

## Current State (verified from codebase)

| Fact | Detail |
|------|--------|
| jsonrpsee version | `0.24`, features `["server", "macros"]` -- HTTP only |
| Server location | `crates/node/rpc/src/server.rs` -- two types: `RpcServer` (HTTP + JSON-RPC) and `JsonRpcServer` (JSON-RPC only) |
| WebSocket | Config mentions `ws_addr` in the design but it is **not wired**. `Server::builder()` does not set WS-specific options. |
| Subscriptions | None. No `#[subscription]` macros anywhere in the codebase. |
| NodeState fields | `current_view`, `finalized_count`, `proposed_count`, `nullified_count`, `peer_count`, `is_leader` |
| Reporter chain | `SeedReporter` > `FinalizedReporter` > `NodeStateReporter` -- composed via `Reporters::from()` tuple nesting |
| Activity handling | `NodeStateReporter` handles 3 of 10: `Notarization` (set_view), `Finalization` (set_view + inc_finalized), `Nullification` (inc_nullified). The remaining 7 are discarded. |
| LedgerEvents | `TransactionSubmitted`, `SnapshotPersisted`, `SeedUpdated` via `futures::channel::mpsc::unbounded` |
| BlockIndex | In-memory: `blocks_by_hash`, `blocks_by_number`, `transactions`, `receipts`, `logs_by_block` |
| Receipt access | Per-tx only: `get_receipt(tx_hash)`. No batch method for all receipts in a block. |

---

## 11.1 Enable WebSocket Transport

### Objective

jsonrpsee 0.24's `Server` already handles WebSocket upgrade on the same port as HTTP when subscription methods are registered. The main requirement is ensuring the `#[subscription]` macro has its server-side support compiled in and that the server builder is configured for both transports.

### 11.1.1 Cargo.toml Change

**File:** `crates/node/rpc/Cargo.toml`

```diff
 # JSON-RPC
-jsonrpsee = { version = "0.24", features = ["server", "macros"] }
+jsonrpsee = { version = "0.24", features = ["server", "macros", "ws"] }
```

The `"ws"` feature enables WebSocket transport in jsonrpsee's server stack. Without it, `#[subscription]` methods compile but the runtime rejects WS upgrade requests.

### 11.1.2 Server Configuration

**File:** `crates/node/rpc/src/config.rs`

Add `ws_addr` to `RpcServerConfig`:

```rust
// --- crates/node/rpc/src/config.rs ---

/// Configuration for the RPC server.
#[derive(Clone, Debug)]
pub struct RpcServerConfig {
    /// Address for the HTTP status endpoints.
    pub http_addr: SocketAddr,
    /// Address for the JSON-RPC server (HTTP + WS on same port).
    pub jsonrpc_addr: SocketAddr,
    /// Optional dedicated WebSocket address. If None, WS shares jsonrpc_addr.
    pub ws_addr: Option<SocketAddr>,
    /// Chain ID for the Ethereum API.
    pub chain_id: u64,
    /// CORS configuration.
    pub cors: CorsConfig,
    /// Rate limiting configuration.
    pub rate_limit: RateLimitConfig,
    /// Maximum number of concurrent connections.
    pub max_connections: u32,
    /// Maximum number of concurrent WebSocket subscriptions per connection.
    pub max_subscriptions_per_connection: u32,
}

impl Default for RpcServerConfig {
    fn default() -> Self {
        Self {
            http_addr: "127.0.0.1:8546".parse().unwrap(),
            jsonrpc_addr: "127.0.0.1:8545".parse().unwrap(),
            ws_addr: None, // shares jsonrpc_addr by default
            chain_id: 1,
            cors: CorsConfig::default(),
            rate_limit: RateLimitConfig::default(),
            max_connections: 100,
            max_subscriptions_per_connection: 64,
        }
    }
}
```

### 11.1.3 Server Builder Changes

**File:** `crates/node/rpc/src/server.rs`

The key change is setting `.enable_ws()` and configuring subscription limits on the server builder. In jsonrpsee 0.24, `Server::builder()` returns a `ServerBuilder` that supports both HTTP and WS:

```rust
// --- crates/node/rpc/src/server.rs ---
// Inside the jsonrpc_handle spawn block, replace the server builder:

let jsonrpc_handle = tokio::spawn(async move {
    let server = match Server::builder()
        .max_connections(max_connections)
        .max_subscriptions_per_connection(max_subscriptions_per_connection)
        // jsonrpsee 0.24 enables WS upgrade automatically when subscription
        // methods are registered. The `ws` feature flag ensures the WS codec
        // is compiled in.
        .build(jsonrpc_addr)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "Failed to build JSON-RPC server");
            return None;
        }
    };

    // ... module registration unchanged ...

    info!(addr = %jsonrpc_addr, "Starting JSON-RPC server (HTTP + WS)");

    let handle = server.start(module);
    handle.stopped().await;
    Some(())
});
```

### 11.1.4 RpcServer Struct Changes

Add `max_subscriptions_per_connection` to the `RpcServer` struct and wire it through constructors:

```rust
// --- crates/node/rpc/src/server.rs ---

pub struct RpcServer<S: StateProvider = NoopStateProvider> {
    state: NodeState,
    http_addr: SocketAddr,
    jsonrpc_addr: SocketAddr,
    chain_id: u64,
    tx_submit: Option<TxSubmitCallback>,
    state_provider: S,
    cors_config: CorsConfig,
    max_connections: u32,
    max_subscriptions_per_connection: u32,  // NEW
    peer_count: u64,
    hdc_api: Option<HdcApiImpl>,
    subscription_manager: Option<Arc<SubscriptionManager>>,  // NEW
}
```

Add a builder method:

```rust
/// Set maximum subscriptions per WebSocket connection.
#[must_use]
pub const fn with_max_subscriptions(mut self, max: u32) -> Self {
    self.max_subscriptions_per_connection = max;
    self
}

/// Attach a subscription manager for WebSocket push.
#[must_use]
pub fn with_subscription_manager(mut self, mgr: Arc<SubscriptionManager>) -> Self {
    self.subscription_manager = Some(mgr);
    self
}
```

### 11.1.5 CORS for WebSocket

WebSocket connections start as HTTP upgrade requests. The existing `build_cors_layer` in `server.rs` handles this because the CORS check happens during the initial HTTP handshake. No additional code is needed. The `allowed_origins` list in `CorsConfig` already governs which origins can initiate WebSocket connections.

For production, ensure the config includes the explorer's origin:

```rust
CorsConfig {
    allowed_origins: vec![
        "http://localhost:3000".to_string(),  // dev
        "https://explorer.kora.network".to_string(),  // production
    ],
    ..CorsConfig::default()
}
```

### 11.1.6 Verification

- [ ] WebSocket client connects to `ws://localhost:8545` and completes handshake
- [ ] HTTP POST to `http://localhost:8545` continues to work (no regression)
- [ ] Existing RPC methods (`eth_blockNumber`, `eth_chainId`, etc.) work over both HTTP and WS
- [ ] CORS preflight for WebSocket upgrade respects `allowed_origins`

---

## 11.2 eth_subscribe -- newHeads

### Objective

Push a block header to all subscribers the instant a block is finalized. This eliminates the 0-1000ms polling latency for the explorer's block arrival animation.

### 11.2.1 SubscriptionManager

**New file:** `crates/node/rpc/src/subscriptions.rs`

```rust
//! WebSocket subscription management.
//!
//! Central hub that receives chain events and fans out to WebSocket subscribers.
//! Each subscription type has its own `tokio::sync::broadcast` channel.

use std::sync::Arc;

use alloy_primitives::{Address, B256};
use jsonrpsee::{PendingSubscriptionSink, SubscriptionMessage};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, trace, warn};

use crate::types::{RpcBlock, RpcLog, RpcLogFilter, RpcTransactionReceipt};

/// Capacity for broadcast channels. 256 is sufficient for ~4 minutes of
/// backlog at 1 block/second. Slow subscribers that fall behind will receive
/// a `Lagged` error and should reconnect.
const DEFAULT_BROADCAST_CAPACITY: usize = 256;

/// Central subscription router. Holds broadcast senders for each subscription type.
/// Cloning is cheap (Arc-wrapped senders).
#[derive(Clone, Debug)]
pub struct SubscriptionManager {
    /// New block headers.
    new_heads_tx: broadcast::Sender<serde_json::Value>,
    /// New logs (per-log granularity, emitted on block finalization).
    logs_tx: broadcast::Sender<serde_json::Value>,
    /// New pending transaction hashes (or full txs).
    pending_txs_tx: broadcast::Sender<serde_json::Value>,
}

impl SubscriptionManager {
    /// Create a new subscription manager with default channel capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BROADCAST_CAPACITY)
    }

    /// Create a new subscription manager with explicit channel capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (new_heads_tx, _) = broadcast::channel(capacity);
        let (logs_tx, _) = broadcast::channel(capacity);
        let (pending_txs_tx, _) = broadcast::channel(capacity);
        Self { new_heads_tx, logs_tx, pending_txs_tx }
    }

    // ---- Producer methods (called by bridge tasks) ----

    /// Broadcast a new block header to all `newHeads` subscribers.
    pub fn notify_new_head(&self, header: serde_json::Value) {
        // send() returns Err only when there are zero receivers, which is fine.
        let _ = self.new_heads_tx.send(header);
    }

    /// Broadcast a log entry to all `logs` subscribers.
    /// Filtering happens on the subscriber side.
    pub fn notify_log(&self, log: serde_json::Value) {
        let _ = self.logs_tx.send(log);
    }

    /// Broadcast a pending transaction hash to all subscribers.
    pub fn notify_pending_tx(&self, tx: serde_json::Value) {
        let _ = self.pending_txs_tx.send(tx);
    }

    // ---- Consumer methods (called by subscription handlers) ----

    /// Subscribe to new block headers.
    pub fn subscribe_new_heads(&self) -> broadcast::Receiver<serde_json::Value> {
        self.new_heads_tx.subscribe()
    }

    /// Subscribe to new logs.
    pub fn subscribe_logs(&self) -> broadcast::Receiver<serde_json::Value> {
        self.logs_tx.subscribe()
    }

    /// Subscribe to pending transactions.
    pub fn subscribe_pending_txs(&self) -> broadcast::Receiver<serde_json::Value> {
        self.pending_txs_tx.subscribe()
    }

    /// Returns true if there are active log subscribers.
    /// Used to skip log scanning when nobody is listening.
    pub fn has_log_subscribers(&self) -> bool {
        self.logs_tx.receiver_count() > 0
    }
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new()
    }
}
```

### 11.2.2 EthSubscription Trait

**File:** `crates/node/rpc/src/eth.rs`

Add a new trait for the subscription namespace. In jsonrpsee, subscription methods must be in a separate trait from regular methods because they use a different macro:

```rust
// --- crates/node/rpc/src/eth.rs --- (add below existing EthApi trait)

/// Ethereum subscription API (WebSocket only).
///
/// Implements `eth_subscribe` / `eth_unsubscribe` per the Ethereum JSON-RPC spec.
/// Subscriptions are server-push: the client subscribes once, the server pushes
/// events as they occur. Requires WebSocket transport.
#[rpc(server, namespace = "eth")]
pub trait EthSubscriptionApi {
    /// Subscribe to Ethereum events.
    ///
    /// Supported subscription types:
    /// - `"newHeads"`: pushed on each new finalized block
    /// - `"logs"`: pushed for matching log entries, with optional filter
    /// - `"newPendingTransactions"`: pushed when a tx enters the mempool
    ///
    /// Returns a subscription ID. Events are pushed as `eth_subscription` notifications.
    /// Use `eth_unsubscribe` with the subscription ID to cancel.
    #[subscription(
        name = "subscribe" => "subscription",
        unsubscribe = "unsubscribe",
        item = serde_json::Value
    )]
    async fn subscribe(
        &self,
        kind: String,
        params: Option<serde_json::Value>,
    ) -> jsonrpsee::core::SubscriptionResult;
}
```

### 11.2.3 EthSubscription Implementation

```rust
// --- crates/node/rpc/src/eth.rs --- (add below existing impl blocks)

use crate::subscriptions::SubscriptionManager;

/// Implementation of eth_subscribe / eth_unsubscribe.
pub struct EthSubscriptionImpl {
    sub_mgr: Arc<SubscriptionManager>,
}

impl EthSubscriptionImpl {
    /// Create a new subscription handler with the shared subscription manager.
    pub fn new(sub_mgr: Arc<SubscriptionManager>) -> Self {
        Self { sub_mgr }
    }
}

#[jsonrpsee::core::async_trait]
impl EthSubscriptionApiServer for EthSubscriptionImpl {
    async fn subscribe(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
        kind: String,
        params: Option<serde_json::Value>,
    ) -> jsonrpsee::core::SubscriptionResult {
        match kind.as_str() {
            "newHeads" => {
                let sink = pending
                    .accept()
                    .await
                    .map_err(|_| "subscription rejected")?;
                let mut rx = self.sub_mgr.subscribe_new_heads();
                tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(header) => {
                                let msg = SubscriptionMessage::from_json(&header)
                                    .expect("header is valid json");
                                if sink.send(msg).await.is_err() {
                                    // Client disconnected.
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(skipped = n, "newHeads subscriber lagged");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
                Ok(())
            }

            "logs" => {
                // Parse optional filter from params.
                let filter: Option<LogSubscriptionFilter> = params
                    .and_then(|v| serde_json::from_value(v).ok());

                let sink = pending
                    .accept()
                    .await
                    .map_err(|_| "subscription rejected")?;
                let mut rx = self.sub_mgr.subscribe_logs();
                tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(log_value) => {
                                // Apply client-side filter if provided.
                                if let Some(ref f) = filter {
                                    if !matches_log_filter(&log_value, f) {
                                        continue;
                                    }
                                }
                                let msg = SubscriptionMessage::from_json(&log_value)
                                    .expect("log is valid json");
                                if sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(skipped = n, "logs subscriber lagged");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
                Ok(())
            }

            "newPendingTransactions" => {
                let sink = pending
                    .accept()
                    .await
                    .map_err(|_| "subscription rejected")?;
                let mut rx = self.sub_mgr.subscribe_pending_txs();
                tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(tx) => {
                                let msg = SubscriptionMessage::from_json(&tx)
                                    .expect("tx is valid json");
                                if sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(skipped = n, "pendingTx subscriber lagged");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
                Ok(())
            }

            other => {
                let _ = pending
                    .reject(jsonrpsee::types::ErrorObject::owned(
                        -32602,
                        format!("unsupported subscription type: {other}"),
                        None::<()>,
                    ))
                    .await;
                Ok(())
            }
        }
    }
}
```

### 11.2.4 Log Subscription Filter

```rust
// --- crates/node/rpc/src/subscriptions.rs --- (add to subscriptions module)

/// Filter for `eth_subscribe("logs", filter)`.
/// Matches the same shape as `eth_getLogs` filter but without block range
/// (subscriptions only deliver logs from new blocks).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSubscriptionFilter {
    /// Contract address(es) to match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<LogSubscriptionAddress>,
    /// Topic filters. Each position is AND-matched; within each position, OR-matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<Option<LogSubscriptionTopic>>>,
}

/// Address filter: single address or array of addresses.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogSubscriptionAddress {
    Single(Address),
    Multiple(Vec<Address>),
}

/// Topic filter: single topic or array of topics.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LogSubscriptionTopic {
    Single(B256),
    Multiple(Vec<B256>),
}

/// Check if a log JSON value matches the subscription filter.
/// The log value is the serialized RpcLog.
pub fn matches_log_filter(log_value: &serde_json::Value, filter: &LogSubscriptionFilter) -> bool {
    // Address check.
    if let Some(ref addr_filter) = filter.address {
        let log_addr = log_value.get("address").and_then(|v| v.as_str()).unwrap_or("");
        let matches = match addr_filter {
            LogSubscriptionAddress::Single(a) => {
                log_addr.eq_ignore_ascii_case(&format!("{a:?}"))
            }
            LogSubscriptionAddress::Multiple(addrs) => {
                addrs.iter().any(|a| log_addr.eq_ignore_ascii_case(&format!("{a:?}")))
            }
        };
        if !matches {
            return false;
        }
    }

    // Topic check.
    if let Some(ref topic_filters) = filter.topics {
        let log_topics = log_value
            .get("topics")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for (i, topic_filter) in topic_filters.iter().enumerate() {
            if let Some(tf) = topic_filter {
                let log_topic = log_topics
                    .get(i)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let matches = match tf {
                    LogSubscriptionTopic::Single(t) => {
                        log_topic.eq_ignore_ascii_case(&format!("{t:?}"))
                    }
                    LogSubscriptionTopic::Multiple(topics) => {
                        topics.iter().any(|t| log_topic.eq_ignore_ascii_case(&format!("{t:?}")))
                    }
                };
                if !matches {
                    return false;
                }
            }
        }
    }

    true
}
```

### 11.2.5 Bridge: FinalizedReporter to SubscriptionManager

The bridge connects the consensus finalization path to WebSocket subscribers. It runs as a background task spawned during node startup.

**File:** `crates/node/runner/src/runner.rs`

```rust
// --- crates/node/runner/src/runner.rs --- (add new function)

use kora_rpc::SubscriptionManager;

/// Spawn a bridge task that forwards LedgerEvents to the SubscriptionManager.
///
/// - `SnapshotPersisted` -> look up indexed block -> `notify_new_head` + `notify_log`
/// - `TransactionSubmitted` -> `notify_pending_tx`
fn spawn_subscription_bridge(
    ledger: &LedgerService,
    sub_mgr: Arc<SubscriptionManager>,
    block_index: Arc<kora_indexer::BlockIndex>,
) {
    let mut events = ledger.subscribe();
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match event {
                LedgerEvent::SnapshotPersisted(digest) => {
                    // The FinalizedReporter has already indexed this block by the time
                    // SnapshotPersisted fires (persist happens before event emission).
                    // Look up the indexed block to build the header JSON.
                    let block_hash = alloy_primitives::B256::from(digest.0);
                    if let Some(indexed_block) = block_index.get_block_by_hash(&block_hash) {
                        // Emit newHeads notification.
                        let header = serde_json::json!({
                            "hash": format!("0x{}", hex::encode(indexed_block.hash)),
                            "parentHash": format!("0x{}", hex::encode(indexed_block.parent_hash)),
                            "number": format!("0x{:x}", indexed_block.number),
                            "timestamp": format!("0x{:x}", indexed_block.timestamp),
                            "gasLimit": format!("0x{:x}", indexed_block.gas_limit),
                            "gasUsed": format!("0x{:x}", indexed_block.gas_used),
                            "stateRoot": format!("0x{}", hex::encode(indexed_block.state_root)),
                            "transactionsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                            "receiptsRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
                            "baseFeePerGas": indexed_block.base_fee_per_gas
                                .map(|f| format!("0x{:x}", f)),
                            "miner": "0x0000000000000000000000000000000000000000",
                            "difficulty": "0x0",
                            "extraData": "0x",
                            "logsBloom": "0x" .to_string() + &"0".repeat(512),
                            "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                            "nonce": "0x0000000000000000",
                        });
                        sub_mgr.notify_new_head(header);

                        // Emit log notifications (only if someone is listening).
                        if sub_mgr.has_log_subscribers() {
                            // Iterate transaction hashes -> receipts -> logs.
                            for tx_hash in &indexed_block.transaction_hashes {
                                if let Some(receipt) = block_index.get_receipt(tx_hash) {
                                    for log in &receipt.logs {
                                        let log_json = serde_json::json!({
                                            "address": format!("{:?}", log.address),
                                            "topics": log.topics.iter()
                                                .map(|t| format!("0x{}", hex::encode(t)))
                                                .collect::<Vec<_>>(),
                                            "data": format!("0x{}", hex::encode(&log.data)),
                                            "blockNumber": format!("0x{:x}", indexed_block.number),
                                            "transactionHash": format!("0x{}", hex::encode(tx_hash)),
                                            "transactionIndex": format!("0x{:x}", receipt.transaction_index),
                                            "blockHash": format!("0x{}", hex::encode(indexed_block.hash)),
                                            "logIndex": format!("0x{:x}", log.log_index),
                                            "removed": false,
                                        });
                                        sub_mgr.notify_log(log_json);
                                    }
                                }
                            }
                        }
                    }
                }
                LedgerEvent::TransactionSubmitted(tx_id) => {
                    let hash_json = serde_json::json!(
                        format!("0x{}", hex::encode(tx_id.0))
                    );
                    sub_mgr.notify_pending_tx(hash_json);
                }
                LedgerEvent::SeedUpdated(..) => {
                    // No subscription type for seed updates.
                }
            }
        }
    });
}
```

### 11.2.6 Register Subscription Module in Server

**File:** `crates/node/rpc/src/server.rs`

In the `start()` method of `RpcServer`, merge the subscription API into the RPC module:

```rust
// Inside the jsonrpc_handle spawn, after existing module merges:

// ---- Subscriptions (optional, requires WebSocket) ----
if let Some(sub_mgr) = subscription_manager {
    let eth_sub = EthSubscriptionImpl::new(sub_mgr.clone());
    if let Err(e) = module.merge(eth_sub.into_rpc()) {
        error!(error = %e, "Failed to merge eth subscription API");
        return None;
    }
    info!("eth_subscribe enabled (newHeads, logs, newPendingTransactions)");
}
```

### 11.2.7 Verification

- [ ] `eth_subscribe("newHeads")` over WebSocket returns a subscription ID
- [ ] On block finalization, subscriber receives a JSON object with block header fields
- [ ] Header includes: `hash`, `parentHash`, `number`, `timestamp`, `gasLimit`, `gasUsed`, `stateRoot`
- [ ] Multiple concurrent subscribers each receive the same header
- [ ] Subscriber disconnect is handled cleanly (no panics, no leaked tasks)
- [ ] `eth_unsubscribe(subId)` stops delivery

---

## 11.3 eth_subscribe -- logs

Covered in 11.2.3 (the `"logs"` branch of the `subscribe` implementation) and 11.2.4 (the `LogSubscriptionFilter`).

### Event Flow

```
Block finalized
  -> FinalizedReporter indexes block (receipts + logs stored in BlockIndex)
  -> LedgerEvent::SnapshotPersisted emitted
  -> spawn_subscription_bridge picks it up
  -> Iterates receipts for the block
  -> For each log: sub_mgr.notify_log(log_json)
  -> In subscription task: each log is checked against the subscriber's filter
  -> Matching logs are pushed to the WebSocket client
```

### Filter Semantics

Matches standard `eth_subscribe("logs")` semantics:

| Filter field | Behavior |
|---|---|
| `address` (single) | Only logs from this contract |
| `address` (array) | Logs from any of these contracts (OR) |
| `topics[0]` | Event signature filter (exact match) |
| `topics[N]` (array) | Any of these topics at position N (OR) |
| `topics[N]` (null) | Wildcard -- any topic at position N |

### Verification

- [ ] `eth_subscribe("logs", {"address": "0x..."})` filters correctly
- [ ] `eth_subscribe("logs", {"topics": [["0xddf...","0xabc..."]]})` matches OR within position
- [ ] `eth_subscribe("logs")` with no filter receives ALL logs
- [ ] Logs include: `address`, `topics`, `data`, `blockNumber`, `transactionHash`, `logIndex`, `removed`

---

## 11.4 eth_subscribe -- newPendingTransactions

Covered in 11.2.3 (the `"newPendingTransactions"` branch) and 11.2.5 (the bridge's `TransactionSubmitted` handler).

### Event Flow

```
User submits tx via eth_sendRawTransaction
  -> LedgerService.submit_tx() succeeds
  -> LedgerEvent::TransactionSubmitted(tx_id) emitted
  -> Bridge task: sub_mgr.notify_pending_tx(hash)
  -> All newPendingTransactions subscribers receive the hash
```

### Default Behavior

Returns transaction hash only (string). This matches `fullPendingTransactions=false` which is the default in the Ethereum spec.

### Optional: Full Transaction Object

To support `fullPendingTransactions=true`, the subscription handler checks the `params` argument:

```rust
// In the "newPendingTransactions" branch:
"newPendingTransactions" => {
    let full = params
        .and_then(|v| v.get("fullPendingTransactions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // ... spawn task ...
    // If `full`, the bridge needs to emit the full RpcTransaction object
    // instead of just the hash. This requires decoding the raw tx bytes
    // in the TransactionSubmitted handler.
}
```

This is deferred to a later phase. The hash-only mode is sufficient for the explorer's pending transaction particle effect (spawn particle at hash, resolve when included in block).

### Verification

- [ ] `eth_subscribe("newPendingTransactions")` returns a subscription ID
- [ ] When a tx is submitted, subscriber receives the tx hash as a hex string
- [ ] Multiple rapid submissions are each delivered (no coalescing)

---

## 11.5 kora_subscribe -- Consensus Events

### Objective

Expose all 10 Simplex `Activity` types to WebSocket subscribers for the Consensus Ring visualization (Scene 4). Currently the `NodeStateReporter` handles only 3 and reduces them to counter increments.

### 11.5.1 ConsensusEvent Enum

**New file:** `crates/node/rpc/src/consensus_events.rs`

```rust
//! Consensus event types for WebSocket subscription streaming.
//!
//! Each variant maps 1:1 to a Simplex Activity type. Serialized as tagged JSON
//! with camelCase field names.

use serde::{Deserialize, Serialize};

/// A consensus event pushed to WebSocket subscribers.
///
/// Tagged union: `{"type": "notarize", ...}`.
/// All 10 Simplex Activity types are represented.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConsensusEvent {
    /// Individual validator voted to notarize a proposal.
    #[serde(rename_all = "camelCase")]
    Notarize {
        view: u64,
        /// Hex-encoded block digest.
        payload: String,
        /// Local timestamp when event was received (ms since epoch).
        timestamp_ms: u64,
    },

    /// Quorum achieved -- enough validators notarized the proposal.
    #[serde(rename_all = "camelCase")]
    Notarization {
        view: u64,
        /// Hex-encoded block digest.
        payload: String,
        /// Hex-encoded VRF seed.
        seed: String,
        timestamp_ms: u64,
    },

    /// Individual validator voted to finalize (certification step).
    #[serde(rename_all = "camelCase")]
    Certification {
        view: u64,
        payload: String,
        timestamp_ms: u64,
    },

    /// Individual finalization vote.
    #[serde(rename_all = "camelCase")]
    Finalize {
        view: u64,
        payload: String,
        timestamp_ms: u64,
    },

    /// Block finalized -- threshold finalization signatures collected.
    #[serde(rename_all = "camelCase")]
    Finalization {
        view: u64,
        /// Hex-encoded block digest.
        payload: String,
        /// Hex-encoded VRF seed.
        seed: String,
        /// Resolved block height (if available from BlockIndex).
        block_height: Option<u64>,
        timestamp_ms: u64,
    },

    /// Individual nullification vote (timeout on current view).
    #[serde(rename_all = "camelCase")]
    Nullify {
        timestamp_ms: u64,
    },

    /// Quorum nullification -- view skipped, no block produced.
    #[serde(rename_all = "camelCase")]
    Nullification {
        /// The view that was nullified.
        view: u64,
        timestamp_ms: u64,
    },

    // ---- Safety Violations ----
    // These should NEVER happen in normal operation.
    // The explorer renders them with full-screen alert + persistent indicator.

    /// Two conflicting notarization votes from the same validator.
    #[serde(rename_all = "camelCase")]
    ConflictingNotarize {
        timestamp_ms: u64,
        /// Always "critical".
        severity: String,
    },

    /// Two conflicting finalization votes from the same validator.
    #[serde(rename_all = "camelCase")]
    ConflictingFinalize {
        timestamp_ms: u64,
        severity: String,
    },

    /// A validator sent both nullify and finalize for the same view.
    #[serde(rename_all = "camelCase")]
    NullifyFinalize {
        timestamp_ms: u64,
        severity: String,
    },
}

impl ConsensusEvent {
    /// Returns true if this is a safety violation event.
    pub const fn is_safety_violation(&self) -> bool {
        matches!(
            self,
            ConsensusEvent::ConflictingNotarize { .. }
                | ConsensusEvent::ConflictingFinalize { .. }
                | ConsensusEvent::NullifyFinalize { .. }
        )
    }

    /// Returns the view number for events that carry one.
    pub const fn view(&self) -> Option<u64> {
        match self {
            ConsensusEvent::Notarize { view, .. }
            | ConsensusEvent::Notarization { view, .. }
            | ConsensusEvent::Certification { view, .. }
            | ConsensusEvent::Finalize { view, .. }
            | ConsensusEvent::Finalization { view, .. }
            | ConsensusEvent::Nullification { view, .. } => Some(*view),
            _ => None,
        }
    }
}
```

### 11.5.2 ConsensusReporter

**New file:** `crates/node/reporters/src/consensus_reporter.rs`

This reporter intercepts ALL 10 Activity types and broadcasts them via `tokio::sync::broadcast`. It composes into the existing reporter chain via `Reporters::from()` -- zero impact on `SeedReporter`, `FinalizedReporter`, or `NodeStateReporter`.

```rust
//! Reporter that broadcasts all consensus Activity events for WebSocket streaming.

use std::{
    marker::PhantomData,
    time::{SystemTime, UNIX_EPOCH},
};

use commonware_consensus::{
    Reporter,
    simplex::types::Activity,
};
use commonware_cryptography::certificate::Scheme;
use commonware_codec::Encode;
use kora_domain::ConsensusDigest;
use kora_rpc::consensus_events::ConsensusEvent;
use tokio::sync::broadcast;

/// Broadcast capacity. 256 events ~= 50 seconds of healthy operation at
/// ~5 events/second. Subscribers that fall behind get a Lagged error.
const DEFAULT_CAPACITY: usize = 256;

/// Reporter that converts all Simplex Activity events into ConsensusEvents
/// and broadcasts them via a tokio broadcast channel.
///
/// # Usage
///
/// ```ignore
/// let (consensus_reporter, _rx) = ConsensusReporter::<ThresholdScheme>::new(256);
/// // Pass consensus_reporter.clone() into the reporter chain.
/// // Pass consensus_reporter into the RPC server for kora_subscribe routing.
/// ```
#[derive(Clone, Debug)]
pub struct ConsensusReporter<S> {
    tx: broadcast::Sender<ConsensusEvent>,
    _scheme: PhantomData<S>,
}

impl<S> ConsensusReporter<S> {
    /// Create a new consensus reporter with explicit capacity.
    ///
    /// Returns the reporter and a receiver for initial setup (usually discarded).
    pub fn new(capacity: usize) -> (Self, broadcast::Receiver<ConsensusEvent>) {
        let (tx, rx) = broadcast::channel(capacity);
        (Self { tx, _scheme: PhantomData }, rx)
    }

    /// Create with default capacity (256).
    pub fn with_default_capacity() -> (Self, broadcast::Receiver<ConsensusEvent>) {
        Self::new(DEFAULT_CAPACITY)
    }

    /// Subscribe to the consensus event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<ConsensusEvent> {
        self.tx.subscribe()
    }

    /// Get a clone of the sender (for passing to the RPC layer).
    pub fn sender(&self) -> broadcast::Sender<ConsensusEvent> {
        self.tx.clone()
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

impl<S> Reporter for ConsensusReporter<S>
where
    S: Scheme + Clone + Send + 'static,
    S::Signature: Encode,
{
    type Activity = Activity<S, ConsensusDigest>;

    fn report(&mut self, activity: Self::Activity) -> impl std::future::Future<Output = ()> + Send {
        let event = match &activity {
            Activity::Notarize(n) => ConsensusEvent::Notarize {
                view: n.proposal.round.view().get(),
                payload: hex::encode(n.proposal.payload.0),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Notarization(n) => ConsensusEvent::Notarization {
                view: n.proposal.round.view().get(),
                payload: hex::encode(n.proposal.payload.0),
                seed: hex::encode(n.seed().encode()),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Certification(c) => ConsensusEvent::Certification {
                view: c.proposal.round.view().get(),
                payload: hex::encode(c.proposal.payload.0),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Finalize(f) => ConsensusEvent::Finalize {
                view: f.proposal.round.view().get(),
                payload: hex::encode(f.proposal.payload.0),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Finalization(f) => ConsensusEvent::Finalization {
                view: f.proposal.round.view().get(),
                payload: hex::encode(f.proposal.payload.0),
                seed: hex::encode(f.seed().encode()),
                block_height: None, // Resolved later by correlation with BlockIndex
                timestamp_ms: Self::now_ms(),
            },
            Activity::Nullify(_) => ConsensusEvent::Nullify {
                timestamp_ms: Self::now_ms(),
            },
            Activity::Nullification(n) => ConsensusEvent::Nullification {
                view: n.round.view().get(),
                timestamp_ms: Self::now_ms(),
            },
            Activity::ConflictingNotarize(_) => ConsensusEvent::ConflictingNotarize {
                timestamp_ms: Self::now_ms(),
                severity: "critical".to_string(),
            },
            Activity::ConflictingFinalize(_) => ConsensusEvent::ConflictingFinalize {
                timestamp_ms: Self::now_ms(),
                severity: "critical".to_string(),
            },
            Activity::NullifyFinalize(_) => ConsensusEvent::NullifyFinalize {
                timestamp_ms: Self::now_ms(),
                severity: "critical".to_string(),
            },
        };

        let _ = self.tx.send(event);
        async {}
    }
}
```

### 11.5.3 Wire ConsensusReporter into Reporter Chain

**File:** `crates/node/runner/src/runner.rs`

Replace the reporter composition block near the end of `ProductionRunner::run`:

```rust
// --- BEFORE (current) ---
let seed_reporter = SeedReporter::<MinSig>::new(ledger.clone());
let node_state_reporter = self
    .rpc_config
    .as_ref()
    .map(|(state, _)| NodeStateReporter::<ThresholdScheme>::new(state.clone()));
let inner_reporters: Reporters<_, MarshalMailbox, Option<NodeStateRptr>> =
    Reporters::from((marshal_mailbox.clone(), node_state_reporter));
let reporter = Reporters::from((seed_reporter, inner_reporters));

// --- AFTER ---
let seed_reporter = SeedReporter::<MinSig>::new(ledger.clone());

// NEW: Create ConsensusReporter for streaming all 10 Activity types.
let (consensus_reporter, _consensus_rx) =
    ConsensusReporter::<ThresholdScheme>::new(256);
// Keep a handle for the RPC subscription layer.
let consensus_reporter_for_rpc = consensus_reporter.clone();

let node_state_reporter = self
    .rpc_config
    .as_ref()
    .map(|(state, _)| NodeStateReporter::<ThresholdScheme>::new(state.clone()));

// Compose: seed -> consensus -> (marshal, node_state)
let inner_reporters: Reporters<_, MarshalMailbox, Option<NodeStateRptr>> =
    Reporters::from((marshal_mailbox.clone(), node_state_reporter));
let with_consensus = Reporters::from((consensus_reporter, inner_reporters));
let reporter = Reporters::from((seed_reporter, with_consensus));
```

The `Reporters::from()` tuple composition means every reporter in the chain receives every Activity event. Adding `ConsensusReporter` into the chain has zero impact on existing reporters.

### 11.5.4 KoraSubscription Trait and Implementation

**File:** `crates/node/rpc/src/kora.rs`

Extend the existing `KoraApi` trait and add subscription support:

```rust
// --- crates/node/rpc/src/kora.rs ---

use crate::consensus_events::ConsensusEvent;
use tokio::sync::broadcast;

/// Kora-specific subscription API (WebSocket only).
#[rpc(server, namespace = "kora")]
pub trait KoraSubscriptionApi {
    /// Subscribe to Kora consensus events.
    ///
    /// Supported types:
    /// - `"consensus"`: all 10 Activity types (full firehose)
    /// - `"consensus.safety"`: only safety violations
    #[subscription(
        name = "subscribe" => "subscription",
        unsubscribe = "unsubscribe",
        item = ConsensusEvent
    )]
    async fn subscribe(
        &self,
        kind: String,
    ) -> jsonrpsee::core::SubscriptionResult;
}

/// Implementation of kora_subscribe / kora_unsubscribe.
pub struct KoraSubscriptionImpl {
    consensus_tx: broadcast::Sender<ConsensusEvent>,
}

impl KoraSubscriptionImpl {
    /// Create a new Kora subscription handler.
    pub fn new(consensus_tx: broadcast::Sender<ConsensusEvent>) -> Self {
        Self { consensus_tx }
    }
}

#[jsonrpsee::core::async_trait]
impl KoraSubscriptionApiServer for KoraSubscriptionImpl {
    async fn subscribe(
        &self,
        pending: jsonrpsee::PendingSubscriptionSink,
        kind: String,
    ) -> jsonrpsee::core::SubscriptionResult {
        match kind.as_str() {
            "consensus" => {
                // Full firehose: all 10 Activity types.
                let sink = pending
                    .accept()
                    .await
                    .map_err(|_| "subscription rejected")?;
                let mut rx = self.consensus_tx.subscribe();
                tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(event) => {
                                let msg = SubscriptionMessage::from_json(&event)
                                    .expect("ConsensusEvent is serializable");
                                if sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(skipped = n, "consensus subscriber lagged");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
                Ok(())
            }

            "consensus.safety" => {
                // Only safety violation events.
                let sink = pending
                    .accept()
                    .await
                    .map_err(|_| "subscription rejected")?;
                let mut rx = self.consensus_tx.subscribe();
                tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(event) if event.is_safety_violation() => {
                                let msg = SubscriptionMessage::from_json(&event)
                                    .expect("ConsensusEvent is serializable");
                                if sink.send(msg).await.is_err() {
                                    break;
                                }
                            }
                            Ok(_) => continue,  // skip non-safety events
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
                Ok(())
            }

            other => {
                let _ = pending
                    .reject(jsonrpsee::types::ErrorObject::owned(
                        -32602,
                        format!("unsupported kora subscription type: {other}"),
                        None::<()>,
                    ))
                    .await;
                Ok(())
            }
        }
    }
}
```

### 11.5.5 Register KoraSubscription in Server

**File:** `crates/node/rpc/src/server.rs`

```rust
// After the KoraApiImpl merge, add:
if let Some(consensus_tx) = consensus_event_sender {
    let kora_sub = KoraSubscriptionImpl::new(consensus_tx);
    if let Err(e) = module.merge(kora_sub.into_rpc()) {
        error!(error = %e, "Failed to merge kora subscription API");
        return None;
    }
    info!("kora_subscribe enabled (consensus, consensus.safety)");
}
```

### 11.5.6 Wire Format Examples

Healthy round (~1s):

```jsonc
// View 100: Validator 0 is leader (100 % 4 == 0)

{"type":"notarize","view":100,"payload":"0xabcd...","timestampMs":1715200000100}
{"type":"notarization","view":100,"payload":"0xabcd...","seed":"0x1234...","timestampMs":1715200000250}
{"type":"certification","view":100,"payload":"0xabcd...","timestampMs":1715200000300}
{"type":"finalize","view":100,"payload":"0xabcd...","timestampMs":1715200000350}
{"type":"finalization","view":100,"payload":"0xabcd...","seed":"0x5678...","blockHeight":286401,"timestampMs":1715200000450}
```

Nullified round (leader timeout):

```jsonc
{"type":"nullify","timestampMs":1715200001100}
{"type":"nullification","view":101,"timestampMs":1715200001500}
```

Safety violation:

```jsonc
{"type":"conflictingNotarize","timestampMs":1715200005000,"severity":"critical"}
```

### 11.5.7 Verification

- [ ] `kora_subscribe("consensus")` delivers all 10 Activity types
- [ ] `kora_subscribe("consensus.safety")` delivers ONLY `ConflictingNotarize`, `ConflictingFinalize`, `NullifyFinalize`
- [ ] Events arrive with <10ms latency from consensus engine processing
- [ ] ~5 events/second during healthy 1-second block time
- [ ] Lagged subscribers receive `RecvError::Lagged` and continue (no crash)
- [ ] Existing `NodeStateReporter` counters still update correctly (chain composition works)

---

## 11.6 kora_consensusState (Polling Endpoint)

### Objective

Richer poll endpoint than `kora_nodeStatus`. Returns consensus round details, timing metrics, and safety stats. This is the fallback before WebSocket subscriptions are available.

### 11.6.1 ConsensusState Response Type

**File:** `crates/node/rpc/src/consensus_events.rs` (extend)

```rust
// --- crates/node/rpc/src/consensus_events.rs --- (add to module)

/// Response for `kora_consensusState` -- a snapshot of current consensus status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusState {
    // ---- Identity ----
    /// Chain ID.
    pub chain_id: u64,
    /// This validator's index.
    pub validator_index: u32,
    /// Total number of validators in the set.
    pub validator_count: u32,
    /// Votes needed for quorum (2f+1).
    pub quorum_threshold: u32,

    // ---- Current Round ----
    /// Current consensus view number.
    pub current_view: u64,
    /// Validator index of the current leader (view % validator_count).
    pub current_leader: u32,
    /// Current round phase.
    pub round_phase: RoundPhase,

    // ---- Cumulative Metrics ----
    pub finalized_count: u64,
    pub proposed_count: u64,
    pub nullified_count: u64,

    // ---- Health ----
    pub peer_count: u64,
    pub uptime_secs: u64,
    pub is_leader: bool,

    // ---- Timing ----
    /// Timestamp of last finalization (ms since epoch). Null if none yet.
    pub last_finalized_at_ms: Option<u64>,
    /// Rolling average finalization latency (ms). Null if insufficient data.
    pub avg_finalization_ms: Option<u64>,
    /// Timestamp of last nullification (ms since epoch).
    pub last_nullified_at_ms: Option<u64>,

    // ---- Safety ----
    /// Total lifetime safety violation events.
    pub safety_violations: u64,
    /// Timestamp of most recent violation. Null if none.
    pub last_violation_at_ms: Option<u64>,
}

/// Phase of the current consensus round.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RoundPhase {
    /// Leader is building a block.
    Proposing,
    /// Validators are voting on the proposal.
    Notarizing,
    /// Validators are finalizing.
    Certifying,
    /// Block committed.
    Finalized,
    /// Round timing out.
    Nullifying,
    /// Between rounds.
    Idle,
}
```

### 11.6.2 Extended NodeState

**File:** `crates/node/rpc/src/state.rs`

Add timing, phase, and safety tracking fields to `NodeStateInner`:

```rust
// --- crates/node/rpc/src/state.rs ---

use std::collections::VecDeque;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use crate::consensus_events::RoundPhase;

#[derive(Debug)]
struct NodeStateInner {
    chain_id: u64,
    validator_index: u32,
    started_at: Instant,
    current_view: AtomicU64,
    finalized_count: AtomicU64,
    proposed_count: AtomicU64,
    nullified_count: AtomicU64,
    peer_count: AtomicU64,
    is_leader: RwLock<bool>,

    // ---- NEW: consensus timing ----
    /// Number of validators in the set (set once at startup).
    validator_count: u32,
    /// Quorum threshold (set once at startup).
    quorum_threshold: u32,
    /// Timestamp of last finalization.
    last_finalized_at: RwLock<Option<Instant>>,
    /// Timestamp of last nullification.
    last_nullified_at: RwLock<Option<Instant>>,
    /// Last 100 finalization durations for rolling average.
    finalization_times: RwLock<VecDeque<u64>>,

    // ---- NEW: round tracking ----
    /// When the current round started.
    current_round_start: RwLock<Instant>,
    /// Current round phase.
    round_phase: RwLock<RoundPhase>,

    // ---- NEW: safety ----
    /// Total lifetime safety violations.
    safety_violation_count: AtomicU64,
    /// Timestamp of most recent safety violation.
    last_violation_at: RwLock<Option<Instant>>,
}
```

Add corresponding update methods to `NodeState`:

```rust
impl NodeState {
    /// Create a new node state with validator set information.
    pub fn new(chain_id: u64, validator_index: u32) -> Self {
        Self::with_validators(chain_id, validator_index, 4, 3)
    }

    /// Create with explicit validator set size and quorum threshold.
    pub fn with_validators(
        chain_id: u64,
        validator_index: u32,
        validator_count: u32,
        quorum_threshold: u32,
    ) -> Self {
        Self {
            inner: Arc::new(NodeStateInner {
                chain_id,
                validator_index,
                started_at: Instant::now(),
                current_view: AtomicU64::new(0),
                finalized_count: AtomicU64::new(0),
                proposed_count: AtomicU64::new(0),
                nullified_count: AtomicU64::new(0),
                peer_count: AtomicU64::new(0),
                is_leader: RwLock::new(false),
                validator_count,
                quorum_threshold,
                last_finalized_at: RwLock::new(None),
                last_nullified_at: RwLock::new(None),
                finalization_times: RwLock::new(VecDeque::with_capacity(100)),
                current_round_start: RwLock::new(Instant::now()),
                round_phase: RwLock::new(RoundPhase::Idle),
                safety_violation_count: AtomicU64::new(0),
                last_violation_at: RwLock::new(None),
            }),
        }
    }

    /// Record a finalization event with its duration.
    pub fn record_finalization(&self) {
        let now = Instant::now();
        *self.inner.last_finalized_at.write() = Some(now);
        let duration_ms = now
            .duration_since(*self.inner.current_round_start.read())
            .as_millis() as u64;
        let mut times = self.inner.finalization_times.write();
        if times.len() >= 100 {
            times.pop_front();
        }
        times.push_back(duration_ms);
    }

    /// Record a nullification event.
    pub fn record_nullification(&self) {
        *self.inner.last_nullified_at.write() = Some(Instant::now());
    }

    /// Record a safety violation.
    pub fn record_safety_violation(&self) {
        self.inner.safety_violation_count.fetch_add(1, Ordering::Relaxed);
        *self.inner.last_violation_at.write() = Some(Instant::now());
    }

    /// Set the current round phase.
    pub fn set_phase(&self, phase: RoundPhase) {
        *self.inner.round_phase.write() = phase;
    }

    /// Mark the start of a new round.
    pub fn start_round(&self) {
        *self.inner.current_round_start.write() = Instant::now();
        *self.inner.round_phase.write() = RoundPhase::Proposing;
    }

    /// Get the rolling average finalization time in milliseconds.
    pub fn avg_finalization_ms(&self) -> Option<u64> {
        let times = self.inner.finalization_times.read();
        if times.is_empty() {
            return None;
        }
        let sum: u64 = times.iter().sum();
        Some(sum / times.len() as u64)
    }

    /// Build the full consensus state response.
    pub fn consensus_state(&self) -> ConsensusState {
        let now_ms = || -> Option<u64> {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as u64)
        };

        let instant_to_epoch_ms = |instant: Instant| -> u64 {
            // Convert Instant to epoch ms by calculating offset from now.
            let elapsed = instant.elapsed();
            now_ms().unwrap_or(0).saturating_sub(elapsed.as_millis() as u64)
        };

        let current_view = self.inner.current_view.load(Ordering::Relaxed);
        ConsensusState {
            chain_id: self.inner.chain_id,
            validator_index: self.inner.validator_index,
            validator_count: self.inner.validator_count,
            quorum_threshold: self.inner.quorum_threshold,
            current_view,
            current_leader: (current_view % self.inner.validator_count as u64) as u32,
            round_phase: *self.inner.round_phase.read(),
            finalized_count: self.inner.finalized_count.load(Ordering::Relaxed),
            proposed_count: self.inner.proposed_count.load(Ordering::Relaxed),
            nullified_count: self.inner.nullified_count.load(Ordering::Relaxed),
            peer_count: self.inner.peer_count.load(Ordering::Relaxed),
            uptime_secs: self.inner.started_at.elapsed().as_secs(),
            is_leader: *self.inner.is_leader.read(),
            last_finalized_at_ms: self.inner.last_finalized_at.read()
                .map(instant_to_epoch_ms),
            avg_finalization_ms: self.avg_finalization_ms(),
            last_nullified_at_ms: self.inner.last_nullified_at.read()
                .map(instant_to_epoch_ms),
            safety_violations: self.inner.safety_violation_count.load(Ordering::Relaxed),
            last_violation_at_ms: self.inner.last_violation_at.read()
                .map(instant_to_epoch_ms),
        }
    }
}
```

### 11.6.3 Add RPC Method

**File:** `crates/node/rpc/src/kora.rs`

```rust
// Extend the existing KoraApi trait:

#[rpc(server, namespace = "kora")]
pub trait KoraApi {
    /// Returns the current node status including consensus information.
    #[method(name = "nodeStatus")]
    async fn node_status(&self) -> RpcResult<NodeStatus>;

    /// Returns detailed consensus state.
    ///
    /// Richer than nodeStatus: includes round phase, timing metrics,
    /// validator set info, and safety violation counts.
    #[method(name = "consensusState")]
    async fn consensus_state(&self) -> RpcResult<ConsensusState>;
}

// Extend the implementation:

#[jsonrpsee::core::async_trait]
impl KoraApiServer for KoraApiImpl {
    async fn node_status(&self) -> RpcResult<NodeStatus> {
        Ok(self.state.status())
    }

    async fn consensus_state(&self) -> RpcResult<ConsensusState> {
        Ok(self.state.consensus_state())
    }
}
```

### 11.6.4 Verification

- [ ] `kora_consensusState` returns all fields with correct types
- [ ] `currentView`, `currentLeader`, `roundPhase` update in real time
- [ ] `avgFinalizationMs` returns a reasonable value after a few blocks
- [ ] `safetyViolations` starts at 0 and increments on safety events
- [ ] `validatorCount` and `quorumThreshold` are correct (4 and 3 for 4-validator devnet)

---

## 11.7 eth_getBlockReceipts

### Objective

Return all receipts for a block in one call. Eliminates N+1 queries for the explorer's block detail view.

### 11.7.1 Add to BlockIndex

**File:** `crates/storage/indexer/src/store.rs`

```rust
// --- crates/storage/indexer/src/store.rs --- (add method to BlockIndex impl)

/// Gets all receipts for a block, ordered by transaction index.
///
/// Returns an empty Vec if the block is not found or has no receipts.
pub fn get_receipts_for_block(&self, block_hash: &B256) -> Vec<IndexedReceipt> {
    // First get the block to find its transaction hashes.
    let blocks = self.blocks_by_hash.read();
    let Some(block) = blocks.get(block_hash) else {
        return Vec::new();
    };
    let tx_hashes = block.transaction_hashes.clone();
    drop(blocks);

    // Collect receipts in transaction-index order.
    let receipts = self.receipts.read();
    let mut result: Vec<IndexedReceipt> = tx_hashes
        .iter()
        .filter_map(|hash| receipts.get(hash).cloned())
        .collect();
    result.sort_by_key(|r| r.transaction_index);
    result
}
```

### 11.7.2 Add to StateProvider Trait

**File:** `crates/node/rpc/src/state_provider.rs`

```rust
// --- crates/node/rpc/src/state_provider.rs --- (add to StateProvider trait)

/// Get all receipts for a block.
async fn block_receipts(
    &self,
    _block: BlockNumberOrTag,
) -> Result<Option<Vec<RpcTransactionReceipt>>, RpcError> {
    Err(RpcError::NotImplemented)
}
```

### 11.7.3 Implement in IndexedStateProvider

**File:** `crates/node/rpc/src/indexed_provider.rs`

```rust
// --- crates/node/rpc/src/indexed_provider.rs --- (add to StateProvider impl)

async fn block_receipts(
    &self,
    block: BlockNumberOrTag,
) -> Result<Option<Vec<RpcTransactionReceipt>>, RpcError> {
    let block_num = self.resolve_block_number(&block)?;
    let Some(indexed_block) = self.index.get_block_by_number(block_num) else {
        return Ok(None);
    };

    let indexed_receipts = self.index.get_receipts_for_block(&indexed_block.hash);
    if indexed_receipts.is_empty() && !indexed_block.transaction_hashes.is_empty() {
        // Block has transactions but no receipts indexed yet.
        return Ok(None);
    }

    let rpc_receipts = indexed_receipts
        .into_iter()
        .map(indexed_receipt_to_rpc)
        .collect();
    Ok(Some(rpc_receipts))
}
```

### 11.7.4 Add to EthApi Trait

**File:** `crates/node/rpc/src/eth.rs`

```rust
// --- Inside the EthApi trait definition ---

/// Returns all transaction receipts for a block.
///
/// More efficient than fetching receipts individually. Returns None
/// if the block is not found.
#[method(name = "getBlockReceipts")]
async fn get_block_receipts(
    &self,
    block: BlockNumberOrTag,
) -> RpcResult<Option<Vec<RpcTransactionReceipt>>>;
```

### 11.7.5 Implement in EthApiImpl

```rust
// --- Inside the EthApiServer impl for EthApiImpl ---

async fn get_block_receipts(
    &self,
    block: BlockNumberOrTag,
) -> RpcResult<Option<Vec<RpcTransactionReceipt>>> {
    let provider = self.state_provider.read().await;
    provider.block_receipts(block).await.map_err(Into::into)
}
```

### 11.7.6 Verification

- [ ] `eth_getBlockReceipts("latest")` returns all receipts for the head block
- [ ] `eth_getBlockReceipts("0x5")` returns receipts for block 5
- [ ] Receipts are ordered by `transactionIndex`
- [ ] Each receipt includes full log arrays
- [ ] Returns `null` for non-existent blocks
- [ ] Returns `[]` for blocks with zero transactions

---

## 11.8 Extended NodeStatus

### Objective

Add `blockTimeMs`, `validatorCount`, and `quorumThreshold` to the `kora_nodeStatus` response. These help the explorer configure itself (ring size, timing estimates).

### 11.8.1 Extend NodeStatus

**File:** `crates/node/rpc/src/state.rs`

```rust
// --- crates/node/rpc/src/state.rs --- (extend NodeStatus struct)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatus {
    // ---- Existing fields ----
    pub chain_id: u64,
    pub validator_index: u32,
    pub uptime_secs: u64,
    pub current_view: u64,
    pub finalized_count: u64,
    pub proposed_count: u64,
    pub nullified_count: u64,
    pub peer_count: u64,
    pub is_leader: bool,

    // ---- NEW fields ----
    /// Number of validators in the consensus set.
    pub validator_count: u32,
    /// Votes needed for quorum (2f+1).
    pub quorum_threshold: u32,
    /// Configured block time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_time_ms: Option<u64>,
}
```

### 11.8.2 Update status() Method

```rust
pub fn status(&self) -> NodeStatus {
    NodeStatus {
        chain_id: self.inner.chain_id,
        validator_index: self.inner.validator_index,
        uptime_secs: self.inner.started_at.elapsed().as_secs(),
        current_view: self.inner.current_view.load(Ordering::Relaxed),
        finalized_count: self.inner.finalized_count.load(Ordering::Relaxed),
        proposed_count: self.inner.proposed_count.load(Ordering::Relaxed),
        nullified_count: self.inner.nullified_count.load(Ordering::Relaxed),
        peer_count: self.inner.peer_count.load(Ordering::Relaxed),
        is_leader: *self.inner.is_leader.read(),
        // NEW
        validator_count: self.inner.validator_count,
        quorum_threshold: self.inner.quorum_threshold,
        block_time_ms: None, // Set from config, not stored in NodeState
    }
}
```

`block_time_ms` is populated from the runner's `block_time_ms` config field. The RPC server doesn't know about block time directly -- either:

- (a) Store it in `NodeState` at construction time, or
- (b) Let the runner inject it after construction.

Option (a) is cleaner:

```rust
pub fn with_validators(
    chain_id: u64,
    validator_index: u32,
    validator_count: u32,
    quorum_threshold: u32,
) -> Self {
    // ... same as 11.6.2 ...
}

/// Set the configured block time. Called once at startup.
pub fn set_block_time_ms(&self, ms: u64) {
    self.inner.block_time_ms.store(ms, Ordering::Relaxed);
}
```

Add `block_time_ms: AtomicU64` to `NodeStateInner` and read it in `status()`.

### 11.8.3 Verification

- [ ] `kora_nodeStatus` response includes `validatorCount`, `quorumThreshold`
- [ ] `validatorCount` matches the actual validator set size
- [ ] `quorumThreshold` is correct (e.g., 3 for 4 validators)
- [ ] `blockTimeMs` is present and matches configured block time
- [ ] Existing fields (`chainId`, `currentView`, etc.) are unchanged
- [ ] JSON uses camelCase: `validatorCount`, not `validator_count`

---

## 11.9 Implementation Sequence

Each item can be deployed independently. Ordered by impact and dependency:

| Priority | Item | Depends On | WebSocket Required | Effort |
|----------|------|------------|-------------------|--------|
| **1** | `eth_getBlockReceipts` (11.7) | Nothing | No | 2h |
| **2** | Extended NodeStatus (11.8) | Nothing | No | 1h |
| **3** | WebSocket transport (11.1) | Nothing | -- | 3h |
| **4** | `eth_subscribe("newHeads")` (11.2) | 11.1 | Yes | 4h |
| **5** | `eth_subscribe("logs")` (11.3) | 11.2 | Yes | 2h |
| **6** | `eth_subscribe("newPendingTransactions")` (11.4) | 11.2 | Yes | 1h |
| **7** | `kora_consensusState` (11.6) | Nothing | No | 3h |
| **8** | `kora_subscribe("consensus")` (11.5) | 11.1 | Yes | 5h |

**Total estimated effort: ~21 hours.**

Items 1-2 provide immediate value with zero risk (no transport changes). Item 3 is foundational for everything with "Yes" in the WebSocket column. Item 4 (`newHeads`) is the single most impactful subscription for the explorer.

### Deployment Strategy

**Phase A -- Polling foundation (items 1-2):**
- Commit `eth_getBlockReceipts` and extended `NodeStatus`.
- Explorer can start building block detail views immediately.
- Zero WebSocket risk.

**Phase B -- WebSocket + eth subscriptions (items 3-6):**
- Enable WebSocket transport.
- Add `eth_subscribe` with all three subscription types.
- Explorer switches from polling to push for live data.

**Phase C -- Consensus visibility (items 7-8):**
- Add `kora_consensusState` for polling.
- Add `ConsensusReporter` + `kora_subscribe` for push.
- Explorer's Consensus Ring scene (Scene 4) comes alive.

---

## 11.10 Verification Matrix

| Test | Method | Expected |
|------|--------|----------|
| WS handshake | Connect to `ws://localhost:8545` | Handshake succeeds |
| HTTP still works | POST `eth_blockNumber` to `http://localhost:8545` | Returns block number |
| newHeads | `eth_subscribe("newHeads")` over WS, wait for block | Receive header JSON within ~50ms of finalization |
| logs (no filter) | `eth_subscribe("logs")` over WS, deploy contract | Receive all logs |
| logs (address filter) | `eth_subscribe("logs", {"address":"0x..."})` | Only logs from that address |
| logs (topic filter) | `eth_subscribe("logs", {"topics":[["0xddf..."]]})` | Only matching topic[0] |
| pendingTx | `eth_subscribe("newPendingTransactions")`, submit tx | Receive hash string |
| consensus (full) | `kora_subscribe("consensus")` | All 10 Activity types stream |
| consensus (safety) | `kora_subscribe("consensus.safety")` | Only safety violations |
| consensusState | `kora_consensusState()` | JSON with all fields |
| blockReceipts | `eth_getBlockReceipts("latest")` | All receipts for head block |
| nodeStatus extended | `kora_nodeStatus()` | Includes `validatorCount`, `quorumThreshold` |
| 10 concurrent WS | Open 10 WS connections, subscribe newHeads | All 10 receive every block |
| subscribe + disconnect | Subscribe, close WS, submit blocks | No server panic, no leaked tasks |
| unsubscribe | `eth_unsubscribe(subId)` | Delivery stops, subscription cleaned up |
| lagged subscriber | Subscribe newHeads, block recv for 300+ blocks | Subscriber gets `Lagged` error, recovers |

### Test Harness

```bash
# Start a 4-validator devnet with RPC enabled.
# Default: HTTP+WS on port 8545, HTTP status on 8546.

# Test 1: HTTP still works
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:8545

# Test 2: WS subscription (using websocat)
echo '{"jsonrpc":"2.0","method":"eth_subscribe","params":["newHeads"],"id":1}' \
  | websocat ws://localhost:8545

# Test 3: Kora consensus subscription
echo '{"jsonrpc":"2.0","method":"kora_subscribe","params":["consensus"],"id":1}' \
  | websocat ws://localhost:8545

# Test 4: Block receipts
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockReceipts","params":["latest"],"id":1}' \
  http://localhost:8545

# Test 5: Consensus state
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_consensusState","params":[],"id":1}' \
  http://localhost:8545

# Test 6: Extended node status
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' \
  http://localhost:8545
```

---

## File Inventory

All files created or modified, grouped by crate:

### `crates/node/rpc/` (kora-rpc)

| File | Action | Description |
|------|--------|-------------|
| `Cargo.toml` | Modify | Add `"ws"` feature to jsonrpsee |
| `src/config.rs` | Modify | Add `ws_addr`, `max_subscriptions_per_connection` |
| `src/server.rs` | Modify | WS server builder, subscription module registration |
| `src/eth.rs` | Modify | Add `EthSubscriptionApi` trait + impl, `eth_getBlockReceipts` |
| `src/kora.rs` | Modify | Add `kora_consensusState`, `KoraSubscriptionApi` trait + impl |
| `src/state.rs` | Modify | Extended `NodeStateInner`, `ConsensusState`, `RoundPhase` |
| `src/subscriptions.rs` | **New** | `SubscriptionManager`, `LogSubscriptionFilter`, filter matching |
| `src/consensus_events.rs` | **New** | `ConsensusEvent` enum, `ConsensusState` response type |
| `src/lib.rs` | Modify | Re-export new modules |

### `crates/node/reporters/` (kora-reporters)

| File | Action | Description |
|------|--------|-------------|
| `src/consensus_reporter.rs` | **New** | `ConsensusReporter` -- all 10 Activity types to broadcast |
| `src/lib.rs` | Modify | Re-export `ConsensusReporter` |

### `crates/node/runner/` (kora-runner)

| File | Action | Description |
|------|--------|-------------|
| `src/runner.rs` | Modify | Wire `ConsensusReporter` into chain, `spawn_subscription_bridge`, pass `SubscriptionManager` to RPC |

### `crates/storage/indexer/` (kora-indexer)

| File | Action | Description |
|------|--------|-------------|
| `src/store.rs` | Modify | Add `get_receipts_for_block()` method |

### `crates/node/rpc/src/state_provider.rs`

| File | Action | Description |
|------|--------|-------------|
| `src/state_provider.rs` | Modify | Add `block_receipts()` default method |

### `crates/node/rpc/src/indexed_provider.rs`

| File | Action | Description |
|------|--------|-------------|
| `src/indexed_provider.rs` | Modify | Implement `block_receipts()` |
