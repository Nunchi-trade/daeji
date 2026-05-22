# Add Application-Level Prometheus Metrics

## Summary

Kora's `/metrics` endpoint currently only exposes metrics from the Commonware SDK framework (consensus, P2P, resolver, QMDB storage, runtime). The Kora application code itself -- transaction pool, block building, block verification, finalization, and RPC server -- has **zero** Prometheus instrumentation. This means operators cannot observe any application-layer behavior through Prometheus/Grafana, leaving critical operational questions unanswerable:

- How many transactions are in the mempool right now?
- Why are blocks being built empty when the mempool has pending transactions?
- How long does block execution actually take?
- How many transactions are being rejected, and why?
- Is a node catching up or stuck?
- What is the RPC request rate and latency?

All of these require application-level metrics that do not exist today.

## Relationship to Issue 20 (Finalization Error Handling)

**Issues 19 and 20 should be implemented together.** Issue 20 replaces the untyped `Err(())` returns in `finalize_block()` with a structured `FinalizationError` enum containing variants like `ExecutionFailed`, `RootComputationFailed`, `StateRootMismatch`, `PersistFailed`, and `MissingParentSnapshot`. These typed errors provide natural label values for the metrics defined here:

- `kora_finalization_failure_total{cause="execution_failed"}` maps to `FinalizationError::ExecutionFailed`
- `kora_finalization_failure_total{cause="root_failed"}` maps to `FinalizationError::RootComputationFailed`
- `kora_finalization_failure_total{cause="state_root_mismatch"}` maps to `FinalizationError::StateRootMismatch`
- `kora_finalization_failure_total{cause="persist_failed"}` maps to `FinalizationError::PersistFailed`
- `kora_finalization_failure_total{cause="missing_parent"}` maps to `FinalizationError::MissingParentSnapshot`

Similarly, the `TxPoolError` enum in `crates/node/txpool/src/error.rs` already provides typed rejection reasons (`AlreadyExists`, `NonceTooLow`, `SenderFull`, `PoolFull`, `ReplacementUnderpriced`, `NonceAlreadyInPool`, `DecodeError`, `InvalidChainId`, `InvalidSignature`, `IntrinsicGasTooLow`, `GasPriceTooLow`, `TxTooLarge`, `InsufficientBalance`, `StateError`, `NonceGap`) that map directly to `kora_txpool_rejected_total{reason=...}` label values.

If implemented separately, the metrics issue would use string-literal labels; if implemented together, the labels can be derived programmatically from the error variant names, avoiding drift between error types and metric labels.

## Current State

### What is currently exposed

The `/metrics` endpoint is served at `crates/node/runner/src/runner.rs` (lines 626-658). The handler calls `metrics_context.encode()` on a clone of the Commonware runtime context:

```rust
// runner.rs lines 626-658
if let Some(metrics_addr) = self.metrics_addr {
    let metrics_context = context.clone();
    context.with_label("metrics").shared(true).spawn(move |_| async move {
        let app = axum::Router::new().route(
            "/metrics",
            axum::routing::get(move || {
                let body = metrics_context.encode();
                async move {
                    (
                        axum::http::StatusCode::OK,
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "application/openmetrics-text; version=1.0.0; charset=utf-8",
                        )],
                        body,
                    )
                }
            }),
        );
        // ... bind and serve ...
    });
}
```

This serializes only the metrics that Commonware SDK components register internally:

- **Consensus** (~14 metrics): `finalized_height`, `processed_height`, `engine_voter_state_*`, `engine_voter_finalization_latency_*`, etc.
- **Batcher** (~8 metrics): `engine_batcher_added_total`, `engine_batcher_batch_size_*`, `marshaled_build_duration_*`, etc.
- **Resolver** (~12 metrics): `engine_resolver_resolver_peers_blocked`, `engine_resolver_resolver_fetch_*`, etc.
- **P2P Network** (~13 metrics): `network_router_messages_dropped_total`, `network_spawner_messages_sent_total`, etc.
- **Runtime** (~12 metrics): `runtime_process_rss`, `runtime_tasks_running`, `runtime_*_bandwidth_total`, etc.
- **QMDB** (~18 metrics): `state_qmdb_{accounts,storage,code}_index_items`, etc.

Total: approximately 200 Kora/Commonware metrics, all originating from the Commonware framework.

### What is NOT exposed

Kora application code contributes zero Prometheus metrics. The following subsystems have no instrumentation:

**Transaction Pool** (`crates/node/txpool/src/pool.rs`):
- Pool size (pending + queued) never reported as Prometheus metrics
- Rejection counts per `TxPoolError` variant never counted
- Eviction counts never counted
- Cleanup (TTL expiry) counts never counted

**Block Building** (`crates/node/runner/src/app.rs`, `build_block()` at line 91):
- Build timing computed (`snapshot_elapsed`, `exec_elapsed`, `root_elapsed`, `total_elapsed`) but only written to `debug!` logs (line 180), never recorded as metrics
- Transaction count per block only logged (line 184), never recorded
- Gas used per block not tracked (the `ExecutionOutcome` has a `gas_used` field but it is not surfaced in `build_block()`)
- Empty-block-with-pending-txs condition detected (line 120) but only logged as `warn!`, never counted as a metric

**Block Verification** (`crates/node/runner/src/app.rs`, `verify_block()` at line 194):
- Verification timing computed (line 262) and logged (line 263-271) but never recorded as metrics
- State root mismatches (line 238-245) only logged, never counted
- Verification failures (lines 205, 219, 232, 245) never counted

**Finalization** (`crates/node/reporters/src/lib.rs`, `finalize_block()` at line 169):
- Returns `Result<(Option<ExecutionOutcome>, Option<BlockContext>), ()>` -- the `Err(())` return type discards all error information
- Finalization success/failure never counted as metrics
- Five distinct failure paths (lines 204, 216, 227, 255, 269, 274) all return `Err(())` with no metric increment
- Finalization latency never recorded

**RPC Server** (`crates/node/rpc/src/server.rs`):
- Request counts per JSON-RPC method never tracked
- Request latency never recorded
- Rate limit rejections (lines 176-186 for HTTP, lines 200-204 for JSON-RPC) never counted

### How the metrics infrastructure works

The Commonware runtime uses the `prometheus-client` crate (v0.24.0, declared as a workspace dependency in the root `Cargo.toml`). The runtime context implements the `commonware_runtime::Metrics` trait with:

```rust
// From commonware_runtime::Metrics (seen in crates/network/transport-sim/src/context.rs):
fn register<N: Into<String>, H: Into<String>>(&self, name: N, help: H, metric: impl Metric);
fn encode(&self) -> String;
```

- `register(name, help, metric)` -- registers a `prometheus_client::registry::Metric` with the runtime's internal registry
- `encode()` -- serializes all registered metrics to OpenMetrics text format

Any metric registered via `context.register(...)` is automatically included in the `/metrics` endpoint output. No additional server configuration is needed.

The `NodeState` struct (`crates/node/rpc/src/state.rs`) already tracks some counters using `AtomicU64` (finalized_count, proposed_count, nullified_count, peer_count) for the `/status` HTTP endpoint, but these are JSON fields served via a separate endpoint and are invisible to Prometheus.

## Proposed Solution

### 1. Create a metrics module

Create `crates/node/metrics/` as a new crate that defines all application-level metrics using `prometheus-client`. This crate acts as a shared registry that hot-path code imports to record observations.

The `prometheus-client` crate uses `prometheus_client::registry::Metric` as the trait bound for registrable types. Since the Commonware runtime's `Metrics::register()` method accepts `impl Metric`, and all `prometheus-client` metric types (`Counter`, `Gauge`, `Histogram`, `Family<L, M>`) implement this trait, registration is straightforward.

```rust
// crates/node/metrics/src/lib.rs

use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{
        counter::Counter,
        family::Family,
        gauge::Gauge,
        histogram::Histogram,
    },
};

/// Labels for transaction pool rejection reasons.
///
/// When Issue 20 is implemented, these labels should be derived from
/// `TxPoolError` variant names via a `From<&TxPoolError>` impl.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ReasonLabel {
    pub reason: String,
}

/// Labels for proposal/verification/finalization failure causes.
///
/// When Issue 20 is implemented, these labels should be derived from
/// typed error variant names (e.g., `FinalizationError::ExecutionFailed`
/// → `cause: "execution_failed"`).
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CauseLabel {
    pub cause: String,
}

/// Labels for RPC method names.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct MethodLabel {
    pub method: String,
}

/// Application-level metrics for Kora.
///
/// All metrics are prefixed with `kora_` to distinguish them from
/// Commonware framework metrics. Every field is `Clone + Send + Sync`
/// because `prometheus-client` metric types use internal atomics.
#[derive(Clone)]
pub struct KoraMetrics {
    // --- Transaction Pool ---

    /// Current number of pending (executable) transactions in the pool.
    pub txpool_pending: Gauge,

    /// Current number of queued (future nonce) transactions in the pool.
    pub txpool_queued: Gauge,

    /// Total transactions rejected by the pool, labeled by rejection reason.
    pub txpool_rejected_total: Family<ReasonLabel, Counter>,

    /// Total transactions evicted from the pool by capacity enforcement.
    pub txpool_evicted_total: Counter,

    /// Total transactions expired by TTL cleanup.
    pub txpool_expired_total: Counter,

    // --- Block Building ---

    /// Time to build a block (seconds).
    pub block_build_seconds: Histogram,

    /// Number of transactions included in each built block.
    pub block_txs_included: Histogram,

    /// Gas used per built block.
    pub block_gas_used: Histogram,

    /// Total empty blocks built despite pending transactions in mempool.
    pub block_empty_with_pending_total: Counter,

    // --- Block Verification ---

    /// Time to verify a block (seconds).
    pub block_verify_seconds: Histogram,

    /// Total state root mismatches during block verification.
    pub state_root_mismatch_total: Counter,

    /// Total block verification failures (any cause).
    pub verify_failure_total: Counter,

    // --- Proposal ---

    /// Total proposal failures, labeled by cause.
    pub proposal_failure_total: Family<CauseLabel, Counter>,

    // --- Finalization ---

    /// Total finalization failures, labeled by cause.
    /// When Issue 20 typed errors exist, use the error variant name as label.
    pub finalization_failure_total: Family<CauseLabel, Counter>,

    /// Total finalization successes.
    pub finalization_success_total: Counter,

    /// Time to finalize a block (seconds), including execution replay and persistence.
    pub finalization_seconds: Histogram,

    // --- Catch-Up ---

    /// Number of blocks this node is behind the network tip.
    pub catchup_blocks_behind: Gauge,

    // --- RPC ---

    /// Total RPC requests, labeled by method name.
    pub rpc_requests_total: Family<MethodLabel, Counter>,

    /// RPC request duration (seconds), labeled by method name.
    pub rpc_request_duration_seconds: Family<MethodLabel, Histogram>,

    /// Total RPC requests rejected by rate limiting.
    pub rpc_rate_limited_total: Counter,
}
```

#### Constructor details

The `KoraMetrics::new()` constructor must create each metric with appropriate histogram buckets. The `prometheus-client` `Histogram` type requires bucket boundaries at construction time.

```rust
impl KoraMetrics {
    pub fn new() -> Self {
        Self {
            txpool_pending: Gauge::default(),
            txpool_queued: Gauge::default(),
            txpool_rejected_total: Family::default(),
            txpool_evicted_total: Counter::default(),
            txpool_expired_total: Counter::default(),

            // Block build: sub-millisecond to ~1s range.
            // Devnet baseline is ~5-10ms per block at 89 blocks/s.
            block_build_seconds: Histogram::new(
                [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0].into_iter(),
            ),
            block_txs_included: Histogram::new(
                [0.0, 1.0, 5.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 5000.0].into_iter(),
            ),
            block_gas_used: Histogram::new(
                [0.0, 21_000.0, 100_000.0, 500_000.0, 1_000_000.0,
                 5_000_000.0, 10_000_000.0, 30_000_000.0, 45_000_000.0].into_iter(),
            ),
            block_empty_with_pending_total: Counter::default(),

            // Block verify: same range as build since it re-executes.
            block_verify_seconds: Histogram::new(
                [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0].into_iter(),
            ),
            state_root_mismatch_total: Counter::default(),
            verify_failure_total: Counter::default(),

            proposal_failure_total: Family::default(),

            finalization_failure_total: Family::default(),
            finalization_success_total: Counter::default(),
            // Finalization includes execution replay + QMDB persist, so
            // allow higher end of range (persist can take seconds under load).
            finalization_seconds: Histogram::new(
                [0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0].into_iter(),
            ),

            catchup_blocks_behind: Gauge::default(),

            rpc_requests_total: Family::default(),
            // RPC latency: sub-millisecond for cached reads, up to 5s for
            // expensive state queries or eth_call.
            rpc_request_duration_seconds: Family::default(),
            rpc_rate_limited_total: Counter::default(),
        }
    }
}
```

### 2. Register metrics at startup

During node initialization in `crates/node/runner/src/runner.rs`, in `ProductionRunner::run()`, metrics must be registered **after** the Commonware runtime context is available but **before** the metrics HTTP server is spawned. This ensures that by the time the `/metrics` endpoint is reachable, all application metrics are already registered.

The current startup ordering in `run()` is:

1. Context and transport available (line 493)
2. Archives and QMDB initialized (lines 517-542)
3. Ledger, txpool, and RPC server created (lines 548-623)
4. Metrics server spawned (lines 626-658)
5. Consensus engine started (lines 755-787)

Application metrics should be created and registered at step 3.5 -- after the ledger/txpool/RPC but before the metrics server spawn:

```rust
// In ProductionRunner::run(), after txpool is created (line 553) and before
// metrics server spawn (line 626):

let kora_metrics = KoraMetrics::new();
kora_metrics.register(&context); // calls context.register() for each metric
```

The `register` method on `KoraMetrics` calls `context.register(name, help, metric.clone())` for each field. Since `prometheus-client` metrics are internally reference-counted, cloning a metric creates a new handle to the same underlying atomic state. The `KoraMetrics` struct can then be passed (by clone) to every subsystem that needs to record observations.

```rust
impl KoraMetrics {
    /// Register all metrics with the Commonware runtime context.
    ///
    /// Uses `context.register(name, help, metric)` which delegates to the
    /// internal `prometheus_client::registry::Registry`. Each metric is
    /// cloned so the KoraMetrics struct retains its own handle.
    pub fn register(&self, context: &impl commonware_runtime::Metrics) {
        context.register("kora_txpool_pending", "Current pending transactions in pool", self.txpool_pending.clone());
        context.register("kora_txpool_queued", "Current queued transactions in pool", self.txpool_queued.clone());
        context.register("kora_txpool_rejected_total", "Transactions rejected by pool", self.txpool_rejected_total.clone());
        context.register("kora_txpool_evicted_total", "Transactions evicted by capacity enforcement", self.txpool_evicted_total.clone());
        context.register("kora_txpool_expired_total", "Transactions expired by TTL cleanup", self.txpool_expired_total.clone());

        context.register("kora_block_build_seconds", "Block build duration in seconds", self.block_build_seconds.clone());
        context.register("kora_block_txs_included", "Transactions included per block", self.block_txs_included.clone());
        context.register("kora_block_gas_used", "Gas used per block", self.block_gas_used.clone());
        context.register("kora_block_empty_with_pending_total", "Empty blocks despite pending transactions", self.block_empty_with_pending_total.clone());

        context.register("kora_block_verify_seconds", "Block verification duration in seconds", self.block_verify_seconds.clone());
        context.register("kora_state_root_mismatch_total", "State root mismatches during verification", self.state_root_mismatch_total.clone());
        context.register("kora_verify_failure_total", "Block verification failures", self.verify_failure_total.clone());

        context.register("kora_proposal_failure_total", "Block proposal failures", self.proposal_failure_total.clone());

        context.register("kora_finalization_failure_total", "Finalization failures", self.finalization_failure_total.clone());
        context.register("kora_finalization_success_total", "Finalization successes", self.finalization_success_total.clone());
        context.register("kora_finalization_seconds", "Finalization duration in seconds", self.finalization_seconds.clone());

        context.register("kora_catchup_blocks_behind", "Blocks behind network tip", self.catchup_blocks_behind.clone());

        context.register("kora_rpc_requests_total", "Total RPC requests", self.rpc_requests_total.clone());
        context.register("kora_rpc_request_duration_seconds", "RPC request duration in seconds", self.rpc_request_duration_seconds.clone());
        context.register("kora_rpc_rate_limited_total", "RPC requests rejected by rate limiter", self.rpc_rate_limited_total.clone());
    }
}
```

### 3. Instrument the hot paths

The following sections describe every instrumentation point with its exact file, method, and line reference (based on the current codebase as of 2026-05-22).

#### 3a. Transaction Pool (`crates/node/txpool/src/pool.rs`)

Add a `metrics: Option<KoraMetrics>` field to `TransactionPool` (alongside the existing `config` and `events` fields). Accept it via a new `new_with_metrics()` constructor or a builder method. The `Option` keeps the pool usable in tests without metrics.

**`TransactionPool::add()` (line 151)**:

After each rejection path in `add()`:

```rust
// Line 160-161 (AlreadyExists or by_id duplicate):
return Err(TxPoolError::AlreadyExists);
// ADD: metrics.txpool_rejected_total.get_or_create(&ReasonLabel { reason: "already_exists".into() }).inc();

// Line 168 (NonceTooLow):
return Err(TxPoolError::NonceTooLow { .. });
// ADD: metrics.txpool_rejected_total.get_or_create(&ReasonLabel { reason: "nonce_too_low".into() }).inc();

// Line 174 (SenderFull):
return Err(TxPoolError::SenderFull(..));
// ADD: metrics.txpool_rejected_total.get_or_create(&ReasonLabel { reason: "sender_full".into() }).inc();

// Line 185 (ReplacementUnderpriced):
return Err(TxPoolError::ReplacementUnderpriced);
// ADD: metrics.txpool_rejected_total.get_or_create(&ReasonLabel { reason: "replacement_underpriced".into() }).inc();

// Lines 277-283 and 290-295 (PoolFull from reject_underpriced_when_full):
return Err(TxPoolError::PoolFull);
// ADD: metrics.txpool_rejected_total.get_or_create(&ReasonLabel { reason: "pool_full".into() }).inc();

// Line 262 (PoolFull after eviction):
return Err(TxPoolError::PoolFull);
// ADD: metrics.txpool_rejected_total.get_or_create(&ReasonLabel { reason: "pool_full".into() }).inc();
```

After each eviction in the eviction loops (lines 198-226):
```rust
// Each eviction:
// ADD: metrics.txpool_evicted_total.inc();
```

After every successful mutation (add, evict, reject), update the gauges:
```rust
// After inner.update_counts() at line 195, and after eviction at lines 211, 226:
// ADD:
metrics.txpool_pending.set(inner.pending_count as i64);
metrics.txpool_queued.set(inner.queued_count as i64);
```

**`TransactionPool::cleanup()` (line 473)**:

```rust
// After line 498 (after counting removed):
// ADD:
if let Some(ref metrics) = self.metrics {
    metrics.txpool_expired_total.inc_by(removed as u64);
    let inner = self.inner.read();
    metrics.txpool_pending.set(inner.pending_count as i64);
    metrics.txpool_queued.set(inner.queued_count as i64);
}
```

**`TransactionPool::remove_confirmed()` (line 388)**:

```rust
// After inner.update_counts() at line 416:
// ADD:
metrics.txpool_pending.set(inner.pending_count as i64);
metrics.txpool_queued.set(inner.queued_count as i64);
```

**`Mempool::prune()` (line 628)**:

```rust
// After inner.update_counts() at line 673:
// ADD:
metrics.txpool_pending.set(inner.pending_count as i64);
metrics.txpool_queued.set(inner.queued_count as i64);
```

#### 3b. Block Building (`crates/node/runner/src/app.rs`)

Add a `metrics: Option<KoraMetrics>` field to `RevmApplication` (alongside the existing `node_state: Option<NodeState>` at line 39).

**`build_block()` (line 91)**:

```rust
// Line 98-106 (missing parent snapshot):
None => {
    // ADD:
    metrics.proposal_failure_total
        .get_or_create(&CauseLabel { cause: "missing_parent".into() })
        .inc();
    // ... existing warn! ...
    return None;
}

// Line 120-125 (empty block with pending txs):
if txs.is_empty() && mempool_len > excluded_len {
    // ADD:
    metrics.block_empty_with_pending_total.inc();
    // ... existing warn! ...
}

// Line 143-155 (execution failure):
Err(err) => {
    // ADD:
    metrics.proposal_failure_total
        .get_or_create(&CauseLabel { cause: "execution_failed".into() })
        .inc();
    // ... existing warn! ...
    return None;
}

// Line 160-172 (root computation failure):
Err(err) => {
    // ADD:
    metrics.proposal_failure_total
        .get_or_create(&CauseLabel { cause: "root_failed".into() })
        .inc();
    // ... existing warn! ...
    return None;
}

// Line 179-191 (successful build, before return):
// ADD:
metrics.block_build_seconds.observe(total_elapsed.as_secs_f64());
metrics.block_txs_included.observe(block.txs.len() as f64);
metrics.block_gas_used.observe(outcome.gas_used as f64);
```

Note: `outcome.gas_used` is available from the `ExecutionOutcome` variable bound at line 143. The `outcome` variable remains in scope until `build_block()` returns, so `outcome.gas_used` can be read directly at the success observation point (lines 179-191) without any additional binding.

#### 3c. Block Verification (`crates/node/runner/src/app.rs`)

**`verify_block()` (line 194)**:

```rust
// Line 205-206 (missing parent snapshot):
// ADD: metrics.verify_failure_total.inc();

// Line 217-220 (execution failure):
// ADD: metrics.verify_failure_total.inc();

// Line 231-233 (root computation failure):
// ADD: metrics.verify_failure_total.inc();

// Line 238-245 (state root mismatch):
// ADD:
metrics.state_root_mismatch_total.inc();
metrics.verify_failure_total.inc();

// Line 262 (on success, before return true):
// ADD:
metrics.block_verify_seconds.observe(total_elapsed.as_secs_f64());
```

#### 3d. Finalization (`crates/node/reporters/src/lib.rs`)

Add a `metrics: Option<KoraMetrics>` field to `FinalizedReporter` and pass it through to `handle_finalized_update()` and `finalize_block()`.

**`finalize_block()` (line 169)**:

```rust
// Line 204-207 (execution failure):
// ADD: metrics.finalization_failure_total
//     .get_or_create(&CauseLabel { cause: "execution_failed".into() }).inc();

// Line 215-218 (root computation failure):
// ADD: metrics.finalization_failure_total
//     .get_or_create(&CauseLabel { cause: "root_failed".into() }).inc();

// Line 220-228 (state root mismatch):
// ADD: metrics.finalization_failure_total
//     .get_or_create(&CauseLabel { cause: "state_root_mismatch".into() }).inc();

// Line 254-256 (missing parent for non-cached block):
// ADD: metrics.finalization_failure_total
//     .get_or_create(&CauseLabel { cause: "missing_parent".into() }).inc();

// Line 268-270 (persist task failure):
// ADD: metrics.finalization_failure_total
//     .get_or_create(&CauseLabel { cause: "persist_failed".into() }).inc();

// Line 273-276 (persist result error):
// ADD: metrics.finalization_failure_total
//     .get_or_create(&CauseLabel { cause: "persist_failed".into() }).inc();

// Line 278 (return Ok):
// ADD: metrics.finalization_success_total.inc();
```

Additionally, wrap the entire `finalize_block()` call in `handle_finalized_update()` with a timer:
```rust
let start = Instant::now();
let result = finalize_block(...).await;
metrics.finalization_seconds.observe(start.elapsed().as_secs_f64());
```

#### 3e. RPC Rate Limiting (`crates/node/rpc/src/server.rs`)

Pass `KoraMetrics` into the rate limiter middleware and JSON-RPC service wrapper.

**`enforce_http_rate_limit()` (line 176)**:

```rust
if !rate_limit_allows(&rate_limiter) {
    // ADD: metrics.rpc_rate_limited_total.inc();
    return (StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded").into_response();
}
```

**`RateLimitedRpcService::call()` (line 200)**:

```rust
if rate_limit_allows(&self.rate_limiter) {
    // ADD: metrics.rpc_requests_total
    //     .get_or_create(&MethodLabel { method: request.method_name().to_string() }).inc();
    // Wrap the inner service call with a timer for rpc_request_duration_seconds
    ResponseFuture::future(self.service.call(request))
} else {
    // ADD: metrics.rpc_rate_limited_total.inc();
    ResponseFuture::ready(rate_limited_rpc_response(request.id().into_owned()))
}
```

Note: Adding per-method duration tracking to the `RpcServiceT` wrapper requires timing the inner future, which may need a custom `ResponseFuture` wrapper. This is straightforward but adds complexity to the RPC middleware layer.

#### 3f. Catch-Up Gauge (`crates/node/runner/src/runner.rs`)

Spawn a periodic task alongside the existing `spawn_txpool_cleanup` (line 339). The `NodeState` struct already tracks `current_view` and `finalized_count`, which together indicate how far behind the node is.

```rust
fn spawn_catchup_gauge(
    node_state: NodeState,
    metrics: KoraMetrics,
    context: cw_tokio::Context,
) {
    context.with_label("catchup_gauge").shared(true).spawn(move |ctx| async move {
        loop {
            ctx.sleep(Duration::from_secs(5)).await;
            let status = node_state.status();
            // current_view tracks the consensus view number; finalized_count
            // tracks how many blocks have been finalized. The gap indicates
            // how many views (potential blocks) the node has not yet finalized.
            let behind = status.current_view.saturating_sub(status.finalized_count);
            metrics.catchup_blocks_behind.set(behind as i64);
        }
    });
}
```

### 4. Metric definitions (complete registry)

| Metric Name | Type | Labels | Histogram Buckets | Help Text |
|---|---|---|---|---|
| `kora_txpool_pending` | Gauge | -- | -- | Current pending (executable) transactions in pool |
| `kora_txpool_queued` | Gauge | -- | -- | Current queued (future nonce) transactions in pool |
| `kora_txpool_rejected_total` | Counter | `reason` | -- | Transactions rejected by the pool |
| `kora_txpool_evicted_total` | Counter | -- | -- | Transactions evicted from pool by capacity enforcement |
| `kora_txpool_expired_total` | Counter | -- | -- | Transactions expired by TTL cleanup |
| `kora_block_build_seconds` | Histogram | -- | 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0 | Block build duration in seconds |
| `kora_block_txs_included` | Histogram | -- | 0, 1, 5, 10, 50, 100, 500, 1000, 5000 | Transactions included per block |
| `kora_block_gas_used` | Histogram | -- | 0, 21000, 100000, 500000, 1M, 5M, 10M, 30M, 45M | Gas used per block |
| `kora_block_empty_with_pending_total` | Counter | -- | -- | Empty blocks built despite pending transactions in mempool |
| `kora_block_verify_seconds` | Histogram | -- | 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0 | Block verification duration in seconds |
| `kora_state_root_mismatch_total` | Counter | -- | -- | State root mismatches during verification |
| `kora_verify_failure_total` | Counter | -- | -- | Block verification failures (any cause) |
| `kora_proposal_failure_total` | Counter | `cause` | -- | Block proposal failures |
| `kora_finalization_failure_total` | Counter | `cause` | -- | Finalization failures |
| `kora_finalization_success_total` | Counter | -- | -- | Finalization successes |
| `kora_finalization_seconds` | Histogram | -- | 0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0 | Finalization duration in seconds |
| `kora_catchup_blocks_behind` | Gauge | -- | -- | Blocks behind the network tip (view - finalized_count) |
| `kora_rpc_requests_total` | Counter | `method` | -- | Total RPC requests |
| `kora_rpc_request_duration_seconds` | Histogram | `method` | 0.0001, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0 | RPC request duration in seconds |
| `kora_rpc_rate_limited_total` | Counter | -- | -- | RPC requests rejected by rate limiter |

#### Histogram bucket rationale

**Block execution time** (`block_build_seconds`, `block_verify_seconds`):
- Devnet baseline shows ~5-10ms per block at 89 blocks/s with empty blocks
- Under load with 100+ txs per block, execution can reach 50-250ms
- Buckets cover sub-millisecond (fast empty blocks) through 5s (extreme load or catchup replay)
- The 0.01, 0.025, 0.05 range provides high resolution in the expected operating range

**Finalization time** (`finalization_seconds`):
- Includes execution replay + QMDB root computation + disk persistence
- QMDB persistence can take 100ms-2s depending on changeset size and disk I/O
- Extended to 10s to capture persistence stalls under heavy I/O
- The 0.25, 0.5, 1.0, 2.5 range captures the persist-bound tail latency

**RPC latency** (`rpc_request_duration_seconds`):
- `eth_blockNumber` and `eth_chainId` return in <0.1ms (cached state)
- `eth_getBalance` and `eth_getCode` require QMDB reads: 1-10ms
- `eth_call` and `eth_estimateGas` require EVM execution: 10-500ms
- `eth_getLogs` over large ranges can take 1-5s
- Starts at 0.0001 (100us) to capture fast cached responses

**Block gas** (`block_gas_used`):
- 21000 = single transfer (minimum useful gas)
- 45M = current Kora gas limit (from `config.execution.gas_limit`)
- Intermediate buckets at common contract interaction sizes (100K-10M)

**Label values for `reason`:** `already_exists`, `nonce_too_low`, `pool_full`, `sender_full`, `replacement_underpriced`, `nonce_already_in_pool`, `decode_error`, `invalid_chain_id`, `invalid_signature`, `intrinsic_gas_too_low`, `gas_price_too_low`, `tx_too_large`, `insufficient_balance`, `nonce_gap`, `state_error`

These map directly to the `TxPoolError` variants in `crates/node/txpool/src/error.rs`. The pool's `add()` method only returns a subset (AlreadyExists, NonceTooLow, SenderFull, PoolFull, ReplacementUnderpriced); the remaining variants are returned by `TransactionValidator` before the pool is reached. Instrumentation should cover both paths.

**Label values for `cause` (proposal):** `missing_parent`, `execution_failed`, `root_failed`

**Label values for `cause` (finalization):** `execution_failed`, `root_failed`, `state_root_mismatch`, `missing_parent`, `persist_failed`

**Label values for `method`:** Dynamic, populated from the JSON-RPC method name string (e.g., `eth_sendRawTransaction`, `eth_blockNumber`, `eth_getBalance`)

### 5. Crate dependency changes

**New crate: `crates/node/metrics/Cargo.toml`**
```toml
[package]
name = "kora-metrics"
version = "0.1.0"
edition = "2021"

[dependencies]
prometheus-client.workspace = true
```

**Also add to workspace `Cargo.toml`** (root-level workspace dependency table, lines ~49-74):
```toml
kora-metrics = { path = "crates/node/metrics" }
```

The workspace glob `"crates/node/*"` (line 2) automatically includes the new crate as a workspace member; the explicit workspace dep entry is needed so other crates can reference it with `kora-metrics.workspace = true`.

**Modified crates (add `kora-metrics` dependency):**
- `crates/node/txpool/Cargo.toml` -- for pool instrumentation
- `crates/node/runner/Cargo.toml` -- for registration and block building/verification
- `crates/node/reporters/Cargo.toml` -- for finalization instrumentation
- `crates/node/rpc/Cargo.toml` -- for RPC rate limiting and request tracking

### 6. Grafana dashboard additions

With these metrics, new Grafana dashboard panels become possible:

- **Mempool Pressure**: `kora_txpool_pending` and `kora_txpool_queued` over time
- **Rejection Breakdown**: `rate(kora_txpool_rejected_total[5m])` grouped by `reason`
- **Block Throughput**: `rate(kora_block_txs_included_sum[1m]) / rate(kora_block_txs_included_count[1m])` (avg txs per block)
- **Build Latency P95**: `histogram_quantile(0.95, rate(kora_block_build_seconds_bucket[5m]))`
- **Finalization Latency P95**: `histogram_quantile(0.95, rate(kora_finalization_seconds_bucket[5m]))`
- **Empty Block Diagnostic**: `rate(kora_block_empty_with_pending_total[5m])` vs `kora_txpool_pending`
- **Proposal Failure Drill-Down**: `rate(kora_proposal_failure_total[5m])` grouped by `cause`
- **Finalization Failure Drill-Down**: `rate(kora_finalization_failure_total[5m])` grouped by `cause`
- **Catch-Up Monitor**: `kora_catchup_blocks_behind` per node
- **RPC Load**: `rate(kora_rpc_requests_total[1m])` grouped by `method`
- **RPC Latency P99**: `histogram_quantile(0.99, rate(kora_rpc_request_duration_seconds_bucket[5m]))` grouped by `method`

New alert rules:

```yaml
- alert: HighTxPoolRejections
  expr: sum(rate(kora_txpool_rejected_total[5m])) > 100
  for: 5m
  annotations:
    summary: "High transaction pool rejection rate"

- alert: PersistentEmptyBlocks
  expr: rate(kora_block_empty_with_pending_total[5m]) > 0 and kora_txpool_pending > 10
  for: 5m
  annotations:
    summary: "Empty blocks being produced despite pending transactions"

- alert: NodeFallingBehind
  expr: kora_catchup_blocks_behind > 1000
  for: 2m
  annotations:
    summary: "Node is more than 1000 blocks behind the network tip"

- alert: HighFinalizationFailureRate
  expr: >
    sum(rate(kora_finalization_failure_total[5m]))
    / clamp_min(rate(kora_finalization_success_total[5m]) + sum(rate(kora_finalization_failure_total[5m])), 1)
    > 0.1
  for: 5m
  annotations:
    summary: "More than 10% of finalizations are failing"

- alert: SlowBlockBuild
  expr: histogram_quantile(0.95, rate(kora_block_build_seconds_bucket[5m])) > 0.5
  for: 5m
  annotations:
    summary: "P95 block build time exceeds 500ms"

- alert: RpcLatencyHigh
  expr: histogram_quantile(0.99, rate(kora_rpc_request_duration_seconds_bucket[5m])) > 2.0
  for: 5m
  annotations:
    summary: "P99 RPC latency exceeds 2 seconds"
```

## Design Considerations

### Why `prometheus-client` instead of the `prometheus` crate?

The Commonware runtime already uses `prometheus-client` (v0.24.0, declared as a workspace dependency in the root `Cargo.toml` at line 130), and its `Metrics::register()` method accepts `impl prometheus_client::registry::Metric`. Using a different metrics crate (e.g., `prometheus` or the `metrics` facade) would create a parallel metrics system not serialized by `context.encode()`, requiring a separate `/metrics` endpoint or manual merging. Sticking with `prometheus-client` ensures all metrics -- framework and application -- are served from the same endpoint.

### Thread safety

All `prometheus-client` metric types (`Counter`, `Gauge`, `Histogram`, `Family<L, M>`) are `Send + Sync` and use internal atomics. The `KoraMetrics` struct can be safely cloned and shared across threads without additional synchronization. This is confirmed by the `prometheus-client` crate documentation and by the fact that Commonware framework code shares metrics across spawned tasks.

### Performance impact

- `Counter::inc()` is a single `AtomicU64::fetch_add(1, Relaxed)` -- negligible overhead.
- `Gauge::set()` is a single `AtomicI64::store(val, Relaxed)` -- negligible overhead.
- `Histogram::observe()` iterates bucket boundaries and increments one atomic counter per bucket plus the sum and count -- slightly more work but well within acceptable bounds for per-block instrumentation (called ~100 times/second at peak throughput).
- The txpool gauges are updated on every `add()`/`remove()`/`prune()` call. Under high load (thousands of txs/sec), this is still just atomic stores and adds no contention beyond what the existing `RwLock<PoolInner>` already imposes.
- `Family::get_or_create()` involves a hash-map lookup (with interior lock). For hot paths like txpool rejection, the label cardinality is small (15 variants) so the overhead is bounded.

### Phased rollout

This can be implemented incrementally:

**Phase 1 (highest value, lowest risk):** txpool gauges + block build histogram + empty-block-with-pending counter + proposal failure counter. These are the metrics most needed for diagnosing the existing empty-block and mempool-not-draining issues observed during devnet testing on 2026-05-22.

**Phase 2:** Verification metrics, finalization counters (with typed labels from Issue 20), finalization latency histogram, catch-up gauge. These help with node recovery monitoring.

**Phase 3:** RPC request metrics. These require changes to the RPC middleware layer and are lower priority since the RPC server is not yet a bottleneck.

## Estimated Effort

- New `kora-metrics` crate with metric definitions and constructor: 2-3 hours
- Registration in runner.rs startup (including startup ordering): 1-2 hours
- Txpool instrumentation (pool.rs + validator.rs): 2-3 hours
- Block building / verification instrumentation (app.rs): 2-3 hours
- Finalization instrumentation (reporters/lib.rs): 1-2 hours
- Catch-up gauge task (runner.rs): 1 hour
- RPC metrics (Phase 3, server.rs middleware): 3-4 hours
- Grafana dashboards and alerts: 2-3 hours

**Total: 2-3 days**

If implemented together with Issue 20, the finalization metrics work overlaps significantly, saving ~1 day combined.

## Related Issues

- **Issue 20 (Finalization Error Handling):** Should be implemented together. Typed `FinalizationError` variants provide natural label values for `kora_finalization_failure_total`. Without Issue 20, finalization metrics must use string literals that may drift from the actual error handling code.
- Empty blocks with pending transactions: the `block_empty_with_pending_total` metric will make this diagnosable
- Node restart recovery monitoring: the `catchup_blocks_behind` metric provides visibility
- Alert calibration: application metrics enable meaningful alerts instead of relying solely on framework metrics
- Issue 08 (Observability Stack Fixes): fixes to the existing Prometheus/Grafana infrastructure should be applied first so that new application metrics are correctly scraped and visualized
