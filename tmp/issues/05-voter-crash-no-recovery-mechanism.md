# Consensus: Voter actor crash ('voter should not finish') has no detection or recovery

**Severity:** Critical
**Component:** Consensus / Simplex BFT voter actor
**Affected files:**
- `crates/node/runner/src/runner.rs` (engine spawn, main loop, timeout constants)
- `crates/node/runner/src/app.rs` (block verification during catch-up)
- `crates/node/reporters/src/lib.rs` (NodeStateReporter -- increments nullification counter on Activity::Nullification)
- `crates/node/rpc/src/state.rs` (NodeState -- stores nullified_count, exposes via kora_nodeStatus RPC)
- `docker/config/alerts.yml` (VoterCrash, ResolverPeersBlocked alerts)
- `docker/grafana/dashboards/kora-logs.json` (panic log panel)
- `docker/grafana/dashboards/kora-stall-diagnostics.json` (active view count panel)
- `docker/compose/devnet.yaml` (restart policy, tmpfs mount)
- `docker/scripts/devnet-stats.sh` (live monitoring dashboard)

---

## Summary

Kora is an EVM-compatible blockchain built on the Commonware Simplex BFT consensus engine. Its consensus voter actor -- the core participant responsible for receiving proposals, casting votes, and advancing views -- can panic and terminate unexpectedly at runtime. When this happens, the affected node remains fully operational at the infrastructure level: its Docker container stays running, its RPC endpoint responds to queries, and Prometheus continues scraping its metrics. However, consensus on that node is permanently broken. No blocks are proposed, no votes are cast, and no views advance. There is no mechanism in Kora to detect this condition programmatically or to recover from it without manual intervention.

With a 4-validator set requiring 3-of-4 quorum, a single voter crash is survivable but any additional failure causes a permanent chain halt.

---

## Background: What is the voter actor?

Kora uses Commonware's Simplex BFT implementation for consensus. Simplex is a view-based protocol where validators cycle through sequential consensus rounds ("views"). In each view:

1. A **leader** is elected via VRF-based random selection (`elector: Random` in the engine config).
2. The leader builds a block from the mempool and proposes it to peers.
3. Other validators verify the block by re-executing its transactions.
4. If 2/3+ validators approve (notarize) and then finalize the block, it becomes canonical.
5. The protocol advances to the next view.

The **voter actor** (`commonware_consensus::simplex::actors::voter::actor`) is the long-lived async task inside the Simplex engine that drives this entire process. It is responsible for:

- Receiving block proposals from leaders via the votes network channel
- Casting notarization votes (first-round approval)
- Casting finalization votes (second-round commitment)
- Broadcasting nullification floor messages when a view fails to produce a block
- Maintaining the consensus state machine (current view number, pending certificates, finalization history)
- Processing timeout cycles when leaders fail to propose

The voter is designed to run for the entire lifetime of the node. If it terminates, the node can no longer participate in consensus in any capacity.

---

## The error message

```
ERROR commonware_runtime::utils::handle: task panicked err="voter should not finish"
```

This message is logged by the Commonware runtime's panic handler. The runtime wraps spawned tasks with a handler that catches panics and logs the error payload. The message "voter should not finish" is the panic string, indicating that the voter future resolved (completed) when it was expected to run indefinitely.

The Grafana Loki log panel that detects this is configured in `docker/grafana/dashboards/kora-logs.json` (line 88):

```json
{
  "expr": "{node_type=~\"validator|secondary\"} |~ \"task panicked|voter should not finish|PANIC\"",
  "refId": "A"
}
```

This panel is titled "Voter Panics / Task Crashes" in the kora-logs dashboard.

---

## Where it happens in the code

### Engine spawn (no Handle monitoring)

The voter is spawned when `engine.start()` is called in `crates/node/runner/src/runner.rs`, line 574:

```rust
// crates/node/runner/src/runner.rs:548-574
let engine = simplex::Engine::new(
    context.with_label("engine"),
    simplex::Config {
        scheme: self.scheme.clone(),
        elector: Random,
        blocker: transport.oracle.clone(),
        automaton: marshaled.clone(),
        relay: marshaled,
        reporter,
        strategy,
        partition: self.partition_prefix.clone(),
        mailbox_size: MAILBOX_SIZE,
        epoch: Epoch::zero(),
        replay_buffer: NZUsize!(16 * 1024 * 1024),
        write_buffer: NZUsize!(16 * 1024 * 1024),
        leader_timeout: CONSENSUS_LEADER_TIMEOUT,
        certification_timeout: CONSENSUS_CERTIFICATION_TIMEOUT,
        timeout_retry: CONSENSUS_TIMEOUT_RETRY,
        fetch_timeout: CONSENSUS_FETCH_TIMEOUT,
        activity_timeout: CONSENSUS_ACTIVITY_TIMEOUT,
        skip_timeout: CONSENSUS_SKIP_TIMEOUT,
        fetch_concurrent: 32,
        page_cache,
        forwarding: simplex::ForwardingPolicy::SilentLeader,
    },
);
engine.start(transport.simplex.votes, transport.simplex.certs, transport.simplex.resolver);
```

The `engine.start()` call hands the voter three network channels (votes, certificates, resolver) and spawns it as a background task. The call returns a Handle, but **this Handle is not stored, awaited, or monitored**. It is silently dropped.

### Main loop (no crash detection)

After starting the engine, the `run()` method returns a `LedgerService` handle, and the standalone runner enters an infinite pending future at line 318:

```rust
// crates/node/runner/src/runner.rs:303-320
executor.start(|context| async move {
    // ... setup ...
    let _ledger = self.run(ctx).await?;

    futures::future::pending::<()>().await;   // <-- line 318: blocks forever
    Ok::<(), RunnerError>(())
})
```

This `futures::future::pending::<()>().await` never resolves. It keeps the process alive but has no mechanism to detect if the voter actor inside the engine has crashed. The node will remain in this state indefinitely -- container running, RPC responding, metrics being scraped -- while consensus is dead.

There are no panic recovery wrappers anywhere in the startup path:

```rust
// The only crash-related setup is diagnostic, not recovery:
kora_cli::Backtracing::enable();
kora_cli::SigsegvHandler::install();
```

---

## What causes voter crashes

The voter panic occurs inside Commonware's Simplex implementation (external dependency, not Kora application code). Based on observed failure patterns, the likely triggers are:

### 1. Internal state machine violation

The Simplex voter maintains state about the current view, pending notarizations, and finalization history. If it detects an inconsistency -- receiving a message for a view it already finalized, or encountering a logical contradiction in its state transitions -- it panics rather than continuing in a corrupt state.

### 2. Channel closure

The voter receives messages from three channels: votes, certificates, and resolver. If any of these channels close unexpectedly (e.g., the network layer drops a sender), the voter's select loop may terminate, which the runtime interprets as the task "finishing" when it should not.

### 3. Accumulated consensus stress

During extended periods of high nullification rates (33%+ sustained), the voter processes many failure paths. The combination of repeated proposal failures (leader returning `None`), timeout cycles, nullification floor broadcasts, and view desynchronization between peers can push the state machine into an edge case that triggers a panic assertion.

### Conditions under which the crash has been observed

Based on production incidents, the voter crash correlates with:

1. Extended periods of high nullification (33%+ nullification ratio sustained for hours)
2. Rapid view advancement with no finalization (timeout cycles accumulating state)
3. Network partition or message loss (channels may close or become inconsistent)
4. After node restarts with stale journal state (journal and peer state diverge)

The crash has NOT been observed under:
- Normal operation with low nullification rates
- Short bursts of high load that resolve quickly
- Clean startup from genesis

### 4. Journal corruption or loss

The consensus journal is stored in the runtime storage directory (defaults to `data_dir/runtime`, or the path specified by `KORA_RUNTIME_DIR`). In the devnet configuration, this is a tmpfs mount:

```yaml
# docker/compose/devnet.yaml
tmpfs:
  - /runtime:size=1g,mode=1777
```

If the journal becomes corrupted or is lost (e.g., tmpfs cleared on container restart), the voter may fail to recover its state on startup and panic when it encounters an inconsistency between the journal and peer messages.

---

## Impact

### Single node crash

| Crashed Nodes | Active Validators | Quorum (2/3+1 = 3) | Effect |
|---|---|---|---|
| 0 | 4/4 | Met | Normal operation |
| 1 | 3/4 | Met | Chain continues with reduced redundancy |
| 2 | 2/4 | **Not met** | **Permanent chain halt** |

With a 4-validator set, the BFT threshold is 3 validators. A single voter crash leaves 3 active validators, which is the bare minimum for quorum. Any additional failure (second voter crash, network partition, resolver bug) immediately causes a permanent chain stall.

### Observed production data

From the reproduction test (node0 crash scenario):
- Node 0: 2 blocks finalized in 19 hours (completely dead consensus)
- Node 3: Also failed (catch-up bug), reducing active validators to 2
- Chain permanently stalled at view 2,794,655

### Observable symptoms

When a voter crashes, the following metrics are affected on the crashed node:

- `engine_voter_state_current_view` -- **frozen** (stops incrementing)
- `engine_voter_state_nullifications_total` -- **frozen** (counter stops)
- `engine_voter_outbound_messages_total` -- **zero** (no messages sent)
- `finalized_height` -- **frozen** (no blocks finalized)
- `up{job="kora-validators"}` -- **1** (node is still up and scrapeable)

From an infrastructure perspective, the node appears "up" but dead. Docker health checks pass because the healthcheck script tests container liveness, not consensus participation. The RPC endpoint responds to queries. Prometheus scrapes succeed.

### Distinguishing voter crash from other failure modes

| Symptom | Voter Crash | Quorum Loss | Mempool Poisoning |
|---|---|---|---|
| Node up (`up{}` = 1) | Yes | Some nodes down | All nodes up |
| View advancing | **No** (frozen) | No (frozen) | Yes (advancing) |
| Nullification count | Frozen | Frozen | Increasing rapidly |
| Finalization rate | Zero | Zero | Zero |
| RPC responding | Yes | Partial | Yes |
| Metrics scrapeable | Yes | Partial | Yes |

The key differentiator: a voter crash shows view **frozen** (voter is dead), while mempool poisoning shows view **advancing** but no finalization (voter is alive, but proposals all fail due to bad transactions).

---

## The VoterCrash Prometheus alert

This failure pattern is codified as an alert rule in `docker/config/alerts.yml`, lines 25-35:

```yaml
# docker/config/alerts.yml:25-35
- alert: VoterCrash
  expr: |
    (rate(finalized_height[1m]) == 0)
    and (up{job="kora-validators"} == 1)
    and (rate(engine_voter_state_current_view[1m]) == 0)
  for: 1m
  labels:
    severity: critical
  annotations:
    summary: "Possible voter crash on {{ $labels.instance }}"
    description: "Node is up but view is not advancing. Voter actor may have panicked."
```

This fires after 1 minute of the node being up but with zero view advancement. The alert is useful for notification but does not trigger any automated recovery. Related alerts that often fire in the same incident:

- `ConsensusStall` -- fires after 2 minutes of zero finalization
- `HeightDrift` -- fires when max-min finalized height exceeds 10 blocks
- `ViewWithoutFinalization` -- fires when other voters are alive but blocks are not being finalized
- `ResolverPeersBlocked` -- fires after restart when catch-up fails

The Grafana stall diagnostics dashboard (`docker/grafana/dashboards/kora-stall-diagnostics.json`) includes a "Nodes w/ Active Views" stat panel:

```json
{"expr": "count(rate(engine_voter_state_current_view[1m]) > 0)", "refId": "A"}
```

This shows how many nodes have actively advancing views. A drop from 4 to 3 (or fewer) indicates voter death.

---

## Recovery is NOT guaranteed after restart

Even after manually restarting the crashed node, recovery is not guaranteed due to a secondary bug in the Commonware resolver. The failure sequence is:

1. Restarted node needs to catch up from peers (it is potentially thousands of blocks behind).
2. The resolver fetches blocks from peer validators and calls `verify_block()` on each one.
3. `verify_block()` in `crates/node/runner/src/app.rs` (line 176) requires the parent block's state snapshot:

```rust
// crates/node/runner/src/app.rs:176-179
let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
    warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
    return false;
};
```

4. After a restart, state snapshots from before the crash are lost (the in-memory snapshot store is not persisted). `parent_snapshot()` returns `None`, and `verify_block()` returns `false`.
5. The resolver interprets verification failure as "invalid data" and **permanently blocks the peer**.
6. With all peers blocked, catch-up stalls indefinitely.

Evidence from production logs:
```
WARN commonware_resolver::p2p::engine: invalid data received peer=6667a1...
WARN commonware_resolver::p2p::engine: invalid data received peer=daf27c...
```

The `ResolverPeersBlocked` alert (line 181-188 of `docker/config/alerts.yml`) monitors for this:

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

### Current manual recovery procedure

1. Stop the container.
2. Clear the runtime storage directory (consensus journal).
3. Restart the container.
4. Hope that the catch-up does not trigger the resolver peer-blocking bug.
5. If catch-up fails, the only option is to reset all validators to a common checkpoint.

---

## Proposed fixes

### Fix 1: Add panic recovery wrapper around engine.start() (monitor Handle)

**Effort:** Low
**Impact:** High -- detects voter death and enables automated response

Instead of dropping the Handle returned by `engine.start()`, store it and spawn a monitoring task that awaits it:

```rust
// Proposed change to crates/node/runner/src/runner.rs
let handle = engine.start(
    transport.simplex.votes,
    transport.simplex.certs,
    transport.simplex.resolver,
);

// Monitor the voter handle for unexpected termination
let monitor_context = context.clone();
monitor_context.spawn(move |_| async move {
    // If engine handle resolves, the voter has died
    handle.await;
    tracing::error!("CRITICAL: consensus voter actor terminated unexpectedly");
    // Option A: Trigger process exit for container restart
    std::process::exit(1);
    // Option B: Attempt in-place engine restart (requires Commonware support)
});
```

This would convert the silent failure into either an immediate process exit (letting Docker's `restart: unless-stopped` policy handle recovery) or a logged critical error that operators can respond to.

### Fix 2: Add consensus health monitor background task

**Effort:** Medium
**Impact:** High -- provides defense-in-depth detection

Spawn a background task that periodically checks whether consensus metrics are advancing:

```rust
// Proposed: background health monitor
let health_ledger = ledger.clone();
context.spawn(move |ctx| async move {
    let mut last_view = 0u64;
    let mut stall_count = 0u32;
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let current_view = /* read from metrics or engine state */;
        if current_view == last_view {
            stall_count += 1;
            if stall_count >= 3 {
                tracing::error!(
                    view = current_view,
                    stall_seconds = stall_count * 10,
                    "consensus health check: view has not advanced for 30+ seconds"
                );
                // Trigger recovery action
            }
        } else {
            stall_count = 0;
        }
        last_view = current_view;
    }
});
```

This would detect voter death even if the Handle monitoring approach is not feasible, and would also catch other forms of consensus stall.

### Fix 3: Improve verify_block tolerance during catch-up

**Effort:** Medium
**Impact:** Medium -- prevents the secondary bug that blocks recovery

When a node is significantly behind (height gap > 100 blocks), relax the verification requirements so the resolver does not permanently block peers on expected parent-snapshot-missing failures. This could involve:

- Allowing state-less verification during catch-up (check block structure and signatures only)
- Using exponential backoff instead of permanent blocking in the resolver
- Pre-populating parent snapshots from the archive before verification

Relevant code in `crates/node/runner/src/app.rs`, lines 166-246 (the `verify_block()` method) and lines 327-383 (the `verify()` trait implementation).

### Fix 4: (Commonware upstream) Voter should return Result instead of panicking

**Effort:** High (requires upstream change)
**Impact:** Complete -- eliminates the root cause

The voter actor in `commonware_consensus::simplex::actors::voter::actor` uses `panic!()` / `unreachable!()` for internal invariant violations. These should be replaced with `Result<>` types so that state machine errors are propagated as errors rather than crashing the entire consensus actor. The engine could then attempt recovery or restart the voter in-place.

---

## Reproduction steps

1. Start the devnet:
   ```bash
   just devnet-run
   ```

2. Wait for all 4 validators to become healthy and producing blocks:
   ```bash
   just devnet-stats
   ```

3. Apply sustained heavy load to create high nullification conditions:
   ```bash
   cargo run --bin loadgen -- \
     --total-txs 50000 \
     --accounts 50 \
     --concurrency 200 \
     --rpc-url http://127.0.0.1:8545 \
     --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
   ```

4. Monitor for the voter crash in logs:
   ```bash
   docker logs kora-devnet-validator-node0-1 2>&1 | grep "voter should not finish"
   docker logs kora-devnet-validator-node0-1 2>&1 | grep "task panicked"
   ```

5. Check Prometheus for frozen view counter:
   ```
   rate(engine_voter_state_current_view[1m]) == 0 and up{job="kora-validators"} == 1
   ```

6. Verify the node is still "up" but consensus-dead:
   ```bash
   # RPC still responds -- kora_nodeStatus returns NodeStatus with view/finalization counters
   curl -s http://localhost:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}'
   # Response includes: currentView, finalizedCount, nullifiedCount, proposedCount
   # (see crates/node/rpc/src/state.rs lines 97-118 for NodeStatus struct)
   # A crashed voter shows frozen currentView and zero-rate finalizedCount
   ```

   The `devnet-stats` script provides a live terminal dashboard of all nodes:
   ```bash
   just devnet-stats
   # Shows per-node: status, view, finalized, nullified, proposed, blocks/sec
   # A crashed voter appears as "online" but with frozen view/finalized counters
   ```

7. Attempt restart and observe resolver peer-blocking:
   ```bash
   docker restart kora-devnet-validator-node0-1
   # Watch for "invalid data received" in logs
   docker logs -f kora-devnet-validator-node0-1 2>&1 | grep "invalid data"
   ```

---

## Load Test Evidence (2026-05-22)

During a 1,000-tx load test, a related panic was observed — this time it was the **resolver** rather than the voter:

```
ERROR commonware_runtime::utils::handle: task panicked err="resolver should not finish"
```

This confirms the general pattern: long-lived consensus actors (voter, resolver) can panic under load, and the current architecture has no recovery mechanism. The resolver panic caused node0 to restart via Docker, after which the block builder was permanently stuck due to stale pool state. The same `commonware_runtime::utils::handle` error path is shared by both the voter and resolver actors.

---

## Testing plan

- [ ] Unit test: Verify that if `engine.start()` returns a Handle that resolves, the monitoring task detects it and triggers the configured recovery action
- [ ] Integration test: Simulate voter death by killing the voter task (or closing a channel) and confirm that the health monitor detects the stall within 30 seconds
- [ ] Integration test: Confirm that the process exits with a non-zero code when the voter crashes (for Docker restart policy to work)
- [ ] Integration test: Restart a node that is 1000+ blocks behind and verify it successfully catches up without the resolver blocking all peers
- [ ] Devnet test: Run the full load scenario and confirm the voter crash is detected and the node auto-recovers via container restart
- [ ] Alert test: Trigger a voter crash and confirm the `VoterCrash` Prometheus alert fires within 2 minutes

---

## Relevant Prometheus metrics

| Metric | Purpose |
|---|---|
| `engine_voter_state_current_view` | Current consensus view. Frozen = voter dead |
| `engine_voter_state_nullifications_total` | Nullification counter. Frozen = voter dead |
| `engine_voter_state_timeouts_total` | Timeout counter by reason |
| `engine_voter_finalization_latency_*` | Finalization timing histogram |
| `engine_voter_outbound_messages_total` | Messages sent by voter. Zero = dead |
| `engine_resolver_resolver_peers_blocked` | Blocked peers preventing catch-up |
| `finalized_height` | Blocks finalized. Zero + node up = problem |
| `up{job="kora-validators"}` | Whether Prometheus can scrape the node |

---

## Code references

| Component | File | Line(s) |
|---|---|---|
| Engine start (voter spawned) | `crates/node/runner/src/runner.rs` | 574 |
| Engine configuration | `crates/node/runner/src/runner.rs` | 548-573 |
| Standalone main loop (no crash detection) | `crates/node/runner/src/runner.rs` | 303-320 |
| `futures::future::pending` await (blocks forever) | `crates/node/runner/src/runner.rs` | 318 |
| Consensus timeout constants | `crates/node/runner/src/runner.rs` | 48-53 |
| verify_block (parent_snapshot check) | `crates/node/runner/src/app.rs` | 166-246 |
| verify trait impl (ancestry iteration) | `crates/node/runner/src/app.rs` | 327-383 |
| NodeStateReporter (nullification tracking) | `crates/node/reporters/src/lib.rs` | 453-475 |
| inc_nullified on Nullification activity | `crates/node/reporters/src/lib.rs` | 468-469 |
| NodeState struct (RPC counters) | `crates/node/rpc/src/state.rs` | 16-31 |
| NodeStatus serializable struct | `crates/node/rpc/src/state.rs` | 97-118 |
| inc_nullified method | `crates/node/rpc/src/state.rs` | 71-72 |
| VoterCrash alert rule | `docker/config/alerts.yml` | 25-35 |
| ResolverPeersBlocked alert | `docker/config/alerts.yml` | 181-188 |
| Voter panic log query (Grafana) | `docker/grafana/dashboards/kora-logs.json` | 88 |
| Stall diagnostics: active view count | `docker/grafana/dashboards/kora-stall-diagnostics.json` | 69 |
| Docker restart policy | `docker/compose/devnet.yaml` | 28 (`restart: unless-stopped`) |
| Runtime storage directory | `crates/node/runner/src/runner.rs` | 74-83 |
| devnet-stats live monitor | `docker/scripts/devnet-stats.sh` | 92, 126-169 |

---

## Verification steps (after fix is applied)

After implementing any of the proposed fixes, verify correctness with:

1. **Handle monitoring test:** Start a devnet, then manually kill the voter task (or simulate engine.start() Handle resolving). Confirm the monitoring task logs a CRITICAL error and triggers the configured recovery action (process exit or restart).

2. **Process exit test:** After voter crash detection, confirm the process exits with a non-zero exit code. Then confirm Docker's `restart: unless-stopped` policy (line 28 of `docker/compose/devnet.yaml`) restarts the container automatically.

3. **Metrics verification:** After a voter crash, query Prometheus to confirm:
   ```
   rate(engine_voter_state_current_view[1m]) == 0  # View frozen on crashed node
   up{job="kora-validators"} == 1                   # Node still scrapeable
   ```
   Then after restart, confirm the view starts advancing again.

4. **RPC verification:** Query the `kora_nodeStatus` endpoint before and after the fix:
   ```bash
   curl -s http://localhost:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq
   ```
   Before fix: `currentView` is frozen, but no indication of crash. After fix: process should have exited and restarted, so `currentView` should be advancing.

5. **Alert verification:** After triggering a voter crash, confirm the `VoterCrash` alert in Prometheus fires within 2 minutes by querying:
   ```
   ALERTS{alertname="VoterCrash"}
   ```

6. **Catch-up verification (Fix 3):** Restart a node that is 1000+ blocks behind and confirm it successfully catches up without the resolver blocking all peers. Monitor with:
   ```
   engine_resolver_resolver_peers_blocked == 0  # No peers should be blocked
   ```
