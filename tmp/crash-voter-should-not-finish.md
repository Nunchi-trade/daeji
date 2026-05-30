# Crash Analysis: "voter should not finish"

## Overview

The "voter should not finish" crash is a fatal failure mode in Kora where the Simplex BFT consensus voter actor terminates unexpectedly. This causes the affected node to appear healthy at the infrastructure level (container running, RPC responding) while consensus is permanently broken on that node.

---

## What Is the Voter Actor?

In Kora's consensus architecture, the **voter actor** is a long-lived async task inside the Commonware Simplex engine. It is the core consensus participant responsible for:

- Receiving block proposals from leaders
- Casting notarization votes (first-round approval)
- Casting finalization votes (second-round commitment)
- Broadcasting nullification floor messages when views fail
- Maintaining the consensus state machine (view number, pending certificates)

The voter is spawned when `engine.start()` is called:

```rust
// crates/node/runner/src/runner.rs:574
engine.start(transport.simplex.votes, transport.simplex.certs, transport.simplex.resolver);
```

This call hands the voter three network channels (votes, certificates, resolver) and starts it as a background task. The voter processes messages from these channels indefinitely. Termination of the voter means the node can no longer participate in consensus.

---

## The Error Message

```
ERROR commonware_runtime::utils::handle: task panicked err="voter should not finish"
```

This error is logged by the Commonware runtime's panic handler. The voter task is designed to run for the lifetime of the node. The runtime wraps spawned tasks with a panic handler that logs the error if any task panics. The message "voter should not finish" is the panic payload, indicating that the voter future resolved (completed) when it was expected to run indefinitely.

The Loki/Grafana log query that detects this:

```logql
{node_type=~"validator|secondary"} |~ "task panicked|voter should not finish|PANIC"
```

This is configured as a panel in the `kora-logs.json` Grafana dashboard (`docker/grafana/dashboards/kora-logs.json:88`).

---

## Where in the Code This Happens

The voter actor lives in the Commonware library:

```
commonware_consensus::simplex::actors::voter::actor
```

This is an external dependency (`commonware-consensus`), not Kora application code. The actor runs as a tokio task spawned by the Commonware runtime. When the internal state machine reaches an unexpected terminal state or an invariant is violated, the actor panics with `"voter should not finish"`.

On the Kora side, the engine is started without any panic recovery wrapper:

```rust
// crates/node/runner/src/runner.rs:548-574
let engine = simplex::Engine::new(context.with_label("engine"), simplex::Config { ... });
engine.start(transport.simplex.votes, transport.simplex.certs, transport.simplex.resolver);
```

The `engine.start()` call returns a `Handle` but this handle is not awaited or monitored for panics. The Kora node then calls `futures::future::pending::<()>().await` (in standalone mode) or simply returns the `LedgerService` handle, with no mechanism to detect voter death.

```rust
// crates/node/runner/src/runner.rs:318
futures::future::pending::<()>().await;
```

The `LegacyNodeService` and `ProductionRunner` implementations do not wrap the engine in panic recovery. Once the voter panics, consensus on that node is permanently dead.

---

## What Causes the Voter to Crash

The voter panic occurs inside Commonware's Simplex implementation. Based on the observed failure patterns, the likely triggers are:

### 1. Internal State Machine Violation

The Simplex voter maintains state about the current view, pending notarizations, and finalization history. If it detects an inconsistency (e.g., receiving a message for a view it already finalized, or encountering a logical contradiction in its state transitions), it panics rather than continuing in a corrupt state.

### 2. Channel Closure

The voter receives messages from three channels: votes, certificates, and resolver. If any of these channels close unexpectedly (e.g., the network layer drops), the voter's select loop may terminate, which the runtime interprets as the task "finishing" when it should not.

### 3. Accumulated Consensus Stress

During extended periods of high nullification rates, the voter processes many failure paths. The combination of:
- Repeated proposal failures (leader returning `None`)
- Timeout cycles
- Nullification floor broadcasts
- View desynchronization between peers

...can push the state machine into an edge case that triggers a panic assertion.

### 4. Journal Corruption or Loss

If the consensus journal (stored in the runtime storage directory) becomes corrupted or is lost (e.g., tmpfs cleared on container restart), the voter may fail to recover its state on startup and panic when it encounters an inconsistency between the journal and peer messages.

---

## Impact

### Immediate Impact

- The node's consensus participation stops completely.
- No more votes are cast, so the node does not contribute to notarization or finalization.
- The node continues running: Docker health checks pass, RPC responds, metrics are scraped.
- From an external monitoring perspective, the node appears "up" but dead.

### Network Impact

With a 4-validator set (BFT threshold = 2/3+1 = 3 validators needed):

| Crashed Nodes | Effect |
|---------------|--------|
| 1 | Chain continues with 3/4 participating (still meets 2/3 threshold) |
| 2 | Chain stalls permanently (2/4 < 2/3, no quorum) |

### Observed Production Data

From the reproduction test (node0 crash scenario):
- Node 0: 2 blocks finalized in 19 hours (completely dead consensus)
- Node 3: Also failed (catch-up bug), reducing active validators to 2
- Chain permanently stalled at view 2,794,655

---

## How to Detect

### Primary Signal: View Stops Advancing

The clearest indicator is that the node's `engine_voter_state_current_view` metric stops incrementing while other nodes continue advancing:

```promql
# Detect: node is up but view is frozen
(rate(finalized_height[1m]) == 0)
  and (up{job="kora-validators"} == 1)
  and (rate(engine_voter_state_current_view[1m]) == 0)
```

### VoterCrash Prometheus Alert

This exact pattern is codified as the `VoterCrash` alert in Kora's alert rules:

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

This fires after 1 minute of the node being up but with zero view advancement.

### Distinguishing from Other Stalls

| Symptom | VoterCrash | Quorum Loss | Mempool Poisoning |
|---------|-----------|-------------|-------------------|
| Node up | Yes | Some down | All up |
| View advancing | No (frozen) | No (frozen) | Yes (advancing) |
| Nullification count | Frozen | Frozen | Increasing rapidly |
| Finalization | Zero | Zero | Zero |
| `up{}` metric | 1 | 0 for dead nodes | 1 |

The key differentiator: VoterCrash shows view frozen (voter is dead), while mempool poisoning shows view advancing but no finalization (voter is alive but proposals all fail).

### Log Detection

Search container logs for the panic message:

```bash
docker logs <container> 2>&1 | grep "voter should not finish"
docker logs <container> 2>&1 | grep "task panicked"
```

The Grafana Loki panel at `docker/grafana/dashboards/kora-logs.json` monitors for these patterns automatically.

---

## Is This Recoverable?

**No.** Once the voter actor panics, the node requires a full restart to recover consensus. There is no mechanism in Kora to:
- Detect the voter panic at the application level
- Restart the consensus engine in-place
- Re-spawn just the voter actor

The Kora main process does not have panic recovery wrappers:

```rust
// Only crash diagnostics, no panic recovery
kora_cli::Backtracing::enable();
kora_cli::SigsegvHandler::install();
```

### Recovery After Restart

Even after restart, recovery is **not guaranteed** due to a second bug in the Commonware resolver:

1. Restarted node needs to catch up from peers (it is thousands of blocks behind).
2. The resolver fetches blocks and calls `verify_block()` on them.
3. Verification may fail due to missing parent snapshots (state was lost).
4. The resolver interprets verification failure as "invalid data" and **permanently blocks the peer**.
5. With all peers blocked, catch-up stalls indefinitely.

Evidence from production logs:
```
WARN commonware_resolver::p2p::engine: invalid data received peer=6667a1...
WARN commonware_resolver::p2p::engine: invalid data received peer=daf27c...
```

### Current Recovery Procedure

1. Stop the container
2. Clear the runtime storage directory (consensus journal)
3. Restart the container
4. Hope that the catch-up does not trigger the resolver peer-blocking bug
5. If catch-up fails, the only option is to reset all validators to a common checkpoint

---

## Under What Conditions This Triggers

Based on observed incidents, the voter crash correlates with:

1. **Extended periods of high nullification** (33%+ nullification ratio sustained for hours)
2. **Rapid view advancement with no finalization** (timeout cycles accumulating state)
3. **Network partition or message loss** (channels may close or become inconsistent)
4. **After node restarts with stale journal state** (journal and peer state diverge)

The crash has NOT been observed under:
- Normal operation with low nullification rates
- Short bursts of high load that resolve quickly
- Clean startup from genesis

---

## Relevant Prometheus Metrics

| Metric | Purpose |
|--------|---------|
| `engine_voter_state_current_view` | Current consensus view. Frozen = voter dead |
| `engine_voter_state_nullifications_total` | Nullification counter. Frozen = voter dead |
| `engine_voter_state_timeouts_total` | Timeout counter by reason |
| `engine_voter_finalization_latency_*` | Finalization timing histogram |
| `engine_voter_notarization_latency_*` | Notarization timing histogram |
| `engine_voter_outbound_messages_total` | Messages sent by voter. Zero = dead |
| `engine_voter_journal_tracked` | Journal state tracking |
| `finalized_height` | Blocks finalized. Zero + node up = problem |
| `up{job="kora-validators"}` | Whether Prometheus can scrape the node |

---

## Related Alerts

Beyond `VoterCrash`, these alerts often fire in the same incident:

| Alert | Relation |
|-------|----------|
| `ConsensusStall` | Fires after 2min of zero finalization. Voter crash is one cause. |
| `ViewWithoutFinalization` | If other voters are still alive, views advance but nothing finalizes. |
| `HeightDrift` | When a crashed node falls behind, max-min height diverges. |
| `ResolverPeersBlocked` | After restart, catch-up failure blocks peers, preventing recovery. |
| `HighNullificationRate` | Often a precursor: sustained high nullification precedes voter crash. |

---

## Code References

| Component | File | Line |
|-----------|------|------|
| Engine start (voter spawned) | `crates/node/runner/src/runner.rs` | 574 |
| Engine configuration | `crates/node/runner/src/runner.rs` | 548-573 |
| Voter log source | `commonware_consensus::simplex::actors::voter::actor` | (external) |
| Panic log source | `commonware_runtime::utils::handle` | (external) |
| Nullification tracking | `crates/node/reporters/src/lib.rs` | 468-470 |
| VoterCrash alert rule | `docker/config/alerts.yml` | 25-35 |
| Log dashboard query | `docker/grafana/dashboards/kora-logs.json` | 88 |
| Standalone await (no panic catch) | `crates/node/runner/src/runner.rs` | 318 |

---

## Fixes Required

### Short-term (Kora Application)

1. **Add panic recovery wrapper around `engine.start()`** - Monitor the returned `Handle` and detect if the voter task completes. On completion, either restart the engine or trigger an alert + graceful shutdown.

2. **Add consensus health monitor** - A background task that checks if `engine_voter_state_current_view` is advancing. If it stalls for >30s while the node is otherwise healthy, log a critical alert and optionally restart.

3. **Improve verify_block tolerance during catch-up** - When a node is significantly behind (height gap > 100), relax verification requirements so the resolver does not block peers on expected parent-state-missing failures.

### Medium-term (Commonware Library)

1. **Voter should not panic** - Replace internal `panic!()` / `unreachable!()` with `Result<>` types so that invariant violations are handled gracefully rather than crashing the entire consensus actor.

2. **Resolver should not permanently block peers** - Use exponential backoff and retry instead of immediately blocking peers that provide blocks that fail application-level verification.

3. **Add journal recovery** - Allow consensus to resume from the last known good state even if the journal is partially corrupted or lost.

4. **Separate catch-up mode** - When a node is thousands of blocks behind, use a lightweight catch-up protocol that does not require full state verification for every block.
