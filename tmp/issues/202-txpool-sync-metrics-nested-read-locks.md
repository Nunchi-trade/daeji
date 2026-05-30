# sync_metrics() Acquires Nested Read Locks on Every Pool Mutation

**Category**: performance -- txpool
**Severity**: high

## Summary

The `sync_metrics()` method acquires two `parking_lot::RwLock` read locks in sequence -- first `self.metrics` then `self.inner` -- on every pool mutation (`add()`, `remove()`, `cleanup()`, `remove_confirmed()`, `clear()`). The `metrics` field is an `Arc<RwLock<Option<AppMetrics>>>` that is written once via `set_metrics()` and then only ever read. This nested double-lock pattern adds unnecessary contention on the hot path. At Kora's throughput of 34 blocks/second with potentially hundreds of transaction insertions per block, the overhead compounds.

## Problem

`sync_metrics()` is called at the end of every pool mutation. Each call acquires two separate read locks:

1. `self.metrics.read()` -- to check if metrics are configured
2. `self.inner.read()` -- to read pool state (sizes, counts)

The `metrics` lock protects a value that is only set once (via `set_metrics()`) and then never changes. Using a `RwLock` for a write-once value is overkill. Additionally, the caller has often just released a write lock on `self.inner` (e.g., in `add()`), and `sync_metrics()` immediately re-acquires a read lock on `self.inner`, creating unnecessary lock acquisition overhead.

## Code Reference

File: `crates/node/txpool/src/pool.rs`, lines 174-182

```rust
/// Update gauge metrics to reflect current pool state.
///
/// Must be called while the caller does NOT hold the inner lock (it takes
/// a read lock internally).
fn sync_metrics(&self) {
    let metrics_guard = self.metrics.read();    // Lock 1: metrics
    if let Some(ref m) = *metrics_guard {
        let inner = self.inner.read();          // Lock 2: inner (nested)
        m.txpool_size.set(inner.by_hash.len() as i64);
        m.txpool_pending.set(inner.pending_count as i64);
        m.txpool_queued.set(inner.queued_count as i64);
    }
}
```

The `metrics` field declaration (line 136):

```rust
pub struct TransactionPool {
    inner: Arc<RwLock<PoolInner>>,
    config: PoolConfig,
    events: Option<broadcast::Sender<MempoolEvent>>,
    metrics: Arc<RwLock<Option<AppMetrics>>>,  // Write-once, then read-only
}
```

Call sites -- `sync_metrics()` is called from:
- `add()` (line 303) -- after every successful insertion
- `remove_with_reason()` (line 430) -- after every removal
- `remove_confirmed()` (line 470) -- after confirming transactions
- `cleanup()` (line 555) -- after TTL cleanup
- `clear()` (line 574) -- after clearing the pool

## Impact

1. **Lock contention**: Every pool mutation acquires two read locks sequentially. Under high transaction throughput, this creates contention between the `sync_metrics()` readers and concurrent writers to `self.inner`.
2. **Re-acquisition overhead**: In `add()`, the write lock on `inner` is dropped at line 287, then `sync_metrics()` at line 303 immediately re-acquires a read lock on `inner`. The lock drop-and-reacquire cycle is wasted work.
3. **Unnecessary RwLock on metrics**: The metrics value is written exactly once via `set_metrics()`. Using a `RwLock` for a write-once value adds overhead for every read (atomic refcount operations, cache line bouncing).

## Root Cause

The `AppMetrics` is stored behind a separate `RwLock` to allow lazy initialization via `set_metrics()`. Once set, it is only ever read. The nested lock pattern emerges because metrics updates require reading both the metrics reference and the inner pool state.

## Suggested Fix

**Option A (Recommended)**: Replace `Arc<RwLock<Option<AppMetrics>>>` with `Arc<OnceLock<AppMetrics>>` since the metrics handle is written once and never changes:

```rust
use std::sync::OnceLock;

pub struct TransactionPool {
    inner: Arc<RwLock<PoolInner>>,
    config: PoolConfig,
    events: Option<broadcast::Sender<MempoolEvent>>,
    metrics: Arc<OnceLock<AppMetrics>>,  // No lock needed for reads
}

fn sync_metrics(&self) {
    if let Some(m) = self.metrics.get() {  // No lock acquisition
        let inner = self.inner.read();
        m.txpool_size.set(inner.by_hash.len() as i64);
        m.txpool_pending.set(inner.pending_count as i64);
        m.txpool_queued.set(inner.queued_count as i64);
    }
}
```

**Option B**: Maintain atomic counters that are updated inline during mutations (avoiding the re-read of `inner`):

```rust
// In add(), after updating inner:
self.pending_count.store(inner.pending_count, Ordering::Relaxed);
self.queued_count.store(inner.queued_count, Ordering::Relaxed);
self.total_count.store(inner.by_hash.len(), Ordering::Relaxed);

// In sync_metrics():
fn sync_metrics(&self) {
    if let Some(m) = self.metrics.get() {
        m.txpool_size.set(self.total_count.load(Ordering::Relaxed) as i64);
        // ...
    }
}
```

## Files to Modify

- `crates/node/txpool/src/pool.rs` -- Replace `Arc<RwLock<Option<AppMetrics>>>` with `Arc<OnceLock<AppMetrics>>` and update `sync_metrics()`, `set_metrics()`, and `record_rejection()`

## Related Issues

- None

## Labels

performance, txpool
