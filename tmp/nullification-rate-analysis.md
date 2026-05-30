# Nullification Rate Analysis

## Background: What Is Kora?

Kora is an EVM-compatible blockchain that uses **Simplex BFT consensus** (from the Commonware library) for block finalization. In Kora's architecture:

- A fixed validator set (typically 4 nodes) participates in consensus.
- A **leader** is elected per view using VRF-based random election.
- The leader builds a block from the mempool, proposes it, and waits for 2/3+ votes.
- If 2/3+ validators notarize and then finalize the block, it becomes part of the canonical chain.
- The system advances through sequential **views** (consensus rounds).

Kora uses QMDB as its state database, REVM for EVM execution, a threshold BLS12-381 VRF for leader election and randomness, and Ed25519 for peer identity.

---

## What Is a Nullification?

A **nullification** is a consensus view that completes without producing a finalized block. When a view fails, the voter actor in Simplex broadcasts a "nullification floor" message to peers, signaling that the view should be abandoned and the network should advance to the next one.

```
WARN commonware_consensus::simplex::actors::voter::actor: broadcasting nullification floor floor=Finalization(...)
```

Nullifications are tracked in Kora's `NodeStateReporter`, which increments the counter on every `Activity::Nullification` event from the consensus engine:

```rust
// crates/node/reporters/src/lib.rs:468-470
Activity::Nullification(_) => {
    self.state.inc_nullified();
}
```

The counter is exposed via both the RPC endpoint (`nullifiedCount` in `NodeStatus`) and Prometheus (`engine_voter_state_nullifications_total`).

A nullified view is wasted consensus capacity. The network must still complete the timeout cycle, communicate the failure to peers, and advance to the next view before trying again.

---

## All Causes of Nullification

### 1. Leader Timeout (No Proposal in Time)

The leader has a fixed window to build and propose a block. In production, this is configured to **2 seconds** (`CONSENSUS_LEADER_TIMEOUT`). If the leader does not produce a valid proposal before the timeout expires, other validators timeout and nullify the view.

```rust
// crates/node/runner/src/runner.rs:48-50
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_secs(2);
const CONSENSUS_CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(4);
const CONSENSUS_TIMEOUT_RETRY: Duration = Duration::from_secs(1);
```

Leader timeout can be caused by:
- Slow block building (ECDSA recovery on every transaction during `mempool.build()`)
- Slow state root computation (QMDB merkle root calculation)
- The leader node being overloaded or in a bad state

### 2. Executor Abort (Block Build Failure)

When the leader builds a block, it executes all transactions against the parent state. If **any single transaction** causes an execution error, the entire block build fails and the leader returns `None` (no proposal):

```rust
// crates/node/runner/src/app.rs:125-136
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        warn!(parent = ?parent_digest, height, txs = txs.len(), error = ?err, "build_block: execution failed");
        return None;
    }
};
```

The executor uses the `?` operator for each transaction, meaning a single bad transaction aborts processing of the entire block:

```rust
// crates/node/executor/src/revm.rs:391-395
let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;  // Abort on decode error
let result_and_state = evm.replay().map_err(|e| ExecutionError::TxExecution(...))?;  // Abort on exec error
```

Transaction failure types that trigger abort:
- `ExecutionError::TxDecode` - malformed RLP or invalid signature recovery
- `ExecutionError::TxExecution` - REVM execution halted (invalid nonce, insufficient balance at execution time, etc.)
- `ExecutionError::State` - state database access failure

### 3. Invalid Block Proposal (Verification Failure)

When a non-leader validator receives a proposed block, it re-executes all transactions and verifies the state root matches. If verification fails, the validator votes to nullify:

```rust
// crates/node/runner/src/app.rs:210-218
if state_root != block.state_root {
    warn!(?digest, expected = ?block.state_root, computed = ?state_root, "state root mismatch");
    return false;
}
```

This causes nullification if 2/3+ validators reject the proposal.

### 4. Missing Parent Snapshot

Both block building and verification require the parent block's state snapshot. If the snapshot is missing (e.g., after a restart or during catch-up), the operation fails:

```rust
// crates/node/runner/src/app.rs:89
let parent_snapshot = self.ledger.parent_snapshot(parent_digest).await?;
```

When `parent_snapshot()` returns `None`, the leader cannot build a block and the view nullifies.

### 5. State Root Computation Failure

Even if execution succeeds, the leader must compute a merkle root from the resulting state changes. If this fails, the block build returns `None`:

```rust
// crates/node/runner/src/app.rs:141-145
let state_root = self.ledger
    .compute_root_from_store(parent_digest, outcome.changes.clone())
    .await
    .ok()?;  // Returns None on failure
```

### 6. Quorum Loss (Insufficient Voters)

If fewer than 2/3 of validators are participating (e.g., nodes are down or their voter actors have crashed), no proposal can achieve certification even if the leader produces a valid block. The certification timeout expires and the view nullifies.

---

## The Poisoned Mempool Cascade

The most dangerous nullification pattern is the **poisoned mempool loop**, which causes sustained high nullification rates:

```
Load test sends many transactions
    |
Some become stale (nonce consumed by finalized blocks)
    |
InMemoryMempool accepts them (no nonce re-validation on insert)
    |
Leader proposes block including stale transactions
    |
Executor hits NonceTooLow / decode error -> ? operator -> abort -> return None
    |
View nullified, failing transactions NOT pruned from mempool
    |
Next leader draws same stale transactions -> same failure
    |
Repeat indefinitely: ~33% of views fail this way
```

The key architectural gaps enabling this cascade:

1. **InMemoryMempool has no nonce awareness** - It stores transactions by hash in a BTreeMap with no per-sender ordering, size limits, or expiry (`crates/node/consensus/src/components/mempool.rs`).

2. **Mempool pruning only happens on successful finalization** - The `FinalizedReporter` calls `state.prune_mempool(&block.txs)` only after a block is persisted successfully (`crates/node/reporters/src/lib.rs:226`). Failed blocks never trigger pruning.

3. **Transaction validation checks QMDB persisted state only** - The `TransactionValidator` checks nonces against the committed state database (`crates/node/txpool/src/validator.rs:92-96`) but not against pending/unfinalized blocks.

---

## Nullification Rate and Chain Health

### Healthy Baseline

From production monitoring data (idle chain, 4 validators):

| Metric | Value |
|--------|-------|
| Block rate | ~4.5 blocks/sec (~0.22s/block) |
| View advancement | ~6 views/sec |
| Nullification rate | ~1.5/sec (idle views when no new blocks) |
| Effective efficiency | ~75% of views produce blocks |

Some nullification at idle is expected: if the mempool is empty and no transactions arrive during a leader's turn, the leader may propose an empty block (which succeeds) or the view may timeout waiting for block propagation. The baseline nullification overhead exists because of timing jitter between leader timeout and network propagation.

### Moderate Load

Under moderate transaction load (hundreds of tx/sec), nullification rate typically stays stable or improves because:
- Every leader has transactions to include
- Blocks build quickly (small batches, fast ECDSA recovery)
- State root computation is fast with small change sets

### Heavy Load Degradation

Under heavy load (thousands of tx/sec), nullification rate spikes because:
- ECDSA recovery during `mempool.build()` takes longer (O(n) over mempool)
- Stale transactions accumulate faster than finalization
- Block build time approaches the 2-second leader timeout
- A critical mass of bad transactions poisons every leader's proposal

### Observed Production Data

From the reproduction test (Test 2: two-node restart scenario):

**Baseline (3 healthy validators, 1 already failed):**
```
net_rate = 4.0-4.5 blk/s (~0.22-0.25 s/block)
node0: null=4723 -> 4765 over 16s = 2.6 nullifications/s
views advanced: 37600 -> 37703 over 16s = 6.4 views/s
nullification ratio: (6.4 - 4.5) / 6.4 = ~30%
```

**Post-restart (degraded, node3 not catching up):**
```
net_rate = 0.5-1.0 blk/s (~1-2 s/block)
node0: null=4797 -> 5249 over 180s = 2.5 nullifications/s
views advanced: 37788 -> 38201 over 180s = 2.3 views/s
nullification ratio: (2.3 - 0.7) / 2.3 = ~70%
```

**Complete stall (quorum lost):**
```
net_rate = 0.0 blk/s
views: frozen at 37788
nullification count: not advancing (consensus dead)
```

### Timeline: Healthy to Degraded to Stalled

1. **Healthy** (0-10 min): 4.5 blk/s, ~30% nullification, normal operation
2. **Degrading** (10-15 min): Stale transactions accumulate, nullification rises to 50%+
3. **Severely Degraded** (15-20 min): 0.5-1.0 blk/s, 70%+ nullification, only lucky proposals succeed
4. **Stalled** (20+ min): Quorum lost (nodes crash/fail), 0 blocks/s, consensus dead

---

## Critical Thresholds

| Nullification Ratio | Meaning | Action |
|---------------------|---------|--------|
| < 20% | Healthy | Normal operation |
| 20-35% | Elevated | Check if mempool is growing, monitor trend |
| 35-50% | Warning | Likely executor failures. Check logs for `build_block: execution failed` |
| 50-70% | Critical | Chain is losing throughput fast. Identify and purge bad transactions |
| 70-90% | Emergency | Chain near-stalled. Only occasional lucky proposals succeed |
| > 90% + finalization = 0 | Dead | Complete stall. Requires intervention (restart, mempool clear) |

The critical indicator is not the absolute nullification rate but the **ratio of nullified views to total views**:

```
nullification_ratio = 1 - (finalization_rate / view_advancement_rate)
```

---

## Prometheus Queries for Diagnosis

### Overall Nullification Rate

```promql
# Nullifications per second across all nodes
sum(rate(engine_voter_state_nullifications_total[5m]))

# Nullification ratio (fraction of wasted views)
1 - (avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m])))
```

### Per-Node Breakdown

```promql
# Which nodes are seeing the most nullifications?
rate(engine_voter_state_nullifications_total[5m])

# Per-node view rate vs finalization rate
rate(engine_voter_state_current_view[1m])
rate(finalized_height[1m])
```

If all nodes nullify at similar rates, the problem is systemic (mempool poisoning, executor bug). If one node is much higher, that specific node is struggling (slow disk, network issues, crashed voter).

### Timeout Analysis

```promql
# Timeout breakdown by reason
sum by (reason) (rate(engine_voter_state_timeouts_total[5m]))
```

Timeout reasons:
- `MissingProposal` = leader did not propose in time
- `LeaderNullify` = leader explicitly nullified
- `LeaderTimeout` = leader was too slow to build

### Detecting the Stall Pattern

```promql
# Chain is stalled: views advancing but no blocks finalized with high nullification
sum(rate(engine_voter_state_current_view[2m])) > 0
  and sum(rate(finalized_height[2m])) == 0
  and sum(rate(engine_voter_state_nullifications_total[2m])) > 5
```

### Consensus Efficiency Over Time

```promql
# Efficiency: what fraction of views produce finalized blocks
avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m]))
```

### View Drift Between Nodes

```promql
# If nodes have divergent views, some may be stuck
max(engine_voter_state_current_view) - min(engine_voter_state_current_view)
```

---

## Relationship to Other Bugs

### Executor Abort

The primary driver of nullifications under load. A single invalid transaction (bad nonce, decode failure, execution halt) causes the `?` operator in `RevmExecutor::execute()` to abort the entire block. No skip-and-continue logic exists.

### Mempool Poisoning

Bad transactions enter the mempool and are never removed because pruning only happens on successful finalization. This creates a permanent nullification loop: every leader draws the same bad transactions, builds a block that fails, and the view nullifies without pruning.

### Height Drift

When nodes restart and fail to catch up (due to the Commonware resolver bug that blocks peers on verification failure), they cannot participate in consensus. This reduces the effective quorum and can cause nullifications if the remaining nodes barely maintain 2/3+ threshold.

### Voter Crash

If a node's voter actor panics (the "voter should not finish" crash), that node stops participating in consensus entirely. With 4 validators, losing one node still allows consensus (3/4 > 2/3), but losing two causes permanent stall since 2/4 < 2/3.

---

## Configuration Reference

| Parameter | Production Value | Source |
|-----------|-----------------|--------|
| Leader timeout | 2s | `CONSENSUS_LEADER_TIMEOUT` in runner.rs |
| Certification timeout | 4s | `CONSENSUS_CERTIFICATION_TIMEOUT` in runner.rs |
| Timeout retry interval | 1s | `CONSENSUS_TIMEOUT_RETRY` in runner.rs |
| Activity timeout | 256 views | `CONSENSUS_ACTIVITY_TIMEOUT` in runner.rs |
| Skip timeout | 32 views | `CONSENSUS_SKIP_TIMEOUT` in runner.rs |
| Nullify retry interval | 5s (default) | `DEFAULT_NULLIFY_RETRY` in simplex/config.rs |
| Max transactions per block | 10,000 | `BLOCK_CODEC_MAX_TXS` in runner.rs |
| Max TX bytes per block | 8 MiB | `BLOCK_CODEC_MAX_TX_BYTES` in runner.rs |
| Fetch timeout | 1s | `CONSENSUS_FETCH_TIMEOUT` in runner.rs |
| Forwarding policy | SilentLeader | runner.rs:571 |

---

## Fix Priority

1. **Replace `?` with skip-and-continue in executor** - A single bad transaction should not abort the entire block. Skip the failing tx and continue with the rest.

2. **Wire the full TransactionPool** - The `TransactionPool` in `crates/node/txpool` has per-sender nonce tracking, gas-price prioritization, and size limits. It is not used in production; only the primitive `InMemoryMempool` (a plain BTreeMap) is wired into the consensus application.

3. **Prune on failed proposals** - When block building fails, identify the offending transactions and remove them from the mempool so the next leader does not hit the same failure.

4. **Add pending-state nonce validation** - Check mempool + pending (unfinalized) blocks when admitting new transactions, not just persisted QMDB state.

5. **Cache ECDSA recovery results** - The `tx_order_key()` function in `InMemoryMempool` calls `recover_signer()` on every call to `build()`, making block building O(n) in mempool size.
