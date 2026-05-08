# 05 — WebSocket Transport & Subscriptions

## Problem

No WebSocket transport exists. `eth_subscribe` and `kora_subscribe` both return
Method not found. The explorer needs real-time block and consensus event
streaming.

## Root Cause

- `jsonrpsee` Cargo feature only enables `"server"` (HTTP). No `"server-ws"`
  or `"ws-server"` feature.
- `RpcServerConfig` defines `ws_addr` in earlier design notes but the current
  `config.rs` has no WebSocket address field.
- `RpcServer::start()` creates a single `jsonrpsee::server::Server` bound to
  one TCP address over HTTP. No WebSocket upgrade path.
- No subscription traits or handlers exist.

## Design

### Architecture

jsonrpsee 0.24 supports HTTP + WebSocket on the **same server** via its
`ServerBuilder`. This is preferable to binding a second port because:

1. Railway exposes one port per service — a second port means a second service
2. Simpler configuration (one address, one TLS cert, one CORS config)
3. jsonrpsee handles the HTTP/WS routing internally (WS upgrade on same port)

The server builder needs only the `ws` transport feature:

```toml
jsonrpsee = { version = "0.24", features = ["server", "macros", "ws-transport"] }
```

**Note**: Verify the exact feature flag name — jsonrpsee 0.24 may use `"server"`
for both HTTP and WS, with WS automatically available when using
`ServerBuilder`. Check the jsonrpsee docs/Cargo.toml for the correct feature.
If `"server"` already includes WS support, no feature change is needed — just
enable `ws_max_connections` on the builder.

### Phase 1: Enable WebSocket Transport

Modify `RpcServer::start()` to enable WebSocket upgrades:

```rust
let server = Server::builder()
    .max_connections(max_connections)
    .ws_max_connections(max_connections)  // Enable WS
    .build(jsonrpc_addr)
    .await?;
```

If jsonrpsee 0.24 doesn't support mixed HTTP+WS on the same `Server`, the
alternative is to create a second `Server` on a dedicated WS port:

```rust
// In RpcServerConfig
pub ws_addr: Option<SocketAddr>,  // None = WS disabled
```

```rust
// In RpcServer::start()
if let Some(ws_addr) = self.ws_addr {
    let ws_server = Server::builder()
        .ws_only()
        .max_connections(max_connections)
        .build(ws_addr)
        .await?;
    // Register same RPC module + subscription methods
    let ws_handle = ws_server.start(module.clone());
    // ...
}
```

### Phase 2: `eth_subscribe` / `eth_unsubscribe`

#### Subscription Types

Per the Ethereum JSON-RPC spec, `eth_subscribe` supports:

| Subscription | Params | Description |
|-------------|--------|-------------|
| `newHeads` | — | New block headers |
| `logs` | `{address?, topics?}` | New logs matching filter |
| `newPendingTransactions` | — | New pending tx hashes |

Start with `newHeads` and `logs`. `newPendingTransactions` can follow later.

#### Event Source: `FinalizedReporter`

The `FinalizedReporter` already processes every finalized block. It's the right
place to emit events that feed subscriptions:

```
Consensus → FinalizedReporter → block_index.insert_block()
                               → *** broadcast to subscribers ***
```

Add a `tokio::sync::broadcast` channel to `FinalizedReporter`:

```rust
pub struct FinalizedReporter<E, P> {
    // ... existing fields ...
    /// Broadcast channel for new finalized blocks.
    block_broadcast: Option<broadcast::Sender<SubscriptionEvent>>,
}
```

The `SubscriptionEvent` enum:

```rust
// In a new file: crates/node/rpc/src/subscription.rs
pub enum SubscriptionEvent {
    NewHead(RpcBlock),
    Log(RpcLog),
}
```

In `FinalizedReporter::report()`, after indexing a block:

```rust
if let Some(tx) = &self.block_broadcast {
    let _ = tx.send(SubscriptionEvent::NewHead(rpc_block.clone()));
    for log in &rpc_block_logs {
        let _ = tx.send(SubscriptionEvent::Log(log.clone()));
    }
}
```

#### Subscription RPC Methods

jsonrpsee 0.24 provides the `#[subscription]` proc macro:

```rust
#[rpc(server, namespace = "eth")]
pub trait EthSubscriptionApi {
    /// Subscribe to new block headers.
    #[subscription(name = "subscribe" => "subscription", unsubscribe = "unsubscribe", item = RpcBlock)]
    async fn subscribe(&self, kind: String, params: Option<serde_json::Value>) -> SubscriptionResult;
}
```

Implementation dispatches based on `kind`:

```rust
impl EthSubscriptionApiServer for EthSubscriptionApiImpl {
    fn subscribe(
        &self,
        pending: PendingSubscriptionSink,
        kind: String,
        params: Option<serde_json::Value>,
    ) -> SubscriptionResult {
        let sink = pending.accept().await?;
        let mut rx = self.block_broadcast.subscribe();

        match kind.as_str() {
            "newHeads" => {
                tokio::spawn(async move {
                    while let Ok(event) = rx.recv().await {
                        if let SubscriptionEvent::NewHead(block) = event {
                            if sink.send(SubscriptionMessage::from_json(&block)?).await.is_err() {
                                break;
                            }
                        }
                    }
                });
            }
            "logs" => {
                let filter = parse_log_filter(params);
                tokio::spawn(async move {
                    while let Ok(event) = rx.recv().await {
                        if let SubscriptionEvent::Log(log) = event {
                            if matches_filter(&log, &filter) {
                                if sink.send(SubscriptionMessage::from_json(&log)?).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
            _ => {
                return Err(ErrorObject::owned(-32602, "unsupported subscription type", None::<()>));
            }
        }
        Ok(())
    }
}
```

### Phase 3: `kora_subscribe`

Kora-specific subscriptions for consensus events.

#### Subscription Types

| Subscription | Params | Description |
|-------------|--------|-------------|
| `consensus` | — | Consensus activity (notarize, finalize, nullify) |
| `nodeStatus` | — | Periodic node status snapshots |

#### Event Source: `NodeStateReporter`

`NodeStateReporter` already receives all consensus activity. Extend it with a
broadcast channel:

```rust
pub struct NodeStateReporter<S> {
    state: NodeState,
    /// Broadcast for consensus events to WS subscribers.
    consensus_broadcast: Option<broadcast::Sender<ConsensusEvent>>,
    _scheme: PhantomData<S>,
}
```

```rust
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConsensusEvent {
    Notarization { view: u64 },
    Finalization { view: u64 },
    Nullification { view: u64 },
}
```

In `NodeStateReporter::report()`:

```rust
fn report(&mut self, activity: Self::Activity) -> impl Future<Output = ()> + Send {
    match &activity {
        Activity::Notarization(n) => {
            let view = n.proposal.round.view().get();
            self.state.set_view(view);
            if let Some(tx) = &self.consensus_broadcast {
                let _ = tx.send(ConsensusEvent::Notarization { view });
            }
        }
        Activity::Finalization(f) => {
            let view = f.proposal.round.view().get();
            self.state.set_view(view);
            self.state.inc_finalized();
            if let Some(tx) = &self.consensus_broadcast {
                let _ = tx.send(ConsensusEvent::Finalization { view });
            }
        }
        Activity::Nullification(n) => {
            self.state.inc_nullified();
            if let Some(tx) = &self.consensus_broadcast {
                let view = n.proof.round.view().get();
                let _ = tx.send(ConsensusEvent::Nullification { view });
            }
        }
        _ => {}
    }
    async {}
}
```

#### Kora Subscription RPC

```rust
#[rpc(server, namespace = "kora")]
pub trait KoraSubscriptionApi {
    #[subscription(name = "subscribe" => "subscription", unsubscribe = "unsubscribe", item = ConsensusEvent)]
    async fn subscribe(&self, kind: String) -> SubscriptionResult;
}
```

### Wiring

The broadcast channels need to be created in `runner.rs` and threaded to both
the reporters and the RPC server:

```
runner.rs:
  let (block_tx, _) = broadcast::channel::<SubscriptionEvent>(1024);
  let (consensus_tx, _) = broadcast::channel::<ConsensusEvent>(256);

  // Pass to reporters
  finalized_reporter.with_block_broadcast(block_tx.clone());
  node_state_reporter.with_consensus_broadcast(consensus_tx.clone());

  // Pass to RPC server
  rpc_server.with_subscriptions(block_tx, consensus_tx);
```

## Files to Change

| File | Change | Phase |
|------|--------|-------|
| `crates/node/rpc/Cargo.toml` | Possibly add WS feature flag | 1 |
| `crates/node/rpc/src/config.rs` | Add `ws_addr` to config (if separate port needed) | 1 |
| `crates/node/rpc/src/server.rs` | Enable WS transport, register subscription APIs | 1, 2, 3 |
| `crates/node/rpc/src/subscription.rs` | **New file**: `SubscriptionEvent`, `ConsensusEvent`, subscription API traits + impls | 2, 3 |
| `crates/node/rpc/src/lib.rs` | Export new `subscription` module | 2 |
| `crates/node/reporters/src/lib.rs` | Add broadcast channels to `FinalizedReporter` and `NodeStateReporter` | 2, 3 |
| `crates/node/runner/src/runner.rs` | Create broadcast channels, wire to reporters and RPC server | 2, 3 |

## Tests

### Phase 1
1. Start server, attempt WebSocket upgrade, verify 101 Switching Protocols
   (not 405).
2. Send a JSON-RPC request over WebSocket, verify response.

### Phase 2
3. Subscribe to `newHeads` over WS. Insert a block. Verify the subscription
   emits the block header.
4. Subscribe to `logs` with an address filter. Insert a block with matching
   and non-matching logs. Verify only matching logs are emitted.
5. Call `eth_subscribe` over HTTP. Verify proper error (subscriptions require
   WS).

### Phase 3
6. Subscribe to `consensus` over WS. Trigger a finalization event through
   `NodeStateReporter`. Verify the subscription emits a `Finalization` event.

## Verification

```bash
# Phase 1: WebSocket connects
websocat ws://$WS_HOST:$WS_PORT

# Phase 2: eth_subscribe
echo '{"jsonrpc":"2.0","method":"eth_subscribe","params":["newHeads"],"id":1}' | \
  websocat ws://$WS_HOST:$WS_PORT
# Should receive subscription ID, then block headers as they finalize

# Phase 3: kora_subscribe
echo '{"jsonrpc":"2.0","method":"kora_subscribe","params":["consensus"],"id":1}' | \
  websocat ws://$WS_HOST:$WS_PORT
# Should receive consensus events
```
