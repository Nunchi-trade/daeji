# RPC Enablement — What New Capabilities Unlock

## Current State (What Works Now)

The kora RPC layer exposes ~25 methods. All are **poll-based** — the client asks, the server answers.
No push. No streaming. No subscriptions.

**Available now:**
- Block queries (`eth_getBlockByNumber/Hash`)
- State reads (`eth_getBalance`, `eth_getCode`, `eth_getStorageAt`)
- Transaction lifecycle (`eth_sendRawTransaction`, `eth_getTransactionByHash`, `eth_getTransactionReceipt`)
- Execution simulation (`eth_call`, `eth_estimateGas`)
- Fee data (`eth_gasPrice`, `eth_feeHistory`)
- Logs (`eth_getLogs` with block range filters)

**What we can build with polling alone:**
- Block-by-block visualization (poll `eth_blockNumber` at 1s)
- Historical state exploration (read any past block)
- Account balance tracking over time
- Gas/fee waveforms from `eth_feeHistory`
- Log/event search across block ranges
- Transaction detail views

**Limitations of polling:**
- 1-second minimum latency (block time = poll interval)
- No sub-block visibility (can't see pending txns)
- No instant event notification (must poll `eth_getLogs` to detect new events)
- Client does redundant work checking "anything new?" every second
- Scales poorly — 100 browser tabs = 100x the RPC load

---

## Tier 1: `eth_subscribe` (Highest Impact)

### What it is
WebSocket push. Client subscribes once, server pushes events as they happen.
The jsonrpsee crate already supports this via `#[subscription]` — the transport is wired, the semantics aren't.

### Subscription types to implement

#### `newHeads` — New block headers
```json
{"jsonrpc":"2.0","method":"eth_subscribe","params":["newHeads"]}
```
**Pushes:** Full block header the instant it's finalized.
**Explorer unlock:** Zero-latency block arrival. The terrain tile / waterfall block / constellation pulse triggers the instant the block exists — not 0-1000ms later when a poll happens to fire. This is the difference between "live" and "laggy."

#### `logs` — Filtered event stream
```json
{"jsonrpc":"2.0","method":"eth_subscribe","params":["logs", {"address":"0x...", "topics":["0x..."]}]}
```
**Pushes:** Log entries matching the filter as blocks finalize.
**Explorer unlock:** Real-time event particle effects. Contract emits a Transfer → a light arc traces sender→receiver *instantly*. No polling delay. Can filter to specific contracts/events for focused visualizations.

#### `newPendingTransactions` — Mempool visibility
```json
{"jsonrpc":"2.0","method":"eth_subscribe","params":["newPendingTransactions"]}
```
**Pushes:** Transaction hash (or full tx) when it enters the mempool.
**Explorer unlock:** **This is the big one for visuals.** Right now, transactions appear fully formed in a block. With pending tx streaming, you see the *lifecycle*:

1. Transaction enters mempool → particle spawns at sender address, glowing, unresolved
2. Transaction sits pending → particle orbits, pulsing, waiting
3. Transaction included in block → particle *snaps* into the block, crystallizes
4. Transaction reverted → particle shatters, red flash

This gives the explorer **anticipation** — you see activity *before* it resolves. The chain feels alive between blocks, not just at block boundaries.

#### `syncing` — Sync status changes
```json
{"jsonrpc":"2.0","method":"eth_subscribe","params":["syncing"]}
```
**Pushes:** Sync progress updates.
**Explorer unlock:** Node health heartbeat. Not critical for visuals but completes the picture.

### Implementation in kora

The infrastructure exists. `jsonrpsee` subscription support, `tokio::broadcast` channels, and `LedgerEvent` notifications from block finalization are all in place. Rough shape:

```rust
// In eth.rs trait
#[subscription(name = "subscribe" unsubscribe = "unsubscribe", item = Value)]
async fn subscribe(&self, kind: String, params: Option<Value>) -> SubscriptionResult;

// In EthApiImpl — wire to broadcast channel
// LedgerService already emits events on finalization
// BlockIndex updates could trigger broadcasts
```

Estimated effort: medium. The hard part is routing finalization events to per-subscription broadcast channels with correct filtering (especially for `logs`).

---

## Tier 2: Filter API (Moderate Impact)

### What it is
Server-side filters for HTTP clients (no WebSocket required).

```
eth_newBlockFilter        → returns filter ID
eth_newPendingTransactionFilter
eth_newFilter             → custom log filter
eth_getFilterChanges      → poll for new matches since last call
eth_uninstallFilter       → cleanup
```

### Explorer unlock
Allows efficient polling without WebSocket. Client creates a filter, then polls `eth_getFilterChanges` to get only *new* items since last poll. Much cheaper than re-scanning block ranges with `eth_getLogs`.

**Useful for:** HTTP-only environments, server-side indexers, fallback when WS isn't available.

**Less exciting than subscriptions** — still polling, still has latency — but more compatible.

---

## Tier 3: Debug/Trace APIs (Deep Exploration)

### `debug_traceTransaction` / `debug_traceBlockByNumber`
**What it does:** Returns the full EVM execution trace — every opcode, every state change, every internal call.

**Explorer unlock:** *Transaction X-ray.* Click a transaction → see the internal call tree rendered as a branching visualization:
- Each internal CALL is a branch from the trunk
- SSTORE operations flash as state mutations
- REVERT paths render as broken/red branches
- Gas consumption shown as thickness — fat branches burned more gas
- Stack depth as vertical position

This turns opaque "tx succeeded" into a visible execution story. Especially powerful for complex DeFi interactions where one user action triggers 15 internal calls across 8 contracts.

### `debug_storageRangeAt`
**What it does:** Dumps a range of storage slots for a contract at a given block.

**Explorer unlock:** *Contract memory visualizer.* Render a contract's storage as a grid/heatmap where each slot is a cell. Color by recency of change, brightness by value magnitude. Watch storage evolve over time — you can literally see where a contract "thinks."

### `trace_filter` / `trace_block`
**What it does:** OpenEthereum-style traces — lighter weight than debug, focused on call trees and state diffs.

**Explorer unlock:** *Block anatomy.* Instead of "block has 5 transactions," see the full internal call graph for the entire block rendered as a circuit diagram. Who called whom, what changed, where value flowed.

### Implementation in kora
Requires REVM tracing hooks. REVM supports inspector-based tracing — you'd implement a custom `Inspector` that records operations during re-execution. More work than subscriptions but the REVM integration point exists.

---

## Tier 4: Extended State APIs (Completeness)

### `eth_getBlockReceipts`
**What it does:** Returns all receipts for a block in one call (vs N individual `eth_getTransactionReceipt` calls).

**Explorer unlock:** Batch efficiency. Render block detail views without N+1 queries. Also gives you gas usage distribution across all txns in one shot — good for the gas waveform visual.

### `eth_getProof`
**What it does:** Returns Merkle proofs for account state (balance, nonce, code hash, storage slots).

**Explorer unlock:** *Merkle tree visualization.* Render the actual proof path from state root to a specific value. Each proof node is a visual element — you see exactly how the trie branches to reach a specific account. Powerful educational tool.

### `eth_createAccessList`
**What it does:** Simulates a transaction and returns the set of addresses/storage keys it would access.

**Explorer unlock:** *Dependency graph.* Before a transaction executes, show exactly which accounts and storage slots it will touch. Render as a connection diagram — "this transaction will read from contracts A, B, C and write to D."

---

## Tier 5: Kora-Specific Methods (Unique Differentiator)

These don't exist in standard Ethereum and would be unique to kora.

### `kora_consensusState`
Expose BLS threshold consensus state — which validators signed, round progress, vote tallies.

**Explorer unlock:** *Consensus constellation.* Each validator is a node in a ring. As a round progresses, signed validators glow (rose). Threshold line is visible. When threshold crosses → block crystallizes. You literally watch consensus happen in real-time.

### `kora_mempoolSnapshot`
Expose the current mempool state — pending transactions, their priority, age.

**Explorer unlock:** *Mempool pressure gauge.* Render pending txns as particles in a chamber. Pressure (count) drives visual density. Priority drives vertical position. As blocks include txns, particles are "consumed" upward. Backlog = visual pressure building.

### `kora_stateMetrics`
Expose QMDB metrics — tree depth, page count, compaction state.

**Explorer unlock:** *Storage health.* Render the state database as a living structure — depth, density, hotspots. Operational insight unique to kora.

### `kora_subscribe` (extended subscriptions)
Push events that go beyond Ethereum standard:
- Consensus round events (pre-vote, pre-commit, finalize)
- Mempool events (tx added, tx evicted, tx promoted)
- Peer connection events
- State compaction events

**Explorer unlock:** Full real-time visibility into the node as a living system, not just the chain it produces.

---

## Priority Order for Explorer Impact

| Priority | Method | Visual Impact | Effort |
|----------|--------|---------------|--------|
| **P0** | `eth_subscribe("newHeads")` | Live block arrival, zero latency | Low-Medium |
| **P0** | `eth_subscribe("logs")` | Real-time event particles | Medium |
| **P1** | `eth_subscribe("newPendingTransactions")` | Mempool lifecycle animation | Medium |
| **P1** | `eth_getBlockReceipts` | Batch block detail rendering | Low |
| **P2** | `debug_traceTransaction` | Transaction X-ray / call tree | High |
| **P2** | `kora_consensusState` | Consensus visualization | Medium |
| **P3** | `eth_getProof` | Merkle tree visualization | Medium |
| **P3** | `debug_storageRangeAt` | Contract storage heatmap | High |
| **P3** | `kora_subscribe` (extended) | Full node lifecycle streaming | High |

The single highest-impact addition is **`eth_subscribe("newPendingTransactions")`** — it transforms the explorer from "a thing that shows you blocks after they happen" into "a thing that shows you the chain *living*."

---

## Implementation Details

### Adding WebSocket Support to jsonrpsee

The current `Cargo.toml` for `kora-rpc` uses `jsonrpsee = { version = "0.24", features = ["server", "macros"] }`. WebSocket support requires adding the `"ws"` feature flag — but jsonrpsee 0.24's `Server` already supports both HTTP and WS on the same port when built correctly. The key change is in server construction:

```rust
// crates/node/rpc/src/server.rs — current (HTTP-only)
let server = Server::builder()
    .max_connections(max_connections)
    .build(jsonrpc_addr)
    .await?;

// After: WS-capable (jsonrpsee handles upgrade automatically)
// No code change needed if using default Server::builder() —
// jsonrpsee 0.24 accepts WS upgrades by default on the same port.
// The missing piece is subscription method registration.
```

### Implementing `eth_subscribe` — Concrete Steps

#### Step 1: Add Subscription Trait

In `crates/node/rpc/src/eth.rs`, extend the `EthApi` trait:

```rust
#[rpc(server, namespace = "eth")]
pub trait EthApi {
    // ... existing methods ...

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

#### Step 2: Create Subscription Router

New file `crates/node/rpc/src/subscriptions.rs`:

```rust
use jsonrpsee::PendingSubscriptionSink;
use tokio::sync::broadcast;

/// Central hub that receives chain events and fans out to subscribers.
pub struct SubscriptionManager {
    /// New block headers — fed by LedgerEvent::SnapshotPersisted
    new_heads_tx: broadcast::Sender<serde_json::Value>,
    /// New logs — fed by indexing receipts on finalization
    logs_tx: broadcast::Sender<serde_json::Value>,
    /// Pending transactions — fed by LedgerEvent::TransactionSubmitted
    pending_tx_tx: broadcast::Sender<serde_json::Value>,
}

impl SubscriptionManager {
    pub fn new(capacity: usize) -> Self {
        let (new_heads_tx, _) = broadcast::channel(capacity);
        let (logs_tx, _) = broadcast::channel(capacity);
        let (pending_tx_tx, _) = broadcast::channel(capacity);
        Self { new_heads_tx, logs_tx, pending_tx_tx }
    }

    /// Subscribe a WS client to the appropriate stream
    pub async fn route(
        &self,
        sink: PendingSubscriptionSink,
        kind: &str,
        params: Option<serde_json::Value>,
    ) -> jsonrpsee::core::SubscriptionResult {
        match kind {
            "newHeads" => self.subscribe_new_heads(sink).await,
            "logs" => self.subscribe_logs(sink, params).await,
            "newPendingTransactions" => self.subscribe_pending_txs(sink).await,
            _ => {
                sink.reject(jsonrpsee::types::ErrorObject::owned(
                    -32602, "Unsupported subscription type", None::<()>,
                )).await;
                Ok(())
            }
        }
    }
}
```

#### Step 3: Wire LedgerEvents → SubscriptionManager

In the runner (`crates/node/runner/src/runner.rs`), spawn a bridge task:

```rust
fn spawn_subscription_bridge(
    ledger: &LedgerService,
    sub_mgr: Arc<SubscriptionManager>,
    block_index: Arc<BlockIndex>,
) {
    let mut events = ledger.subscribe();
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match event {
                LedgerEvent::SnapshotPersisted(digest) => {
                    // Look up the finalized block from BlockIndex
                    if let Some(block) = block_index.get_block_by_hash(&digest.into()) {
                        let header = serialize_block_header(&block);
                        let _ = sub_mgr.new_heads_tx.send(header);

                        // Extract logs for log subscribers
                        if let Some(logs) = block_index.get_logs_for_block(&digest.into()) {
                            for log in logs {
                                let _ = sub_mgr.logs_tx.send(serialize_log(&log));
                            }
                        }
                    }
                }
                LedgerEvent::TransactionSubmitted(tx_id) => {
                    let _ = sub_mgr.pending_tx_tx.send(
                        serde_json::Value::String(format!("0x{}", hex::encode(tx_id.0)))
                    );
                }
                _ => {}
            }
        }
    });
}
```

#### Step 4: Log Filter Matching for `logs` Subscriptions

Each `logs` subscriber provides an optional filter (address + topics). The SubscriptionManager must match incoming logs against per-subscriber filters:

```rust
struct LogSubscription {
    sink: SubscriptionSink,
    filter: Option<LogFilter>, // reuse existing LogFilter from indexer
}

// On each new log:
// 1. Check if log.address matches filter.address (if specified)
// 2. Check if log.topics match filter.topics (positional, with None = wildcard)
// 3. If match, send to this subscriber's sink
```

The `LogFilter` struct already exists in `crates/storage/indexer/src/filter.rs` and supports address + topic matching — reuse it directly.

### Implementing `kora_consensusState`

The `NodeState` struct (`crates/node/rpc/src/state.rs`) already tracks:
- `current_view: AtomicU64`
- `finalized_count: AtomicU64`
- `proposed_count: AtomicU64`
- `nullified_count: AtomicU64`

To expose real consensus round state (pre-vote, pre-commit, threshold), the Simplex engine needs to expose its internal round state. This requires changes in `crates/node/consensus/`:

```rust
// New struct to expose via RPC
pub struct ConsensusRoundState {
    pub height: u64,
    pub round: u32,
    pub step: ConsensusStep, // Propose | Prevote | Precommit | Finalize
    pub proposer_index: u32,
    pub votes_received: u32,
    pub votes_required: u32, // threshold (2f+1)
    pub validator_votes: Vec<ValidatorVote>, // who has voted
}

pub struct ValidatorVote {
    pub validator_index: u32,
    pub public_key: Vec<u8>, // ed25519 pubkey
    pub voted: bool,
    pub timestamp: Option<u64>,
}
```

**Challenge:** The Simplex engine from Commonware may not expose internal round state directly. Options:
1. Instrument the Simplex engine with an observer callback
2. Track votes externally in the marshal/networking layer (votes pass through `CHANNEL_VOTES`)
3. Add a state observer to the `ConsensusApplication` trait

Option 2 is most practical — intercept vote messages at the transport layer and maintain a shadow state.

### Implementing `debug_traceTransaction`

Requires REVM Inspector integration. REVM supports this via the `Inspector` trait:

```rust
// New file: crates/node/executor/src/tracer.rs
use revm::interpreter::{
    CallInputs, CallOutcome, CreateInputs, CreateOutcome,
    Interpreter, InterpreterResult,
};
use revm::Inspector;

pub struct CallTracer {
    pub call_stack: Vec<CallFrame>,
    pub current_depth: usize,
}

pub struct CallFrame {
    pub call_type: CallType, // CALL, STATICCALL, DELEGATECALL, CREATE, CREATE2
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub input: Bytes,
    pub output: Bytes,
    pub error: Option<String>,
    pub children: Vec<CallFrame>,
    pub storage_changes: Vec<StorageChange>,
    pub logs: Vec<Log>,
}

impl<DB: Database> Inspector<DB> for CallTracer {
    fn call(&mut self, context: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.current_depth += 1;
        self.call_stack.push(CallFrame::from_inputs(inputs));
        None // continue execution
    }

    fn call_end(&mut self, _context: &mut EvmContext<DB>, _inputs: &CallInputs, outcome: CallOutcome) -> CallOutcome {
        if let Some(frame) = self.call_stack.last_mut() {
            frame.gas_used = outcome.gas().spent();
            frame.output = outcome.output().clone();
        }
        self.current_depth -= 1;
        outcome
    }

    fn sstore(&mut self, _interp: &mut Interpreter, _context: &mut EvmContext<DB>, address: Address, slot: U256, new_value: U256) {
        if let Some(frame) = self.call_stack.last_mut() {
            frame.storage_changes.push(StorageChange { address, slot, new_value });
        }
    }
}
```

**Integration point:** Add a new method to `RevmExecutor` that re-executes a transaction with the `CallTracer` inspector attached. The block context must be reconstructed from the original block.

### Implementing `eth_getBlockReceipts`

Simplest addition — BlockIndex already stores all receipts. Add to `EthApi` trait:

```rust
#[method(name = "getBlockReceipts")]
async fn get_block_receipts(
    &self,
    block_id: BlockNumberOrTag,
) -> RpcResult<Option<Vec<RpcTransactionReceipt>>>;
```

Implementation: resolve block number → get all tx hashes for block → batch-fetch receipts from `block_index.receipts`. This is a ~20-line implementation.

### Implementation Sequence

```
Phase 0 (foundation):
  ├── Add `eth_getBlockReceipts` to EthApi         [1-2 hours]
  └── Verify WS upgrade works with current server   [1 hour]

Phase 1 (subscriptions):
  ├── Create SubscriptionManager                    [4-6 hours]
  ├── Add `eth_subscribe` trait method              [1 hour]
  ├── Wire LedgerEvents → SubscriptionManager      [2-3 hours]
  ├── Implement newHeads subscription               [2 hours]
  ├── Implement logs subscription (with filtering)  [3-4 hours]
  └── Implement newPendingTransactions              [1-2 hours]

Phase 2 (kora-specific):
  ├── Design ConsensusRoundState struct             [2 hours]
  ├── Instrument vote observation                   [4-6 hours]
  ├── Add kora_consensusState RPC method           [2 hours]
  └── Add kora_subscribe for consensus events      [3-4 hours]

Phase 3 (debug/trace):
  ├── Implement CallTracer Inspector               [4-6 hours]
  ├── Add debug_traceTransaction                    [3-4 hours]
  └── Add debug_traceBlockByNumber                  [2 hours]
```

---

## Consensus Subscriptions — kora_subscribe

The `kora_subscribe` method extends the standard `eth_subscribe` pattern with Kora-specific consensus event streaming. See [08-consensus-data.md](./08-consensus-data.md) for the full specification.

### Subscription Types

| Type | Events Pushed | Use Case |
|------|--------------|----------|
| `"consensus"` | All 10 Activity types | Consensus Ring scene (Scene 4) — full lifecycle |
| `"consensus.rounds"` | Notarization, Finalization, Nullification only | Status panel — round outcomes without vote noise |
| `"consensus.votes"` | Notarize, Certification, Finalize, Nullify | Vote-level detail for debugging |
| `"consensus.safety"` | ConflictingNotarize, ConflictingFinalize, NullifyFinalize | Safety monitoring — triggers alert overlay |
| `"mempool"` | txAdded, txIncluded, txEvicted | Particle Constellation pending tx lifecycle |

### Wire Format

```json
{"jsonrpc":"2.0","method":"kora_subscription","params":{"subscription":"0x1","result":{"type":"finalization","view":100,"payload":"0xabcd...","seed":"0x5678...","blockHeight":286401,"timestampMs":1715200000450}}}
```

### Implementation

The `ConsensusReporter` (a new `Reporter` impl) intercepts all Simplex Activity events and broadcasts them via `tokio::sync::broadcast`. It composes into the existing reporter chain via `Reporters::from()` — zero impact on SeedReporter, FinalizedReporter, or NodeStateReporter.

See [08-consensus-data.md](./08-consensus-data.md) for:
- `ConsensusEvent` enum (the wire format)
- `ConsensusReporter` implementation
- `ConsensusState` snapshot endpoint
- Wiring into `runner.rs`
- Event volume estimates (~5 events/sec at 1s block time)
