# block_txs_included Metric Should Be a Histogram, Not a Gauge

**Category**: bug -- metrics
**Severity**: medium

**Labels**: `bug`, `metrics`, `correctness`

---

## Summary

The `kora_block_txs_included` metric is registered as a `Gauge` that records only the transaction count of the most recently built block. Because Prometheus scrapes at ~10-second intervals while blocks are produced at ~34/s, approximately 340 blocks are produced between scrapes and only the final one is visible. This makes the metric nearly useless for statistical analysis. It should be a `Histogram` to capture the full distribution of transactions per block.

---

## Problem

Kora tracks the number of transactions included in each block via a `Gauge` metric. A `Gauge` only stores the last value set -- it has no memory of intermediate values between Prometheus scrapes. With 34 blocks/s and 10-second scrape intervals, 339 out of every 340 block sizes are lost. The metric cannot be used to compute percentiles, averages, or distributions of block sizes over time.

---

## Code Reference

**Metric declaration** -- `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs:53-54`:
```rust
    /// Number of transactions included in the most recently built block.
    pub block_txs_included: Gauge,
```

**Metric initialization** -- `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs:139`:
```rust
            block_txs_included: Gauge::default(),
```

**Metric registration** -- `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs:190-194`:
```rust
        registry.register(
            "kora_block_txs_included",
            "Transactions in the most recently built block",
            self.block_txs_included.clone(),
        );
```

**Usage in build_block** -- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:417`:
```rust
            m.block_txs_included.set(block.txs.len() as i64);
```

---

## Impact

Operators cannot answer critical questions like:

- "What percentage of blocks have zero transactions?" -- important for diagnosing empty-block problems and verifying that the transaction submission pipeline is working.
- "What is the average number of transactions per block?" -- needed for capacity planning and throughput analysis.
- "What is the 99th percentile block size?" -- needed for burst analysis and gas limit tuning.
- "What is the median block fullness over the last hour?" -- needed for load testing analysis.

These questions require a histogram or summary, not a gauge. The current gauge is essentially useless for anything beyond spot-checking the single most recent block at the exact moment Prometheus scrapes.

---

## Root Cause

The metric was implemented as a gauge for simplicity during initial development. Histograms require bucket configuration, which may have been deferred. The gauge was not revisited when the block production rate increased to 34 blocks/s, making the data loss far more severe.

---

## Suggested Fix

Change the type from `Gauge` to `Histogram` with appropriate buckets.

**Before** (in `crates/node/metrics/src/lib.rs`):
```rust
pub block_txs_included: Gauge,
// ...
block_txs_included: Gauge::default(),
```

**After**:
```rust
pub block_txs_included: Histogram,

const BLOCK_TXS_BUCKETS: [f64; 10] = [0.0, 1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0];
// ...
block_txs_included: Histogram::new(BLOCK_TXS_BUCKETS),
```

**Before** (in `crates/node/runner/src/app.rs`):
```rust
m.block_txs_included.set(block.txs.len() as i64);
```

**After**:
```rust
m.block_txs_included.observe(block.txs.len() as f64);
```

This allows Prometheus queries like:
- `histogram_quantile(0.5, rate(kora_block_txs_included_bucket[5m]))` -- median block size over 5 minutes
- `rate(kora_block_txs_included_sum[5m]) / rate(kora_block_txs_included_count[5m])` -- average txs per block

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/metrics/src/lib.rs` -- change field type from `Gauge` to `Histogram`, add bucket configuration, update registration
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- change `.set()` to `.observe()`

---

## Related Issues

- `061-metrics-missing-gas-persist-rpc-latency.md` -- other missing/incorrect metrics
- `059-metrics-missing-finalized-height-view-nullification.md` -- related metrics gaps
