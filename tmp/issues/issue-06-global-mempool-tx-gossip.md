# Design and Implement Transaction Gossip Protocol

**Priority:** P2 (feature -- not blocking devnet, but required for production)

## Summary

Kora validators currently maintain completely isolated, local-only transaction pools. A transaction submitted via RPC to validator A exists solely in A's mempool and is invisible to validators B, C, and D. This means the current block proposer can only include transactions from its own pool, users must know which validator will propose next for optimal inclusion latency, and any validator crash permanently destroys all unincluded transactions in its pool. This issue proposes a phased transaction gossip protocol that propagates pending transactions across the validator set.

## Problem Description

### Transactions are validator-local

Each Kora validator runs its own `TransactionPool` instance (`crates/node/txpool/src/pool.rs`). When a user calls `eth_sendRawTransaction` on a validator's RPC endpoint, the transaction flows through:

1. **RPC layer** (`crates/node/rpc/src/eth.rs`) -- decodes and forwards to the `TxSubmitCallback`
2. **TxSubmitCallback** (`crates/node/runner/src/runner.rs:579-606`) -- creates a `TransactionValidator`, validates the transaction, then calls `ledger.submit_tx(tx)`
3. **LedgerService.submit_tx()** (`crates/node/ledger/src/lib.rs:494`) -- delegates to `LedgerView.submit_tx()` which calls `inner.mempool.insert(tx)`
4. **TransactionPool.insert()** (`crates/node/txpool/src/pool.rs:573`) -- decodes envelope, creates `OrderedTransaction`, calls `pool.add(ordered)` which inserts into `by_hash`, `by_id`, `by_sender` maps

At no point in this pipeline is the transaction forwarded to any peer. The `TxSubmitCallback` validates and inserts locally, then returns. There is no gossip step.

### Empirical evidence

Load testing on the devnet (4 validators, Simplex BFT, threshold=3) confirmed this behavior:

**Single-validator submission** -- 10 transactions sent to port 8545 only:
```
Port 8545: pending=10, queued=0   <-- has transactions
Port 8546: pending=0,  queued=0   <-- empty
Port 8547: pending=0,  queued=0   <-- empty
Port 8548: pending=0,  queued=0   <-- empty
```

### The loadgen workaround

The load generator (`bin/loadgen/src/main.rs`) works around this limitation by accepting a `--broadcast-rpc-urls` flag that duplicates each `eth_sendRawTransaction` call to every validator's RPC endpoint. This is an external, application-layer workaround -- not a protocol-level solution.

### No existing P2P transaction channel

The Commonware P2P transport (`crates/network/transport/src/channels.rs`) defines five channel IDs:

| Channel ID | Purpose             | Consumer    |
|-----------|---------------------|-------------|
| 0         | Votes               | Simplex     |
| 1         | Certificates        | Simplex     |
| 2         | Resolver            | Simplex     |
| 3         | Block broadcast     | Marshal     |
| 4         | Backfill            | Marshal     |

There is no channel for transaction gossip. The `NetworkTransport` struct (`crates/network/transport/src/transport.rs:23`) bundles only `SimplexChannels` and `MarshalChannels` -- no transaction dissemination channels exist.

## Root Cause Analysis

### 1. Proposer-blind transactions

With Simplex BFT's threshold VRF leader election, the proposer for any given view is determined pseudorandomly. In a 4-validator network, a transaction submitted to a single validator has approximately a 25% chance of being proposed in any given round (only if that validator happens to be the leader). If a different validator is elected leader, the transaction sits idle in the non-leader's pool until that validator eventually gets its turn.

The block proposal path (`crates/node/runner/src/app.rs`) calls `mempool.build(self.max_txs, &excluded)` which only queries the local `TransactionPool`. The proposer has no mechanism to pull transactions from peer pools.

### 2. Permanent transaction loss on crash

When a validator crashes, its in-memory `TransactionPool` is destroyed. The pool is backed by a `HashMap` in memory (`crates/node/txpool/src/pool.rs:63-65`), with no on-disk persistence. Any transactions that existed only in that validator's pool are permanently lost.

### 3. Users must route to the right validator

Without gossip, users (or their wallets/SDKs) must either:
- Submit transactions to all validators (duplicating RPC calls)
- Use a load balancer that fans out to all validators
- Accept the ~25% per-round inclusion probability by submitting to a single validator

None of these are acceptable for a production chain. Standard Ethereum clients expect to submit a transaction to any node and have it propagate through the network.

## Impact

| Area | Impact |
|------|--------|
| **Transaction inclusion latency** | Up to 4x worse than optimal -- transactions wait for the submitting validator's leader turn |
| **Fault tolerance** | A validator crash permanently destroys its pending transactions |
| **User experience** | Users must know the network topology or broadcast to multiple RPCs |
| **Block utilization** | Proposers produce empty blocks while peers hold pending transactions |
| **Production readiness** | No EVM-compatible chain ships without transaction gossip |

## Design Constraints from Audit

### CRITICAL: O(N^2) message amplification

If the gossip actor naively re-broadcasts every transaction it receives (including those received via gossip from other peers), the network will experience O(N^2) message amplification. In a 4-node cluster this means 4x redundant messages; at 20 nodes it becomes 400x. The gossip actor MUST distinguish between locally-originated transactions (received via RPC) and remotely-gossiped transactions. Only locally-originated transactions should be broadcast. Remotely-received transactions are inserted into the local pool but never re-gossiped.

### No `pool.get_raw()` method exists

The `TransactionPool` API (as of this writing) provides these relevant methods:
- `pool.add(ordered: OrderedTransaction) -> Result<(), TxPoolError>` -- insert a validated, ordered transaction
- `pool.get(hash: &B256) -> Option<OrderedTransaction>` -- retrieve by hash (returns decoded `OrderedTransaction`, not raw bytes)
- `pool.contains(hash: &B256) -> bool` -- check existence
- `pool.pending(max_txs: usize) -> Vec<OrderedTransaction>` -- get sorted pending transactions
- `pool.remove(hash: &B256) -> Option<OrderedTransaction>` -- remove by hash
- `pool.remove_confirmed(sender: &Address, confirmed_nonce: u64)` -- prune confirmed transactions
- `pool.has_nonce(sender: &Address, nonce: u64) -> bool` -- check for existing nonce
- `pool.insert(tx: Tx) -> bool` -- insert from raw `Tx` (via `Mempool` trait)

There is no `get_raw(&hash)` method. The gossip design must work with raw transaction bytes that are already available at the point of gossip (either from the RPC submission path or from the gossip message itself), rather than retrieving raw bytes back from the pool.

### LedgerMempool does not support event subscriptions

The `LedgerMempool` wrapper (`crates/node/ledger/src/lib.rs:34`) delegates to `TransactionPool` but does not expose event subscription. The `TransactionPool` itself supports `MempoolEvent` broadcasting via `tokio::sync::broadcast` when constructed with `new_with_events()`, but the `LedgerMempool` always creates the pool with `TransactionPool::new()` (no events). The gossip trigger mechanism must account for this -- either by:
1. Modifying `LedgerMempool::new()` to accept and pass through a `broadcast::Sender<MempoolEvent>`, or
2. Hooking directly into the RPC submission path (the `TxSubmitCallback`) to capture raw bytes before they enter the ledger, or
3. Using a dedicated gossip channel that the `TxSubmitCallback` writes to directly.

Option 2 or 3 is preferred for Phase 1 because it naturally avoids the re-gossip amplification problem: only RPC-originated transactions trigger gossip.

## Proposed Solution

### Architecture Overview

```
                     +---- RPC tx_submit callback ---+
                     |    (gossip trigger point)      |
                     v                                |
  User RPC    +-------------+    gossip msg     +-------------+
  --------->  |  Validator A |  ------------>   |  Validator B |
  eth_send    |  TxPool     |                   |  TxPool     |
  RawTx       +-------------+                   +-------------+
                     |                                ^
                     | gossip msg                     | gossip msg
                     v                                |
              +-------------+                   +-------------+
              |  Validator C |                   |  Validator D |
              |  TxPool     |                   |  TxPool     |
              +-------------+                   +-------------+

  1. User submits tx to validator A via RPC
  2. TxSubmitCallback validates and inserts into local TxPool
  3. TxSubmitCallback also sends raw tx bytes to gossip actor
  4. Gossip actor broadcasts to all peers via P2P channel
  5. B, C, D receive gossip message
  6. B, C, D validate independently and insert into their own TxPools
  7. B, C, D do NOT re-broadcast (prevents O(N^2) amplification)
  8. Whichever validator is elected proposer can include the tx
```

Key design decision: gossip is triggered at the RPC ingress point, not by subscribing to pool events. This naturally solves two problems:
1. The raw transaction bytes are available (no need for a `get_raw()` method)
2. Only locally-submitted transactions are gossiped (no re-broadcast amplification)

## Implementation Plan

### Phase 1: Devnet Gossip (MVP)

**Goal:** Allow transactions submitted to any validator to propagate to all validators. Independently useful for devnet testing -- eliminates the need for the loadgen `--broadcast-rpc-urls` workaround.

**Files to create:**
- `crates/node/txpool/src/gossip.rs` -- `TxGossipMessage`, `SeenTxFilter`, `TxGossipActor`, `GossipConfig`

**Files to modify:**
- `crates/node/txpool/src/lib.rs` -- Export gossip module
- `crates/network/transport/src/channels.rs` -- Add `CHANNEL_TX_GOSSIP = 5`
- `crates/network/transport/src/transport.rs` -- Add optional `gossip` channel field to `NetworkTransport`
- `crates/node/runner/src/runner.rs` -- Wire gossip into `TxSubmitCallback` and spawn `TxGossipActor`

#### Message Format

A single message type for the gossip channel:

```rust
// crates/node/txpool/src/gossip.rs

/// A gossip message containing a single raw signed transaction.
///
/// The receiver is expected to independently validate the transaction
/// (signature, nonce, balance, gas) before inserting it into their local pool.
/// Invalid or duplicate transactions are silently dropped.
#[derive(Clone, Debug)]
pub struct TxGossipMessage {
    /// Keccak256 hash of the raw transaction bytes, used for deduplication.
    pub tx_hash: B256,
    /// EIP-2718 encoded signed transaction envelope.
    pub raw_tx: Bytes,
}
```

#### Deduplication (Seen Set)

Receivers maintain a bounded LRU set of recently-seen transaction hashes:

```rust
/// Tracks recently-seen transaction hashes for gossip deduplication.
pub struct SeenTxFilter {
    seen: IndexSet<B256>,
    max_size: usize,
}

impl SeenTxFilter {
    pub fn new(max_size: usize) -> Self { ... }

    /// Returns `true` if this hash has already been seen.
    /// If not seen, records it and returns `false`.
    pub fn check_and_insert(&mut self, hash: B256) -> bool { ... }
}
```

The seen filter serves double duty:
1. On the **receiving** side, it prevents redundant validation of transactions already in the pool.
2. On the **sending** side (if a transaction is submitted to multiple validators via RPC), it prevents broadcasting a transaction that was already received via gossip.

#### Gossip Actor

The gossip actor runs as a long-lived task alongside the consensus engine. It has two input sources:

```rust
pub struct TxGossipActor {
    config: GossipConfig,
    seen: SeenTxFilter,
}

impl TxGossipActor {
    /// Main gossip loop.
    ///
    /// - `gossip_sender` / `gossip_receiver`: P2P channel pair for tx gossip
    /// - `local_tx_rx`: channel receiving raw bytes of locally-submitted transactions
    ///   from the TxSubmitCallback (NOT from pool events)
    pub async fn run(
        &mut self,
        gossip_sender: Sender,
        gossip_receiver: Receiver,
        local_tx_rx: mpsc::Receiver<(B256, Bytes)>,
        pool: TransactionPool,
        validator: TransactionValidator<S>,
    ) {
        loop {
            select! {
                // A new transaction was submitted via THIS node's RPC.
                // Broadcast it to peers. Do NOT re-gossip received txs.
                Some((tx_hash, raw_tx)) = local_tx_rx.recv() => {
                    if self.seen.check_and_insert(tx_hash) {
                        continue; // already seen (e.g., received via gossip first)
                    }
                    let msg = TxGossipMessage { tx_hash, raw_tx };
                    gossip_sender.broadcast(msg.encode()).await;
                }

                // A peer is sharing a transaction via gossip.
                // Validate, deduplicate, insert into local pool.
                // Do NOT re-broadcast.
                (peer, bytes) = gossip_receiver.recv() => {
                    let msg = TxGossipMessage::decode(bytes);
                    if self.seen.check_and_insert(msg.tx_hash) {
                        continue; // already in pool or already processed
                    }
                    // Validate exactly like the RPC path does
                    let tx = Tx::new(msg.raw_tx);
                    match validator.validate(tx.clone()).await {
                        Ok(validated) => {
                            let ordered = validated.into_ordered(timestamp());
                            match pool.add(ordered) {
                                Ok(()) => { /* accepted */ }
                                Err(TxPoolError::AlreadyExists) => { /* fine */ }
                                Err(e) => {
                                    trace!(?peer, error = %e, "gossip tx rejected");
                                }
                            }
                        }
                        Err(e) => {
                            trace!(?peer, error = %e, "gossip tx invalid");
                        }
                    }
                }
            }
        }
    }
}
```

#### Wiring Into the RPC Submission Path

The gossip trigger is wired into the `TxSubmitCallback` closure in `runner.rs`, not into pool events. This is the key design decision that prevents re-broadcast amplification:

```rust
// crates/node/runner/src/runner.rs (modified TxSubmitCallback)

// Create a channel for the gossip actor to receive locally-submitted txs
let (gossip_tx, gossip_rx) = tokio::sync::mpsc::channel::<(B256, Bytes)>(4096);

let tx_submit: kora_rpc::TxSubmitCallback = Arc::new(move |data: Bytes| {
    let ledger = tx_ledger.clone();
    let state = tx_state.clone();
    let pool = tx_pool.clone();
    let gossip_tx = gossip_tx.clone();
    Box::pin(async move {
        let tx = Tx::new(data.clone());
        let tx_id = tx.id();
        let tx_hash = alloy_primitives::keccak256(&data);

        // Validate
        let validator = TransactionValidator::new(chain_id, state, PoolConfig::default())
            .with_pool(pool);
        validator.validate(tx.clone()).await.map_err(|err| {
            kora_rpc::RpcError::InvalidTransaction(err.to_string())
        })?;

        // Insert into local mempool
        if ledger.submit_tx(tx).await {
            // Notify gossip actor (best-effort, non-blocking)
            let _ = gossip_tx.try_send((tx_hash, data));
            Ok(())
        } else {
            Err(kora_rpc::RpcError::InvalidTransaction(
                "transaction rejected by mempool".to_string(),
            ))
        }
    })
});
```

#### GossipConfig (Phase 1 -- minimal)

```rust
/// Configuration for the transaction gossip protocol.
#[derive(Debug, Clone)]
pub struct GossipConfig {
    /// Whether transaction gossip is active.
    /// Default: false (backward compatible).
    pub enabled: bool,

    /// Maximum raw transaction size accepted from gossip messages (bytes).
    /// Should match PoolConfig::max_tx_size.
    /// Default: 131072 (128 KB).
    pub max_tx_size: usize,

    /// Maximum entries in the deduplication filter.
    /// At 32 bytes per hash, 100K entries = ~3.2 MB.
    /// Default: 100_000.
    pub seen_filter_size: usize,
}
```

#### Phase 1 Deliverable

After Phase 1, the devnet can be configured with `gossip.enabled = true` and:
- Transactions submitted to any validator propagate to all validators
- The loadgen `--broadcast-rpc-urls` workaround is no longer necessary
- Validator crashes preserve transactions in other validators' pools
- No O(N^2) message amplification

### Phase 2: Production Hardening

**Goal:** Add rate limiting, metrics, and configuration surface for production deployments.

**Files to modify:**
- `crates/node/txpool/src/gossip.rs` -- Add rate limiting, metrics
- `crates/node/config/src/node.rs` -- Add `gossip: GossipConfig` to `NodeConfig`
- `bin/kora/src/cli.rs` -- Add `--gossip-enabled` CLI flag
- `docker/compose/devnet.yaml` -- Add gossip configuration

#### Rate Limiting

Add per-peer rate limiting to prevent a single peer from flooding the gossip channel:

```rust
pub struct GossipConfig {
    // ... Phase 1 fields ...

    /// Maximum gossip messages per second per peer.
    /// Default: 1000.
    pub max_inbound_rate_per_peer: u32,

    /// Maximum outbound gossip buffer size.
    /// Default: 4096.
    pub outbound_buffer_size: usize,

    /// P2P channel ID for gossip. Must not conflict with 0-4.
    /// Default: 5.
    pub channel_id: u64,
}
```

#### Metrics

Export counters via the Commonware runtime metrics system:
- `kora_gossip_tx_sent_total` -- transactions broadcast to peers
- `kora_gossip_tx_received_total` -- gossip messages received from peers
- `kora_gossip_tx_accepted_total` -- gossiped transactions accepted into local pool
- `kora_gossip_tx_rejected_total` -- gossiped transactions rejected (invalid)
- `kora_gossip_tx_deduplicated_total` -- transactions dropped as duplicates by seen filter
- `kora_gossip_tx_rate_limited_total` -- messages dropped by per-peer rate limiter
- `kora_gossip_seen_filter_size` -- current size of the deduplication filter (gauge)

#### Configuration Integration

Add `GossipConfig` to `NodeConfig` with `#[serde(default)]` for backward compatibility:

```rust
// crates/node/config/src/node.rs
pub struct NodeConfig {
    pub chain_id: u64,
    pub data_dir: PathBuf,
    pub consensus: ConsensusConfig,
    pub network: NetworkConfig,
    pub execution: ExecutionConfig,
    pub rpc: RpcConfig,
    #[serde(default)]
    pub gossip: GossipConfig,  // <-- new
}
```

Add CLI override: `kora validator --gossip-enabled`.

#### Phase 2 Deliverable

After Phase 2, gossip is production-ready with:
- Rate limiting prevents abuse
- Metrics enable monitoring and alerting
- First-class configuration (config file, CLI, Docker env)

### Phase 3: Pool Coordination (Optional)

**Goal:** Improve pool consistency across validators after finalization.

This phase is optional and can be deferred. The core gossip protocol (Phases 1-2) is independently useful without these refinements.

**Files to modify:**
- `crates/node/reporters/src/lib.rs` -- Enhance finalization-triggered pruning

#### Nonce-aware cross-validator pruning

When a block is finalized, every validator should prune stale transactions. The existing `prune()` method removes transactions by `TxId` hash, but with gossip, validator B might hold a different nonce-5 transaction than what validator A proposed. The fix is to also prune by `(sender, confirmed_nonce)`.

The pool already has `remove_confirmed(sender, confirmed_nonce)` at `crates/node/txpool/src/pool.rs:388-417`. The finalization reporter should call this for every sender in the finalized block:

```rust
// In the finalization reporter, after prune():
for tx_bytes in &block.txs {
    if let Some((sender, nonce)) = decode_sender_and_nonce(tx_bytes) {
        pool.remove_confirmed(&sender, nonce);
    }
}
```

#### Gossip-aware pool size guidance

With gossip enabled, the pool receives transactions from all validators' RPCs. The default `max_pending_txs` (4096) and `max_queued_txs` (1024) may need to be increased. This is a documentation/guidance item -- the existing `PoolConfig` fields are sufficient, operators just need to know to adjust them when gossip is enabled.

## Testing Plan

### Unit Tests

- **`TxGossipMessage` codec roundtrip:** Encode, decode, verify fields match. Test boundary conditions (empty tx, max size, oversized).
- **`SeenTxFilter` behavior:** Verify `check_and_insert` returns `false` on first insert and `true` on re-insert. Verify FIFO eviction at `max_size`. Edge cases: capacity 0 and 1.
- **No re-broadcast:** Submit a `TxGossipMessage` via the gossip receiver. Verify the actor inserts into the local pool but does NOT broadcast anything on the gossip sender.
- **Local-to-gossip:** Submit a transaction via `local_tx_rx`. Verify the actor broadcasts a `TxGossipMessage` on the gossip sender.
- **Deduplication:** Feed the same hash twice (once via `local_tx_rx`, once via gossip receiver). Verify only one pool insertion.
- **Invalid transaction rejection:** Feed a `TxGossipMessage` with invalid signature. Verify rejection and no pool insertion.
- **`GossipConfig` defaults:** Verify `GossipConfig::default().enabled == false`.

### Integration Tests (e2e)

- **Cross-validator propagation:** Start 4 validators with gossip enabled. Submit 100 transactions to validator 0's RPC. Verify all 4 validators have the transactions in their pools (query `txpool_status` on all RPCs). Wait for finalization; verify all are included.
- **No amplification:** Start 4 validators with gossip. Submit 1 transaction to validator 0. Monitor P2P traffic: validator 0 should send exactly 3 gossip messages (one to each peer). Validators 1, 2, 3 should send 0 gossip messages for this transaction.
- **Mixed gossip/no-gossip:** Start 4 validators, 2 with gossip and 2 without. Submit transactions to a gossip-enabled validator. Verify the other gossip-enabled validator receives them. Verify the non-gossip validators do not.
- **Gossip after crash recovery:** Start 4 validators with gossip. Submit 100 transactions to validator 0. Kill validator 0. Verify the other 3 still have the transactions. Verify inclusion without re-submission.
- **Backward compatibility:** Start 4 validators with gossip disabled (default). Run existing load test suite. Verify behavior is identical to pre-gossip.

### Load Tests

- **Before/after comparison:** Run loadgen (1000 txs, 10 accounts) with gossip disabled + broadcast mode, then with gossip enabled + single-validator submission. Compare inclusion rate and latency.
- **Gossip bandwidth:** Measure P2P bandwidth under 10K tx load. Each tx is ~128 bytes on the wire + 37 bytes gossip envelope. 10K txs total ~ 1.6 MB.
- **Memory under gossip:** Monitor per-validator memory with gossip at 10K transactions. Verify `SeenTxFilter` stays bounded.

## Existing API Reference

These are the actual methods on `TransactionPool` that the implementation will use:

| Method | Signature | Notes |
|--------|-----------|-------|
| `add` | `pub fn add(&self, tx: OrderedTransaction) -> Result<(), TxPoolError>` | Primary insertion path for validated transactions |
| `get` | `pub fn get(&self, hash: &B256) -> Option<OrderedTransaction>` | Returns decoded OrderedTransaction, not raw bytes |
| `contains` | `pub fn contains(&self, hash: &B256) -> bool` | Cheap existence check |
| `has_nonce` | `pub fn has_nonce(&self, sender: &Address, nonce: u64) -> bool` | Same-nonce duplicate check |
| `remove` | `pub fn remove(&self, hash: &B256) -> Option<OrderedTransaction>` | Remove by hash |
| `remove_confirmed` | `pub fn remove_confirmed(&self, sender: &Address, confirmed_nonce: u64)` | Nonce-based pruning |
| `pending` | `pub fn pending(&self, max_txs: usize) -> Vec<OrderedTransaction>` | Sorted pending txs |
| `insert` | `fn insert(&self, tx: Tx) -> bool` | Via Mempool trait, decodes raw bytes internally |

The `TransactionValidator` API:
| Method | Signature | Notes |
|--------|-----------|-------|
| `new` | `pub const fn new(chain_id: u64, state: S, config: PoolConfig) -> Self` | Creates validator |
| `with_pool` | `pub fn with_pool(mut self, pool: TransactionPool) -> Self` | Attach pool for nonce-dup check |
| `validate` | `pub async fn validate(&self, tx: Tx) -> Result<ValidatedTransaction, TxPoolError>` | Full validation |

The `ValidatedTransaction` has `into_ordered(timestamp: u64) -> OrderedTransaction` for pool insertion.

## File References

- `crates/node/txpool/src/pool.rs` -- `TransactionPool` (line 131), `PoolInner` (line 63), `Mempool` impl (line 572)
- `crates/node/txpool/src/config.rs` -- `PoolConfig` struct (line 5)
- `crates/node/txpool/src/traits.rs` -- `Mempool` trait (line 10)
- `crates/node/txpool/src/validator.rs` -- `TransactionValidator` (line 45), `ValidatedTransaction` (line 24)
- `crates/node/txpool/src/ordering.rs` -- `OrderedTransaction` (line 10), `SenderQueue` (line 71)
- `crates/node/txpool/src/error.rs` -- `TxPoolError` enum (line 8)
- `crates/node/runner/src/runner.rs` -- `ProductionRunner::run()` (line 492), `TxSubmitCallback` closure (line 579)
- `crates/node/runner/src/app.rs` -- `RevmApplication::build_block()` uses `mempool.build()`
- `crates/node/ledger/src/lib.rs` -- `LedgerMempool` (line 34, wraps `TransactionPool` without events), `LedgerService::submit_tx()` (line 494)
- `crates/node/rpc/src/eth.rs` -- `TxSubmitCallback` type alias (line 238), `send_raw_transaction` (line 462)
- `crates/node/reporters/src/lib.rs` -- `FinalizedReporter` struct (line 856), mempool pruning via `state.prune_mempool()` (line 154 inside `handle_finalized_update`)
- `crates/network/transport/src/channels.rs` -- P2P channel definitions (lines 10-22), channels 0-4 in use
- `crates/network/transport/src/transport.rs` -- `NetworkTransport` struct (line 23), only `SimplexChannels` + `MarshalChannels`
- `crates/node/config/src/node.rs` -- `NodeConfig` (line 17), currently has no gossip field
- `crates/node/domain/src/events.rs` -- `MempoolEvent` enum (line 27), `LedgerEvents` (line 63)
