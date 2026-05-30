# P2P: No transaction gossip protocol between validators

## Summary

Kora has no mechanism for validators to share pending transactions with each other over the P2P network. Each validator's mempool is entirely local: it only contains transactions that were submitted directly to that specific validator's JSON-RPC endpoint. There is no transaction relay, forwarding, or gossip between validators.

This means that if a user submits a transaction to validator B, but validator A is the current leader, the transaction will not be included until validator B becomes the leader -- which could be multiple rounds away. If validator B crashes before it becomes leader, the transaction is lost entirely.

**Severity:** High

## Background

### How Kora's P2P Network Works

Kora is an EVM-compatible blockchain built on [Commonware](https://github.com/commonwarexyz/monorepo), which provides authenticated peer discovery and multiplexed communication channels. The P2P layer uses Commonware's `authenticated::discovery` protocol, where validators connect to each other using Ed25519 identity keys.

The network currently uses **5 multiplexed channels**, defined in `crates/network/transport/src/channels.rs`:

```rust
/// Channel ID for vote messages.
pub const CHANNEL_VOTES: u64 = 0;

/// Channel ID for certificate messages.
pub const CHANNEL_CERTS: u64 = 1;

/// Channel ID for resolver messages.
pub const CHANNEL_RESOLVER: u64 = 2;

/// Channel ID for block broadcast messages.
pub const CHANNEL_BLOCKS: u64 = 3;

/// Channel ID for backfill messages.
pub const CHANNEL_BACKFILL: u64 = 4;
```

These channels are grouped into two bundles (defined in `crates/network/transport/src/channels.rs`, lines 33-65):
- **SimplexChannels**: votes, certs, resolver -- used by the Simplex consensus engine
- **MarshalChannels**: blocks, backfill -- used by the marshal block dissemination layer

All 5 channels are registered in the transport builder (`crates/network/transport/src/builder.rs`, lines 80-87):

```rust
// Register simplex channels
let votes = network.register(CHANNEL_VOTES, quota, backlog);
let certs = network.register(CHANNEL_CERTS, quota, backlog);
let resolver = network.register(CHANNEL_RESOLVER, quota, backlog);

// Register marshal channels
let blocks = network.register(CHANNEL_BLOCKS, quota, backlog);
let backfill = network.register(CHANNEL_BACKFILL, quota, backlog);
```

Each channel is rate-limited to **1000 messages per second** (defined by `default_quota()` at `builder.rs` line 22-24) with a **backlog of 256 messages** (`DEFAULT_BACKLOG` at `config.rs` line 15) and a **max message size of 1 MB** (`DEFAULT_MAX_MESSAGE_SIZE` at `config.rs` line 12).

There is **no channel for transaction gossip**. All 5 channels are dedicated to consensus and block dissemination.

### How Transactions Currently Enter the System

Transactions are submitted via the `eth_sendRawTransaction` JSON-RPC method. The RPC handler (`crates/node/rpc/src/eth.rs`, line 299-308) calls a `TxSubmitCallback`, which is wired up in the runner. The callback is defined in `crates/node/runner/src/runner.rs`, lines 406-431:

```rust
let tx_submit: kora_rpc::TxSubmitCallback = Arc::new(move |data| {
    let ledger = tx_ledger.clone();
    let state = tx_state.clone();
    Box::pin(async move {
        let tx = Tx::new(data);
        let tx_id = tx.id();
        let validator =
            TransactionValidator::new(chain_id, state, PoolConfig::default());
        validator.validate(tx.clone()).await.map_err(|err| {
            warn!(?tx_id, error = %err, "rpc submit: validator rejected tx");
            kora_rpc::RpcError::InvalidTransaction(err.to_string())
        })?;
        if ledger.submit_tx(tx).await {
            debug!(?tx_id, "rpc submit: tx inserted into mempool");
            Ok(())
        } else {
            warn!(
                ?tx_id,
                "rpc submit: ledger.submit_tx returned false (duplicate or pool error)"
            );
            Err(kora_rpc::RpcError::InvalidTransaction(
                "transaction rejected by mempool".to_string(),
            ))
        }
    })
});
```

The path is:

1. User sends raw transaction bytes to a validator's RPC endpoint
2. `TransactionValidator` checks signature, chain ID, nonce, gas limits
3. `ledger.submit_tx(tx)` inserts into the **local** mempool
4. **Nothing else happens.** The transaction is not forwarded to any other validator.

When a validator becomes the leader, it proposes a block containing transactions from its own local mempool. Transactions sitting in other validators' mempools are invisible to the leader.

### Consensus Forwarding Policy

The consensus engine is configured with `ForwardingPolicy::SilentLeader` (`crates/node/runner/src/runner.rs`, lines 548-574):

```rust
let engine = simplex::Engine::new(
    context.with_label("engine"),
    simplex::Config {
        scheme: self.scheme.clone(),
        elector: Random,
        blocker: transport.oracle.clone(),
        automaton: marshaled.clone(),
        relay: marshaled,
        reporter,
        // ... additional fields ...
        fetch_concurrent: 32,
        page_cache,
        forwarding: simplex::ForwardingPolicy::SilentLeader,  // line 571
    },
);
engine.start(transport.simplex.votes, transport.simplex.certs, transport.simplex.resolver);
```

`SilentLeader` means the leader does not broadcast its proposal payload back to itself. This is a consensus-level optimization -- it does NOT provide transaction relay. The consensus layer deals with block proposals (which contain already-selected transactions), not individual pending transactions.

## Impact

### 1. Unpredictable Transaction Inclusion Latency

In a 4-validator network with round-robin leader election, if a transaction is submitted to a non-leader validator, it must wait until that validator is elected leader. With the current timeout settings (~2 second leader timeout), this could mean waiting up to 3 rounds (approximately 0.6-6 seconds depending on nullification behavior) before the transaction is included.

From the user's perspective, transaction confirmation time depends entirely on which validator's RPC endpoint they happen to connect to, and where that validator falls in the current leader rotation.

### 2. Single Point of Failure for Transaction Submission

If a user submits a transaction to validator B, and validator B crashes or is restarted before it becomes leader, the transaction is **permanently lost**. There is no redundancy -- the transaction exists only in validator B's in-memory mempool.

The user would need to detect the failure and resubmit the transaction to a different validator. There is no automatic retry or recovery mechanism.

### 3. Loadgen Workaround Reveals the Problem

The load generator tool (`bin/loadgen/src/main.rs`) has an explicit `--broadcast-rpc-urls` flag to work around this limitation:

```rust
/// Additional RPC endpoint URLs to broadcast each transaction to.
///
/// Kora's current devnet mempools are validator-local, so devnet load tests
/// should submit to all validator RPCs to ensure the active proposer has the
/// transaction in its local mempool.
#[arg(long, value_delimiter = ',')]
broadcast_rpc_urls: Vec<String>,
```

When used, the loadgen sends every transaction to ALL validator RPC endpoints via HTTP:

```rust
let mut rpc_urls = Vec::with_capacity(args.broadcast_rpc_urls.len() + 1);
rpc_urls.push(args.rpc_url.clone());
rpc_urls.extend(args.broadcast_rpc_urls.iter().cloned());
```

This creates a **duplicate transaction storm**: every transaction is submitted N times (once per validator), each validator validates and inserts it independently, and then only one copy gets included in a block. The N-1 duplicates waste validation CPU, mempool space, and network bandwidth.

This workaround also only works for load testing. Production users connecting to a single RPC endpoint have no equivalent mechanism.

### 4. Production Risk

In a production deployment where users connect to a load balancer or a single RPC endpoint:

- If the load balancer routes to the current leader, transactions are included in the next block (~200ms)
- If routed to a non-leader, latency is unpredictable (0.2-6+ seconds)
- If routed to a validator that then goes down, transactions are silently lost

This behavior diverges significantly from what Ethereum users and tooling expect.

## How Ethereum Handles This

Ethereum nodes gossip pending transactions over the P2P network using the devp2p `eth/68` protocol:

1. When a node receives a new transaction (via RPC or from a peer), it validates it
2. It announces the transaction hash to all connected peers via `NewPooledTransactionHashes`
3. Peers that don't have the transaction request the full body via `GetPooledTransactions`
4. This ensures every node's mempool converges to approximately the same set of pending transactions

This gossip protocol means any node can include any pending transaction, regardless of which node originally received it.

## Proposed Implementation

### Option 1: P2P Transaction Gossip Channel (Recommended)

Add a 6th multiplexed channel for transaction gossip:

**Step 1:** Add a new channel ID in `crates/network/transport/src/channels.rs`:

```rust
/// Channel ID for transaction gossip messages.
pub const CHANNEL_TX_GOSSIP: u64 = 5;
```

**Step 2:** Register the channel in the transport builder (`crates/network/transport/src/builder.rs`). Add a new line after the existing marshal channel registrations (after line 87):

```rust
// Register transaction gossip channel
let tx_gossip = network.register(CHANNEL_TX_GOSSIP, quota, backlog);
```

Then expose it. The cleanest approach is to add a `tx_gossip` field to `NetworkTransport` (`crates/network/transport/src/transport.rs`, after line 39):

```rust
/// Channel for transaction gossip.
pub tx_gossip: (Sender<P, E>, Receiver<P>),
```

And wire it in `builder.rs` at the `NetworkTransport` construction (line 94-99).

**Step 3:** In the RPC transaction submission callback (`crates/node/runner/src/runner.rs`, lines 418-419), after a transaction passes validation and is inserted into the local mempool, broadcast the raw transaction bytes to all connected peers. The current code at line 418 is:

```rust
if ledger.submit_tx(tx).await {
    debug!(?tx_id, "rpc submit: tx inserted into mempool");
```

Modify it to also gossip:

```rust
if ledger.submit_tx(tx.clone()).await {
    debug!(?tx_id, "rpc submit: tx inserted into mempool");
    // Gossip to all peers via P2P channel
    let raw_bytes = tx.bytes.clone();
    if let Err(e) = tx_gossip_sender.broadcast(&raw_bytes).await {
        warn!(?tx_id, error = %e, "failed to gossip transaction");
    }
    Ok(())
}
```

Note: you will need to clone the `tx` since it is currently consumed by `submit_tx`. The `data` parameter (of type `alloy_primitives::Bytes`) passed to the callback contains the raw transaction bytes.

**Step 4:** In `crates/node/runner/src/runner.rs`, spawn a receiver task that listens on the gossip channel. This should be placed near where the engine is started (around line 574). The receiver takes the `Receiver<P>` half of the gossip channel pair. When raw transaction bytes arrive from a peer:
1. Decode and validate the transaction using `TransactionValidator::new(chain_id, state, PoolConfig::default())` -- the same validation used in the RPC path (lines 412-417)
2. Check if it already exists in the local mempool (dedup by transaction hash). `ledger.submit_tx()` already returns `false` for duplicates, so this is handled automatically.
3. If new and valid, insert into the local mempool via `ledger.submit_tx()`
4. Do NOT re-broadcast to avoid gossip storms (single-hop broadcast is sufficient for small validator sets)

Example receiver task structure:

```rust
// After engine.start() (line 574):
let gossip_ledger = ledger.clone();
let gossip_state = state.qmdb_state().await;
let gossip_chain_id = self.chain_id;
let (_, mut gossip_rx) = transport.tx_gossip;
tokio::spawn(async move {
    while let Some((peer, msg)) = gossip_rx.recv().await {
        let tx = Tx::new(Bytes::from(msg));
        let tx_id = tx.id();
        let validator = TransactionValidator::new(
            gossip_chain_id, gossip_state.clone(), PoolConfig::default()
        );
        match validator.validate(tx.clone()).await {
            Ok(()) => {
                if gossip_ledger.submit_tx(tx).await {
                    trace!(?tx_id, ?peer, "gossip: accepted tx from peer");
                }
                // submit_tx returns false for duplicates -- silently ignore
            }
            Err(err) => {
                warn!(?tx_id, ?peer, error = %err, "gossip: rejected invalid tx from peer");
            }
        }
    }
});
```

Note: The exact `recv()` API depends on the Commonware `discovery::Receiver<P>` type. Check its trait implementation for the correct method signature.

**Step 5:** Consider rate limiting. The existing channels already have rate limits configured in the transport builder:
- Default: **1000 messages per second** per channel per peer (`Quota::per_second(1000)` in `crates/network/transport/src/builder.rs`, line 22-24)
- Channel backlog: **256 messages** (`DEFAULT_BACKLOG` in `crates/network/transport/src/config.rs`, line 15)
- Max message size: **1 MB** (`DEFAULT_MAX_MESSAGE_SIZE` in `crates/network/transport/src/config.rs`, line 12)

For the gossip channel, you may want a **lower** rate quota than the default 1000 msg/s, since transaction gossip is less time-critical than consensus messages. Consider using `build_with_quota` or registering the gossip channel with a custom quota:

```rust
let gossip_quota = Quota::per_second(NonZeroU32::new(100).expect("non-zero"));
let tx_gossip = network.register(CHANNEL_TX_GOSSIP, gossip_quota, backlog);
```

Additional rate limiting considerations:
- Per-peer rate limit (already provided by Commonware's per-channel quota) to prevent a malicious peer from flooding the mempool
- Global rate limit to cap total gossip bandwidth across all peers

**Effort estimate:** ~100-200 lines of code, 1-2 days including testing.

### Option 2: RPC-Level Leader Forwarding (Simpler, Less Robust)

Instead of a P2P channel, when a validator receives a transaction via RPC and is NOT the current leader, it forwards the raw transaction to the current leader's RPC endpoint via HTTP.

```rust
if !is_current_leader {
    let leader_rpc_url = get_leader_rpc_url(current_view);
    let _ = http_client.post(leader_rpc_url)
        .json(&json!({"jsonrpc":"2.0","method":"eth_sendRawTransaction","params":[hex_tx],"id":1}))
        .send().await;
}
```

Pros:
- Simple to implement (~30 lines)
- Reuses existing RPC infrastructure

Cons:
- Requires validators to know each other's RPC URLs (configuration burden)
- Does not provide redundancy (if the leader is down, forwarding fails)
- Adds HTTP overhead vs direct P2P messaging
- Only forwards to 1 validator (the leader), not all peers

**Effort estimate:** ~30-50 lines, half a day including testing.

### Option 3: Broadcast to All Validators at Submission Time

Similar to what the loadgen does, but built into the node itself. When a validator receives a transaction, it forwards to all other validators' RPC endpoints.

Cons:
- Same as Option 2 plus the duplicate transaction overhead
- Every transaction is validated N times (once per validator)
- Requires all validators' RPC URLs in configuration

This is essentially productizing the loadgen workaround. Not recommended as a long-term solution.

## Design Considerations

- **Relay scope:** For small validator sets (4-20 validators), broadcasting to all peers is fine. For larger networks, consider relay-to-leader-only or gossip with fanout limits.
- **Deduplication:** The mempool already rejects duplicate transactions (`ledger.submit_tx()` returns `false` for duplicates), so receiving the same transaction from multiple peers is handled.
- **Ordering:** Transaction gossip is best-effort and unordered. Nonce ordering is enforced at block proposal time, not at gossip time.
- **Mempool consistency:** With gossip, all validators will have approximately the same mempool contents, which means blocks will be more consistently full and latency will be more predictable.

## Testing Plan

1. **Unit test:** Verify that a transaction submitted to validator A's RPC appears in validator B's mempool within one gossip interval.

2. **Integration test:** Submit 100 transactions to a single validator's RPC in a 4-validator devnet. Verify that all 100 are included in blocks within a reasonable time (~1-2 seconds each), regardless of leader rotation.

3. **Duplicate handling:** Submit the same transaction to two different validators' RPCs. Verify it is only included in one block (no double-execution).

4. **Performance test:** Run the loadgen WITHOUT `--broadcast-rpc-urls` (submitting to a single validator only). Compare inclusion latency and throughput to the current behavior with broadcast. The gossip-enabled version should match or exceed the broadcast workaround.

5. **Crash resilience:** Submit transactions to validator B, then kill validator B before it becomes leader. With gossip enabled, the transactions should still be included by other validators that received them via gossip.

## Verification Steps (Post-Implementation)

After implementing Option 1 (P2P Transaction Gossip Channel), run through these checks:

1. **Compile check:** `cargo build` passes with no errors in `kora-transport`, `kora-runner`, and related crates.

2. **Verify channel registration:**
   ```bash
   # Confirm the new channel constant exists
   grep -n 'CHANNEL_TX_GOSSIP' crates/network/transport/src/channels.rs
   # Should show: pub const CHANNEL_TX_GOSSIP: u64 = 5;

   # Confirm it is registered in the builder
   grep -n 'CHANNEL_TX_GOSSIP' crates/network/transport/src/builder.rs
   # Should show a network.register() call
   ```

3. **Verify gossip send path:**
   ```bash
   # Confirm the tx_submit callback now broadcasts after insert
   grep -A5 'submit_tx' crates/node/runner/src/runner.rs | grep -i 'gossip\|broadcast'
   # Should show gossip/broadcast logic after mempool insertion
   ```

4. **Verify gossip receive path:**
   ```bash
   # Confirm there is a receiver task spawned for incoming gossip
   grep -n 'tx_gossip\|CHANNEL_TX_GOSSIP' crates/node/runner/src/runner.rs
   # Should show both sender and receiver wiring
   ```

5. **Single-endpoint latency test on devnet:**
   ```bash
   # Submit a transaction to ONLY node1's RPC (not the leader)
   curl -s -X POST http://127.0.0.1:8546 \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","method":"eth_sendRawTransaction","params":["0x<signed_tx_hex>"],"id":1}'

   # Wait 2 seconds, then check if it was included in a block
   curl -s -X POST http://127.0.0.1:8545 \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","method":"eth_getTransactionReceipt","params":["0x<tx_hash>"],"id":1}'
   # Expected: receipt is non-null (transaction was included regardless of which validator received it)
   ```

6. **Loadgen without broadcast flag:**
   ```bash
   # Run loadgen against a SINGLE validator RPC (no --broadcast-rpc-urls)
   cargo run -p kora-loadgen -- --rpc-url http://127.0.0.1:8545 --total-txs 100
   # Expected: all 100 transactions should be included (not just the ones submitted to the leader)
   ```

7. **Dedup check:** Submit the same signed transaction to two different validators' RPCs. Verify it appears in exactly one block.

## Files Referenced

| File | Lines | Description |
|------|-------|-------------|
| `crates/node/runner/src/runner.rs` | 406-431 | Transaction submission callback (RPC -> local mempool only) |
| `crates/node/runner/src/runner.rs` | 548-574 | Consensus engine config with `ForwardingPolicy::SilentLeader` (line 571) |
| `crates/node/rpc/src/eth.rs` | 299-308 | `send_raw_transaction` RPC handler that calls `TxSubmitCallback` |
| `crates/network/transport/src/channels.rs` | 10-22 | 5 existing P2P channel ID constants (no tx gossip channel) |
| `crates/network/transport/src/channels.rs` | 33-65 | `SimplexChannels` and `MarshalChannels` struct definitions |
| `crates/network/transport/src/builder.rs` | 22-24 | `default_quota()` -- 1000 msg/s per channel |
| `crates/network/transport/src/builder.rs` | 80-87 | Where the 5 channels are registered with the network |
| `crates/network/transport/src/builder.rs` | 94-99 | `NetworkTransport` construction with channel assignment |
| `crates/network/transport/src/config.rs` | 12-15 | `DEFAULT_MAX_MESSAGE_SIZE` (1 MB) and `DEFAULT_BACKLOG` (256) |
| `crates/network/transport/src/bundle.rs` | 14-43 | `TransportBundle` with `SimplexChannels` and `MarshalChannels` |
| `crates/network/transport/src/transport.rs` | 23-40 | `NetworkTransport` struct with `oracle`, `simplex`, and `marshal` fields |
| `bin/loadgen/src/main.rs` | 33-39 | `--broadcast-rpc-urls` workaround flag with doc comment |
| `bin/loadgen/src/main.rs` | 190-215 | `send_raw_transaction_to()` with fallback broadcast logic |
| `bin/loadgen/src/main.rs` | 226-228 | RPC URL aggregation for broadcast |
| `crates/node/rpc/src/kora.rs` | 1-38 | `KoraApiImpl` returning `NodeStatus` with `kora_nodeStatus` |
