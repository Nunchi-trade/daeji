# Missing Block Gas Used, QMDB Persist Duration, and RPC Method Latency Metrics

**Category**: enhancement -- metrics
**Severity**: medium

**Labels**: `enhancement`, `metrics`, `performance`, `executor`, `storage`, `rpc`

---

## Summary

Three important operational metrics are not tracked in Kora: block gas used, QMDB persistence duration, and per-method RPC latency. Without these, operators cannot monitor gas utilization, detect disk I/O degradation, or identify slow RPC methods -- all critical for production observability.

---

## Problem

### 1. Block Gas Used Not Tracked

After every block build and verify, an `ExecutionOutcome` is available that contains the `gas_used` value. This value is never recorded as a metric. A `kora_block_gas_used` gauge or histogram would enable monitoring gas utilization vs. gas limit, detecting block stuffing attacks, and capacity planning.

The `gas_used` value is available in `build_block` (where `outcome.gas_used` is used to call `record_block_fees`) and in `verify_block` (where `execution.outcome.gas_used` is similarly used), but it is not exposed via Prometheus.

### 2. QMDB Persist Duration Not Measured

The `persist_snapshot` call is the most expensive I/O operation in the finalization path. Its duration is never measured. A `kora_persist_duration_seconds` histogram would detect disk I/O degradation, QMDB write amplification, and contention between persistence and block production.

### 3. RPC Method-Level Latency Not Tracked

Only a single `kora_rpc_requests` counter is tracked for all RPC methods combined. There is no per-method latency histogram. Standard Ethereum node practice (Reth, Geth) is to track latency per method (e.g., `rpc_request_duration_seconds{method="eth_getBalance"}`).

---

## Code Reference

**Metrics registration** -- all metrics are gauges/counters, no gas or persist histograms exist:

`/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs:39-112`
```rust
pub struct AppMetrics {
    // ... (no block_gas_used, no persist_duration_seconds, no rpc_duration_seconds)
    pub rpc_requests_total: Counter,  // line 85: only a counter, no per-method latency
    // ...
}
```

**Gas used is available but not recorded** -- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:410-417`:
```rust
        // Cache gas usage so that the next block can derive its base fee.
        self.record_block_fees(block_digest, outcome.gas_used, base_fee);

        let total_elapsed = start.elapsed();

        if let Some(ref m) = self.metrics {
            m.block_build_time.observe(total_elapsed.as_secs_f64());
            m.evm_execution_seconds.observe(exec_elapsed.as_secs_f64());
            m.block_txs_included.set(block.txs.len() as i64);
            // NOTE: outcome.gas_used is NOT recorded as a metric
        }
```

**Persist duration not timed** -- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:600-612`:
```rust
    if persist_checkpoint {
        let persist_state = state.clone();
        let persist_handle = context
            .child("persist")
            .shared(true)
            .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
        let persist_result = persist_handle
            .await  // Duration not measured!
            .map_err(|err| FinalizationError::PersistTaskFailed(format!("{err}")))?;
        if let Err(err) = persist_result {
            return Err(FinalizationError::PersistFailed(err));
        }
    }
```

**RPC counter with no per-method latency** -- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs:305-307`:
```rust
        if let Some(ref counter) = self.rpc_requests_total {
            counter.inc();
        }
        // No per-method timing, no latency histogram
```

---

## Impact

- **Without gas used**: Operators cannot monitor whether the chain is approaching its gas limit, cannot detect gas-based DoS attacks, and cannot plan capacity. On a 10-node devnet producing ~34 blocks/s, this data is essential for load testing and capacity forecasting.
- **Without persist duration**: Disk I/O degradation is invisible until it causes finalization failures. On the devnet where 10 nodes share one NVMe drive, QMDB write contention is a likely bottleneck. Operators have no visibility into whether persistence latency is increasing.
- **Without RPC latency**: Slow `eth_call` or `eth_estimateGas` calls are invisible. Operators cannot identify which methods contribute to load or set SLO-based alerts. All 20+ Ethereum JSON-RPC methods are aggregated into a single counter.

---

## Root Cause

The metrics instrumentation was added incrementally and focused on consensus/execution timing. These three metrics were not included in the initial instrumentation pass.

---

## Suggested Fix

Add the following metrics to `AppMetrics` in `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs`:

```rust
pub block_gas_used: Gauge,                                // kora_block_gas_used
pub persist_duration_seconds: Histogram,                   // kora_persist_duration_seconds
pub rpc_duration_seconds: Family<MethodLabel, Histogram>,  // kora_rpc_duration_seconds{method}
```

Instrument them at the appropriate call sites:

1. In `build_block` and `verify_block` (app.rs), after execution:
   ```rust
   m.block_gas_used.set(outcome.gas_used as i64);
   ```

2. In `finalize_block` (reporters/src/lib.rs), around `persist_snapshot`:
   ```rust
   let persist_start = Instant::now();
   let persist_result = persist_handle.await ...;
   if let Some(ref m) = metrics {
       m.persist_duration_seconds.observe(persist_start.elapsed().as_secs_f64());
   }
   ```

3. In `RateLimitedRpcService::call` (server.rs), wrap method dispatch with timing:
   ```rust
   let method_name = request.method_name().to_string();
   let start = Instant::now();
   let response = self.service.call(request).await;
   if let Some(ref family) = self.rpc_duration_seconds {
       family.get_or_create(&MethodLabel { method: method_name })
           .observe(start.elapsed().as_secs_f64());
   }
   ```

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs` -- add metric definitions and registration
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- record `block_gas_used` after execution
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` -- time `persist_snapshot` and record duration
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` -- add per-method latency recording in `RateLimitedRpcService`

---

## Related Issues

- `059-metrics-missing-finalized-height-view-nullification.md` -- other missing metrics
- `060-metrics-missing-peer-count-gauge.md` -- another missing metric
- `062-metrics-block-txs-included-wrong-type.md` -- related metrics type issue
- `069-perf-pipeline-finalization-execution-persistence.md` -- persist_snapshot performance is invisible without this metric
