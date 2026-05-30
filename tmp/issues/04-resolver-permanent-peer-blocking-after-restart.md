# Resolver: Permanent peer blocking after node restart makes catch-up impossible

**Severity:** Critical
**Component:** `crates/node/runner/src/app.rs` -- `verify_block` / `VerifyingApplication`, `crates/network/marshal/src/peers.rs` -- `PeerInitializer`
**Affects:** Any validator node that restarts while the rest of the cluster continues producing blocks
**Related issues:** #03 (Mempool pruning skipped on error paths -- compounds with this bug to make stall recovery impossible)

---

## Summary

When a Kora validator restarts, the Commonware resolver permanently blocks all peers within milliseconds, making block catch-up impossible. The node remains stuck at its pre-restart height forever. This happens because EVM block verification is inherently sequential (verifying block N requires the state from block N-1), but after a restart the in-memory snapshot cache is empty. The resolver asks the application to verify blocks from peers, the application cannot find the parent snapshot, it returns `false`, and the resolver interprets this as "peer sent invalid data" and permanently blocks the peer. In the default 4-validator devnet, all 3 peers are blocked within ~50ms and the node can never catch up.

This makes any node restart potentially fatal to the network. Two restarts in a 4-validator cluster (which requires 3/4 quorum) means quorum loss. Rolling upgrades are impossible. The only reliable recovery is a full cluster restart.

---

## Background

### What is Kora?

Kora is an EVM-compatible blockchain built on the [Commonware](https://github.com/commonwarexyz/monorepo) consensus framework. It uses the Simplex BFT protocol for consensus among a fixed validator set with BLS12-381 threshold signatures. The repository is at `https://github.com/refcell/kora`.

### What is the resolver?

The resolver is a Commonware component that handles block synchronization ("catch-up") when a node falls behind the rest of the network. When a node restarts, it may be many blocks behind. The resolver:

1. Discovers peers that have blocks the node is missing
2. Requests block data from those peers
3. Passes received blocks to the application's `verify()` method for validation
4. If validation succeeds, applies the block and requests the next one
5. **If validation fails, the resolver interprets this as the peer providing invalid data and permanently blocks that peer from future interactions**

The resolver is initialized in `crates/network/marshal/src/peers.rs` via `PeerInitializer::init()` and wired into the node in `crates/node/runner/src/runner.rs` (lines 492-498).

### How catch-up verification works

When the resolver needs to verify blocks received from a peer, it calls the application's `verify()` method, which is the `VerifyingApplication` trait implementation on `RevmApplication` in `crates/node/runner/src/app.rs` (lines 321-383).

The `verify()` implementation:
1. Receives an ancestry stream of blocks (tip-first, newest to oldest)
2. Collects blocks until it finds one already verified (line 342-349)
3. Reverses the list to process oldest-first (line 363)
4. Calls `verify_block()` on each block sequentially (line 364)

The `verify_block()` method (lines 166-246) does the following:
1. Checks if block is already verified via `query_state_root()` (line 171) -- early return `true` if so
2. **Looks up the parent snapshot** via `parent_snapshot()` (line 176) -- **returns `false` if missing**
3. Executes the block against the parent state using the EVM executor (line 184-193)
4. Computes the state root and verifies it matches (lines 197-218)
5. Inserts the snapshot into the cache (lines 220-232)

### The snapshot dependency chain

EVM state verification is fundamentally sequential. To verify block N, you need the complete EVM state after block N-1 (the "parent snapshot"). This snapshot includes account balances, nonces, contract storage, and code -- everything the EVM needs to re-execute the block's transactions and verify the resulting state root.

After a restart, the in-memory snapshot store is empty. The startup recovery process (`recover_finalized_state` in `crates/node/runner/src/runner.rs` lines 170-228) iterates through the finalized block archive and restores only the LAST finalized block's snapshot via `restore_persisted_snapshot()` (line 219). It does NOT pre-populate snapshots for intermediate blocks.

The `restore_persisted_snapshot()` method (in `crates/node/ledger/src/lib.rs` lines 239-252) creates a minimal snapshot:

```rust
pub async fn restore_persisted_snapshot(&self, block: &Block) {
    let inner = self.inner.lock().await;
    let digest = block.commitment();
    let state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
    let snapshot = Snapshot::new(
        Some(block.parent()),
        state,
        block.state_root,
        QmdbChangeSet::default(),
        tx_ids(&block.txs),
    );
    inner.snapshots.insert(digest, snapshot);
    inner.snapshots.mark_persisted(&[digest]);
}
```

This snapshot is keyed by the block's own digest (`block.commitment()`). When `verify_block(H+1)` is called, it looks up the parent snapshot using `parent_snapshot(block_H+1.parent())`, which is `block_H.commitment()`. If `block_H` was the last restored block, this lookup should succeed. However, the actual failure mode depends on whether the resolver requests blocks sequentially starting from H+1, or whether it requests a block further ahead whose parent was NOT the last restored block.

---

## The Bug

### The verification failure cascade

When a restarted node tries to catch up, the following happens:

1. The node restarts at height H (the last persisted finalized block)
2. The cluster has advanced to height H+K (K blocks ahead)
3. The resolver asks peer P1 for the missing blocks
4. P1 sends blocks H+1 through H+K
5. `verify()` is called with these blocks
6. `verify()` collects blocks until it finds one already verified (block H)
7. It starts verifying from block H+1 (oldest unverified)
8. `verify_block(H+1)` calls `parent_snapshot(digest_of_H)`

**Here is where it breaks:** After restart, `restore_persisted_snapshot()` creates a minimal snapshot for block H with an empty `ChangeSet` pointing at the current QMDB state. However, it creates the snapshot keyed by the BLOCK's digest (the commitment of block H). The `parent_snapshot()` call looks up by the PARENT digest field of block H+1, which is block H's digest. If the recovery successfully created this entry, verification of H+1 might work. But for H+2, the parent is H+1 -- and if verify_block returned false for H+1 for any reason (or if the ancestry stream delivered blocks in a way that skipped H+1), the snapshot for H+1 was never inserted.

More critically, the actual failure mode observed in production is:

```
verify_block(H+1):
  parent_digest = block_H+1.parent()  // = digest of block H
  parent_snapshot(parent_digest) -> None  // snapshot cache miss after restart
  return false
```

The `parent_snapshot()` returns `None` because the snapshot for block H was either not restored (recovery failed or was incomplete) or was keyed differently than expected. The `verify_block` method logs a warning and returns `false`:

```rust
// crates/node/runner/src/app.rs, lines 176-179
let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
    warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
    return false;
};
```

### The permanent blocking

When `verify()` returns `false`, the Commonware resolver does not retry. It interprets this as: "the peer sent invalid block data." The resolver permanently blocks that peer using the `Blocker` trait:

```rust
// crates/network/marshal/src/peers.rs, lines 70-81
let resolver_cfg = Config {
    public_key,
    peer_provider,
    blocker,            // <-- transport.oracle, same as peer_provider
    mailbox_size: Self::DEFAULT_MAILBOX_SIZE,
    initial: Self::DEFAULT_INITIAL_DELAY,
    timeout: Self::DEFAULT_TIMEOUT,
    fetch_retry_timeout: Self::DEFAULT_FETCH_RETRY_TIMEOUT,
    priority_requests: Self::PRIORITY_REQUESTS,
    priority_responses: Self::PRIORITY_RESPONSES,
};
commonware_consensus::marshal::resolver::p2p::init(ctx, resolver_cfg, backfill)
```

### The oracle dual-role problem

The critical wiring is at `crates/node/runner/src/runner.rs` lines 492-498:

```rust
let resolver = PeerInitializer::init::<_, _, _, Block, _, _, _>(
    &context.with_label("resolver"),
    my_pk.clone(),
    transport.oracle.clone(),       // peer_provider: where to find peers
    transport.oracle.clone(),       // blocker: SAME oracle used for blocking!
    transport.marshal.backfill,
);
```

The same `transport.oracle` is used as both the `peer_provider` (where to find peers to sync from) AND the `blocker` (where to record permanently blocked peers). When a peer is blocked via the oracle, it is removed from the set of available peers for ALL subsystems -- not just the resolver, but also consensus voting and block propagation.

### The cascade timeline

In a 4-validator cluster (V0, V1, V2, V3), when V0 restarts:

```
T+0ms:     V0 restarts, snapshot cache is empty, only has block H
T+5ms:     Resolver contacts V1 for blocks H+1..H+K
T+10ms:    verify(blocks from V1) -> verify_block(H+1) -> missing parent snapshot -> false
T+10ms:    Resolver BLOCKS V1 permanently
T+15ms:    Resolver contacts V2 for blocks H+1..H+K
T+25ms:    verify(blocks from V2) -> verify_block(H+1) -> missing parent snapshot -> false
T+25ms:    Resolver BLOCKS V2 permanently
T+30ms:    Resolver contacts V3 for blocks H+1..H+K
T+40ms:    verify(blocks from V3) -> verify_block(H+1) -> missing parent snapshot -> false
T+40ms:    Resolver BLOCKS V3 permanently
T+40ms:    V0 has blocked ALL peers. No more catch-up sources available.
T+forever: V0 is permanently stuck at height H. Never catches up.
```

All 3 peers blocked in under 50ms. The node is permanently stranded.

Production log evidence shows 5 failures within 310 microseconds, all for the same peer, indicating the resolver fires off multiple concurrent requests that all fail instantly:

```
2026-05-20T17:39:16.105565Z INFO  consensus initialized current_view=36404
2026-05-20T17:39:16.296762Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.296866Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.296939Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.297011Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
2026-05-20T17:39:16.297072Z WARN  commonware_resolver::p2p::engine: invalid data received peer=fcd86abea...
```

### The fundamental design mismatch

The Commonware resolver was designed with the assumption that block verification is **stateless** or at least **independently verifiable** -- that you can verify any block in isolation given only the block data itself. This assumption holds for some blockchain designs (e.g., those using finality certificates or validity proofs), but it does NOT hold for EVM-based chains like Kora.

EVM block verification is **sequential**: verifying block N requires the complete state after block N-1. This state is not included in the block -- it must be computed by executing all prior blocks in order. When the resolver sends blocks for verification, it expects a simple pass/fail answer. Kora's `verify_block` returns `false` when it lacks the prerequisite state, but this "false" means "I can't verify this YET" -- not "this block is invalid." The resolver has no way to distinguish these two cases.

---

## Impact

### Any node restart is potentially fatal

In the default 4-validator Simplex cluster (3/4 threshold for quorum):

- **1 restart**: The restarted node blocks all peers and gets stuck. The remaining 3 validators still have quorum (3/4) and can continue. But the restarted node is permanently left behind and effectively lost from consensus.
- **2 restarts**: Two nodes are stuck, only 2 remain. Quorum is lost (2/4 < 3/4). **The entire chain is permanently dead.** No blocks can be finalized. No recovery is possible without manual intervention.

This is confirmed by production testing: after restarting node3, it remained stuck at height 25,348 while the other 3 nodes advanced to ~26,029 (gap growing at ~5.7 blocks/second). In a subsequent test with two restarts (nodes 2 and 3), quorum was lost and the block rate dropped from 4.5 to 0.5-1.0 blocks/second with 80% of rounds producing no block.

### Rolling upgrades are impossible

Standard operational practice for distributed systems is rolling upgrades -- restart one node at a time. With this bug, each restarted node falls behind and never recovers. After restarting all nodes, the cluster may be in a state where no node is caught up to the latest height. Even if nodes are restarted one at a time with sufficient gaps, the first restarted node is permanently stuck and effectively lost from the cluster.

### Production failure scenario

```
Day 1: 4-validator cluster running at height 10,000
Day 2: V0 OOM-killed and restarts
        V0 blocks V1, V2, V3 -- stuck at height 10,000
        V1, V2, V3 continue (quorum of 3/4), reach height 15,000
Day 3: V1 needs security patch, restarts
        V1 blocks V0, V2, V3 -- stuck at height 15,000
        Only V2 and V3 are functional -- quorum lost (2/4 < 3/4)
        CHAIN IS PERMANENTLY DEAD
```

In production, the actual observed failure was even worse: after node0's voter actor panicked (with `"voter should not finish"` error) and auto-restarted via Docker's `restart: unless-stopped` policy, the consensus journal on tmpfs was cleared. Node0 initialized at view=1 and the resolver immediately blocked all peers. With two nodes unable to participate, quorum was permanently lost.

---

## Prometheus Metrics and Alerting

### Metric

The Commonware resolver exposes the `engine_resolver_resolver_peers_blocked` gauge metric. This tracks how many peers have been permanently blocked by the resolver.

### Alert (already configured)

From `docker/config/alerts.yml` (lines 181-188):

```yaml
# Resolver peers blocked -- catch-up impaired
- alert: ResolverPeersBlocked
  expr: engine_resolver_resolver_peers_blocked > 0
  for: 1m
  labels:
    severity: warning
  annotations:
    summary: "Node {{ $labels.instance }} has {{ $value }} blocked resolver peers"
    description: "Blocked peers cannot provide blocks for catch-up. This caused permanent stall after node restarts."
```

### Grafana dashboard

The `kora-performance.json` dashboard (line 584) includes a panel titled "Resolver Health (Blocked Peers & Fetch Queue)" that tracks:
- `engine_resolver_resolver_peers_blocked` -- blocked peer count
- `engine_resolver_resolver_fetch_active` -- active fetch count
- `engine_resolver_resolver_fetch_pending` -- pending fetch count

The `kora-transaction-flow.json` dashboard (line 217) has a dedicated "Resolver Blocked Peers" panel.

---

## Workarounds

### Full cluster restart (only reliable option today)

The only way to reliably recover from this state is to stop ALL validators simultaneously and restart them together. When all nodes restart at the same time, they all have the same finalized height (restored from their local archives), and no catch-up is needed. The cluster resumes consensus from the common height.

This is obviously unacceptable for production use. It requires coordinated downtime and is not automatable without risk.

### Manual peer unblocking (not currently possible)

There is no API, CLI command, or mechanism to unblock a peer that has been blocked by the resolver. The blocking is permanent for the lifetime of the process.

---

## Proposed Fixes

### Fixes at the Kora level

#### 1. Trust finality certificates during catch-up

During catch-up, blocks come with finality certificates (2/3+ threshold signatures). These certificates are cryptographic proof that the block was finalized by the network. The node should accept finality-certified blocks without requiring full EVM re-execution during catch-up. The EVM execution can happen later in the `FinalizedReporter` pipeline.

```rust
// In verify() or verify_block():
async fn verify_block(&self, block: &Block, has_finality_cert: bool) -> bool {
    // If we have a finality certificate, trust it during catch-up.
    // The FinalizedReporter will re-execute and verify state roots later.
    if has_finality_cert {
        // Optionally: store a placeholder snapshot or mark for later verification
        return true;
    }

    // Normal verification path for live consensus (no finality cert yet)
    // ... existing code ...
}
```

#### 2. Pre-populate snapshot cache on startup

During `recover_finalized_state()` (runner.rs lines 170-228), the current code iterates through the finalized block archive and calls `restore_persisted_snapshot` only for the last block (line 218-219):

```rust
if let Some(head) = head {
    ledger.restore_persisted_snapshot(&head).await;
    // ...
}
```

This restores a snapshot keyed by `head.commitment()`. When `verify_block(head+1)` is later called by the resolver, it looks up `parent_snapshot(head+1.parent())` which equals `head.commitment()`. In theory this should succeed. However, if the resolver requests blocks non-sequentially or if there are gaps, the parent may not be available.

Ensure the restored snapshot is correctly queryable and add a debug assertion:

```rust
if let Some(head) = head {
    ledger.restore_persisted_snapshot(&head).await;

    // Verify the snapshot is queryable as a parent for the next block
    let digest = head.commitment();
    debug_assert!(
        ledger.query_state_root(digest).await.is_some(),
        "restored snapshot must be queryable for catch-up to work"
    );
    info!(height = head.height, ?digest, "restored head snapshot for catch-up");
}
```

If the resolver requests blocks beyond head+1, pre-populating more snapshots would be needed, but this requires re-executing blocks sequentially from the archive (adding startup latency).

#### 3. Return "retry later" instead of "invalid"

Modify `verify_block` to distinguish between "invalid block" and "cannot verify yet":

```rust
async fn verify_block(&self, block: &Block) -> VerifyResult {
    let digest = block.commitment();
    let parent_digest = block.parent();

    if self.ledger.query_state_root(digest).await.is_some() {
        return VerifyResult::Valid;
    }

    let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
        warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
        // Return RetryLater instead of Invalid
        return VerifyResult::RetryLater;
    };
    // ... rest of verification ...
}
```

This requires changes to the Commonware `VerifyingApplication` trait to support a tri-state return value.

### Fixes at the Commonware level

#### 4. Do not permanently block on verification failure

The resolver should not permanently block a peer on the first verification failure. Instead, use a backoff strategy:

- First failure: retry after 1 second
- Second failure: retry after 5 seconds
- Third failure: retry after 30 seconds
- Persistent failure (10+ attempts): temporarily block for 5 minutes, then retry

This allows transient issues (like a cold snapshot cache) to resolve themselves.

#### 5. Support sequential catch-up

The resolver should understand that some applications require sequential block verification. Instead of sending all blocks at once and expecting independent verification, it should support a mode where:

1. Send block N
2. Wait for verification result
3. If verified, send block N+1
4. If "retry later", wait and retry block N

#### 6. Provide an unblock mechanism

Add an API to the resolver or oracle to unblock previously blocked peers:

```rust
trait Blocker {
    fn block(&self, peer: &PublicKey);
    fn unblock(&self, peer: &PublicKey);  // new
    fn is_blocked(&self, peer: &PublicKey) -> bool;  // new
}
```

---

## Reproduction Steps

### Using the Docker devnet (4 validators)

1. Start the 4-validator devnet:
   ```bash
   cd docker
   just reset
   just trusted-devnet
   ```

2. Wait for the chain to produce blocks and reach steady state. Check with:
   ```bash
   just stats
   ```
   Wait until `Blocks/s` shows a stable rate (typically 50-150 b/s). Alternatively, check that at least 100 blocks have been finalized:
   ```bash
   curl -s http://localhost:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result.finalizedCount'
   ```

3. Restart one validator (node0):
   ```bash
   docker restart kora-devnet-validator-node0-1
   ```
   (The container name may vary by compose project name. Use `docker compose -f compose/devnet.yaml ps` to find the exact name.)

4. Observe the restarted validator's logs:
   ```bash
   docker logs -f kora-devnet-validator-node0-1
   ```

   Expected log output (within milliseconds of restart):
   ```
   WARN missing parent snapshot digest=0xabc... parent_digest=0xdef... height=101
   ```
   Followed by resolver warnings:
   ```
   WARN commonware_resolver::p2p::engine: invalid data received peer=...
   ```

5. Check the blocked peers metric on the restarted node (metrics port 9000 for node0):
   ```bash
   curl -s http://localhost:9000/metrics | grep peers_blocked
   ```

   Expected output:
   ```
   engine_resolver_resolver_peers_blocked 3
   ```
   (3 blocked peers = all other validators in the 4-validator cluster)

6. Verify the node is stuck -- its finalized height never advances:
   ```bash
   # Query node0's status twice with a 10-second gap
   curl -s http://localhost:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result.finalizedCount'
   # Wait 10 seconds
   sleep 10
   curl -s http://localhost:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result.finalizedCount'
   ```
   Both should show the same number -- the restarted node's height is frozen.

7. Verify the other 3 validators are still advancing:
   ```bash
   for port in 8546 8547 8548; do
     echo -n "Port $port: "
     curl -s http://localhost:$port -X POST \
       -H "Content-Type: application/json" \
       -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq -r '.result.finalizedCount'
   done
   ```
   These should show increasing values (the remaining 3 nodes maintain quorum).

### Recovery after reproduction

The only working recovery is a full cluster restart:
```bash
cd docker
just reset
just trusted-devnet
```

### Automated e2e test

Using the existing test harness at `crates/e2e/src/harness.rs`:

1. Start a 4-node cluster
2. Submit transactions and wait for 50+ finalized blocks
3. Simulate a node restart by:
   a. Dropping the node's consensus engine
   b. Clearing its in-memory snapshot store
   c. Re-running `recover_finalized_state()` to simulate startup
   d. Re-initializing consensus with the resolver
4. Assert that the restarted node catches up to the cluster's current height within 30 seconds
5. Assert that `engine_resolver_resolver_peers_blocked` remains at 0

Currently, step 4 will fail -- the node will never catch up, and step 5 will show 3 blocked peers (all other validators).

---

## Testing Plan

### Unit tests

1. **Test verify_block with empty snapshot cache**: Call `verify_block` when `parent_snapshot()` returns `None`. Currently returns `false`. After fix, should return a retriable error or `true` (if finality-cert path is used).

2. **Test snapshot restoration covers parent lookup**: After calling `restore_persisted_snapshot(&block_H)`, call `parent_snapshot(digest_of_H)` from the context of block H+1. Verify it returns `Some`.

3. **Test verify() handles sequential dependencies**: Create a chain of 5 blocks. Start with only the genesis snapshot. Call `verify()` with blocks 1-5. Verify that either all blocks are verified successfully (sequential execution) or the method signals "retry" rather than "invalid."

### Integration tests

4. **Restart recovery e2e**: Full integration test as described in the reproduction steps above. Start cluster, produce blocks, restart one node, assert catch-up completes.

5. **Rolling restart e2e**: Restart nodes one at a time. After each restart, wait for catch-up. Verify no peers are blocked and the cluster maintains liveness.

6. **Double restart stress test**: Restart two of four validators (sequentially, with a gap). Verify the cluster recovers and both restarted nodes catch up.

### Observability

7. Add a metric `kora_catchup_verify_skip_total` that counts how many times `verify_block` returns `false` due to a missing parent snapshot (vs. actual invalid block detection). This allows operators to distinguish between the two failure modes.

8. Add structured logging with distinct error codes:
   ```
   warn!(code = "MISSING_PARENT", ?digest, ?parent_digest, height, "cannot verify block: parent snapshot not available");
   ```
   vs:
   ```
   warn!(code = "INVALID_BLOCK", ?digest, expected = ?block.state_root, computed = ?state_root, "block verification failed: state root mismatch");
   ```

---

## Verification Steps

After implementing the fix, verify correctness with these checks:

1. **Single node restart recovery**: Start the 4-validator devnet, wait for 100+ blocks, restart one node. Verify that within 30 seconds the restarted node's `finalizedCount` starts advancing again and matches the other nodes.

2. **Blocked peers metric stays zero**: After the restart, check `engine_resolver_resolver_peers_blocked` on the restarted node. It should remain at 0 (or return to 0 after successful catch-up).

3. **Rolling restart**: Restart all 4 validators one at a time, with 30 seconds between each restart. After all are restarted, verify all 4 nodes are at the same height and producing blocks.

4. **Double restart stress test**: Restart 2 of 4 validators simultaneously. After both restart, verify both catch up and the cluster maintains liveness.

5. **Code review of verify_block**: Verify that the `verify_block` method at `crates/node/runner/src/app.rs:166` no longer returns `false` when the parent snapshot is missing. It should either return a retriable signal or accept finality-certified blocks without re-execution.

6. **Log check**: After a successful restart-and-recovery, verify logs do NOT contain `commonware_resolver::p2p::engine: invalid data received` messages. The `"missing parent snapshot"` warning should either not appear or be followed by successful verification after the snapshot is available.

---

## Load Test Evidence (2026-05-22)

During a 1,000-tx load test against a fresh 4-validator devnet, the resolver panicked on node0:

```
ERROR commonware_runtime::utils::handle: task panicked err="resolver should not finish"
```

After Docker auto-restarted node0 (`restart: unless-stopped`), the node came back online but the block builder entered a permanent failure loop. The pool retained ~195 stale txs from before the crash, all with nonces ahead of the reset on-chain state (nonce 0). Every `build_block` attempt failed with:

```
WARN build_block: execution failed height=288 txs=195 error=TxExecution("Transaction(NonceTooHigh { tx: 24, state: 0 })")
```

This confirms the failure cascade: resolver panic → node restart → stale pool → block builder stuck → no blocks finalized. The "resolver should not finish" panic is distinct from the previously documented "voter should not finish" but follows the same pattern of a long-lived actor terminating unexpectedly.

---

## Related Files

| File | Role |
|------|------|
| `crates/node/runner/src/app.rs` | `verify_block()` (lines 166-246) -- returns `false` on missing parent snapshot at line 178 |
| `crates/node/runner/src/app.rs` | `VerifyingApplication::verify()` (lines 321-383) -- iterates ancestry and calls `verify_block` |
| `crates/network/marshal/src/peers.rs` | `PeerInitializer::init()` -- resolver initialization with blocker (lines 52-82) |
| `crates/node/runner/src/runner.rs` | Resolver wiring with oracle as both provider AND blocker (lines 492-498) |
| `crates/node/runner/src/runner.rs` | `recover_finalized_state()` -- startup recovery (lines 170-228) |
| `crates/node/runner/src/runner.rs` | `restore_persisted_snapshot()` call (line 219) |
| `crates/node/ledger/src/lib.rs` | `restore_persisted_snapshot()` implementation (lines 239-252) |
| `crates/node/ledger/src/lib.rs` | `LedgerView::parent_snapshot()` lookup (lines 212-215); `LedgerService::parent_snapshot()` delegation (lines 413-415) |
| `crates/node/ledger/src/lib.rs` | `LedgerView::query_state_root()` -- used by `verify_block` to check if already verified (lines 188-191) |
| `crates/node/reporters/src/lib.rs` | `handle_finalized_update` -- where finalized blocks are processed (related to issue #03) |
| `docker/config/alerts.yml` | `ResolverPeersBlocked` alert (lines 181-188) |
| `docker/grafana/dashboards/kora-performance.json` | Resolver health dashboard panel (line 584) |
| `docker/grafana/dashboards/kora-transaction-flow.json` | Blocked peers panel (line 217) |
| `crates/e2e/src/harness.rs` | E2E test harness for integration tests |
