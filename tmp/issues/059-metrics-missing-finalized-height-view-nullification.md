# Missing Prometheus Metrics: Finalized Height, Current View, Nullification Count

**Category:** metrics, enhancement
**Severity:** medium

## Summary

Three of the most operationally important metrics for monitoring a BFT consensus node -- finalized block height, current consensus view, and nullification count -- are tracked internally by the `NodeState` struct and exposed via the `/status` JSON-RPC endpoint, but are not registered as Prometheus gauges/counters. This means Prometheus-based alerting (e.g., "finalized height stopped advancing") and Grafana dashboards cannot access this data without fragile JSON endpoint scraping.

## Problem

Kora is an EVM execution client using commonware simplex BFT consensus. It exposes Prometheus metrics via a `/metrics` HTTP endpoint (powered by the commonware runtime's metrics registry). Application-level metrics are defined in the `AppMetrics` struct (`crates/node/metrics/src/lib.rs`), which includes counters/gauges for txpool stats, block build time, EVM execution time, gossip stats, and equivocation detection -- but NOT for finalized height, current view, or nullification count.

Meanwhile, the `NodeState` struct (`crates/node/rpc/src/state.rs`) tracks all three values internally:

1. **Finalized height** (line 71): `finalized_height: AtomicU64` -- updated via `set_finalized_height()` (line 145)
2. **Current view** (line 69): `current_view: AtomicU64` -- updated via `set_view()` (line 130)
3. **Nullification count** (line 73): `nullified_count: AtomicU64` -- updated via `inc_nullified()` (line 160)

These values are available via the `/status` JSON endpoint (`status()` method, lines 213-232) but are not registered with the Prometheus metrics registry.

The `NodeStateReporter` (in `crates/node/reporters/src/lib.rs`, lines 1575-1649) is the component that receives consensus activity events and updates `NodeState`. Its `report()` method handles three relevant events:

- `Activity::Notarization(n)` -> `self.state.set_view(...)` (line 1613)
- `Activity::Finalization(f)` -> `self.state.set_view(...)` and `self.state.inc_finalized()` (lines 1616-1617)
- `Activity::Nullification(_)` -> `self.state.inc_nullified()` (line 1620)

The finalized height is separately updated in `handle_finalized_update()` (line 257): `ns.set_finalized_height(block.height)`.

None of these update paths also update Prometheus metrics.

## Code Reference

File: `crates/node/metrics/src/lib.rs`, lines 39-112 (the `AppMetrics` struct -- note the ABSENCE of finalized_height, current_view, or nullification metrics):

```rust
pub struct AppMetrics {
    // -- Transaction Pool --
    pub txpool_size: Gauge,
    pub txpool_pending: Gauge,
    pub txpool_queued: Gauge,
    pub txpool_rejected: Family<ReasonLabel, Counter>,

    // -- Block Building --
    pub block_build_time: Histogram,
    pub block_txs_included: Gauge,

    // -- Proposal health --
    pub proposal_snapshot_misses: Counter,
    pub proposal_lag_skips: Counter,
    pub snapshot_poll_wait: Histogram,

    // -- Finalization --
    pub finalization_failures: Counter,
    pub blocks_finalized: Counter,           // <-- counts, but NOT height gauge

    // -- EVM Execution --
    pub evm_execution_seconds: Histogram,

    // -- RPC --
    pub rpc_requests_total: Counter,

    // -- Snapshot Store --
    pub unpersisted_snapshot_depth: Gauge,
    pub snapshot_store_total: Gauge,

    // -- Transaction Gossip --
    pub gossip_tx_broadcast: Counter,
    pub gossip_tx_received: Counter,
    pub gossip_tx_broadcast_failed: Counter,
    pub gossip_tx_invalid: Counter,

    // -- Equivocation --
    pub equivocations: Family<EquivocationTypeLabel, Counter>,

    // MISSING: finalized_height: Gauge
    // MISSING: current_view: Gauge
    // MISSING: nullifications_total: Counter
}
```

File: `crates/node/rpc/src/state.rs`, lines 69-73 (the values ARE tracked internally):

```rust
    current_view: AtomicU64,
    finalized_count: AtomicU64,
    finalized_height: AtomicU64,
    proposed_count: AtomicU64,
    nullified_count: AtomicU64,
```

File: `crates/node/reporters/src/lib.rs`, lines 1610-1621 (where updates happen but no Prometheus metrics are set):

```rust
    fn report(&mut self, activity: Self::Activity) -> Feedback {
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
```

File: `crates/node/reporters/src/lib.rs`, line 257 (finalized height set in a different reporter):

```rust
                ns.set_finalized_height(block.height);
```

## Impact

1. **No stall detection alerting**: The most critical consensus health check is "finalized height stopped advancing for N seconds". Without a `kora_finalized_height` gauge, this alert cannot be configured in Prometheus/Alertmanager. Operators must manually watch logs or poll the `/status` endpoint, which is not reliable for automated monitoring.

2. **No nullification rate dashboard**: The primary quality metric for BFT consensus is nullification rate (`nullifications / (nullifications + finalizations)`). Without a `kora_nullifications_total` counter, this cannot be computed in Grafana. The existing `kora_blocks_finalized` counter provides the denominator but the numerator is missing.

3. **No view liveness monitoring**: If the current view stops advancing, consensus is completely halted. Without a `kora_current_view` gauge, there is no way to alert on this condition via Prometheus.

4. **Incomplete observability**: The Prometheus `/metrics` endpoint includes detailed metrics for txpool, execution, gossip, and equivocation, but is missing the three most fundamental consensus health signals. This creates a misleading picture where secondary metrics are available but primary ones are not.

Note: The commonware SDK exposes `scratch_marshal_finalized_height` as an internal SDK metric, but this is under SDK control and its semantics may not match the application-level finalized height exactly.

## Root Cause

The `AppMetrics` struct was built incrementally, starting with txpool and execution metrics. The fundamental consensus gauges (height, view, nullification) were tracked in `NodeState` for the `/status` endpoint but were never also registered as Prometheus metrics because the two systems were developed independently.

## Suggested Fix

Add three new metrics to `AppMetrics`:

**In `crates/node/metrics/src/lib.rs`**, add to the struct:

```rust
    // -- Consensus State --
    /// Latest finalized block height.
    pub finalized_height: Gauge,
    /// Current consensus view number.
    pub current_view: Gauge,
    /// Total number of nullified rounds.
    pub nullifications_total: Counter,
```

**In `AppMetrics::new()`**, initialize them:

```rust
    finalized_height: Gauge::default(),
    current_view: Gauge::default(),
    nullifications_total: Counter::default(),
```

**In `AppMetrics::register()`**, register them:

```rust
    registry.register(
        "kora_finalized_height",
        "Latest finalized block height",
        self.finalized_height.clone(),
    );
    registry.register(
        "kora_current_view",
        "Current consensus view number",
        self.current_view.clone(),
    );
    registry.register(
        "kora_nullifications",
        "Total nullified consensus rounds",
        self.nullifications_total.clone(),
    );
```

**In `crates/node/reporters/src/lib.rs`**, update `NodeStateReporter::report()` to also set the Prometheus metrics:

```rust
Activity::Finalization(f) => {
    self.state.set_view(f.proposal.round.view().get());
    self.state.inc_finalized();
    if let Some(ref m) = self.metrics {
        m.current_view.set(f.proposal.round.view().get() as i64);
    }
}
Activity::Nullification(_) => {
    self.state.inc_nullified();
    if let Some(ref m) = self.metrics {
        m.nullifications_total.inc();
    }
}
```

**In `handle_finalized_update()`** (line 257), set the finalized height metric:

```rust
if let Some(ref ns) = node_state {
    ns.set_finalized_height(block.height);
}
if let Some(ref m) = metrics {
    m.finalized_height.set(block.height as i64);
}
```

## Files to Modify

- `crates/node/metrics/src/lib.rs` -- add three new metrics to `AppMetrics` struct and `register()` method
- `crates/node/reporters/src/lib.rs` -- update `NodeStateReporter::report()` and `handle_finalized_update()` to set metrics

## Related Issues

- `060-metrics-missing-peer-count-gauge.md` -- another missing Prometheus metric (peer count)
- `058-docker-dialable-addr-not-set-4node-devnet.md` -- nullification rate metrics are needed to detect the effects of partial P2P mesh

## Labels

enhancement, metrics, consensus
