# Network Partition Detection and Resilience

## Summary

Kora has no mechanism to detect, report, or recover from network partitions. When a validator loses connectivity to a quorum of peers, it silently enters an indefinite nullification loop with no logging, no metrics, and no RPC indication distinguishing this state from normal operation. The `/health` endpoint unconditionally returns `200 OK` regardless of node state, and the Docker healthcheck does not use the `/health` endpoint at all. Operators have no way to tell whether a node is slow, partitioned, or permanently stalled without manually comparing finalization counters across all nodes.

## Problem

### Current behavior

When a network partition occurs (or even when a single validator goes offline), the following happens:

1. The remaining validators continue advancing consensus views at full speed.
2. Every view where the missing/partitioned validator is leader triggers a 5-second `leader_timeout` followed by nullification.
3. The `peerCount` field in `kora_nodeStatus` remains unchanged because it is set once at startup and never updated.
4. There is no log line, metric, or RPC signal to indicate that the node cannot reach a quorum of peers.
5. In a 2-vs-2 split, both sides enter permanent nullification with zero blocks finalized and no alerting.
6. The `/health` endpoint always returns `200 OK` even when the node is partitioned or consensus is stalled.
7. The Docker healthcheck uses `eth_chainId` via JSON-RPC instead of the `/health` HTTP endpoint, meaning Docker considers a node "healthy" as long as the RPC server is accepting TCP connections -- regardless of consensus state.

### Evidence from testing

Four test reports document these behaviors:

**Single-node failure** (`controlled-single-node-failure.md`): Stopping one of four validators caused a 99.99% throughput drop (349 blocks/sec to 0.05 blocks/sec). All surviving nodes continued to report `peerCount: 3` even though one validator was confirmed down. The `peerCount` field is static -- it is computed once during startup in `runner.rs` and never refreshed:

```rust
// crates/node/runner/src/runner.rs, lines 568-569
let peer_count = self.scheme.participants().len().saturating_sub(1) as u64;
node_state.set_peer_count(peer_count);
```

**Rolling restart** (`rolling-restart-cascading-failure.md`): After sequential restarts, nodes ended up on different chain forks. The 2-vs-2 split (two nodes on the old chain, two on a fresh genesis chain) produced permanent deadlock. Neither partition could reach the 3/4 quorum threshold. No node logged any indication that it was in a split-brain state.

**Node failure recovery** (`node-failure-recovery-test.md`): After a two-node restart, the network entered a permanent stall with all four validators running but two unable to participate. Healthy nodes broadcast `nullification floor` messages endlessly. No metric or log distinguished this from the baseline 26% nullification rate during normal operation.

**Network partition** (`network-partition-test.md`): Partition testing could not be completed due to devnet instability, but architectural analysis confirmed there is no partition detection logic anywhere in the codebase. The predicted behavior for a 2-vs-2 partition is permanent deadlock with no alerting.

### Root cause analysis

There are four separate gaps:

**1. `peerCount` is static, not dynamic.**

In `crates/node/rpc/src/state.rs`, the `NodeState` struct stores `peer_count` as an `AtomicU64` (line 34), but it is only ever set once via `set_peer_count()` during runner initialization (line 569 of `runner.rs`). The `NodeStateReporter` in `crates/node/reporters/src/lib.rs` (lines 1089-1111) tracks view, finalization, and nullification activity but never updates peer connectivity:

```rust
// crates/node/reporters/src/lib.rs, lines 1095-1110
fn report(&mut self, activity: Self::Activity) -> impl std::future::Future<Output = ()> + Send {
    match &activity {
        Activity::Notarization(n) => {
            self.state.set_view(n.proposal.round.view().get());
        }
        Activity::Finalization(f) => {
            self.state.set_view(f.proposal.round.view().get());
            self.state.inc_finalized();
        }
        Activity::Nullification(_) => {
            self.state.inc_nullified();
        }
        _ => {}
    }
    async {}
}
```

There is no call to `set_peer_count()` anywhere in this reporter, nor is there any periodic task that queries for actual connection state.

**2. No partition detection heuristic.**

There is no logic anywhere in the codebase that correlates the nullification rate with peer connectivity. During normal operation, the baseline nullification rate is approximately 26% (idle nullification due to leader election scheduling). During a partition, the nullification rate jumps to near 100%. These are operationally very different states, but nothing in the code distinguishes them.

**3. No degraded-mode behavior.**

The node has exactly two operational states: running and crashed. There is no intermediate "degraded" state where the node would:
- Log a warning about inability to reach quorum
- Expose a "partitioned" or "degraded" status via RPC
- Return an error to RPC clients indicating the chain is stalled
- Fire a Prometheus alert for partition detection

**4. Health endpoint is a no-op; Docker healthcheck bypasses it entirely.**

The `/health` handler in `crates/node/rpc/src/server.rs` (line 573) unconditionally returns `200 OK` without inspecting any node state:

```rust
// crates/node/rpc/src/server.rs, line 573
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
```

The handler does not accept `State(state)`, even though the HTTP router has `.with_state(node_state)` at line 221 -- the state is available but unused.

Meanwhile, the Docker healthcheck in `docker/scripts/healthcheck.sh` uses the `ready` mode for validators, which calls `eth_chainId` on the JSON-RPC port (8545), not the `/health` endpoint on the HTTP port (8546):

```bash
# docker/scripts/healthcheck.sh, lines 13-19
ready)
    # Verify the RPC server is responsive with a real method call
    RESULT=$(curl -sf -X POST http://localhost:8545 \
        -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' 2>/dev/null) || exit 1
    echo "$RESULT" | jq -e '.result' >/dev/null 2>&1
    ;;
```

This means:
- A partitioned node with a responsive RPC server is Docker-healthy.
- A node in permanent consensus stall is Docker-healthy.
- A node with zero peers reachable is Docker-healthy.
- The `/health` endpoint serves no operational purpose -- nothing queries it.

### Why this matters

In a production multi-server deployment, network partitions are common:
- Cross-datacenter link failures
- Firewall/security group misconfigurations
- DNS or BGP route changes
- Cloud provider network maintenance

Without partition detection, operators must manually compare `finalizedCount` and `nullifiedCount` across all nodes and do arithmetic to determine whether the network is healthy. By the time a human notices, the chain may have been stalled for minutes or hours. In a 2-vs-2 split, there is no automatic recovery -- the chain deadlocks permanently until an operator manually intervenes.

## Proposed Solution

### Proposal 1: Finalization stall detector (no upstream dependencies)

This is the primary detection mechanism. It uses data already tracked by `NodeState` and requires zero changes to the commonware framework.

**Rationale**: Direct peer connectivity (e.g., `oracle.connected_count()`) is not exposed by the `commonware_p2p::authenticated::discovery::Oracle` type. The Oracle implements the `Manager` trait (for `track()`) and the `Blocker` trait (for `block()`), but neither trait provides a method to query how many peers are currently connected. Adding such an API would require an upstream commonware change. Instead, this proposal detects the *symptom* of partition -- consensus stall -- using existing counters.

**File: `crates/node/runner/src/runner.rs`**

Add a new function alongside the existing `spawn_txpool_cleanup`:

```rust
const STALL_DETECTION_INTERVAL: Duration = Duration::from_secs(10);
const STALL_THRESHOLD_SECS: u64 = 30; // no finalization for 30s = stall

fn spawn_stall_detector(
    node_state: NodeState,
    context: cw_tokio::Context,
) {
    context.with_label("stall-detector").shared(false).spawn(move |ctx| async move {
        let mut last_finalized = 0u64;
        let mut stall_start: Option<Instant> = None;

        loop {
            ctx.sleep(STALL_DETECTION_INTERVAL).await;
            let status = node_state.status();

            if status.finalized_count > last_finalized {
                // Making progress
                last_finalized = status.finalized_count;
                if stall_start.is_some() {
                    info!(
                        finalized = status.finalized_count,
                        "consensus resumed: finalization progressing again"
                    );
                    node_state.set_degraded(false);
                }
                stall_start = None;
            } else if status.current_view > 0 {
                // Views advancing but no finalization = stall
                let start = stall_start.get_or_insert_with(Instant::now);
                let stall_duration = start.elapsed();

                if stall_duration.as_secs() >= STALL_THRESHOLD_SECS {
                    warn!(
                        stall_secs = stall_duration.as_secs(),
                        last_finalized = status.finalized_count,
                        current_view = status.current_view,
                        nullified = status.nullified_count,
                        "CONSENSUS STALL: no finalization for {}s -- \
                         possible network partition or insufficient quorum",
                        stall_duration.as_secs(),
                    );
                    node_state.set_degraded(true);
                }
            }
        }
    });
}
```

Call this from `ProductionRunner::run()` near line 553 where `spawn_txpool_cleanup` is already called:

```rust
spawn_txpool_cleanup(txpool.clone(), context.clone());
if let Some((node_state, _)) = &self.rpc_config {
    spawn_stall_detector(node_state.clone(), context.clone());
}
```

### Proposal 2: Extend `NodeState` and `NodeStatus` with degraded flag

**File: `crates/node/rpc/src/state.rs`**

Add an `is_degraded` field to `NodeStateInner` and `NodeStatus`:

```rust
use std::sync::atomic::AtomicBool;

// Add to NodeStateInner (around line 26):
is_degraded: AtomicBool,

// Add to NodeStatus (around line 126):
/// Whether the node believes it is in a consensus stall
/// (no finalization for longer than the stall threshold).
pub is_degraded: bool,
```

Add corresponding setter and getter methods on `NodeState`:

```rust
/// Mark the node as degraded (consensus stalled).
pub fn set_degraded(&self, degraded: bool) {
    self.inner.is_degraded.store(degraded, Ordering::Relaxed);
}

/// Check whether the node is in degraded state.
pub fn is_degraded(&self) -> bool {
    self.inner.is_degraded.load(Ordering::Relaxed)
}
```

Update `status()` to include `is_degraded` in the returned `NodeStatus`.

### Proposal 3: Fix the `/health` endpoint to reflect node state

**File: `crates/node/rpc/src/server.rs`**

The current handler ignores the `NodeState` that is already available via the axum router state. Change the handler to accept and inspect state:

```rust
async fn health_handler(State(state): State<Arc<NodeState>>) -> impl IntoResponse {
    let status = state.status();

    if status.is_degraded {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "status": "degraded",
                "reason": "consensus stall detected",
                "currentView": status.current_view,
                "finalizedCount": status.finalized_count,
                "nullifiedCount": status.nullified_count,
                "peerCount": status.peer_count
            })),
        );
    }

    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "status": "healthy",
            "currentView": status.current_view,
            "finalizedCount": status.finalized_count
        })),
    )
}
```

No route registration change is needed because the router already has `.with_state(node_state)` at line 221. The handler signature change is sufficient for axum to inject the state.

The existing test at line 917 (`http_status_rate_limiter_returns_too_many_requests`) already constructs a `NodeState` for the router, so updating the handler signature will not break it. However, the test should be updated to verify that a non-degraded node returns `200 OK` and a degraded node returns `503`.

### Proposal 4: Fix Docker healthcheck to use the `/health` endpoint

**File: `docker/scripts/healthcheck.sh`**

Change the `ready` mode to use the HTTP `/health` endpoint instead of `eth_chainId`. The HTTP status server runs on port 8546 (JSON-RPC port + 1, per `default_http_addr` in `server.rs` line 261):

```bash
ready)
    # Check the /health endpoint on the HTTP status server (port = RPC + 1).
    # This returns 503 when the node detects a consensus stall, which
    # causes Docker to mark the container as unhealthy.
    HTTP_PORT="${HEALTH_HTTP_PORT:-8546}"
    curl -sf "http://localhost:${HTTP_PORT}/health" >/dev/null 2>&1
    ;;
```

This creates a feedback loop: stall detector sets degraded flag -> `/health` returns 503 -> Docker marks container unhealthy -> load balancers/orchestrators can react.

**Note on port mapping**: The `devnet.yaml` compose file maps the JSON-RPC port (8545) but does not expose the HTTP status port (8546) outside the Docker network. For the healthcheck this is fine because it runs inside the container. For external monitoring, either expose port 8546 or add a `/health` route to the JSON-RPC server as well.

### Proposal 5: Prometheus metrics for partition detection

**File: `crates/node/runner/src/runner.rs`** (in the metrics endpoint, or via `context.gauge(...)`)

Expose new Prometheus gauges that alerting rules can trigger on:

- `kora_node_degraded` (gauge, 0 or 1): Whether the node detects a consensus stall.
- `kora_seconds_since_last_finalization` (gauge): Seconds since the last block was finalized. Alerting threshold: > 30s.
- `kora_nullification_ratio` (gauge): `nullified_count / current_view`. Alert if > 0.5 sustained for > 30s.

These can be derived from the existing `NodeState` fields in the `/metrics` handler, or exposed as commonware runtime metrics via `context.gauge(...)`.

### Proposal 6: Dynamic peer connectivity (requires upstream commonware change)

**Status: BLOCKED -- requires upstream API addition to `commonware_p2p`.**

The `discovery::Oracle<P>` type (used as `transport.oracle` in `runner.rs`) currently exposes:
- `Manager::track()` -- register the set of known peers for an epoch
- `Blocker::block()` -- ban a misbehaving peer

It does **not** expose any method to query how many peers are currently connected (e.g., a hypothetical `connected_count()` or `connected_peers()` method). The `TrackedPeers` struct passed to `track()` describes *authorized* peers, not *connected* peers.

To implement dynamic peer counting, one of the following upstream changes is needed:

**Option A**: Add `fn connected_count(&self) -> impl Future<Output = usize>` to the `Manager` trait or directly on `Oracle<P>`.

**Option B**: Add a `fn connected_peers(&self) -> impl Future<Output = Vec<P>>` method that returns the subset of tracked peers with active connections.

**Option C**: Expose connection lifecycle events (peer connected / peer disconnected) as a stream or callback so Kora can maintain its own counter.

Until one of these is available, Kora must rely on the stall detector (Proposal 1) for partition detection. If an upstream API becomes available, add a periodic task:

```rust
const PEER_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(5);

fn spawn_peer_health_monitor(
    oracle: discovery::Oracle<Peer>,
    node_state: NodeState,
    validator_count: usize,
    context: cw_tokio::Context,
) {
    let quorum_threshold = (validator_count * 2 / 3) + 1; // 2f+1
    context.with_label("peer-health").shared(false).spawn(move |ctx| async move {
        loop {
            ctx.sleep(PEER_HEALTH_CHECK_INTERVAL).await;
            // Hypothetical API -- does not exist today
            let connected = oracle.connected_count().await;
            node_state.set_peer_count(connected as u64);

            if connected + 1 < quorum_threshold {
                warn!(
                    connected_peers = connected,
                    quorum_needed = quorum_threshold,
                    "below quorum: cannot reach enough peers for consensus"
                );
            }
        }
    });
}
```

### Health semantics by node state

The `/health` endpoint should reflect these distinct node states:

| State | Description | HTTP Status | `status` field |
|-------|------------|-------------|----------------|
| **Starting** | RPC server is up but consensus has not yet started (view == 0, finalized == 0) | `200 OK` | `"starting"` |
| **Healthy** | Finalization is progressing (finalized count increased within the last stall threshold window) | `200 OK` | `"healthy"` |
| **Degraded** | Stall detector has fired (no finalization for > `STALL_THRESHOLD_SECS`) | `503 Service Unavailable` | `"degraded"` |

The "syncing" and "caught-up" distinction is not yet meaningful for Kora because there is no state sync protocol. When state sync is implemented, add a `"syncing"` status that returns `200 OK` but with a `syncing: true` flag so load balancers can distinguish a syncing node from a fully caught-up one.

The "participating-in-consensus" state is implicitly covered by the `is_leader` field in `NodeStatus` and the finalization counter -- a node that is finalizing blocks is participating. A node that has stopped finalizing is reported as degraded.

## Implementation order

The proposals are independent and can be implemented incrementally:

1. **Proposal 2** (add `is_degraded` to `NodeState`) -- prerequisite for proposals 1 and 3.
2. **Proposal 1** (stall detector) -- the core detection logic.
3. **Proposal 3** (fix `/health` endpoint) -- make the detection visible via HTTP.
4. **Proposal 4** (fix Docker healthcheck) -- wire Docker to the fixed endpoint.
5. **Proposal 5** (Prometheus metrics) -- enable alerting.
6. **Proposal 6** (dynamic peer count) -- deferred until upstream API exists.

Proposals 1-4 can be shipped as a single PR. Proposal 5 can follow separately. Proposal 6 is blocked on upstream.

## Files to modify

| File | Change |
|------|--------|
| `crates/node/rpc/src/state.rs` | Add `is_degraded: AtomicBool` to `NodeStateInner`; add `is_degraded: bool` to `NodeStatus`; add `set_degraded()` / `is_degraded()` methods |
| `crates/node/runner/src/runner.rs` | Add `spawn_stall_detector()`; call from `ProductionRunner::run()` after `spawn_txpool_cleanup` |
| `crates/node/rpc/src/server.rs` | Change `health_handler()` to accept `State(state)` and return 503 when degraded; update test |
| `docker/scripts/healthcheck.sh` | Change `ready` mode to `curl` the `/health` endpoint on port 8546 instead of calling `eth_chainId` on port 8545 |
| `crates/node/rpc/src/kora.rs` | No changes needed (already returns full `NodeStatus` which will include `is_degraded`) |
| `crates/node/reporters/src/lib.rs` | No changes needed (stall detector reads from `NodeState` which the reporter already updates) |

## Complexity

Medium. Proposals 1-4 are self-contained, require no upstream commonware changes, and can be implemented with careful review despite the local toolchain limitation (Rust 1.83, needs 1.91+). The stall detector is a simple state machine comparing `finalized_count` snapshots on a timer. The health endpoint change is a one-line handler signature change plus response logic.

## Testing

Due to local toolchain limitations, unit testing requires careful code review rather than local compilation. However:

- The stall detector logic is pure state machine code that can be unit tested by mocking `NodeState` -- inject a `NodeState`, advance the finalized count (or not), and verify `is_degraded` transitions.
- The health endpoint change can be tested with the existing test pattern in `server.rs` (line 917). Construct a `NodeState`, call `set_degraded(true)`, send a request to `/health`, and assert `503`. Similarly test the non-degraded path.
- The Docker healthcheck change can be verified by running the devnet and checking `docker inspect --format='{{.State.Health.Status}}'` during normal operation and during a simulated partition (stop 2 of 4 validators).
- Full integration testing requires a stable 4-node devnet. The partition scenarios described in `network-partition-test.md` provide reproduction steps once the devnet stability issues are resolved.

## Related issues

- The 5-second `leader_timeout_secs` default in `crates/node/config/src/consensus.rs` amplifies partition impact by making each dead-leader view waste 5 seconds. Reducing this to 1 second is an orthogonal improvement.
- State sync (not yet implemented) is required for nodes that restart during a partition to rejoin the chain. Partition detection helps operators identify when this has happened, but does not solve the underlying state sync gap.
- The `NoOpBlocker` in `crates/node/runner/src/runner.rs` (lines 72-89) prevents permanent peer blocking after restart but does not provide positive peer health information.
- PR #131 (fix resolver blocking) is related but addresses a different problem (preventing permanent bans vs. detecting partitions).
