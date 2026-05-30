# Metrics and Observability Analysis

## Summary

Kora has Prometheus infrastructure fully set up (client library, `/metrics` endpoint, OpenMetrics encoding) but **zero application-level metrics are actually registered**. The `/metrics` endpoint returns empty data. All "metrics" currently exposed are simple atomic counters accessible only via the RPC `/status` endpoint. There are 9+ categories of critical metrics that should be added.

---

## 1. Current Metrics Infrastructure

### Prometheus Client Setup

**Dependency:** `prometheus-client = "0.24.0"` (workspace `Cargo.toml:130`)

**Metrics Server:** `crates/node/runner/src/runner.rs` (Lines 444-477)
- Endpoint: `GET /metrics` at configured `metrics_addr`
- Encoding: OpenMetrics text format (version 1.0.0)
- Uses Commonware Runtime's `context.encode()` method

```rust
// runner.rs:450 - Metrics endpoint handler
let metrics_route = axum::Router::new().route("/metrics", get(move || async move {
    let body = context.encode();  // Returns OpenMetrics text
    ([(header::CONTENT_TYPE, "application/openmetrics-text; version=1.0.0; charset=utf-8")], body)
}));
```

### Commonware Runtime Metrics API

The `Metrics` trait provides:
- `fn register<N, H>(&self, name: N, help: H, metric: impl Metric)` - Register a metric
- `fn with_label(&self, label: &str) -> Self` - Add scope label
- `fn with_attribute(&self, key: &str, value: impl Display) -> Self` - Add attributes
- `fn encode(&self) -> String` - Serialize all metrics to OpenMetrics

**Problem:** `context.register()` is never called anywhere in the Kora codebase. The encode method returns whatever Commonware's runtime registers internally (process-level metrics only).

---

## 2. Metrics That DO Exist (RPC-Only, NOT Prometheus)

**File:** `crates/node/rpc/src/state.rs`

These are exposed via `GET /status` (HTTP JSON), NOT via `/metrics` (Prometheus):

| Metric | Type | Updated At | What It Measures |
|--------|------|------------|------------------|
| `current_view` | AtomicU64 | reporters/lib.rs:462 | Current consensus view number |
| `finalized_count` | AtomicU64 | reporters/lib.rs:466 | Total finalized blocks |
| `proposed_count` | AtomicU64 | runner/app.rs:305 | Total blocks proposed |
| `nullified_count` | AtomicU64 | reporters/lib.rs:469 | Total nullified rounds |
| `peer_count` | AtomicU64 | rpc/server.rs:255,403 | Connected peer count |
| `is_leader` | RwLock<bool> | rpc/state.rs:56 | Current leader status |
| `uptime_secs` | Calculated | rpc/state.rs:85 | Seconds since start |

**Exposed as:** `NodeStatus` JSON struct via `GET /status` and `kora_nodeStatus` RPC method.

---

## 3. Commonware Runtime Metrics (Auto-Registered)

These come from Commonware's runtime layer and ARE exposed via `/metrics`:

| Metric | Type | Source |
|--------|------|--------|
| `runtime_process_rss` | Gauge | Process RSS memory |
| `runtime_tasks_running` | Gauge | Active async tasks |
| `runtime_tasks_spawned_total` | Counter | Total tasks spawned |
| `runtime_outbound_bandwidth_total` | Counter | Bytes sent |
| `runtime_inbound_bandwidth_total` | Counter | Bytes received |
| `runtime_storage_write_bytes_total` | Counter | Storage writes |
| `runtime_storage_read_bytes_total` | Counter | Storage reads |
| `finalized_height` | Gauge | Finalized block height |
| `engine_voter_state_current_view` | Gauge | Current consensus view |
| `engine_voter_finalization_latency_*` | Histogram | Finalization latency |
| `engine_voter_notarization_latency_*` | Histogram | Notarization latency |
| `engine_voter_state_nullifications_total` | Counter | Nullification events |
| `engine_voter_state_timeouts_total` | Counter | Timeout events (by reason) |
| `engine_voter_outbound_messages_total` | Counter | Outbound consensus messages |
| `engine_batcher_verify_latency_*` | Histogram | Signature verification latency |
| `marshaled_build_duration_*` | Histogram | Block build duration |
| `network_spawner_messages_sent_total` | Counter | P2P messages sent |
| `network_spawner_messages_received_total` | Counter | P2P messages received |
| `broadcast_get_total` | Counter | Broadcast retrieval attempts (by status) |
| `broadcast_receive_total` | Counter | Broadcast receives (by status) |

These are the ~20 metrics that the Grafana dashboard uses. They all come from Commonware, not Kora application code.

---

## 4. MISSING METRICS — Critical Gaps

### A. Transaction Lifecycle (MISSING)

```
kora_txpool_size                              (Gauge)    Current mempool transaction count
kora_txpool_additions_total                   (Counter)  Transactions added to mempool
kora_txpool_evictions_total                   (Counter)  Transactions evicted
kora_txpool_pruned_total                      (Counter)  Transactions pruned after finalization
kora_transaction_validation_duration_seconds  (Histogram) Validation latency
kora_transaction_validation_errors_total      (Counter)  Failed validations by error type
kora_transaction_submission_duration_seconds  (Histogram) RPC submission latency
kora_transaction_submission_errors_total      (Counter)  RPC submission failures
```

### B. Block Execution (MISSING)

```
kora_block_execution_duration_seconds   (Histogram) Block execution time
kora_block_execution_errors_total       (Counter)  Failed block executions
kora_block_gas_used                     (Histogram) Gas consumed per block
kora_block_transactions_total           (Histogram) Transactions per block
kora_block_skipped_txs_total            (Counter)  Transactions skipped during execution
kora_block_state_root_duration_seconds  (Histogram) State root computation time
```

### C. Consensus (MISSING — application layer)

```
kora_consensus_proposal_success_total   (Counter)  Successful block proposals
kora_consensus_proposal_failure_total   (Counter)  Failed proposals (returned None)
kora_consensus_proposal_duration_seconds (Histogram) Time to build proposal
kora_consensus_nullification_rate       (Gauge)    Ratio of nullified to total rounds
kora_consensus_leader_proposals_total   (Counter)  Proposals by validator index
```

### D. Finalization/Persistence (MISSING)

```
kora_finalized_blocks_total                  (Counter)   Total blocks finalized
kora_finalization_latency_seconds            (Histogram)  Proposal → finalization time
kora_snapshot_persistence_duration_seconds   (Histogram)  persist_snapshot() duration
kora_snapshot_persistence_errors_total       (Counter)   Failed persistence attempts
kora_qmdb_changes_applied_total             (Counter)   State changes committed
```

### E. State/Storage (MISSING)

```
kora_qmdb_operation_duration_seconds  (Histogram) QMDB read/write latency
kora_qmdb_cached_snapshots            (Gauge)    Snapshots in memory
kora_overlay_state_size_bytes          (Gauge)    Uncommitted state size
kora_storage_read_errors_total         (Counter)  Failed storage reads
kora_storage_write_errors_total        (Counter)  Failed storage writes
```

### F. RPC (MISSING)

```
kora_rpc_requests_total                (Counter)   Total RPC requests by method
kora_rpc_request_duration_seconds      (Histogram)  RPC latency by method
kora_rpc_request_errors_total          (Counter)   RPC errors by method
kora_rpc_active_connections            (Gauge)     Current connections
kora_eth_send_transaction_errors_total (Counter)   Failed tx submissions
```

### G. P2P/Network (MISSING)

```
kora_p2p_peer_connections       (Gauge)    Active peer connections
kora_p2p_peer_disconnections    (Counter)  Peer disconnections
kora_p2p_message_errors_total   (Counter)  Send/receive failures by type
kora_p2p_resolver_invalid_total (Counter)  "Invalid data received" events
```

### H. Marshal Actor (MISSING)

```
kora_marshal_blocks_processed_total   (Counter)   Blocks processed
kora_marshal_repair_requests_total    (Counter)   Block repair requests
kora_marshal_repair_success_total     (Counter)   Successful repairs
kora_marshal_pruning_duration_seconds (Histogram)  Pruning time
```

### I. System/Health (MISSING)

```
kora_node_started_timestamp_seconds (Gauge)  Node start time
kora_validator_index                (Gauge)  Validator index in committee
kora_validator_is_leader            (Gauge)  Current leader status
```

---

## 5. Implementation Priority

| Priority | Category | Rationale |
|----------|----------|-----------|
| **HIGH** | Block execution metrics | Core visibility into why blocks fail |
| **HIGH** | Transaction lifecycle | Understand mempool health and throughput |
| **HIGH** | Consensus application metrics | Distinguish Kora failures from Commonware events |
| **MEDIUM** | Finalization/persistence | Track persistence bottlenecks |
| **MEDIUM** | RPC metrics | API performance visibility |
| **MEDIUM** | State/storage | QMDB performance tracking |
| **LOW** | P2P application layer | Network health (partially covered by Commonware) |
| **LOW** | Marshal actor | Internal pipeline visibility |

---

## 6. How to Add Metrics

Metrics can be registered using Commonware's runtime context:

```rust
// In runner.rs during initialization:
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::histogram::Histogram;

let block_execution_errors = Counter::default();
context.register("kora_block_execution_errors_total", "Failed block executions", block_execution_errors.clone());

// Then pass to executor/application via Arc:
let metrics = Arc::new(KoraMetrics { block_execution_errors });
```

The `context.encode()` call in the metrics endpoint handler will automatically include all registered metrics.
