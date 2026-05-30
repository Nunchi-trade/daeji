# Resolver Catch-Up Failure: Permanent Peer Blocking After Restart

## Summary

When a Kora validator restarts, the Commonware resolver (responsible for fetching missed blocks from peers) permanently blocks all peers within milliseconds, making catch-up impossible. The node remains stuck at its pre-restart height indefinitely. This is an upstream bug in Commonware's `commonware_resolver::p2p::engine` module.

**Severity**: CRITICAL -- this bug makes any node restart potentially fatal to the network.

---

## What Is the Resolver?

The resolver is a Commonware component that handles block catch-up. When a node falls behind the current chain height (due to restart, network partition, or slow processing), the resolver:

1. Identifies which blocks it is missing
2. Selects a peer to request the missing blocks from
3. Sends fetch requests via the P2P backfill channel (Channel 4)
4. Receives block data from the peer
5. Passes received blocks to the application's `verify()` method for validation
6. If validation succeeds, applies the block and moves to the next
7. If validation fails, marks the peer as providing "invalid data" and **blocks** that peer

The resolver is initialized in `crates/network/marshal/src/peers.rs` via `PeerInitializer::init()` and is wired into the node in `crates/node/runner/src/runner.rs` (lines 492-498):

```rust
let resolver = PeerInitializer::init::<_, _, _, Block, _, _, _>(
    &context.with_label("resolver"),
    my_pk.clone(),
    transport.oracle.clone(),       // peer_provider: where to find peers
    transport.oracle.clone(),       // blocker: same oracle used for blocking!
    transport.marshal.backfill,     // P2P channel for requests/responses
);
```

Key configuration values:
- Initial delay: **200ms** (time before first fetch attempt)
- Timeout: **200ms** (how long to wait for a response)
- Fetch retry timeout: **100ms** (delay between retries)
- Priority requests/responses: **true** (high-priority channel scheduling)
- Mailbox size: **1024**

---

## How Catch-Up Is Supposed to Work

Normal catch-up flow after a node restart:

```
1. Node starts, loads persisted finalized blocks from archive
2. Simplex engine initializes, discovers current_view is ahead of local state
3. Engine requests missing blocks via resolver
4. Resolver sends backfill requests to peers via Channel 4
5. Peers respond with block data
6. Application's verify() method validates each block:
   a. Look up parent snapshot (state after parent block)
   b. Re-execute all transactions against parent state
   c. Verify computed state_root matches block header
7. On success: block applied, snapshot stored, move to next height
8. On failure: peer is blocked, try another peer
```

---

## The Bug: Why Verification Always Fails After Restart

### Root Cause

EVM block verification is **stateful** -- verifying block N requires having the state snapshot from block N-1. The verification code in `crates/node/runner/src/app.rs` (lines 166-246):

```rust
async fn verify_block(&self, block: &Block) -> bool {
    let parent_digest = block.parent();

    // THIS LOOKUP FAILS AFTER RESTART:
    let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
        warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
        return false;  // Returns false -> resolver sees "invalid data"
    };

    // ... execute transactions against parent state ...
    // ... compare computed state_root to block's state_root ...
}
```

After a restart, the in-memory snapshot cache is empty. The node has persisted finalized blocks in its archive, but the **working state snapshots** needed for verification are gone. When the resolver fetches block H:

1. `verify_block(H)` needs parent snapshot for block H-1
2. Parent snapshot does not exist in memory (cleared on restart)
3. `verify_block` returns `false`
4. Resolver interprets this as the peer sending invalid/malicious data
5. Resolver **permanently blocks** that peer

### The Blocking Cascade

With 4 validators, a restarted node has 3 potential peers to fetch from:

```
Restart -> empty snapshot cache
    |
    v
Resolver requests block H from peer A
    |
    v
verify_block(H) fails: missing parent snapshot for H-1
    |
    v
Resolver BLOCKS peer A permanently
    |
    v
Resolver tries peer B -> same failure -> BLOCKS peer B
    |
    v
Resolver tries peer C -> same failure -> BLOCKS peer C
    |
    v
ALL PEERS BLOCKED -> no block sources remaining
    |
    v
Node permanently stuck at pre-restart height
```

This cascade happens in **under 50 milliseconds** from startup. The log evidence shows:

```
2026-05-20T17:39:16.105565Z INFO  consensus initialized current_view=36404
2026-05-20T17:39:16.296762Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.296866Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.296939Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.297011Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.297072Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
```

Five failures in 310 microseconds, all from the same peer -- indicating the resolver fires off multiple concurrent requests that all fail instantly.

---

## The Fundamental Design Mismatch

The resolver expects blocks to be **independently verifiable** -- that is, receiving a block from a peer should be sufficient to determine whether the block is valid. This assumption holds for simple chains where blocks are self-contained.

However, EVM block verification is **sequential** -- it requires re-executing all transactions against the parent block's state. This means:
- Block H cannot be verified without state from block H-1
- Block H-1 cannot be verified without state from block H-2
- This chain goes all the way back to genesis (or the last persisted snapshot)

The resolver does not understand this dependency. It treats a verification failure as evidence that the peer is malicious (sending fake blocks), when in reality the local node simply lacks the prerequisite state.

---

## Observed Impact

### Test 1: Single Validator Restart

- Node3 restarted after reaching height 25,348
- Immediately stuck; height never advanced
- Three healthy nodes continued at ~4.5 blocks/second
- After 120 seconds of monitoring: node3 still at 25,348, healthy nodes at ~26,029
- Gap grew at ~5.7 blocks/second with no recovery

### Test 2: Two Validator Restart (Quorum Loss + Recovery)

- Nodes 2 and 3 stopped (quorum lost, chain frozen as expected)
- Both restarted after 45 seconds
- Node2 recovered (it had relatively fresh state)
- Node3 remained stuck at height 25,514 (680 blocks behind)
- Network severely degraded: block rate dropped from 4.5 to 0.5-1.0 blocks/second
- Nullification rate exploded: node0 accumulated 452 additional nullifications in 180 seconds
- Effective throughput: ~80% of rounds produced no block (only 3 of 4 validators participating)

### Production Failure Scenario

In production, the following sequence leads to **permanent chain death**:

```
1. Rolling upgrade: restart node0
2. Node0 stuck (resolver blocks all peers)
3. Network runs on 3/4 validators (still has quorum)
4. Restart node1 for upgrade
5. Node1 stuck (same resolver bug)
6. Only 2/4 validators active: below 3/4 quorum threshold
7. Chain halts permanently -- no blocks finalized
8. Recovery requires full cluster restart (all nodes simultaneously)
```

---

## Prometheus Metric: Detection

The blocked-peers state is exposed via Prometheus:

```
# HELP engine_resolver_resolver_peers_blocked Current number of blocked peers.
# TYPE engine_resolver_resolver_peers_blocked gauge
engine_resolver_resolver_peers_blocked 0
```

There is also a second instance under a different label prefix:

```
# HELP resolver_resolver_peers_blocked Current number of blocked peers.
# TYPE resolver_resolver_peers_blocked gauge
resolver_resolver_peers_blocked 0
```

These correspond to the two resolver instances -- one inside the Simplex engine (prefix `engine_`) and one in the marshal layer (prefix `resolver_`).

### Alert Configuration

The alert is defined in `docker/config/alerts.yml`:

```yaml
- alert: ResolverPeersBlocked
  expr: engine_resolver_resolver_peers_blocked > 0
  for: 1m
  labels:
    severity: warning
  annotations:
    summary: "Node {{ $labels.instance }} has {{ $value }} blocked resolver peers"
    description: "Blocked peers cannot provide blocks for catch-up. This caused permanent stall after node restarts."
```

This alert fires within 1 minute of a node restart (since blocking happens in milliseconds). It should be treated as a critical operational signal requiring immediate intervention.

---

## Workarounds

### Full Cluster Restart (Current Fix)

Stopping and restarting ALL validators simultaneously clears the in-memory blocked-peers state on every node. Because all nodes restart together, they are all at the same height and no catch-up is needed. This is the only reliable recovery mechanism today.

**Limitation**: This causes chain downtime and is not suitable for production.

### Avoid Individual Restarts

Since the bug only triggers when a node needs to catch up, avoiding individual restarts prevents the issue. This means:
- No rolling upgrades
- No individual node maintenance
- If a node crashes, it cannot recover without full cluster restart

### Increase Archive Retention (Partial Mitigation)

If the node can restore its persisted state snapshots on startup (not just finalized blocks), it may have a valid parent snapshot for recent blocks. However, the current `recover_finalized_state` function in `runner.rs` only replays the finalized block archive -- it does not reconstruct the in-memory snapshot cache that `verify_block` requires.

---

## Is This Fixable at the Kora Level?

### Short answer: Partially, but the real fix must come from Commonware.

### What Kora Could Do

1. **Trust finality certificates during catch-up**: If a block has a valid finalization certificate (2/3+ BLS threshold signature from validators), accept it without full re-execution. This would bypass the verify_block failure entirely. However, this requires changes to how the resolver interacts with the application layer.

2. **Pre-populate snapshot cache on startup**: Before starting consensus, replay persisted blocks sequentially to rebuild the snapshot cache. This would make verify_block succeed, but adds potentially large startup latency (replaying thousands of blocks).

3. **Return a "retry later" signal instead of false**: If verify_block returned a third state (not-yet-verifiable vs. definitely-invalid), the resolver could retry instead of blocking. But the Commonware `VerifyingApplication` trait only supports `bool` (true/false).

### What Commonware Must Fix

1. **Do not permanently block peers on application-level verification failure**: The resolver should distinguish between "peer sent garbage data" (block fails to decode) vs. "application cannot verify right now" (verification returns false due to missing state). At minimum, use exponential backoff instead of permanent blocking.

2. **Support sequential catch-up**: The resolver should be aware that blocks may have dependencies and fetch/verify them in order (from the divergence point forward).

3. **Provide an unblock mechanism**: Even if blocking is appropriate, there should be a way to unblock peers (e.g., after a timeout, or when the application signals readiness).

---

## Related Source Files

| File | Role |
|------|------|
| `crates/network/marshal/src/peers.rs` | Resolver initialization (`PeerInitializer`) |
| `crates/node/runner/src/runner.rs` | Wires resolver with oracle as both provider and blocker |
| `crates/node/runner/src/app.rs` | `verify_block()` implementation that returns false |
| `crates/node/simplex/src/engine.rs` | Simplex engine that consumes resolver channel |
| `docker/config/alerts.yml` | `ResolverPeersBlocked` alert definition |
| `docker/grafana/dashboards/kora-performance.json` | Dashboard panel showing blocked peers |
| `repro-logs/commonware-evidence/` | Full reproduction evidence from chaos tests |

---

## Reproduction Steps

1. Start a 4-validator devnet: `just devnet`
2. Wait for steady-state block production (~25,000+ blocks)
3. Restart a single validator: `docker compose restart validator-node3`
4. Observe node3's height remains frozen while other nodes advance
5. Check logs for `commonware_resolver::p2p::engine: invalid data received`
6. Query metrics: `curl localhost:9003/metrics | grep peers_blocked` (will show > 0)
7. Wait indefinitely -- node3 will never catch up

Recovery: `docker compose restart` (full cluster restart)
