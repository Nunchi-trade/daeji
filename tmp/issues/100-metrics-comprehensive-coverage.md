# Missing and Broken Prometheus Metrics: 9 Missing Metrics, 1 Wrong Type

**Category**: metrics, observability
**Severity**: medium

**Labels**: `enhancement`, `metrics`, `reliability`

## Summary

Multiple critical Prometheus metrics are missing or broken. Core consensus signals -- finalized height, current view, and nullification count -- are tracked internally via `AtomicU64` in the `NodeState` struct and exposed through the `/status` JSON endpoint, but are never registered as Prometheus gauges. This means Prometheus-based alerting and Grafana dashboards cannot display or alert on the most fundamental health indicators. Additionally, one existing metric (`block_txs_included`) is registered as a Gauge (point-in-time snapshot) rather than a Histogram (distribution), which silently discards approximately 329 out of every 330 data points at 33 blocks/s with 10-second scrape intervals.

## Problem

### Missing metrics (8 total)

The `AppMetrics` struct at `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs` provides operational metrics, but several critical signals are absent:

1. **`kora_finalized_height`** (Gauge) -- The finalized height is set in the reporters:
   ```rust
   // /Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:257
   if let Some(ref ns) = node_state {
       ns.set_finalized_height(block.height);
   }
   ```
   But no Prometheus gauge is set alongside this call. Without it, there is no way to alert on stalled finalization.

2. **`kora_current_view`** (Gauge) -- The view is set in the activity reporter:
   ```rust
   // /Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:1613
   self.state.set_view(n.proposal.round.view().get());
   ```
   Not mirrored to Prometheus.

3. **`kora_nullifications_total`** (Counter) -- Nullifications are counted:
   ```rust
   // /Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:1620
   self.state.inc_nullified();
   ```
   Not mirrored to Prometheus. Cannot compute nullification rate in Grafana.

4. **`kora_peer_count`** (Gauge) -- Peer count is tracked in `NodeState` but not exposed as a Prometheus metric. Cannot detect network partitions via monitoring.

5. **`kora_block_gas_used`** (Gauge) -- Gas usage per block is computed during execution but not recorded. Cannot monitor gas utilization or do capacity planning.

6. **`kora_persist_duration_seconds`** (Histogram) -- QMDB persistence happens inside a spawned task:
   ```rust
   // /Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:600-606
   if persist_checkpoint {
       let persist_state = state.clone();
       let persist_handle = context
           .child("persist")
           .shared(true)
           .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
   }
   ```
   The duration of this operation is not measured. Cannot detect disk I/O degradation.

7. **`kora_rpc_duration_seconds`** (Family<MethodLabel, Histogram>) -- RPC call duration is not measured. The rate limit check in `RateLimitedRpcService::call()` (server.rs:304) counts requests but does not time them.

8. **`kora_rpc_rate_limited_total`** (Counter) -- Rate limiting occurs at two points but is not counted:
   ```rust
   // /Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs:315-318
   if !limiter.try_acquire(id) {
       return Box::pin(std::future::ready(rate_limited_rpc_response(
           request.id().into_owned(),
       )));
   }
   // /Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs:332-335
   if !global_rate_limit_allows(&self.global_limiter) {
       return Box::pin(std::future::ready(rate_limited_rpc_response(
           request.id().into_owned(),
       )));
   }
   ```

### Broken metric (1)

9. **`kora_block_txs_included`** -- Registered as a Gauge but should be a Histogram:
   ```rust
   // /Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs:54
   pub block_txs_included: Gauge,
   ```
   Used as:
   ```rust
   // /Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:417
   m.block_txs_included.set(block.txs.len() as i64);
   ```
   At 33 blocks/s with a 10-second Prometheus scrape interval, only the last value is captured. Approximately 329 out of every 330 data points are silently discarded. A Histogram would preserve the distribution across all blocks.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs`, lines 38-112 (AppMetrics struct, line 54: `block_txs_included: Gauge`)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs`, line 257 (`set_finalized_height`), line 1613 (`set_view`), line 1620 (`inc_nullified`), lines 600-606 (persist task)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs`, line 417 (`block_txs_included.set()`)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 315-318, 332-335 (rate limiting without counter)

## Impact

- **Cannot alert on stalled finalization**: If finalized height stops advancing, no Prometheus alert fires. This is the most fundamental consensus health signal.
- **Cannot detect consensus stuck conditions**: View stalls (no view advancement) indicate consensus failure but are invisible to monitoring.
- **Cannot compute nullification rate in Grafana**: Nullification rate is a key performance indicator. Without a counter, it cannot be plotted or alerted on.
- **Cannot correlate peer drops with issues**: Peer count drops often precede nullification spikes or partitions, but this correlation is invisible.
- **Cannot monitor gas utilization**: No visibility into how much of the block gas limit is being used.
- **Cannot detect QMDB I/O degradation**: Persistence duration spikes indicate disk problems but are not measured.
- **Cannot diagnose slow RPC calls**: Slow `eth_call` or `eth_estimateGas` calls are invisible without per-method latency histograms.
- **Cannot determine when clients are rate-limited**: Operators have no visibility into whether clients are being throttled.
- **Misleading transaction count metric**: The Gauge-based `block_txs_included` only shows the last block's count, not the distribution.

## Root Cause

The metrics system was built with a minimal set of operational metrics focused on txpool and block building. The `NodeState` struct provides the data for consensus health signals but they are only surfaced via the `/status` JSON endpoint, not as Prometheus metrics. The `block_txs_included` was likely registered as a Gauge by mistake (the pattern matches other "current value" metrics like `txpool_size`), but transaction count per block is a per-event signal that needs a Histogram.

## Suggested Fix

### Step 1: Add new fields to `AppMetrics`

```rust
pub finalized_height: Gauge,
pub current_view: Gauge,
pub nullifications_total: Counter,
pub peer_count: Gauge,
pub block_gas_used: Gauge,
pub persist_duration: Histogram,
pub rpc_duration: Family<MethodLabel, Histogram>,
pub rpc_rate_limited: Counter,
```

### Step 2: Register in `AppMetrics::register()`

Add registration calls alongside the existing metrics.

### Step 3: Instrument at call sites

- `finalized_height`: set at `reporters/src/lib.rs:257` alongside `ns.set_finalized_height()`
- `current_view`: set at `reporters/src/lib.rs:1613` alongside `self.state.set_view()`
- `nullifications_total`: increment at `reporters/src/lib.rs:1620` alongside `self.state.inc_nullified()`
- `peer_count`: set wherever `NodeState::set_peer_count()` is called
- `block_gas_used`: set at `app.rs:414-418` alongside other block metrics
- `persist_duration`: wrap the persist task at `reporters/src/lib.rs:602-608` with timing
- `rpc_duration`: add timing in `RateLimitedRpcService::call()`
- `rpc_rate_limited`: increment at `server.rs:316` and `server.rs:333`

### Step 4: Fix `block_txs_included` gauge to histogram

Change the type in `AppMetrics` from `Gauge` to `Histogram`, update `register()`, and update `app.rs:417` to use `.observe()` instead of `.set()`.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs` -- Add new metric fields to `AppMetrics` struct; register them; fix `block_txs_included` type
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` -- Instrument `finalized_height` (line 257), `current_view` (line 1613), `nullifications_total` (line 1620), `persist_duration` (lines 600-612)
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- Instrument `block_gas_used` (lines 414-418), fix `block_txs_included` to Histogram (line 417)
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` -- Instrument `rpc_duration` and `rpc_rate_limited` (lines 304-336)

## Related Issues

- `097-docker-observability-not-enabled.md` -- Observability stack not enabled (alerting depends on these metrics existing)
- `102-audit-tracking-49-issues.md` -- Audit tracking (#288 in the audit covers missing operational metrics)
