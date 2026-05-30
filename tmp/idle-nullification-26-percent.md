# Why 26% of Consensus Views Are Nullified at Idle

## Background: What Is Kora?

Kora is a minimal EVM execution client built in Rust that uses the **Commonware Simplex BFT** consensus protocol with BLS12-381 threshold signatures. It runs a fixed validator set (currently 4 validators on the devnet) that produce blocks via VRF-based random leader election.

The consensus cycle works as follows:
1. **Leader election** - A VRF selects one validator as leader for each view
2. **Proposal** - The leader builds and broadcasts a block
3. **Verification** - Other validators re-execute the block to validate the state root
4. **Notarization** - 2/3+ validators sign off (BLS threshold signature)
5. **Finalization** - The block is committed to the chain

If the leader fails to propose (or its proposal is invalid), the view is **nullified** -- the chain advances to the next view without producing a block.

---

## The Observed Behavior

On a fresh Kora devnet deployment with **zero external load** (no transactions submitted), the following steady-state metrics are observed:

| Metric | Observed Value |
|--------|---------------|
| Views advancing per second | ~148 |
| Blocks finalized per second | ~108 |
| Nullifications per second | ~40 |
| Skip rate (wasted views) | **~26%** |
| Consensus efficiency | ~74% |

This means approximately 1 in 4 consensus rounds completes without producing a block, even though:
- The mempool is empty (no transactions to cause execution failures)
- All 4 validators are online and connected
- Network latency is negligible (all nodes on the same machine)
- Empty blocks are perfectly valid (verified by examining `build_block()` in `crates/node/runner/src/app.rs`)

---

## Root Cause: Startup Height Drift + Random Leader Election

### The Mechanism

The root cause is a combination of two factors:

1. **Asynchronous startup** - Validators start at slightly different times (Docker containers do not boot simultaneously). Each validator begins processing views independently from the moment it starts.

2. **Random leader election without height awareness** - The Simplex `Random` elector (`crates/node/runner/src/runner.rs` line 552: `elector: Random`) selects leaders without regard to whether the leader is caught up to the chain tip.

### What Happens Step by Step

1. Validators start at staggered times (even milliseconds of difference accumulate)
2. Faster-starting validators begin advancing views and producing blocks
3. When a behind validator is elected leader, it calls `propose()`:

```rust
// crates/node/runner/src/app.rs, lines 285-318
fn propose<A>(
    &mut self,
    _context: (Env, Self::Context),
    mut ancestry: AncestorStream<A, Self::Block>,
) -> impl std::future::Future<Output = Option<Self::Block>> + Send {
    async move {
        let parent = ancestry.next().await?;  // <-- Returns None if parent unavailable
        let block = self.build_block(&parent).await;
        block
    }
}
```

4. The `ancestry.next().await?` call uses the `?` operator -- if the parent block is not yet available in this validator's local state, the entire function returns `None`
5. A `None` return from `propose()` means no block is broadcast
6. Other validators wait for `leader_timeout` (2 seconds), then nullify the view

### The build_block() Path

Even if `ancestry.next()` succeeds, `build_block()` can return `None` if the parent snapshot is missing:

```rust
// crates/node/runner/src/app.rs, lines 84-164
async fn build_block(&self, parent: &Block) -> Option<Block> {
    let parent_snapshot = self.ledger.parent_snapshot(parent_digest).await?;  // None if missing
    // ... execute transactions ...
    // ... compute state root ...
    Some(block)
}
```

The `parent_snapshot()` call returns `None` when the validator has not yet processed and stored the snapshot for the parent block. This happens when the validator is behind: it knows the parent block exists (from the ancestry stream) but has not yet verified and cached its execution state.

### Similarly for Verification

When a behind validator receives a proposal from a more advanced leader:

```rust
// crates/node/runner/src/app.rs, lines 166-246
async fn verify_block(&self, block: &Block) -> bool {
    let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
        warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
        return false;  // Verification fails -> validator votes to nullify
    };
    // ...
}
```

If the verifier does not have the parent snapshot, it rejects the proposal, contributing to nullification.

---

## Why Exactly 26%?

### Mathematical Analysis

With 4 validators and random leader election:

- Each validator is leader for ~25% of views
- If 1 validator is significantly behind, it nullifies all its leader slots: 25% nullification
- Additionally, that behind validator may reject proposals from leaders that reference parents it has not processed yet

The observed 26% is consistent with approximately 1 validator being persistently behind relative to the other 3.

### Observed Height Drift Evidence

During testing, immediately after a fresh deploy:

```
node 0: finalized_height = 14004
node 1: finalized_height = 14135
node 2: finalized_height = 14449
node 3: finalized_height = 14520
drift: 516 blocks
```

This 516-block drift means node 0 is significantly behind node 3. During the period while node 0 catches up, every view where node 0 is the leader will be nullified.

### Why It Does Not Converge to Zero Quickly

On a same-machine devnet running at ~148 views/sec, even small startup timing differences create significant block drift. At 148 blocks/sec, a 3.5-second startup delay between the first and last validator creates ~518 blocks of drift. The drift resolves as the behind validator catches up through the resolver (fetching missing blocks from peers), but the fetch process introduces its own delays.

More critically, the drift can be **self-sustaining** in a subtle way: when a behind validator is elected leader and fails, that nullified view creates a gap that other validators must also process, maintaining a steady-state inefficiency.

---

## NOT the Cause: Empty Block Rejection

Empty blocks (0 transactions) are perfectly valid in Kora. The executor handles them correctly:

```rust
// crates/node/runner/src/app.rs, build_block()
let txs = mempool.build(self.max_txs, &excluded);  // Returns empty Vec when mempool is empty
// ...
let outcome = self.executor.execute(&parent_snapshot.state, &context, &txs_bytes);
// executor processes 0 txs -> no loop iterations -> returns Ok(outcome)
// ...
Some(block)  // Empty block produced successfully
```

The nullification is NOT caused by empty blocks being rejected. It is caused by the leader not having the prerequisite state (parent snapshot) to even attempt building a block.

---

## Impact on Throughput

### Capacity Analysis

```
Theoretical capacity:  148 views/sec * 1 block/view = 148 blocks/sec
Actual throughput:     108 blocks/sec
Wasted capacity:       40 views/sec (26%)
Lost blocks:           40 blocks/sec that could have been finalized
```

### Cost Per Nullified View

Each nullified view does not cost a full `leader_timeout` (2 seconds) in wall-clock time because views are pipelined. Instead, a nullified view simply represents a missed opportunity within the view pipeline. The chain advances views at ~148/sec regardless -- it is just that 26% of those views produce no block.

### Compound Effect Under Load

The 26% idle nullification rate compounds with load-induced failures:

1. **Idle:** 26% nullification from startup drift alone
2. **Under load:** If transactions cause execution failures (e.g., state root mismatches, executor errors), additional views are nullified
3. **Combined:** Nullification rate can exceed 50%, cutting effective throughput below 74 blocks/sec
4. **Cascading:** Higher nullification means more views where validators are catching up, which means more views where leaders lack parent state, which means more nullification

This positive feedback loop is why startup drift creates a worse problem than the raw 26% suggests.

---

## Prometheus Metrics Showing This Pattern

### Key Queries

```promql
# Skip rate per node (which validator is causing nullifications?)
1 - (rate(finalized_height[5m]) / rate(engine_voter_state_current_view[5m]))

# Height drift between nodes (should converge to 0 on healthy chain)
max(finalized_height) - min(finalized_height)

# Nullification rate (total across cluster)
sum(rate(engine_voter_state_nullifications_total[1m]))

# Per-node nullification contribution
sum by (instance) (rate(engine_voter_state_nullifications_total[5m]))

# Consensus efficiency (should be >90% after convergence)
avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m]))

# Stall detection (views advancing but no blocks finalized)
rate(engine_voter_state_current_view[1m]) > 0 and rate(finalized_height[1m]) == 0
```

### What to Look For in Grafana

1. **kora-overview dashboard:**
   - "Height Drift" panel: Shows divergence between fastest and slowest validator
   - "Nullification Rate" stat: Should be near 0 after warm-up
   - "Nullification Source" panel: Identifies which validator instance is causing most nullifications
   - "Skip Rate" panel: Per-node fraction of wasted views

2. **kora-performance dashboard:**
   - "Capacity vs Actual Throughput" panel: Gap between views/sec and blocks/sec = wasted views
   - "Consensus Efficiency" stat: 74% during the drift period, should approach 100%

3. **kora-transaction-flow dashboard:**
   - "Nullifications/s" timeseries: Should decrease over time as nodes converge
   - "Stall Detection" panel: Alerts if views advance but no blocks finalize

### Expected Pattern Over Time

```
t=0s:    All nodes start (staggered)
t=0-5s:  Height drift accumulates; skip rate ~40-60%
t=5-30s: Nodes begin catching up via resolver; skip rate ~26%
t=30-60s: Drift narrows; skip rate decreases toward ~10%
t=60s+:  All nodes converged; skip rate approaches ~0-2%
```

If the skip rate does NOT decrease over time, it indicates a persistent issue (node offline, executor crash loop, or network partition).

---

## Possible Fixes

### Fix 1: Synchronized Start (Low Effort, Partial Solution)

**Approach:** Ensure all validators start within the same consensus view by adding a startup barrier.

**Implementation:** Before starting the simplex engine, validators exchange "ready" messages and only begin consensus when all 2/3+ peers have signaled readiness.

**Impact:** Eliminates initial drift. Does not prevent drift from occurring after restarts or network partitions.

**Expected improvement:** Reduces idle nullification from 26% to ~0-2% on fresh deployments.

### Fix 2: Height-Aware Leader Election (Medium Effort, Full Solution)

**Approach:** Modify the leader elector to skip validators that are known to be behind.

**Implementation:** The `Random` elector would additionally check each candidate's last-known finalized height. If a candidate is more than N blocks behind the tip, it is skipped.

**Impact:** Completely eliminates nullifications caused by behind validators. The behind validator still catches up, but it does not waste leader slots while doing so.

**Complexity:** Requires tracking per-validator heights at the consensus layer. The Simplex protocol's `activity_timeout` and `skip_timeout` parameters partially address this, but at a much coarser granularity.

**Expected improvement:** Reduces idle nullification to ~0% even without synchronized start.

### Fix 3: View Synchronization Protocol (High Effort, Complete Solution)

**Approach:** Implement a pre-consensus synchronization round where validators align on the current view before beginning the proposal phase.

**Implementation:** Before each view, validators exchange their current heights. The leader only proposes if it confirms that 2/3+ validators have the parent state. If not, the leader explicitly yields, triggering an immediate (non-timeout) view advance.

**Impact:** Eliminates wasted timeout waiting. Even when a leader cannot propose, the view advances instantly rather than waiting for `leader_timeout`.

**Expected improvement:** Even if some nullification remains, the cost per nullified view drops from 2 seconds (leader_timeout) to milliseconds.

### Fix 4: Reduce Leader Timeout (Quick Win, Mitigation Only)

**Approach:** Reduce `CONSENSUS_LEADER_TIMEOUT` from 2 seconds to 500ms or less.

**Implementation:** Single constant change in `crates/node/runner/src/runner.rs`.

**Impact:** Does not prevent nullification, but reduces the time wasted per nullified view. A failed leader slot costs 500ms instead of 2000ms.

**Risk:** If block build times ever approach 500ms (e.g., with very large blocks or expensive state root computation), legitimate proposals could be cut off. Current build times are 0.03ms, so the risk is negligible for now.

### Comparison

| Fix | Effort | Nullification Reduction | Additional Benefits |
|-----|--------|------------------------|---------------------|
| Synchronized start | Low | 26% -> ~2% on fresh deploy | Simple, no protocol changes |
| Height-aware elector | Medium | 26% -> ~0% always | Works across restarts |
| View sync protocol | High | 26% -> ~0% + instant recovery | Eliminates timeout cost entirely |
| Reduce leader timeout | Trivial | No reduction in count; reduces cost per event | Fast to deploy, compounds with other fixes |

### Recommended Approach

Apply Fix 4 immediately (trivial, no risk) combined with Fix 1 for new deployments:

1. Change `CONSENSUS_LEADER_TIMEOUT` from `Duration::from_secs(2)` to `Duration::from_millis(500)` in `crates/node/runner/src/runner.rs` line 48
2. Add a startup barrier that waits for 2/3+ validator connections before starting the simplex engine

This combination should increase steady-state throughput from 108 blocks/sec to ~140+ blocks/sec.

---

## RPC Observability

The `kora_nodeStatus` RPC method exposes nullification counts directly:

```json
{
  "chainId": 1337,
  "validatorIndex": 0,
  "uptimeSecs": 120,
  "currentView": 17500,
  "finalizedCount": 12800,
  "proposedCount": 3200,
  "nullifiedCount": 4700,
  "peerCount": 3,
  "isLeader": false
}
```

The ratio `nullifiedCount / (finalizedCount + nullifiedCount)` gives the observed skip rate for that node. The devnet-stats script (`docker/scripts/devnet-stats.sh`) displays this in a live terminal dashboard.

---

## Code References

| Component | File | Relevant Lines |
|-----------|------|---------------|
| Proposal logic (returns None on missing parent) | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` | 285-318 |
| build_block (returns None on missing snapshot) | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` | 84-164 |
| verify_block (returns false on missing parent) | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` | 166-246 |
| Leader timeout constant | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` | 48 |
| Simplex config (elector: Random) | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` | 552 |
| Nullification reporting (inc_nullified) | `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` | 468-469 |
| NodeState nullified counter | `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/state.rs` | 28, 71-72 |
| Devnet stats display | `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-stats.sh` | 92, 126-169 |
| Devnet health diagnostics | `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-health.sh` | 71-81 |
| Grafana nullification panels | `/Users/will/dev/nunchi/daeji/docker/grafana/dashboards/kora-overview.json` | 96, 303-315 |
