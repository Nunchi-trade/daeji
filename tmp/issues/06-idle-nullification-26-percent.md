# Consensus: 26% of views nullified at idle due to startup height drift

**Severity:** Medium-High
**Component:** Consensus / Simplex BFT leader election and block proposal
**Affected files:**
- `crates/node/runner/src/app.rs` (propose, build_block, verify_block -- the None/false return paths)
- `crates/node/runner/src/runner.rs` (consensus config, timeout constants, elector: Random)
- `crates/node/reporters/src/lib.rs` (NodeStateReporter -- increments nullified counter on Activity::Nullification, lines 468-469)
- `crates/node/rpc/src/state.rs` (NodeState -- stores nullified_count exposed via kora_nodeStatus RPC, line 28/71-72)
- `docker/compose/devnet.yaml` (staggered container startup, healthcheck start_period)
- `docker/scripts/devnet-run.sh` (startup orchestration, wait loop)
- `docker/scripts/devnet-health.sh` (health diagnostics, skip rate query)
- `docker/scripts/devnet-stats.sh` (live monitoring dashboard showing per-node nullification counts)
- `docker/config/alerts.yml` (HighNullificationRate, HighSkipRate, HeightDrift)
- `docker/config/recording-rules.yml` (kora:consensus_efficiency, kora:skip_rate, kora:height_drift)
- `docker/grafana/dashboards/kora-overview.json` (Nullifications/s stat panel ID 5)
- `docker/grafana/dashboards/kora-stall-diagnostics.json` (Skip Rate stat panel ID 202)

---

## Summary

Kora is an EVM-compatible blockchain that uses Commonware's Simplex BFT consensus for block finalization. On a fresh devnet with zero transaction load, approximately 26% of consensus views are nullified -- meaning they complete without producing a finalized block. With 4 validators running, the network advances through approximately 148 views per second but only produces approximately 108 blocks per second, wasting roughly 40 views per second.

This is not caused by invalid transactions, executor failures, or network issues. It is a structural inefficiency caused by height drift between validators that start at different times. The 26% idle nullification rate is a permanent tax on consensus capacity that compounds with any load-induced failures, creating a positive feedback loop that contributed to a production chain stall.

---

## Background: How Simplex BFT consensus works

Simplex BFT is a view-based consensus protocol. The network advances through sequential "views" (consensus rounds). In each view:

1. **Leader election:** A leader is selected from the validator set using deterministic random election (`elector: Random` at line 552 of `crates/node/runner/src/runner.rs`). Each validator has an equal probability of being selected per view.

2. **Proposal:** The elected leader builds a block from the mempool by calling `propose()`, which retrieves the parent block from the ancestry stream and calls `build_block()` to construct a new block on top of it.

3. **Verification:** Non-leader validators receive the proposed block and verify it by re-executing all transactions against the parent state via `verify()` / `verify_block()`.

4. **Notarization:** If 2/3+ validators approve the block, it receives a notarization certificate (first-round approval).

5. **Finalization:** After notarization, validators cast finalization votes. If 2/3+ finalize, the block becomes canonical.

6. **Nullification:** If the leader fails to propose a valid block within the leader timeout, or if verification fails on 1/3+ validators, the view is nullified. The voter actor broadcasts a "nullification floor" message, and the network advances to the next view without producing a block.

A nullified view is wasted consensus capacity. The network must still complete the timeout cycle (up to 2 seconds for `CONSENSUS_LEADER_TIMEOUT`), communicate the failure to peers, and advance to the next view.

Key consensus configuration from `crates/node/runner/src/runner.rs`, lines 48-53:

```rust
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_secs(2);
const CONSENSUS_CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(4);
const CONSENSUS_TIMEOUT_RETRY: Duration = Duration::from_secs(1);
const CONSENSUS_FETCH_TIMEOUT: Duration = Duration::from_secs(1);
const CONSENSUS_ACTIVITY_TIMEOUT: ViewDelta = ViewDelta::new(256);
const CONSENSUS_SKIP_TIMEOUT: ViewDelta = ViewDelta::new(32);
```

---

## Observed metrics

From production monitoring data on a fresh devnet with zero transaction load (Hetzner dedicated server):

| Metric | Value |
|---|---|
| Views/sec | ~148 |
| Blocks/sec (finalized) | ~108 |
| Nullifications/sec | ~40 |
| Skip rate (nullification ratio) | ~26% |
| Consensus efficiency | ~74% |
| Block time (effective) | ~9.28ms |
| Block build duration (p95) | 0.030ms |
| Finalization latency (p95) | 47.5ms |

**Note on measurement variability:** These values were measured after several minutes of operation on a Hetzner dedicated server with all nodes on the same machine. Separate measurements taken earlier in a run (under 30 seconds after start) showed different values: 0.735 blocks/sec, 53% efficiency, ~3.5 nullifications/sec. The discrepancy is because the skip rate is worst during the initial convergence period and improves as nodes catch up, but may not fully converge due to the structural height drift issue described below.

These values are from the recording rules in `docker/config/recording-rules.yml`:

```yaml
# docker/config/recording-rules.yml:53-59
- record: kora:consensus_efficiency
  expr: avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m]))

- record: kora:skip_rate
  expr: 1 - (avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m])))
```

Approximately 1 in 4 views produces no block. This is observable on both the Grafana overview dashboard (`docker/grafana/dashboards/kora-overview.json`, "Nullifications/s" stat panel at ID 5) and the stall diagnostics dashboard (`docker/grafana/dashboards/kora-stall-diagnostics.json`, "Skip Rate" stat panel at ID 202).

---

## Root cause: Asynchronous Docker container startup creates height drift

### The startup sequence

The devnet Docker Compose configuration (`docker/compose/devnet.yaml`) starts validators sequentially. Node 0 is the bootstrap node; nodes 1-3 depend on node 0 being healthy before they start:

```yaml
# docker/compose/devnet.yaml:205-211
validator-node1:
  <<: *validator-common
  hostname: node1
  depends_on:
    validator-node0:
      condition: service_healthy
  # ...
```

The devnet-run script (`docker/scripts/devnet-run.sh`) launches all containers with `docker compose up -d` and then waits for all 4 to become healthy:

```bash
# docker/scripts/devnet-run.sh:326-346
while true; do
    HEALTHY=$(docker compose -f compose/devnet.yaml ps --format json 2>/dev/null | \
        jq -r 'select(.Service | startswith("validator-")) | select(.Health == "healthy") | .Service' | wc -l)
    # ...
    if [[ "$HEALTHY" -ge 4 ]]; then
        # all healthy
        break
    fi
done
```

However, the health check (lines 31-36 of `docker/compose/devnet.yaml`) has a 30-second start period (`start_period: 30s`), and nodes 1-3 only start after node 0 passes its health check. This means:

- Node 0 starts first and begins advancing views and finalizing blocks immediately.
- Nodes 1-3 start seconds to tens of seconds later.
- By the time all 4 nodes are running, node 0 has already advanced hundreds or thousands of views ahead.

### Observed height drift

From production test data, after all nodes have been running for several minutes:

```
node 0: finalized_height = 14004
node 3: finalized_height = 14520
```

A drift of 516 blocks between the slowest and fastest node. The `devnet-health.sh` script (`docker/scripts/devnet-health.sh`) monitors this:

```bash
# docker/scripts/devnet-health.sh:35-40
drift=$(query 'max(finalized_height)-min(finalized_height)')
drift_val=$(val "$drift")
echo "  Height drift: ${drift_val}"
```

And the recording rules track it continuously:

```yaml
# docker/config/recording-rules.yml:62-63
- record: kora:height_drift
  expr: max(finalized_height) - min(finalized_height)
```

### How height drift causes nullification

When a validator that is behind in height is elected as leader, it cannot produce a valid block because its state is stale:

1. **`propose()` calls `ancestry.next().await`** (`crates/node/runner/src/app.rs`, line 296). If the ancestry stream cannot provide a valid parent (because the node's view of the chain tip is behind the network's current view), this returns `None` and the proposal fails.

```rust
// crates/node/runner/src/app.rs:285-318
fn propose<A>(
    &mut self,
    _context: (Env, Self::Context),
    mut ancestry: AncestorStream<A, Self::Block>,
) -> impl std::future::Future<Output = Option<Self::Block>> + Send
where
    A: BlockProvider<Block = Self::Block>,
{
    let node_state = self.node_state.clone();
    async move {
        let start = Instant::now();
        let parent = ancestry.next().await?;   // <-- Returns None if parent unavailable
        let ancestry_elapsed = start.elapsed();

        let build_start = Instant::now();
        let block = self.build_block(&parent).await;  // <-- May return None
        // ...
        block
    }
}
```

2. **`build_block()` calls `self.ledger.parent_snapshot(parent_digest).await?`** (`crates/node/runner/src/app.rs`, line 89). If the parent block's state snapshot is missing (because the node has not yet processed that block), `parent_snapshot()` returns `None` and the `?` operator causes `build_block()` to return `None`.

```rust
// crates/node/runner/src/app.rs:84-164
async fn build_block(&self, parent: &Block) -> Option<Block> {
    use kora_consensus::Mempool as _;

    let start = Instant::now();
    let parent_digest = parent.commitment();
    let parent_snapshot = self.ledger.parent_snapshot(parent_digest).await?;  // <-- None if missing
    // ...
    Some(block)
}
```

3. **Similarly, `verify_block()` fails** (`crates/node/runner/src/app.rs`, lines 176-179). When a behind node receives a block proposal from a leader that is ahead, it cannot verify the block because it lacks the parent state snapshot:

```rust
// crates/node/runner/src/app.rs:166-179
async fn verify_block(&self, block: &Block) -> bool {
    let start = Instant::now();
    let digest = block.commitment();
    let parent_digest = block.parent();

    if self.ledger.query_state_root(digest).await.is_some() {
        trace!(?digest, "block already verified");
        return true;
    }

    let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
        warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
        return false;   // <-- Verification fails
    };
    // ...
}
```

When the leader returns `None` from `propose()`, no proposal is sent, and the view times out after `CONSENSUS_LEADER_TIMEOUT` (2 seconds). The voter actors on all nodes then nullify the view and advance to the next one.

---

## Why exactly 26%

The nullification rate is approximately 25-27% because of the relationship between the number of validators, random leader election, and the number of validators that are behind:

- With 4 validators and random leader election, each validator is elected as leader in approximately 25% of views.
- If 1 validator is consistently behind (or recently started and still syncing), it fails to produce valid proposals when elected.
- That validator's 25% leader share becomes 25% nullified views.
- Additional nullification comes from verification failures: when behind validators cannot verify proposals from ahead validators.

The math: if 1 out of 4 validators is behind, and leader election is uniform random, then approximately `1/4 = 25%` of views will have that behind validator as leader, and those views will nullify. The observed 26% includes a small additional contribution from cross-validator verification failures and timing jitter.

This is NOT caused by empty block rejection. Empty blocks (blocks with zero transactions) are perfectly valid in Kora. The `build_block()` method happily produces empty blocks:

```rust
// crates/node/runner/src/app.rs:96-117
// txs can be empty -- that's fine
let txs = mempool.build(self.max_txs, &excluded);

// The diagnostic warning below only fires when there ARE unincluded txs
// but the result is empty (i.e., max_txs or excluded set issue).
// An empty mempool producing an empty block does NOT trigger this warning.
if txs.is_empty() && mempool_len > excluded_len {
    warn!(
        mempool_len,
        excluded_len,
        max_txs = self.max_txs,
        "build_block: mempool has unincluded txs but produced empty block"
    );
} else {
    trace!(
        mempool_len,
        excluded_len,
        drained = txs.len(),
        max_txs = self.max_txs,
        "build_block: mempool drain"
    );
}
```

The block is then constructed and returned regardless of whether it contains transactions:

```rust
// crates/node/runner/src/app.rs:148
let block = Block { parent: parent.id(), height, prevrandao, state_root, txs };
```

---

## Impact

### Wasted capacity

At 40 nullifications per second, the network wastes approximately 40 consensus rounds per second. Each wasted round involves:
- Timeout waiting for the failed leader (up to 2 seconds)
- Nullification floor broadcast to all peers
- View advancement protocol messages

While individual nullified views resolve quickly (the timeout is short when the leader immediately returns `None` rather than building slowly), they still consume network bandwidth and CPU time for message processing.

### Compounds under load (the poisoned mempool cascade)

The 26% idle nullification rate compounds with any load-induced failures. Under heavy transaction load:

- Bad transactions in the mempool cause additional leader failures (executor aborts).
- Slow block building (ECDSA recovery at scale) causes additional timeouts.
- The combined nullification rate can exceed 50-70%.

The most dangerous compound effect is the **poisoned mempool cascade**:
1. Load test sends many transactions
2. Some become stale (nonce consumed by finalized blocks)
3. The mempool accepts them (no nonce re-validation on insert)
4. Leader proposes block including stale transactions
5. Executor hits decode error / NonceTooLow -> `?` operator -> abort -> returns `None`
6. View nullified, failing transactions NOT pruned from mempool
7. Next leader draws same stale transactions -> same failure
8. Repeat indefinitely: ~33% additional views fail this way, compounding on top of the 26% baseline

The timeline from production testing:

| Phase | Nullification Rate | Effect |
|---|---|---|
| Idle baseline | ~26% | Structural waste from height drift |
| Light load | ~26-30% | Minimal additional impact |
| Heavy load | ~35-50% | Height drift + executor failures compound |
| Sustained heavy load | ~50-70% | Positive feedback loop: more nullification -> more stale state -> more failures |
| Chain degradation | >70% | Near-stall, only occasional blocks finalize |

The baseline 26% means the network starts in a degraded state. It takes less additional stress to push it past critical thresholds compared to a network that starts at 0% nullification.

### Positive feedback loop

Height drift creates a self-reinforcing cycle:

1. Behind validator fails to propose -> view nullified
2. Nullified view wastes time -> behind validator falls further behind
3. Other validators advance -> drift increases
4. More leader slots hit the behind validator -> more nullification
5. Under load, additional failures compound on top of the structural 26%

---

## Proposed fixes

### Fix 1: Synchronized start (Low effort, Partial fix)

**Effort:** Low (Docker Compose and startup script changes only)
**Impact:** Partial -- eliminates initial drift but not drift from restarts or transient failures

Modify the devnet startup to synchronize validator starts. Options:

**Option A: Barrier-based start.** Add a coordination service or script that holds all validators in a "waiting" state until all 4 are ready, then releases them simultaneously.

**Option B: Genesis timestamp.** Set a future genesis timestamp and have all validators wait until that time before starting consensus. This is the approach used by most production blockchain networks.

**Option C: Remove sequential dependency.** Change `docker/compose/devnet.yaml` so nodes 1-3 do not depend on node 0's health check. Instead, have all nodes start simultaneously and retry peer connections until they succeed:

```yaml
# Instead of:
validator-node1:
  depends_on:
    validator-node0:
      condition: service_healthy

# Use:
validator-node1:
  depends_on:
    validator-node0:
      condition: service_started  # Start immediately, don't wait for healthy
```

This would reduce the startup time gap from 30+ seconds to near-zero, eliminating most of the initial height drift.

### Fix 2: Height-aware leader election (Medium effort, Full fix)

**Effort:** Medium (requires changes to consensus engine or wrapper)
**Impact:** Full fix for height-drift-induced nullification

Modify the leader election logic to prefer validators that are at or near the current chain height. A validator that is significantly behind (more than N views behind the max) should be skipped in leader election until it catches up.

This could be implemented as a wrapper around the `Random` elector:

```rust
struct HeightAwareElector {
    inner: Random,
    min_height_delta: u64,  // Max allowed gap before skipping
}

impl Elector for HeightAwareElector {
    fn elect(&self, view: View, validators: &[PublicKey]) -> PublicKey {
        let candidate = self.inner.elect(view, validators);
        // If candidate's last known height is too far behind, skip to next
        if self.is_behind(candidate, self.min_height_delta) {
            self.inner.elect(view + 1, validators)  // Try next candidate
        } else {
            candidate
        }
    }
}
```

### Fix 3: View sync protocol (High effort, Complete fix)

**Effort:** High (requires Commonware upstream changes)
**Impact:** Complete fix for all sources of view drift

Implement a view synchronization protocol where validators exchange their current view/height state and reach agreement before starting consensus rounds. This is a standard feature in production BFT implementations but is not present in Commonware Simplex.

### Fix 4: Reduce leader timeout from 2s to 500ms (Trivial effort, Mitigation)

**Effort:** Trivial (single constant change)
**Impact:** Mitigation -- does not prevent nullification but reduces the time wasted per nullified view

```rust
// crates/node/runner/src/runner.rs:48
// Change from:
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_secs(2);
// To:
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_millis(500);
```

At idle, blocks build in ~0.030ms (p95). Even under moderate load, block build time is well under 500ms. Reducing the leader timeout from 2s to 500ms means each nullified view wastes 500ms instead of 2s, reducing the impact of the 26% nullification rate by 4x in terms of time wasted.

However, this must be carefully balanced against block build time under heavy load. If block build time exceeds the leader timeout, valid proposals will fail, increasing nullification rather than reducing it. The `SlowBlockBuild` alert (in `docker/config/alerts.yml`, line 129) monitors this:

```yaml
- alert: SlowBlockBuild
  expr: kora:build_duration:p95 > 1
  for: 2m
  labels:
    severity: warning
  annotations:
    summary: "Block build p95 is {{ $value | humanizeDuration }}"
    description: "Block build time p95 exceeding 1s (leader timeout is 2s). ECDSA recovery or mempool size may be the cause."
```

If the timeout is reduced to 500ms, this alert threshold would need to be adjusted accordingly.

### Recommended approach

**Immediate (this week):** Apply Fix 4 (reduce leader timeout to 500ms). This is a single-line change with no risk and immediately reduces the impact of each nullified view.

**Short-term (this sprint):** Apply Fix 1, Option C (remove sequential dependency in Docker Compose). This eliminates the initial height drift that causes the 26% baseline. Combined with Fix 4, the idle nullification rate should drop to near zero.

**Medium-term:** Evaluate Fix 2 (height-aware leader election) if height drift from restarts or transient failures continues to cause problems.

---

## Prometheus metrics and Grafana panels for monitoring

### Key metrics

| Metric | PromQL | Purpose |
|---|---|---|
| Consensus efficiency | `kora:consensus_efficiency` | Fraction of views that produce finalized blocks. Target: >0.95 |
| Skip rate | `kora:skip_rate` | Fraction of wasted views. Target: <0.05 |
| Nullification rate | `kora:nullification_rate` | Nullifications per second across all nodes |
| Height drift | `kora:height_drift` | Max - min finalized height across validators |
| Views per second | `kora:views_per_sec` | Total view advancement rate |
| Blocks per second | `kora:blocks_per_sec` | Finalized block production rate |

These recording rules are defined in `docker/config/recording-rules.yml` (lines 44-67).

### Relevant Grafana panels

1. **kora-overview dashboard** (`docker/grafana/dashboards/kora-overview.json`):
   - "Nullifications/s" stat panel (ID 5): `sum(rate(engine_voter_state_nullifications_total[5m]))`
   - "Blocks/sec" stat panel (ID 3): `avg(rate(finalized_height[1m]))`

2. **kora-stall-diagnostics dashboard** (`docker/grafana/dashboards/kora-stall-diagnostics.json`):
   - "Skip Rate" stat panel (ID 202): `1 - (avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m])))`
   - "Nodes w/ Active Views" stat panel (ID 203): `count(rate(engine_voter_state_current_view[1m]) > 0)`
   - "Current View per Node" time series (ID in panel set): `engine_voter_state_current_view` per validator
   - "View Rate vs Finalization Rate" time series: `rate(engine_voter_state_current_view[1m])` vs `rate(finalized_height[1m])` per node

3. **kora-transaction-flow dashboard** (`docker/grafana/dashboards/kora-transaction-flow.json`):
   - "Consensus Efficiency" stat panel: `kora:consensus_efficiency`
   - "Skip Rate" stat panel: `kora:skip_rate`

### Relevant alerts

From `docker/config/alerts.yml`:

```yaml
# Lines 49-57: Fires when nullification rate exceeds 5/sec
- alert: HighNullificationRate
  expr: sum(rate(engine_voter_state_nullifications_total[5m])) > 5
  for: 2m

# Lines 59-68: Fires when skip rate exceeds 30%
- alert: HighSkipRate
  expr: |
    (1 - (avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m])))) > 0.3
  for: 3m
  annotations:
    description: "Over 30% of consensus views are wasted. Network was at 33% skip rate before the production stall."

# Lines 39-47: Fires when height drift exceeds 10 blocks
- alert: HeightDrift
  expr: max(finalized_height) - min(finalized_height) > 10
  for: 1m
```

Note that at the current 26% idle nullification rate, the `HighNullificationRate` alert (threshold 5/sec) fires constantly since the baseline is ~40/sec. The `HighSkipRate` alert (threshold 30%) is just barely below firing at 26%. Both thresholds may need adjustment after fixing the root cause.

---

## Reproduction steps

1. Start a fresh devnet from scratch (clear all volumes):
   ```bash
   just devnet-clean
   just devnet-run
   ```

2. Wait 2-3 minutes for metrics to stabilize.

3. Check the skip rate (should be approximately 26%):
   ```bash
   # Using devnet-health.sh
   docker/scripts/devnet-health.sh
   # Look for "Avg skip rate (wasted views): 0.26"
   ```

4. Or query Prometheus directly:
   ```bash
   curl -sg 'http://localhost:9090/api/v1/query?query=kora:skip_rate'
   curl -sg 'http://localhost:9090/api/v1/query?query=kora:consensus_efficiency'
   curl -sg 'http://localhost:9090/api/v1/query?query=kora:height_drift'
   ```

5. Check per-node finalized height to confirm drift:
   ```bash
   curl -sg 'http://localhost:9090/api/v1/query?query=finalized_height'
   ```

6. Observe that the nullification rate is ~40/sec at zero load:
   ```bash
   curl -sg 'http://localhost:9090/api/v1/query?query=kora:nullification_rate'
   ```

7. Query per-node nullification counts via RPC:
   ```bash
   # kora_nodeStatus returns a NodeStatus struct (crates/node/rpc/src/state.rs:97-118)
   # with fields: currentView, finalizedCount, nullifiedCount, proposedCount
   for port in 8545 8546 8547 8548; do
     echo "=== Node on port $port ==="
     curl -s http://localhost:$port -X POST \
       -H "Content-Type: application/json" \
       -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result'
   done
   # The ratio nullifiedCount / (finalizedCount + nullifiedCount) gives the skip rate per node
   ```

8. Use the live monitoring dashboard:
   ```bash
   just devnet-stats
   # Shows per-node: view, finalized, nullified, proposed, blocks/sec
   # The "Nullified" column shows the running counter of nullified views per node
   ```

9. Confirm empty blocks are valid (this is NOT the cause):
   ```bash
   # Submit zero transactions, observe blocks still being produced
   # The ~108 blocks/sec at idle are almost all empty blocks
   ```

---

## Testing plan

- [ ] Baseline measurement: Deploy fresh devnet with current code, measure skip rate and confirm it is approximately 26%
- [ ] Fix 4 validation: Change `CONSENSUS_LEADER_TIMEOUT` to 500ms, redeploy, measure that block production rate is maintained and nullified view duration is reduced
- [ ] Fix 1 validation: Modify `docker/compose/devnet.yaml` to use `condition: service_started` instead of `condition: service_healthy`, redeploy, measure that initial height drift is reduced and skip rate drops below 5%
- [ ] Load test regression: After applying fixes, run the standard load test and confirm that the reduced baseline nullification does not cause new failures under load
- [ ] Alert threshold review: After fixing, review `HighNullificationRate` (threshold 5/sec) and `HighSkipRate` (threshold 30%) and adjust to appropriate levels for the new baseline
- [ ] Restart resilience: Restart a single validator mid-operation and measure how quickly it catches up and how the skip rate is affected during catch-up
- [ ] Metric validation: Confirm that `kora:consensus_efficiency`, `kora:skip_rate`, and `kora:height_drift` recording rules accurately reflect the improved state after fixes

---

## Code references

| Component | File | Line(s) |
|---|---|---|
| propose() implementation | `crates/node/runner/src/app.rs` | 285-318 |
| ancestry.next().await (returns None) | `crates/node/runner/src/app.rs` | 296 |
| build_block() (parent_snapshot check) | `crates/node/runner/src/app.rs` | 84-164 |
| parent_snapshot returns None | `crates/node/runner/src/app.rs` | 89 |
| verify_block() (parent_snapshot check) | `crates/node/runner/src/app.rs` | 166-246 |
| verify_block missing parent snapshot | `crates/node/runner/src/app.rs` | 176-179 |
| verify() trait impl (ancestry iteration) | `crates/node/runner/src/app.rs` | 327-382 |
| Empty block is valid (no rejection) | `crates/node/runner/src/app.rs` | 96-117, 148 |
| CONSENSUS_LEADER_TIMEOUT (2s) | `crates/node/runner/src/runner.rs` | 48 |
| All consensus timeout constants | `crates/node/runner/src/runner.rs` | 48-53 |
| Engine configuration (elector: Random) | `crates/node/runner/src/runner.rs` | 548-573 (elector at 552) |
| Docker validator dependency chain | `docker/compose/devnet.yaml` | 205-211 |
| Docker healthcheck config (30s start_period) | `docker/compose/devnet.yaml` | 31-36 |
| Startup wait loop | `docker/scripts/devnet-run.sh` | 326-346 |
| Skip rate recording rule | `docker/config/recording-rules.yml` | 57-59 |
| Consensus efficiency recording rule | `docker/config/recording-rules.yml` | 53-55 |
| Height drift recording rule | `docker/config/recording-rules.yml` | 62-63 |
| HighNullificationRate alert | `docker/config/alerts.yml` | 49-57 |
| HighSkipRate alert | `docker/config/alerts.yml` | 59-68 |
| HeightDrift alert | `docker/config/alerts.yml` | 39-47 |
| NodeStateReporter (nullification tracking) | `crates/node/reporters/src/lib.rs` | 468-469 |
| NodeState nullified_count field | `crates/node/rpc/src/state.rs` | 28 |
| inc_nullified() method | `crates/node/rpc/src/state.rs` | 71-72 |
| NodeStatus struct (RPC response) | `crates/node/rpc/src/state.rs` | 97-118 |
| Grafana skip rate panel (stall diagnostics) | `docker/grafana/dashboards/kora-stall-diagnostics.json` | 52 |
| Grafana Nodes w/ Active Views panel | `docker/grafana/dashboards/kora-stall-diagnostics.json` | 69 |
| Grafana nullification rate by reporter | `docker/grafana/dashboards/kora-overview.json` | 314-315 |
| Grafana efficiency panel | `docker/grafana/dashboards/kora-transaction-flow.json` | 65 |
| devnet-health.sh skip rate query | `docker/scripts/devnet-health.sh` | 80-81 |
| devnet-stats.sh live monitoring | `docker/scripts/devnet-stats.sh` | 92, 126-169 |

---

## Verification steps (after fix is applied)

After implementing any of the proposed fixes, verify correctness with:

1. **Baseline measurement (before fix):** Deploy a fresh devnet with the current code and record the skip rate:
   ```bash
   just devnet-clean && just devnet-run
   sleep 120  # Wait for metrics to stabilize
   curl -sg 'http://localhost:9090/api/v1/query?query=kora:skip_rate' | jq '.data.result[0].value[1]'
   # Expected: approximately 0.26 (26%)
   ```

2. **Fix 4 validation (leader timeout reduction):**
   - Change `CONSENSUS_LEADER_TIMEOUT` from `Duration::from_secs(2)` to `Duration::from_millis(500)` in `crates/node/runner/src/runner.rs` line 48
   - Rebuild and redeploy: `just devnet-clean && just devnet-run`
   - Verify block production rate is maintained: `curl -sg 'http://localhost:9090/api/v1/query?query=kora:blocks_per_sec'`
   - Verify no increase in nullification count -- the rate of wasted views may be the same, but each wastes less time
   - Update the `SlowBlockBuild` alert threshold (line 129 of `docker/config/alerts.yml`) from `> 1` to `> 0.4` to match the new timeout

3. **Fix 1 validation (synchronized start):**
   - Change `condition: service_healthy` to `condition: service_started` in `docker/compose/devnet.yaml` lines 209-210 (and similarly for nodes 2 and 3)
   - Rebuild and redeploy: `just devnet-clean && just devnet-run`
   - Measure skip rate after 2 minutes: `curl -sg 'http://localhost:9090/api/v1/query?query=kora:skip_rate'`
   - Expected: skip rate drops below 5% (from 26%)
   - Measure height drift: `curl -sg 'http://localhost:9090/api/v1/query?query=kora:height_drift'`
   - Expected: drift drops from hundreds of blocks to single digits

4. **Per-node RPC verification:**
   ```bash
   # Query each node's kora_nodeStatus and compute per-node skip rate
   for port in 8545 8546 8547 8548; do
     result=$(curl -s http://localhost:$port -X POST \
       -H "Content-Type: application/json" \
       -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result')
     finalized=$(echo "$result" | jq '.finalizedCount')
     nullified=$(echo "$result" | jq '.nullifiedCount')
     total=$((finalized + nullified))
     echo "Port $port: finalized=$finalized nullified=$nullified skip_rate=$(echo "scale=2; $nullified * 100 / $total" | bc)%"
   done
   ```

5. **Load test regression:** After applying fixes, run the standard load test and confirm the reduced baseline nullification does not cause new failures:
   ```bash
   cargo run --bin loadgen -- --total-txs 10000 --accounts 30 --concurrency 100 \
     --rpc-url http://127.0.0.1:8545 \
     --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
   ```

6. **Alert threshold review:** After fixing the baseline, verify that the `HighNullificationRate` alert (threshold 5/sec, line 51 of `docker/config/alerts.yml`) does NOT fire at idle, and the `HighSkipRate` alert (threshold 30%, line 62) does NOT fire at idle.

7. **Restart resilience:** Restart a single validator mid-operation and measure how quickly the skip rate returns to baseline:
   ```bash
   docker restart kora-devnet-validator-node2-1
   # Monitor skip rate via devnet-health.sh or Prometheus
   docker/scripts/devnet-health.sh
   ```
