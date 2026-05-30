# Production Failure Timeline: Permanent Chain Stall Under Load

## What Is Kora

Kora is a blockchain node implementation that uses BFT (Byzantine Fault Tolerant) consensus to produce and finalize blocks containing EVM transactions. The system consists of:

- **Validators**: Nodes that participate in consensus by proposing blocks and voting on proposals. A quorum of 3 out of 4 validators is required to finalize a block.
- **Simplex consensus engine** (from the Commonware framework): Manages leader election, block proposals, voting rounds, and finality.
- **EVM executor** (REVM): Processes transactions within proposed blocks using the Ethereum Virtual Machine (Cancun specification).
- **Mempool** (`InMemoryMempool`): An in-memory BTreeMap that stores pending transactions awaiting inclusion in blocks.
- **Threshold cryptography** (BLS12-381): Validators hold secret shares of a group key and produce threshold signatures to finalize blocks.
- **QMDB**: Persistent state database for account balances, nonces, and contract storage.
- **Commonware journals**: Write-ahead logs for consensus state, stored on tmpfs in devnet configuration.

---

## Production Architecture

The devnet runs as a Docker Compose stack (`docker/compose/devnet.yaml`) containing:

| Component | Count | Purpose |
|-----------|-------|---------|
| Validator nodes | 4 | Consensus participation, block production |
| Secondary node | 1 | Non-voting follower for read-only queries |
| Prometheus | 1 | Metrics collection (10-second scrape interval) |
| Grafana | 1 | Dashboard visualization |
| Loki + Promtail | 2 | Log aggregation |

Each validator exposes:
- P2P port (30400-30403): Inter-validator communication
- RPC port (8545-8548): JSON-RPC API for transaction submission
- Metrics port (9000-9003): Prometheus scrape endpoint

Key configuration constants (from `crates/node/runner/src/runner.rs`):
- `BLOCK_CODEC_MAX_TXS`: 10,000 transactions per block
- `CONSENSUS_LEADER_TIMEOUT`: 2 seconds (time allowed for a leader to propose)
- `CONSENSUS_CERTIFICATION_TIMEOUT`: 4 seconds (time for vote collection)
- `CONSENSUS_ACTIVITY_TIMEOUT`: 256 views (before a node is considered inactive)
- `EPOCH_LENGTH`: u64::MAX (single epoch, no validator rotation)

---

## Baseline Performance (Healthy Chain)

Measured on Hetzner dedicated server (65.21.232.29) with zero transaction load:

| Metric | Value |
|--------|-------|
| Block time | 9.28ms |
| Blocks/sec | 107.8 |
| Views/sec | ~148 |
| Consensus efficiency | 73.2% |
| Skip rate (idle nullification) | 26.8% |
| Block build duration (p95) | 0.030ms |
| Finalization latency (p95) | 47.5ms |
| Memory per node | 200-400 MB |

Even at idle, approximately 26% of consensus views produce no finalized block. This is an existing baseline inefficiency unrelated to the failure described below.

---

## The Failure: Detailed Timeline

### T+0: Chain Running Healthy

The chain is operating at baseline performance after a fresh deployment. All four validators are healthy, producing blocks at approximately 107 blocks/sec with 73% consensus efficiency. The mempool on each node is empty or contains only recently submitted transactions.

**Metrics at T+0:**
- `kora:blocks_per_sec` = 107.8
- `kora:consensus_efficiency` = 0.73
- `kora:nullification_rate` = ~40/sec (baseline idle nullifications)
- `kora:height_drift` = 0
- `finalized_height` = advancing continuously

---

### T+1: Heavy Load Test Started

A load generator submits 50,000 transactions with the following parameters:
```
--total-txs 50000
--accounts 50
--concurrency 200
--rpc-url http://127.0.0.1:8545
--broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

Critical characteristics of this load:
1. **High concurrency (200)**: Many transactions from the same account are in-flight simultaneously.
2. **Broadcast to all validators**: Every transaction is sent to all four RPC endpoints, meaning all four mempools receive copies.
3. **Nonce assignment via `fetch_add(1, Relaxed)`**: The loadgen assigns sequential nonces but sends them asynchronously. Out-of-order delivery is guaranteed at this concurrency level.

Of 50,000 attempted submissions:
- 12,851 (25.7%) accepted by mempools
- 37,149 (74.3%) rejected as duplicates (same tx hash already in BTreeMap)

---

### T+2: Efficiency Begins Dropping, Nullification Rate Rising

As transactions accumulate in the mempool, some nonces become stale. This happens because:
1. Loadgen assigns nonce N, then nonce N+1 to the same account
2. Nonce N+1 is delivered and finalized before nonce N arrives at the validator
3. When nonce N is finally processed, the account's on-chain nonce has already advanced past it
4. Transaction with nonce N is now permanently invalid (NonceTooLow)

These stale transactions cannot be detected by `InMemoryMempool` because it performs no nonce validation -- it is simply a hash-keyed BTreeMap.

**Metrics at T+2:**
- `kora:blocks_per_sec` = declining from 93 toward 0
- `kora:nullification_rate` = 55+ per second (rising from baseline 40)
- `kora:consensus_efficiency` = dropping below 0.5

---

### T+3: Stale Transactions Accumulate in Mempools Across Validators

Because the loadgen broadcasts every transaction to all four validators, all four mempools contain the same set of stale transactions. When a leader proposes a block, `mempool.build(max_txs, &excluded)` returns a transaction set that includes stale transactions.

The executor processes this set:
```rust
// crates/node/executor/src/revm.rs:388-411
for tx_bytes in txs {
    let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;  // Line 391
    evm.set_tx(tx_env);
    let result_and_state = evm.replay()
        .map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;  // Line 395
    // ...
}
```

When the executor encounters a stale transaction (NonceTooLow), `evm.replay()` returns an error. The `?` operator on line 395 propagates this error immediately, aborting the entire block execution. No remaining transactions in the block are processed.

The calling code in `app.rs` handles this:
```rust
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        warn!("build_block: execution failed");
        return None;  // ENTIRE BLOCK PROPOSAL ABORTED
    }
};
```

`propose()` returns `None`, signaling to Simplex that the leader has no block to propose. The view is nullified.

---

### T+4: Every Leader Proposal Includes Stale Transactions, Executor Aborts ALL Proposals

This is where the failure becomes permanent. The critical insight is:

1. **Stale transactions are NOT removed from the mempool after a failed proposal.** Mempool pruning (`prune_mempool()`) only occurs after successful block finalization (in `FinalizedReporter::report()`).
2. **Since no blocks are being finalized, no pruning ever occurs.**
3. **All four validators have the same stale transactions** (due to broadcast-to-all).
4. **Whoever is elected leader next will propose a block containing the same stale transactions.**
5. **The executor will abort again, producing another `None` proposal.**

This creates an infinite loop:
```
Leader proposes block → includes stale tx → executor errors → proposal = None
→ view nullified → next leader → same stale txs → same failure → forever
```

**Metrics at T+4:**
- `kora:blocks_per_sec` = 0
- `kora:nullification_rate` = 55-80/sec (every view nullified)
- `kora:consensus_efficiency` = 0%
- `kora:height_drift` = 0 (all nodes equally stuck)
- `engine_voter_state_current_view` = still advancing (views progress, nothing finalizes)

---

### T+5: Chain Permanently Stalled

The chain is now in a terminal state. Views continue advancing (the consensus protocol still runs rounds), but no block will ever finalize again. The view counter reached 19,698 before the voter actor panicked.

In the remote production failure, the chain ran in this degraded state for approximately 25 hours before complete seizure. During that period:
- Nodes 1 and 2 finalized 1,857,595 blocks but accumulated 937,061 nullifications (33.5% nullification rate)
- The nullification rate represents the fraction of views where the leader's proposal was aborted due to stale mempool transactions

Eventually, node 0's voter actor panicked:
```
ERROR commonware_runtime::utils::handle: task panicked err="voter should not finish"
```

After the panic and automatic restart (Docker `restart: unless-stopped`):
- Node 0 restarted with an empty consensus journal (tmpfs was cleared)
- Consensus initialized at view=1
- The Commonware resolver attempted to fetch missing blocks from peers
- `verify_block()` failed due to missing parent snapshots (sequential state dependency)
- The resolver permanently blocked those peers ("invalid data received")
- Node 0 could never catch up

With nodes 0 and 3 unable to participate (both crashed and failed to recover), only 2 of 4 validators remained active. Since BFT quorum requires 3 of 4, no further blocks could be finalized. The chain was permanently dead.

**Final metrics at discovery (after ~25 hours):**
- `finalized_height` = 1,857,595 (frozen)
- `engine_voter_state_current_view` = 2,794,655 (frozen)
- Nullification rate (historical) = 937,061 / 2,794,655 = 33.5%
- Peers reported = 0 (reporting bug; P2P was actually connected until crash)
- All nodes at same view = consensus completely stalled

---

### T+6: Only Recovery Is Full State Reset

The chain cannot self-recover. The stale transactions will never be removed from the mempool because:
- No blocks finalize (so `prune_mempool()` never runs)
- There is no TTL (time-to-live) on mempool entries
- There is no mempool expiration or size limit
- Restarting individual nodes does not help (resolver catch-up bug prevents sync)
- The in-memory mempool IS cleared on restart, but the resolver bug prevents restarted nodes from rejoining consensus

The only recovery path is a full cluster reset that destroys all runtime state:
```bash
docker compose -f compose/devnet.yaml --profile observability --profile interactive-dkg down -v
```

---

## Prometheus Alerts That Would Fire

Based on the alert rules in `docker/config/alerts.yml`, the following alerts would activate during this failure:

### Critical Alerts

| Alert | Trigger Condition | When It Fires |
|-------|-------------------|---------------|
| **MempoolPoisoning** | Views advancing + 0 finalization + nullifications > 5/s | T+4 (after 1 minute of zero finalization with active nullifications) |
| **AllLeadersFailing** | Nullifications > 20/s + 0 finalization for 30s | T+4 (within 30 seconds of all proposals failing) |
| **EfficiencyCliff** | Efficiency < 10% AND was > 50% five minutes ago | T+4 (efficiency drops from 73% to 0%) |
| **ConsensusStall** | `rate(finalized_height[5m]) == 0` while node is up | T+4 (after 2 minutes of zero finalization) |
| **VoterCrash** | Node up but view not advancing | After voter panic at T+5 |

### Warning Alerts

| Alert | Trigger Condition | When It Fires |
|-------|-------------------|---------------|
| **HighNullificationRate** | Nullifications > 5/sec for 2 minutes | T+2 (early warning as stale txs begin causing failures) |
| **HighSkipRate** | Skip rate > 30% for 3 minutes | T+2 (skip rate exceeds 30% threshold) |
| **HighTimeoutRate** | Timeouts > 5/sec for 2 minutes | T+3 (leaders timing out on failed proposals) |
| **ViewWithoutFinalization** | Views advancing but no finalization for 3 min | T+4 (views advance, blocks never finalize) |
| **LowConsensusEfficiency** | Efficiency < 50% for 5 minutes | T+3 (efficiency drops below 50%) |
| **ThroughputDrop** | blocks/sec < 30% of 1-hour average | T+3 (throughput collapses) |
| **HeightDrift** | Max - min height > 10 blocks | After node crashes (nodes diverge) |
| **ResolverPeersBlocked** | Any blocked resolver peers | After node restart (resolver blocks peers) |
| **NodeLagging** | One node's finalization rate < 10% of average | After node restart failure |

### Alert Escalation Timeline

```
T+1:30  HighNullificationRate fires (nullifications > 5/s for 2 min)
T+2:00  HighSkipRate fires (skip > 30% for 3 min)
T+3:00  LowConsensusEfficiency fires (efficiency < 50% for 5 min)
T+3:00  ThroughputDrop fires (< 30% of 1h average for 5 min)
T+4:00  HighTimeoutRate fires (timeouts > 5/s for 2 min)
T+4:30  AllLeadersFailing fires (nullifications > 20/s, 0 finalization for 30s)
T+5:00  MempoolPoisoning fires (views advancing, 0 finalization, high nullification for 1 min)
T+5:00  EfficiencyCliff fires (instant: efficiency crashed from >50% to <10%)
T+6:00  ConsensusStall fires (0 finalization for 2 min while up)
T+7:00  ViewWithoutFinalization fires (views but no blocks for 3 min)
```

---

## Root Cause Analysis: Three Bugs Compounding

The permanent stall results from three independent bugs that combine catastrophically:

### Bug 1: Executor Fatal Abort (`?` operator)

**Location**: `crates/node/executor/src/revm.rs`, lines 391 and 395

The executor uses Rust's `?` operator inside a transaction processing loop. A single invalid transaction aborts the entire block proposal. This converts any mempool contamination into a consensus-level failure.

**Expected behavior**: Skip the bad transaction, continue processing remaining valid transactions, produce a partial block.

### Bug 2: No Mempool Pruning on Failed Proposals

**Location**: `crates/node/reporters/src/lib.rs`, line 226

`prune_mempool()` only runs after successful block finalization. If a block is never finalized (because all proposals fail), stale transactions remain in the mempool indefinitely. There is no TTL, no periodic expiration, and no maximum size limit.

**Expected behavior**: Failed transactions should be pruned after repeated proposal failures, or a TTL should expire them automatically.

### Bug 3: Broadcast Copies to All Validators

**Location**: Loadgen `--broadcast-rpc-urls` parameter

The load generator sends every transaction to all validator RPC endpoints. This ensures that all four validators have identical poisoned mempools. If only one validator had the bad transactions, other leaders could still produce blocks and the chain would survive (at reduced efficiency).

**Combined effect**: Bug 3 ensures all validators are poisoned. Bug 1 ensures any poisoned validator fails to produce blocks. Bug 2 ensures the poison is permanent. Together, they create an irrecoverable stall.

### Contributing Factor: Resolver Catch-Up Bug

**Location**: Commonware framework (`commonware_resolver::p2p::engine`)

After a validator crashes and restarts, it cannot catch up to the current chain height because:
1. The resolver requests blocks from peers
2. Block verification requires parent state snapshots
3. A freshly restarted node has no parent snapshots
4. Verification fails, and the resolver permanently blocks the peer as "malicious"
5. All peers get blocked, leaving no source for historical blocks

This bug prevents partial recovery -- even if 2 of 4 nodes have clean mempools after restart, they cannot rejoin consensus.

---

## Lessons Learned

1. **A single `?` operator in a loop can kill an entire blockchain.** Error handling in transaction processing must be granular: skip individual failures, never abort the batch.

2. **Mempools need garbage collection.** Any in-memory data structure holding untrusted input must have size limits, TTLs, or periodic eviction. The absence of all three is a ticking time bomb.

3. **Testing at 10x expected load is necessary.** The chain handles 5,000 transactions perfectly but dies at 50,000. The failure mode is not graceful degradation -- it is permanent death.

4. **Broadcast-to-all amplifies failures.** When all validators share identical state (including identical poison), there is no diversity to provide resilience. Transaction gossip protocols typically handle deduplication and validation; raw broadcast bypasses those safety mechanisms.

5. **Crash recovery must be tested independently.** The resolver catch-up failure means that any crash (even from unrelated causes) leads to permanent quorum loss. A node that cannot rejoin is worse than a node that is simply down.

6. **Alerts must be actionable.** The `MempoolPoisoning` and `AllLeadersFailing` alerts correctly identify the failure, but without an automated remediation path (no mempool flush command exists), the alerts merely confirm that manual intervention is required.

---

## What Would Prevent This in Production

### The Critical Fix: Executor Skip-and-Continue

Replace the fatal `?` operator with `match` + `continue`:

```rust
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
        Ok(env) => env,
        Err(err) => {
            warn!(?tx_hash, error = ?err, "skipping failed tx decode");
            skipped_txs.push(tx_hash);
            continue;
        }
    };

    let result_and_state = match evm.replay() {
        Ok(result) => result,
        Err(err) => {
            warn!(?tx_hash, error = ?err, "skipping failed tx execution");
            skipped_txs.push(tx_hash);
            continue;
        }
    };

    // Process successful transaction normally...
}
```

This single change transforms the failure mode from "permanent stall" to "temporary reduced throughput." Bad transactions are skipped, valid transactions in the same block are still processed, blocks still finalize, and pruning still occurs.

### Additional Prevention Measures

| Fix | Effect |
|-----|--------|
| Wire `TransactionPool` (replaces `InMemoryMempool`) | Rejects stale nonces at ingress; per-sender ordering |
| Add mempool TTL (expire entries after N seconds) | Prevents unbounded accumulation of stale transactions |
| Always prune on all error paths in `FinalizedReporter` | Prevents re-proposal of already-finalized transactions |
| Fix resolver catch-up (trust finality certificates) | Allows crashed nodes to rejoin without full re-execution |
| Add mempool size limit with eviction policy | Bounds resource usage under sustained load |
| Separate transaction-level errors from system errors | Only abort blocks on true system failures (DB errors) |
