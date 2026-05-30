# P2P Networking Architecture and Gaps

## What Is Kora?

Kora is a blockchain node implementation that uses the Commonware SDK as its consensus and networking foundation. It runs a BFT consensus protocol called Simplex with BLS12-381 threshold signatures, executing EVM-compatible transactions via REVM. The network currently operates as a 4-validator devnet with an optional secondary (non-voting) peer.

## Network Architecture

Kora uses a **static, permissioned validator set** with no dynamic peer discovery. The network topology is:

- **4 validators** (node0 through node3), each identified by an ed25519 public key
- **Optional secondary peers** that receive block data but do not participate in consensus
- **No open membership** -- the validator set is fixed at genesis and requires a DKG ceremony to change
- **No peer discovery protocol** -- all peers are configured at startup via bootstrap peer lists

### How Nodes Find Each Other

Nodes use Commonware's `authenticated::discovery::Network`, which provides mutually-authenticated, encrypted connections between peers. Each node is configured with:

1. Its own ed25519 private key for identity
2. A listen address (port 30303 inside containers)
3. A list of bootstrap peers in `PUBLIC_KEY_HEX@HOST:PORT` format
4. A network namespace (`_COMMONWARE_KORA_NETWORK`) to prevent cross-network connections

The bootstrap peer list is **not** a discovery mechanism -- it is the complete peer list. Node0 acts as the bootstrap node; all other nodes specify `BOOTSTRAP_PEERS=node0:30303` and rely on node0 to introduce them to the rest of the set. Once connected, the oracle's `track()` call registers the full validator set (epoch 0), allowing Commonware to maintain connections to all listed peers.

### Port Layout (Devnet)

| Service | Internal Port | External Port(s) | Purpose |
|---------|--------------|-------------------|---------|
| P2P (all nodes) | 30303 | 30400-30403 (validators), 30500 (secondary) | Authenticated discovery, all consensus and block traffic |
| RPC | 8545 | 8545-8548 | JSON-RPC for transaction submission and queries |
| Metrics | 9002 | 9000-9003 | Prometheus OpenMetrics endpoint |

All P2P traffic flows over a single TCP port (30303) using Commonware's multiplexed channel system. The DKG ceremony uses the same port but a different namespace (`_KORA_DKG_CEREMONY`).

---

## The P2P Stack: Protocols and Channels

Kora's P2P layer is built entirely on `commonware-p2p`'s authenticated discovery network. A single TCP connection between each pair of peers multiplexes **5 logical channels**:

```
NetworkTransport
+----------------------------------------------------------+
|  Oracle (peer management, tracking, Byzantine blocking)   |
+----------------------------------------------------------+
|  Simplex Channels              |  Marshal Channels        |
|  +- Channel 0: Votes          |  +- Channel 3: Blocks    |
|  +- Channel 1: Certificates   |  +- Channel 4: Backfill  |
|  +- Channel 2: Resolver       |                          |
+----------------------------------------------------------+
```

### Channel Descriptions

| Channel | ID | Direction | Purpose |
|---------|----|-----------|---------|
| Votes | 0 | Bidirectional | Consensus vote messages: leaders broadcast proposed blocks, validators respond with votes |
| Certificates | 1 | Bidirectional | Notarization and finalization certificates (aggregated BLS threshold signatures) |
| Resolver | 2 | Bidirectional | Request/response for fetching missing blocks during catch-up (used by Simplex's internal resolver) |
| Blocks | 3 | Bidirectional | Full block broadcast via buffered broadcast engine (marshal layer) |
| Backfill | 4 | Bidirectional | Resolver requests for the marshal layer's block archive (separate from Simplex resolver) |

### Rate Limiting

All channels share a default rate quota of **1000 messages per second per channel** (`Quota::per_second(1000)`). The DKG transport uses a separate, lower quota of 100 messages/second (appropriate for its low-volume ceremony messages).

### Message Size Limits

- Maximum message size: **1 MB** (`DEFAULT_MAX_MESSAGE_SIZE = 1024 * 1024`)
- Maximum block size: up to 10,000 transactions, up to 8 MB total (`BLOCK_CODEC_MAX_TX_BYTES = 8 * 1024 * 1024`)
- Channel backlog: **256 messages** per channel

### Additional Protocol: DKG

The DKG ceremony runs on a **separate** authenticated discovery network instance with:
- Its own namespace (`_KORA_DKG_CEREMONY`)
- A single channel (ID 10) for ceremony messages
- Lower rate limits (100 msg/s)
- A fallback raw TCP transport for legacy DKG (direct point-to-point TCP connections)

---

## Critical Gap: No Transaction Gossip Protocol

### The Problem

Kora has **no mechanism for validators to share pending transactions with each other**. Each validator's mempool is entirely local -- it only contains transactions submitted directly to that validator's RPC endpoint.

### How It Works Today

```
User submits tx to node1's RPC (port 8546)
    |
    v
node1's mempool has the tx
    |
    v
When node1 is leader, it includes the tx in a block
    |
    v
If node0 or node2 is leader instead, the tx sits idle
    (node0/node2 don't know about it)
```

The transaction submission path in the code (`runner.rs` lines 406-431) is strictly local:

1. User sends raw transaction bytes to RPC
2. `TransactionValidator` checks signature, nonce, chain ID
3. `ledger.submit_tx(tx)` inserts into the local mempool
4. During `propose()`, the leader drains its own mempool via `mempool.build(max_txs, &excluded)`

There is no step where transactions are forwarded to other validators.

### Impact

- **Unpredictable inclusion latency**: A transaction submitted to a non-leader validator must wait until that validator becomes leader (round-robin election, so up to 3 rounds x ~0.2s = 0.6s in the best case)
- **Single point of failure for tx submission**: If the validator receiving a transaction goes down before becoming leader, the transaction is lost
- **Loadgen workaround**: The load generator explicitly broadcasts each transaction to ALL validators via `--broadcast-rpc-urls`, simulating gossip at the application layer
- **Production risk**: Any user/wallet connected to a single RPC endpoint will experience variable latency depending on which validator they hit

### Severity: HIGH

This is a fundamental gap that will cause user-visible issues in any non-test deployment. The `ForwardingPolicy::SilentLeader` setting in Simplex means even the consensus layer does not relay proposal payloads to non-leaders.

---

## Gap: Silent Broadcast Failures

### DKG Ceremony Broadcasts

In `crates/node/dkg/src/ceremony.rs`, network failures during DKG are silently suppressed:

```rust
// Line 346: Send result completely ignored
let _ = network.send_to(leader_pk, &request_msg);

// Lines 388-395: Failures logged at debug level only
if let Err(e) = network.send_to(&pk, &msg) {
    debug!(?pk, ?e, "Failed to send to peer");
}
if let Err(e) = network.broadcast(&msg) {
    debug!(?e, "Failed to broadcast");
}
```

If a network partition occurs during DKG, the ceremony hangs indefinitely without actionable error messages at the default log level (`info`). The DKG raw-TCP transport has similar issues -- broadcasts iterate over peers and `warn` on individual failures but continue, potentially leaving some peers without critical polynomial shares.

### Block Broadcast (Marshal Layer)

The buffered broadcast engine (`BroadcastInitializer`) sends blocks to all peers with priority, but there is no feedback mechanism to detect when a peer fails to receive a block. The only recovery path is through the backfill resolver, which has its own blocking issues (see `resolver-catchup-failure.md`).

### Severity: MEDIUM

Silent DKG failures can require full ceremony restarts. Silent block broadcast failures are partially mitigated by the resolver but become critical when the resolver itself is broken.

---

## Gap: No Connection Health Monitoring

### The Problem

There is no heartbeat, liveness check, or connection-state metric exposed by the P2P layer. The Kora application layer has no visibility into whether peers are connected, disconnected, or experiencing packet loss.

### Detection Latency

The only mechanism that detects a failed peer is the consensus timeout system:
- Leader timeout: **2 seconds** (`CONSENSUS_LEADER_TIMEOUT`)
- Certification timeout: **4 seconds** (`CONSENSUS_CERTIFICATION_TIMEOUT`)
- Activity timeout: **256 views** before a peer is considered inactive

This means a crashed validator is not detected for at least 2 seconds. During that time, the network produces nullified views (empty rounds).

### Missing Metrics

There are currently **no Prometheus metrics** for:
- Per-peer connection state (connected/disconnected)
- Messages sent/received per channel
- Bandwidth utilization per peer
- Message delivery latency
- Mailbox fill level

The only P2P-adjacent metric available is `engine_resolver_resolver_peers_blocked` (a gauge of blocked resolver peers).

### Severity: MEDIUM

Operators have no visibility into network health until consensus-level symptoms appear (nullification rate increase, block production slowdown).

---

## Gap: No Peer Scoring or DOS Protection at P2P Level

### The Problem

Kora does not implement peer scoring, reputation tracking, or application-level DOS protection beyond Commonware's built-in rate limiting (1000 msg/s per channel).

### What Exists

- **Rate quotas**: 1000 messages/second per channel (configured in `builder.rs`)
- **Byzantine blocking via oracle**: The oracle can block peers, but this is only triggered by the resolver's "invalid data" detection (which itself is buggy -- see resolver doc)
- **Activity timeout**: Simplex will eventually skip inactive validators after 256 views

### What Is Missing

- No bandwidth-based throttling
- No reputation scoring (e.g., downgrading peers that send invalid votes)
- No eclipse attack protection (a compromised bootstrap node could isolate validators)
- No connection diversity requirements
- No amplification attack mitigation

### Severity: LOW (for permissioned devnet), HIGH (for production)

In the current 4-validator permissioned setup, all peers are trusted by construction. This becomes a real concern only with open membership or adversarial environments.

---

## Gap: Mailbox Overflow Behavior

### Configuration

| Parameter | Value | Source |
|-----------|-------|--------|
| Channel backlog | 256 messages | `DEFAULT_BACKLOG` in `config.rs` |
| Broadcast mailbox | 1024 messages | `BroadcastInitializer::DEFAULT_MAILBOX_SIZE` |
| Broadcast deque | 256 messages | `BroadcastInitializer::DEFAULT_DEQUE_SIZE` |
| Resolver mailbox | 1024 messages | `PeerInitializer::DEFAULT_MAILBOX_SIZE` |
| Simplex mailbox | `MAILBOX_SIZE` | Defined in `kora_simplex` |

### The Problem

When inbound message buffers are full (e.g., during a burst of blocks after a node restart), the behavior is determined by Commonware internals. Messages may be dropped without any Kora-level logging or metric emission. Under high load, critical consensus messages (votes, finalization certificates) could be silently discarded.

### Severity: LOW (current devnet load), MEDIUM (under stress)

---

## Gap: Hardcoded Fetch Concurrency

```rust
// runner.rs line 569
fetch_concurrent: 32
```

The number of concurrent block fetch requests is hardcoded at 32. This value is not configurable. In geo-distributed deployments with higher RTT, this may be insufficient for rapid catch-up. Conversely, in local devnets the aggressive concurrency combined with 200ms resolver timeouts contributes to peer-blocking cascades.

### Severity: LOW

---

## Summary of Severity Levels

| Gap | Severity | Impact |
|-----|----------|--------|
| No transaction gossip | HIGH | Unpredictable tx inclusion, requires application-layer workarounds |
| Resolver peer blocking | CRITICAL | Permanent catch-up failure after restart (see separate doc) |
| Silent broadcast failures | MEDIUM | DKG ceremonies can hang; block propagation failures hidden |
| No connection health monitoring | MEDIUM | No operator visibility into P2P health |
| No peer scoring / DOS protection | LOW (devnet) | Relevant only for adversarial environments |
| Mailbox overflow | LOW | Could matter under extreme load |
| Hardcoded fetch concurrency | LOW | Not tunable for different deployment topologies |

---

## Key Configuration Reference

| Parameter | Value | Location |
|-----------|-------|----------|
| Network namespace | `_COMMONWARE_KORA_NETWORK` | `crates/network/transport/src/config.rs` |
| Max message size | 1 MB | `crates/network/transport/src/config.rs` |
| Channel backlog | 256 | `crates/network/transport/src/config.rs` |
| Rate quota | 1000 msg/s/channel | `crates/network/transport/src/builder.rs` |
| Broadcast mailbox | 1024 | `crates/network/marshal/src/broadcast.rs` |
| Broadcast deque | 256 | `crates/network/marshal/src/broadcast.rs` |
| Resolver mailbox | 1024 | `crates/network/marshal/src/peers.rs` |
| Resolver initial delay | 200ms | `crates/network/marshal/src/peers.rs` |
| Resolver timeout | 200ms | `crates/network/marshal/src/peers.rs` |
| Fetch retry timeout | 100ms | `crates/network/marshal/src/peers.rs` |
| Fetch concurrent | 32 | `crates/node/runner/src/runner.rs` |
| Leader timeout | 2s | `crates/node/runner/src/runner.rs` |
| Certification timeout | 4s | `crates/node/runner/src/runner.rs` |
| Activity timeout | 256 views | `crates/node/runner/src/runner.rs` |
| Skip timeout | 32 views | `crates/node/runner/src/runner.rs` |
| DKG rate quota | 100 msg/s | `crates/node/dkg/src/transport.rs` |
| DKG namespace | `_KORA_DKG_CEREMONY` | `crates/node/dkg/src/transport.rs` |
| DKG max message | 256 KB | `crates/node/dkg/src/transport.rs` |
| Bootstrap format | `PK_HEX@HOST:PORT` | `crates/network/transport/src/config.rs` |

---

## Relevant Source Files

| File | Purpose |
|------|---------|
| `crates/network/transport/src/config.rs` | Transport configuration, peer parsing, namespace |
| `crates/network/transport/src/builder.rs` | Channel registration, network startup |
| `crates/network/transport/src/channels.rs` | Channel ID constants and type definitions |
| `crates/network/transport/src/transport.rs` | NetworkTransport struct (oracle + channels) |
| `crates/network/transport/src/network_provider.rs` | Production transport provider |
| `crates/network/transport/src/error.rs` | Transport error types (config-only, no runtime errors) |
| `crates/network/marshal/src/broadcast.rs` | Buffered block broadcast initialization |
| `crates/network/marshal/src/peers.rs` | P2P resolver initialization and configuration |
| `crates/node/dkg/src/network.rs` | Raw TCP DKG networking |
| `crates/node/dkg/src/transport.rs` | Authenticated DKG transport |
| `crates/node/dkg/src/ceremony.rs` | DKG ceremony with silent broadcast failures |
| `crates/node/runner/src/runner.rs` | Main node runner: wires together all P2P components |
| `crates/node/simplex/src/engine.rs` | Simplex engine default constructor |
| `docker/compose/devnet.yaml` | Devnet port layout and peer configuration |
