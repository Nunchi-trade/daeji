# Kora Observability Infrastructure Reference

## What is Kora?

Kora is a minimal, high-performance EVM-compatible blockchain execution client built in Rust. It combines three key technologies:

- **Commonware Simplex** -- BLS12-381 threshold consensus (Byzantine Fault Tolerant finality)
- **REVM** -- Ethereum Virtual Machine execution for state transitions
- **QMDB** -- High-performance state storage

The devnet runs a 4-validator network (3-of-4 threshold for finality) plus one secondary (non-voting follower) peer. Each validator participates in consensus, proposes blocks, executes EVM transactions, and persists finalized state. The observability stack monitors all of this: consensus health, block production, execution latency, network connectivity, and resource consumption.

---

## Architecture Overview

The observability stack consists of four components working together:

```
                        +------------------+
                        |     Grafana      |  :3000 (dashboards)
                        |  (visualization) |
                        +--------+---------+
                                 |
                  +--------------+--------------+
                  |                             |
         +--------+--------+          +--------+--------+
         |   Prometheus     |          |      Loki       |  :3100
         |   (metrics)      |  :9090   |  (log storage)  |
         +--------+---------+          +--------+--------+
                  |                             |
    +------+------+------+------+     +--------+--------+
    |      |      |      |      |     |    Promtail      |
  node0  node1  node2  node3  sec0    | (log collector)  |
  :9002  :9002  :9002  :9002  :9002   +-----------------+
                                              |
                                       Docker Socket
                                   (reads container logs)
```

**Data flow:**
1. **Prometheus** scrapes metrics from each validator (port 9002 internal) every 10 seconds
2. **Promtail** discovers Docker containers via socket, tails their logs, parses structured fields, and pushes to Loki
3. **Loki** stores logs with labels for fast querying (72-hour retention)
4. **Grafana** queries both Prometheus and Loki to render dashboards and fire alerts

All services run in the `observability` Docker Compose profile and communicate over the `kora-net` bridge network.

---

## Access Information

| Component | Internal Port | Host Port | URL | Credentials |
|-----------|--------------|-----------|-----|-------------|
| Grafana | 3000 | 3000 | http://localhost:3000 | admin / admin (anonymous viewing enabled) |
| Prometheus | 9090 | 9090 | http://localhost:9090 | None (open) |
| Loki | 3100 | 3100 | http://localhost:3100 | None (auth disabled) |
| Promtail | 9080 | Not exposed | Internal only | N/A |
| Validator 0 metrics | 9002 | 9000 | http://localhost:9000/metrics | N/A |
| Validator 1 metrics | 9002 | 9001 | http://localhost:9001/metrics | N/A |
| Validator 2 metrics | 9002 | 9002 | http://localhost:9002/metrics | N/A |
| Validator 3 metrics | 9002 | 9003 | http://localhost:9003/metrics | N/A |

**Grafana datasources** (auto-provisioned):
- `Prometheus` (uid: `prometheus`) -- default, at http://prometheus:9090
- `Loki` (uid: `loki`) -- at http://loki:3100

---

## Grafana Dashboards

### 1. Kora Devnet Overview

| | |
|---|---|
| **URL** | `/d/kora-overview` |
| **UID** | `kora-overview` |
| **Purpose** | At-a-glance chain health. The first place to look. |
| **Refresh** | Every 5 seconds |
| **Time range** | Last 15 minutes |

**Sections and panels:**

| Section | Panels | What to look for |
|---------|--------|------------------|
| Overview (stat row) | Validators Up, Finalized Height, Blocks/sec, Avg Finalization Latency, Nullifications/s, Timeouts/s, Avg Memory, Height Drift | Any stat in red/yellow indicates a problem |
| Consensus Health | Finalized Height (per node), Finalization Rate (blocks/s per node), Node Divergence (height drift + view drift) | Lines diverging = lagging node; flat line = stall |
| Latency | Notarization Latency, Finalization Latency, Block Build Duration | Spikes above 1s (build) or 2s (finalization) are critical |
| Faults & Anomalies | Timeouts by Reason, Nullification Rate by Reporter, Skip Rate (wasted views) | MissingProposal timeouts = executor issue; high skip rate >30% = pre-stall |
| Network | Network Bandwidth, Message Rate, Consensus Message Types | Drop to zero = network isolation |
| Resources | Memory (RSS), Disk I/O, Tasks (running + spawned/s) | Unbounded growth = memory leak |
| Broadcast & Resolver | Broadcast Gets (success/failure), Broadcast Receives, Signature Verify Latency | Failures spiking = P2P degradation |

---

### 2. Kora Performance & Block Time

| | |
|---|---|
| **URL** | `/d/kora-performance` |
| **UID** | `kora-performance` |
| **Purpose** | Deep performance analysis, optimization targets, throughput benchmarking |
| **Refresh** | Every 5 seconds |
| **Time range** | Last 30 minutes |

**Sections and panels:**

| Section | Panels | What to look for |
|---------|--------|------------------|
| Block Time & Throughput (stat row) | Avg Block Time, Peak Blocks/sec, Consensus Efficiency, Avg Build Time, Avg Finalization Latency, Avg Sig Verify | Build time must stay well under 2s leader timeout |
| Block Time Breakdown | Block Time Composition (stacked: build + notarization + finalization), Actual vs Theoretical Block Time + Sig Verify Overhead | Gap between actual and theoretical = wasted views |
| Latency Percentiles | Block Build Duration p50/p95/p99, Finalization Latency p50/p95/p99, Sig Verify Latency p50/p95/p99 | p99 crossing red thresholds = imminent failures |
| Consensus Pipeline Efficiency | Capacity vs Actual Throughput (views/s vs blocks/s), Consensus Messages/sec by Type, Signature Batch Size & Throughput | Large gap = many wasted rounds |
| Storage & I/O Performance | Aggregate Storage I/O (read/write B/s), Storage IOPS per Node, Average Write Size | Plateau in writes while finalization slows = disk bottleneck |
| Network Performance | Network Bandwidth per Node, Network Cost per Block (bytes/block), Resolver Fetch/Serve Latency | Rising bytes/block = protocol overhead growing |
| Resource Utilization vs Throughput | Memory (RSS), Task Concurrency, Memory Growth per Block | Rising bytes/block in memory = leak |
| Optimization Targets | Time Budget Breakdown (pie chart), Resolver Health (blocked peers + fetch queue), Voter Journal Activity | Largest pie slice = where to optimize; blocked peers > 0 = catch-up broken |

---

### 3. Kora Stall Diagnostics

| | |
|---|---|
| **URL** | `/d/kora-stall-diagnostics` |
| **UID** | `kora-stall-diagnostics` |
| **Purpose** | Root-cause analysis when the chain stalls or degrades |
| **Refresh** | Every 5 seconds |
| **Time range** | Last 30 minutes |

**Sections and panels:**

| Section | Panels | What to look for |
|---------|--------|------------------|
| Stall Detection (stat row) | Blocks/sec (0 = stalled), Skip Rate (>30% = danger), Nodes w/ Active Views, Nodes Finalizing, Height Drift, Nullifications/s | Active views > finalizing nodes = proposals failing |
| Per-Node Consensus State | Current View per Node, Finalized Height per Node, View Rate vs Finalization Rate | View advancing but height frozen = nullification loop |
| Nullification & Timeout Analysis | Nullifications/s per Node, Timeouts by Reason (stacked), Skip Rate per Node | Symmetric nullifications = systemic (mempool); asymmetric = single node issue |
| Block Building & Execution | Block Build Duration (with 2s threshold line), Finalization Latency, Signature Verify Latency | Build time crossing 2s line = proposals timing out |
| Network & P2P Health | Broadcast Success vs Failure, Consensus Message Types, Network Bandwidth | High failure rate = resolver blocking; drop to zero = network isolation |
| Prometheus Alerts | Firing Alerts table | Shows all currently active alerts |
| Resource Correlation | Memory (RSS) per Node (with 2GB threshold), Disk I/O, Active Tasks | Sustained memory growth without plateau = mempool leak |

---

### 4. Kora Logs Explorer

| | |
|---|---|
| **URL** | `/d/kora-logs` |
| **UID** | `kora-logs` |
| **Purpose** | Log-based debugging via Loki |
| **Refresh** | Every 10 seconds |
| **Time range** | Last 30 minutes |

**Sections and panels:**

| Section | Panels | What to look for |
|---------|--------|------------------|
| Log Volume & Errors | Log Volume by Level (stacked rate), Error+Warn Rate per Node | Spikes in WARN/ERROR precede stalls |
| Critical Error Patterns | Voter Panics / Task Crashes, Resolver Invalid Data (Catch-up Failures) | Any voter panic = consensus actor died; "invalid data received" = catch-up broken after restart |
| Transaction & Mempool Errors | Duplicate TX Rejections/s, TX Validation Failures/s, Block Execution Failures/s | Burst of duplicates = tx storm; execution failures = mempool poisoning |
| Finalization & Persistence Errors | Log panel: "failed to persist", "state root mismatch", "missing parent snapshot" | Indicates FinalizedReporter early-return bug or QMDB issues |
| Consensus Activity Logs | Consensus Lifecycle Events: initialization, restarts, state changes | Track node restart behavior and recovery |
| Full Log Stream | All Warnings & Errors (filterable) | Ad-hoc investigation |

---

### 5. Kora Transaction Flow & Load Test

| | |
|---|---|
| **URL** | `/d/kora-txflow` |
| **UID** | `kora-txflow` |
| **Purpose** | Monitor transaction throughput and detect stall patterns under load |
| **Refresh** | Every 5 seconds |
| **Time range** | Last 15 minutes |

**Sections and panels:**

| Section | Panels | What to look for |
|---------|--------|------------------|
| Transaction Throughput (stat row) | Blocks/sec, Block Time, Consensus Efficiency, Skip Rate, Nullifications/s, Height Drift | Uses pre-computed recording rules for instant display |
| Finalized Height & Consensus Progress | Finalized Height (all validators), Consensus View (all validators) | Diverging lines = node lagging |
| Block Building & Execution | Build Duration p50/p95/p99, Finalization Latency p50/p95/p99, Nullifications vs Timeouts rate | Correlate nullification spikes with build time spikes |
| Per-Node Skip Rate & Health | Skip Rate per Node, Resolver Blocked Peers | Blocked peers after restart = permanent stall risk |
| Resource Usage | Memory (RSS) per Node, Storage Write Rate | Correlate resource usage with throughput |
| Stall Indicators | Views Without Finalization (STALL INDICATOR), Timeout Rate by Reason, Broadcast Failures | Red highlight when view advances but finalization stops |

---

## Prometheus Alert Rules (22 Total)

### Critical Severity (Immediate action required -- chain is broken or about to break)

| Alert | Expression | For | Condition | Meaning & Response |
|-------|-----------|-----|-----------|-------------------|
| `ValidatorDown` | `up{job="kora-validators"} == 0` | 30s | Node is unreachable by Prometheus | Container crashed or network partition. Check `docker compose ps` and container logs. |
| `ConsensusStall` | `rate(finalized_height[5m]) == 0 and up == 1` | 2m | No blocks finalized while node is up | Chain is dead. Likely mempool poisoning or quorum loss. Check nullification rate and executor logs. |
| `VoterCrash` | `rate(finalized_height[1m]) == 0 and up == 1 and rate(current_view[1m]) == 0` | 1m | Node is up but view is frozen | Voter actor panicked. Restart the node. Check logs for "voter should not finish". |
| `CriticalBlockBuild` | `kora:build_duration:p99 > 1.8` | 1m | Block build p99 exceeds 1.8s (approaching 2s leader timeout) | Proposals will start timing out causing nullifications. Reduce BLOCK_CODEC_MAX_TXS or investigate mempool size and ECDSA recovery cost. |
| `StorageWriteStall` | `rate(finalized_height[5m]) > 0 and rate(runtime_storage_writes_total[5m]) == 0` | 2m | Blocks finalize but nothing is persisted to disk | State will be lost on restart. Check QMDB health and disk space. |
| `MempoolPoisoning` | `views advancing + no finalization + nullifications > 5/s` | 1m | Every leader fails to propose a valid block | Invalid transactions stuck in mempool cause executor abort on every proposal. Requires chain reset or mempool flush. |
| `AllLeadersFailing` | `nullifications > 20/s + finalization == 0` | 30s | Extremely high nullification rate with zero finalization | All leaders are failing. Check for bad transactions in mempool. Immediate intervention needed. |
| `EfficiencyCliff` | `efficiency < 0.1 and efficiency_5m_ago > 0.5` | Instant | Sudden drop from >50% to <10% efficiency | Pattern that precedes permanent stalls from mempool poisoning. This is an early warning that needs immediate investigation. |

---

### Warning Severity (Degradation, investigate soon)

| Alert | Expression | For | Condition | Meaning & Response |
|-------|-----------|-----|-----------|-------------------|
| `HeightDrift` | `max(finalized_height) - min(finalized_height) > 10` | 1m | Validators diverge by >10 blocks | A node is struggling to keep up or stuck in catch-up. Check per-node finalization rate. |
| `HighNullificationRate` | `sum(rate(nullifications_total[5m])) > 5` | 2m | Over 5 nullifications per second sustained | Block building is failing. Check executor errors and mempool state. |
| `HighSkipRate` | `1 - (finalized_rate / view_rate) > 0.3` | 3m | Over 30% of consensus views are wasted | Approaching stall territory. The production stall was preceded by 33% skip rate. |
| `HighTimeoutRate` | `sum(rate(timeouts_total[5m])) > 5` | 2m | Over 5 timeouts per second | Leaders are failing to propose blocks in time. Check build duration and network latency. |
| `NodeLagging` | `node_rate < 0.1 * avg(rate)` and `rate > 0` | 3m | One node finalization rate is <10% of cluster average | Resolver catch-up issues or degraded local performance. |
| `HighMemoryUsage` | `runtime_process_rss > 2e9` | 5m | RSS exceeds 2 GB | Possible memory leak or unbounded mempool growth. Check mempool size. |
| `BroadcastFailures` | `rate(broadcast_get_total{status="Failure"}[5m]) > 1` | 2m | Block broadcast failing over 1/s | P2P connectivity degraded. Check network and peer connections. |
| `ViewWithoutFinalization` | `rate(current_view[5m]) > 0 and rate(finalized_height[5m]) == 0` | 3m | Views advancing but no blocks finalized | Possible quorum loss or executor failures. Check how many nodes are actually participating. |
| `SlowBlockBuild` | `kora:build_duration:p95 > 1` | 2m | Block build p95 exceeds 1 second | ECDSA recovery or mempool size may be the cause. Leader timeout is 2s so this is getting close. |
| `HighFinalizationLatency` | `kora:finalization_latency:p95 > 2` | 3m | Takes over 2s to collect 2/3+ votes | Check network connectivity and signature verification performance. |
| `ThroughputDrop` | `blocks_per_sec < 0.3 * avg_over_time(blocks_per_sec[1h])` | 5m | Block production dropped 70%+ from 1h baseline | Something caused a major throughput regression. |
| `LowConsensusEfficiency` | `kora:consensus_efficiency < 0.5` | 5m | Less than 50% of views produce blocks | Too many nullifications. This pattern preceded the production stall (which was at 67% inefficiency). |
| `ResolverPeersBlocked` | `engine_resolver_resolver_peers_blocked > 0` | 1m | Any blocked resolver peers | Blocked peers cannot provide blocks for catch-up. This caused permanent stall after node restarts in production. |
| `MemoryLeakSuspected` | `deriv(runtime_process_rss[15m]) > 10e6` | 10m | Memory growing >10MB/s sustained for 10 minutes | Possible unbounded mempool or state accumulation. |

---

## Prometheus Recording Rules (22 Total)

Pre-computed metrics evaluated every 10 seconds for fast dashboard loading.

### Performance Recording Group

| Rule Name | Expression | Purpose |
|-----------|-----------|---------|
| `kora:build_duration:p50` | `histogram_quantile(0.50, sum(rate(marshaled_build_duration_bucket[5m])) by (le))` | Block build median latency |
| `kora:build_duration:p95` | `histogram_quantile(0.95, sum(rate(marshaled_build_duration_bucket[5m])) by (le))` | Block build 95th percentile |
| `kora:build_duration:p99` | `histogram_quantile(0.99, sum(rate(marshaled_build_duration_bucket[5m])) by (le))` | Block build 99th percentile |
| `kora:finalization_latency:p50` | `histogram_quantile(0.50, sum(rate(engine_voter_finalization_latency_bucket[5m])) by (le))` | Finalization median latency |
| `kora:finalization_latency:p95` | `histogram_quantile(0.95, sum(rate(engine_voter_finalization_latency_bucket[5m])) by (le))` | Finalization 95th percentile |
| `kora:finalization_latency:p99` | `histogram_quantile(0.99, sum(rate(engine_voter_finalization_latency_bucket[5m])) by (le))` | Finalization 99th percentile |
| `kora:notarization_latency:p50` | `histogram_quantile(0.50, sum(rate(engine_voter_notarization_latency_bucket[5m])) by (le))` | Notarization median latency |
| `kora:notarization_latency:p95` | `histogram_quantile(0.95, sum(rate(engine_voter_notarization_latency_bucket[5m])) by (le))` | Notarization 95th percentile |
| `kora:verify_latency:p50` | `histogram_quantile(0.50, sum(rate(engine_batcher_verify_latency_bucket[5m])) by (le))` | Signature verification median |
| `kora:verify_latency:p95` | `histogram_quantile(0.95, sum(rate(engine_batcher_verify_latency_bucket[5m])) by (le))` | Signature verification 95th percentile |
| `kora:resolver_fetch:p50` | `histogram_quantile(0.50, sum(rate(engine_resolver_resolver_fetch_duration_bucket[5m])) by (le))` | Block fetch median latency |
| `kora:resolver_fetch:p95` | `histogram_quantile(0.95, sum(rate(engine_resolver_resolver_fetch_duration_bucket[5m])) by (le))` | Block fetch 95th percentile |

### Throughput Recording Group

| Rule Name | Expression | Purpose |
|-----------|-----------|---------|
| `kora:blocks_per_sec` | `avg(rate(finalized_height[1m]))` | Core throughput metric |
| `kora:views_per_sec` | `avg(rate(engine_voter_state_current_view[1m]))` | Consensus round rate |
| `kora:block_time` | `1 / (avg(rate(finalized_height[1m])) > 0)` | Effective block time in seconds |
| `kora:consensus_efficiency` | `avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m]))` | Ratio of finalized blocks to total views (1.0 = perfect) |
| `kora:skip_rate` | `1 - (avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m])))` | Fraction of wasted consensus rounds |
| `kora:height_drift` | `max(finalized_height) - min(finalized_height)` | Block height difference between fastest and slowest node |
| `kora:nullification_rate` | `sum(rate(engine_voter_state_nullifications_total[5m]))` | Total nullifications per second across all nodes |
| `kora:network_bytes_per_block` | `avg((rate(outbound_bandwidth[5m]) + rate(inbound_bandwidth[5m])) / (rate(finalized_height[5m]) > 0))` | Network cost per finalized block |
| `kora:storage_write_rate` | `sum(rate(runtime_storage_write_bytes_total[1m]))` | Aggregate storage write throughput |
| `kora:storage_iops` | `sum(rate(runtime_storage_writes_total[1m]))` | Aggregate storage write operations per second |

---

## Key Prometheus Metrics

All metrics are exposed by the Commonware runtime layer on port 9002.

### Consensus Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `finalized_height` | Gauge | instance, validator_index | Current finalized block height |
| `engine_voter_state_current_view` | Gauge | instance, validator_index | Current consensus view number |
| `engine_voter_state_nullifications_total` | Counter | instance, validator_index | Total nullified (failed) proposals |
| `engine_voter_state_timeouts_total` | Counter | instance, validator_index, reason | Consensus timeouts (reason: MissingProposal, LeaderNullify, LeaderTimeout) |
| `engine_voter_finalization_latency` | Histogram | instance, validator_index | Time to collect 2/3+ finalization votes (seconds) |
| `engine_voter_notarization_latency` | Histogram | instance, validator_index | Time to collect 2/3+ notarization votes (seconds) |
| `engine_voter_outbound_messages_total` | Counter | instance, message | Consensus messages sent (by type) |
| `engine_voter_journal_tracked` | Gauge | instance | Voter journal tracked items |
| `engine_voter_journal_synced` | Counter | instance | Voter journal sync operations |
| `engine_voter_journal_pruned` | Counter | instance | Voter journal prune operations |

### Block Building Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `marshaled_build_duration` | Histogram | instance, validator_index | Time to build a block (transaction execution + marshaling) |
| `engine_batcher_verify_latency` | Histogram | instance, validator_index | BLS signature verification latency |
| `engine_batcher_batch_size` | Histogram | instance | Number of signatures verified per batch |
| `engine_batcher_added` | Counter | instance | Messages submitted to the signature batcher |

### Resolver & Catch-up Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `engine_resolver_resolver_peers_blocked` | Gauge | instance, validator_index | Number of peers blocked from providing blocks |
| `engine_resolver_resolver_fetch_duration` | Histogram | instance | Duration to fetch a block from peers |
| `engine_resolver_resolver_serve_duration` | Histogram | instance | Duration to serve a block to peers |
| `engine_resolver_resolver_fetch_active` | Gauge | instance | Currently active block fetch operations |
| `engine_resolver_resolver_fetch_pending` | Gauge | instance | Pending block fetch operations in queue |

### Broadcast / P2P Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `broadcast_get_total` | Counter | instance, status (Success/Failure) | Block broadcast get operations |
| `broadcast_receive_total` | Counter | instance, status | Block broadcast receive operations |
| `network_spawner_messages_sent_total` | Counter | instance | P2P messages sent |
| `network_spawner_messages_received_total` | Counter | instance | P2P messages received |
| `runtime_outbound_bandwidth_total` | Counter | instance, validator_index | Total outbound bytes |
| `runtime_inbound_bandwidth_total` | Counter | instance, validator_index | Total inbound bytes |

### Resource Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `runtime_process_rss` | Gauge | instance, validator_index | Resident Set Size (memory) in bytes |
| `runtime_storage_write_bytes_total` | Counter | instance, validator_index | Total bytes written to storage |
| `runtime_storage_read_bytes_total` | Counter | instance, validator_index | Total bytes read from storage |
| `runtime_storage_writes_total` | Counter | instance, validator_index | Total write operations |
| `runtime_storage_reads_total` | Counter | instance, validator_index | Total read operations |
| `runtime_tasks_running` | Gauge | instance, validator_index | Currently active async tasks |
| `runtime_tasks_spawned_total` | Counter | instance, validator_index | Total async tasks spawned |

### Prometheus Infrastructure Metric

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `up` | Gauge | job, instance | Whether the target is reachable (1 = up, 0 = down) |

---

## Loki Log Collection

### How It Works

Promtail uses Docker service discovery to automatically find and tail logs from all containers in the `kora-devnet` compose project. It parses the structured output from Rust's `tracing-subscriber` and extracts labels for fast filtering.

### Log Format

Kora validators emit logs in this format:
```
2026-05-21T12:00:00.000Z  WARN module::path: message key=value
```

Promtail parses this into: `timestamp`, `level`, `module`, `message`.

### Available Labels

| Label | Values | Use |
|-------|--------|-----|
| `container` | Container name (e.g., `kora-devnet-validator-node0-1`) | Filter by Docker container |
| `service` | Compose service name (e.g., `validator-node0`, `secondary-node0`) | Filter by specific service |
| `project` | `kora-devnet` | All Kora containers |
| `node_type` | `validator`, `secondary` | Filter by role |
| `node_index` | `0`, `1`, `2`, `3` | Filter by node number |
| `level` | `INFO`, `WARN`, `ERROR`, `DEBUG`, `TRACE` | Filter by severity |
| `module` | Rust module path (e.g., `kora_runner::consensus`) | Filter by code module |
| `warn_type` | `invalid data received`, `ledger.submit_tx returned false`, `validator rejected tx`, `state root mismatch`, `missing parent snapshot`, `execution failed` | Pre-extracted warning patterns |
| `error_type` | `task panicked`, `failed to persist`, `failed to execute`, `failed to compute` | Pre-extracted error patterns |

### Common Loki Queries

**All errors from validators:**
```logql
{node_type="validator"} |~ "ERROR"
```

**Voter panics (fatal consensus crashes):**
```logql
{node_type=~"validator|secondary"} |~ "task panicked|voter should not finish|PANIC"
```

**Resolver catch-up failures:**
```logql
{node_type=~"validator|secondary"} |~ "invalid data received"
```

**Transaction rejections on a specific node:**
```logql
{service="validator-node0"} |~ "ledger.submit_tx returned false"
```

**Execution failures (mempool poisoning indicator):**
```logql
{node_type="validator"} |~ "execution failed|state root mismatch"
```

**Consensus lifecycle events (startup, recovery):**
```logql
{node_type=~"validator|secondary"} |~ "consensus initialized|Validator started|recovered finalized|Starting production"
```

**Error rate by node (metric query in Grafana):**
```logql
sum by (service) (rate({node_type="validator"} |~ "(?i)(ERROR|WARN)" [1m]))
```

### Loki Configuration

- **Retention**: 72 hours
- **Storage**: Filesystem-based TSDB (schema v13)
- **Chunks directory**: `/loki/chunks`
- **Index prefix**: `index_` (24h period)
- **Max query length**: 721 hours
- **Max query series**: 100,000
- **Compaction**: Enabled with retention enforcement

---

## Quick Diagnostic Commands

### Check if the chain is producing blocks

```bash
# Returns current blocks/sec (0 = stalled)
curl -s "http://localhost:9090/api/v1/query?query=kora:blocks_per_sec" | jq '.data.result[0].value[1]'
```

### Check all node heights

```bash
curl -s "http://localhost:9090/api/v1/query?query=finalized_height" | jq '.data.result[] | {instance: .metric.instance, height: .value[1]}'
```

### Check consensus efficiency

```bash
curl -s "http://localhost:9090/api/v1/query?query=kora:consensus_efficiency" | jq '.data.result[0].value[1]'
```

### Check firing alerts

```bash
curl -s "http://localhost:9090/api/v1/alerts" | jq '.data.alerts[] | select(.state=="firing") | {alert: .labels.alertname, instance: .labels.instance, severity: .labels.severity}'
```

### Check if any node is down

```bash
curl -s "http://localhost:9090/api/v1/query?query=up{job=\"kora-validators\"}" | jq '.data.result[] | {instance: .metric.instance, up: .value[1]}'
```

### Check nullification rate

```bash
curl -s "http://localhost:9090/api/v1/query?query=kora:nullification_rate" | jq '.data.result[0].value[1]'
```

### Check block build time (p95)

```bash
curl -s "http://localhost:9090/api/v1/query?query=kora:build_duration:p95" | jq '.data.result[0].value[1]'
```

### Check height drift

```bash
curl -s "http://localhost:9090/api/v1/query?query=kora:height_drift" | jq '.data.result[0].value[1]'
```

### Check resolver blocked peers

```bash
curl -s "http://localhost:9090/api/v1/query?query=engine_resolver_resolver_peers_blocked" | jq '.data.result[] | {instance: .metric.instance, blocked: .value[1]}'
```

### Query Loki for recent errors (last 5 minutes)

```bash
curl -sG "http://localhost:3100/loki/api/v1/query_range" \
  --data-urlencode 'query={node_type="validator"} |~ "ERROR"' \
  --data-urlencode "start=$(date -v-5M +%s 2>/dev/null || date -d '-5min' +%s)" \
  --data-urlencode "end=$(date +%s)" \
  --data-urlencode "limit=20" | jq '.data.result[].values[][1]'
```

### Query Loki for voter panics

```bash
curl -sG "http://localhost:3100/loki/api/v1/query_range" \
  --data-urlencode 'query={node_type="validator"} |~ "task panicked|voter should not finish"' \
  --data-urlencode "start=$(date -v-30M +%s 2>/dev/null || date -d '-30min' +%s)" \
  --data-urlencode "end=$(date +%s)" \
  --data-urlencode "limit=50" | jq '.data.result[].values[][1]'
```

### Reload Prometheus configuration (no restart needed)

```bash
curl -X POST http://localhost:9090/-/reload
```

### Check Prometheus targets health

```bash
curl -s http://localhost:9090/api/v1/targets | jq '.data.activeTargets[] | {instance: .labels.instance, health: .health, lastScrape: .lastScrape}'
```

---

## Deployment Commands

### Start the full devnet with observability

```bash
cd docker && just trusted-devnet
```

This builds the Docker image, runs DKG (trusted dealer mode), starts all 4 validators + 1 secondary + Prometheus + Grafana + Loki + Promtail.

### Start devnet with interactive DKG (production-like)

```bash
cd docker && just devnet
```

### Start only observability containers (validators already running)

```bash
docker compose -f docker/compose/devnet.yaml --profile observability up -d prometheus grafana loki promtail
```

### Stop everything

```bash
cd docker && just down
```

### Full reset (destroys all data volumes)

```bash
cd docker && just reset
```

### Restart just Prometheus (after config changes)

```bash
docker compose -f docker/compose/devnet.yaml --profile observability restart prometheus
```

### Restart just Grafana (after dashboard changes)

```bash
docker compose -f docker/compose/devnet.yaml --profile observability restart grafana
```

### Restart Loki

```bash
docker compose -f docker/compose/devnet.yaml --profile observability restart loki
```

### Restart validators only

```bash
cd docker && just restart-validators
```

### View validator logs

```bash
cd docker && just logs
```

### View specific node logs

```bash
cd docker && just logs-node validator-node0
```

### Live stats monitor (TUI)

```bash
cd docker && just stats
```

### Check container status

```bash
cd docker && just status
```

---

## File Locations Reference

| File | Absolute Path | Purpose |
|------|---------------|---------|
| Docker Compose | `docker/compose/devnet.yaml` | All service definitions including observability profile |
| Prometheus config | `docker/config/prometheus.yml` | Scrape targets (10s interval), rule file paths |
| Alert rules | `docker/config/alerts.yml` | 22 alert definitions across 5 groups |
| Recording rules | `docker/config/recording-rules.yml` | 22 pre-computed metrics in 2 groups |
| Loki config | `docker/config/loki.yml` | 72h retention, TSDB schema, filesystem storage |
| Promtail config | `docker/config/promtail.yml` | Docker SD, relabeling, pipeline stages for log parsing |
| Overview dashboard | `docker/grafana/dashboards/kora-overview.json` | At-a-glance health dashboard |
| Performance dashboard | `docker/grafana/dashboards/kora-performance.json` | Deep performance analysis |
| Stall diagnostics | `docker/grafana/dashboards/kora-stall-diagnostics.json` | Root-cause stall investigation |
| Logs explorer | `docker/grafana/dashboards/kora-logs.json` | Log-based debugging via Loki |
| Transaction flow | `docker/grafana/dashboards/kora-transaction-flow.json` | Load test monitoring |
| Dashboard provisioning | `docker/grafana/provisioning/dashboards/default.yaml` | Auto-loads dashboards from filesystem |
| Datasource provisioning | `docker/grafana/provisioning/datasources/prometheus.yaml` | Configures Prometheus + Loki datasources |
| Justfile | `docker/Justfile` | Developer commands (devnet, stats, logs, etc.) |
| Devnet startup script | `docker/scripts/devnet-run.sh` | Build + DKG + start with spinner UI |
| Stats monitor script | `docker/scripts/devnet-stats.sh` | Live TUI monitoring via RPC |

---

## How to Add New Dashboards

1. Create a new JSON file in `docker/grafana/dashboards/`:
   - Use an existing dashboard as a template
   - Set a unique `uid` (e.g., `kora-my-dashboard`)
   - Set `"id": null` (Grafana assigns IDs dynamically)
   - Use `"datasource": {"type": "prometheus", "uid": "prometheus"}` or `"type": "loki", "uid": "loki"`

2. The dashboard is automatically loaded by the provisioning system (the `default.yaml` provider watches `/var/lib/grafana/dashboards/` which is volume-mounted from `docker/grafana/dashboards/`).

3. Restart Grafana to pick up changes:
   ```bash
   docker compose -f docker/compose/devnet.yaml --profile observability restart grafana
   ```

4. Add navigation links to existing dashboards by adding to their `"links"` array.

---

## How to Add New Alerts

1. Edit `docker/config/alerts.yml`

2. Add a new rule to an appropriate group (`consensus_critical`, `consensus_warnings`, `resource_alerts`, `performance_alerts`, `transaction_alerts`) or create a new group:
   ```yaml
   - alert: MyNewAlert
     expr: some_metric > threshold
     for: 2m
     labels:
       severity: warning  # or critical
     annotations:
       summary: "Short description with {{ $labels.instance }}"
       description: "Longer explanation of what this means and what to do."
   ```

3. Reload Prometheus (no restart needed):
   ```bash
   curl -X POST http://localhost:9090/-/reload
   ```
   Or restart:
   ```bash
   docker compose -f docker/compose/devnet.yaml --profile observability restart prometheus
   ```

---

## How to Add New Recording Rules

1. Edit `docker/config/recording-rules.yml`

2. Add to an existing group or create a new one:
   ```yaml
   - record: kora:my_new_metric
     expr: some_prometheus_expression
   ```

3. Recording rules are named with the `kora:` prefix by convention.

4. Reload Prometheus:
   ```bash
   curl -X POST http://localhost:9090/-/reload
   ```

---

## Common Debugging Workflows

### Workflow: Chain Stall (No blocks finalizing)

1. **Check blocks/sec**: Is it zero?
   ```bash
   curl -s "http://localhost:9090/api/v1/query?query=kora:blocks_per_sec" | jq '.data.result[0].value[1]'
   ```

2. **Check which nodes are up**:
   ```bash
   curl -s "http://localhost:9090/api/v1/query?query=up{job=\"kora-validators\"}" | jq '.data.result[] | {instance: .metric.instance, up: .value[1]}'
   ```

3. **Determine stall type** -- open the Stall Diagnostics dashboard (`/d/kora-stall-diagnostics`):
   - **Views advancing + no finalization + high nullifications** --> Mempool poisoning (every leader's block fails execution)
   - **Views advancing + no finalization + zero nullifications** --> Quorum loss (not enough validators voting)
   - **Views frozen** --> Voter actor crash (check for "task panicked" in logs)
   - **One node stuck, others fine** --> Single-node catch-up failure (check resolver blocked peers)

4. **Check for mempool poisoning pattern**:
   ```bash
   # High nullification rate = mempool has bad transactions
   curl -s "http://localhost:9090/api/v1/query?query=kora:nullification_rate" | jq '.data.result[0].value[1]'
   ```

5. **Check logs for the root cause**:
   ```logql
   {node_type="validator"} |~ "execution failed|state root mismatch|task panicked"
   ```

6. **Resolution**:
   - Mempool poisoning: Restart all validators (`just restart-validators`) to clear mempools
   - Voter crash: Restart the affected node
   - Quorum loss: Ensure at least 3 of 4 validators are up and healthy
   - Full reset: `just reset` (destroys all data, fresh start)

---

### Workflow: Node Crash / Restart Loop

1. **Check container status**:
   ```bash
   cd docker && just status
   ```

2. **Check if the node comes back up** (restart policy is `unless-stopped`):
   ```bash
   docker compose -f docker/compose/devnet.yaml ps
   ```

3. **Check logs for crash reason**:
   ```bash
   cd docker && just logs-node validator-node0
   ```

4. **After restart, check for catch-up failure**:
   - Open Stall Diagnostics, look at "Resolver Blocked Peers"
   - A restarted node may fail to catch up if resolver blocks peers (known bug)

5. **Check if the node recovered**:
   ```logql
   {service="validator-node0"} |~ "recovered finalized|consensus initialized"
   ```

6. **If catch-up fails permanently**, the node will remain at a lower height. Workaround: full restart of all validators.

---

### Workflow: Performance Degradation (Slow blocks, high latency)

1. **Open Performance dashboard** (`/d/kora-performance`)

2. **Check the Time Budget Breakdown** pie chart -- identify the largest slice:
   - **Build** (blue): Block building is slow. Check ECDSA recovery and mempool size.
   - **Notarization** (green): Vote collection slow. Check network latency.
   - **Finalization** (orange): Second round of votes slow. Check validator count.
   - **Wasted/Nullified** (red): Skipped rounds. Check nullification reasons.

3. **Check block build duration percentiles**:
   - p50 > 200ms: Beginning to be slow
   - p95 > 1s: Warning territory (alert fires)
   - p99 > 1.8s: Critical -- proposals will start timing out

4. **Check signature verification**:
   - If `engine_batcher_verify_latency` p99 > 50ms, increase signature verification parallelism

5. **Check storage I/O**:
   - If write throughput plateaus while finalization slows, disk is the bottleneck
   - Check "Average Write Size" -- many small writes = journal overhead

6. **Check network cost**:
   - Rising bytes/block over time = protocol overhead growing with state size
   - Check resolver fetch latency -- high values mean slow block propagation

7. **Check memory**:
   - If RSS is growing linearly with blocks = possible state leak
   - Use "Memory Growth per Block" panel to confirm

---

### Workflow: Load Test Monitoring

1. **Open Transaction Flow dashboard** (`/d/kora-txflow`)

2. **Before starting load**: Note baseline blocks/sec, consensus efficiency, skip rate

3. **During load test**:
   - Watch "Blocks/sec" -- should be stable
   - Watch "Skip Rate" -- should stay below 20%
   - Watch "Nullifications vs Timeouts" -- spikes indicate the load is too high
   - Watch "Views Without Finalization" -- any non-zero value means the chain is stalling

4. **Key thresholds**:
   - Skip rate > 30% = approaching stall territory
   - Nullification rate > 5/s = block building failures
   - Build time p99 > 1.8s = proposals will start timing out
   - Memory growth without plateau = mempool leak under load

5. **If chain stalls during load test**:
   - Reduce transaction rate
   - Check for duplicate transactions (tx storm pattern)
   - Verify nonces are sequential (race conditions in load generator cause mempool poisoning)

---

## Prometheus Configuration Details

### Scrape Configuration

```yaml
global:
  scrape_interval: 10s      # How often to pull metrics from nodes
  evaluation_interval: 10s  # How often to evaluate rules and alerts
```

### Scrape Jobs

| Job | Targets | Labels Added |
|-----|---------|--------------|
| `prometheus` | `localhost:9090` | (self-monitoring) |
| `kora-validators` | `validator-node0:9002` through `validator-node3:9002` | `validator_index` (0-3) |
| `kora-secondary` | `secondary-node0:9002` | `secondary_index` (0) |

### Relabeling

The `validator_index` label is extracted from the target address using regex:
```yaml
regex: 'validator-node(\d+):.*'
target_label: validator_index
replacement: '$1'
```

This allows filtering and grouping metrics by validator in dashboards and alerts.

---

## Loki Storage Architecture

| Parameter | Value |
|-----------|-------|
| Auth | Disabled |
| Listen port | 3100 |
| Storage backend | Filesystem |
| Schema | TSDB v13 |
| Chunks directory | `/loki/chunks` |
| Rules directory | `/loki/rules` |
| Replication factor | 1 (single instance) |
| KV store | In-memory |
| Index prefix | `index_` |
| Index period | 24 hours |
| Retention period | 72 hours |
| Compaction | Enabled |
| Max query length | 721 hours |
| Max query series | 100,000 |

---

## Docker Volumes

| Volume | Purpose | Persistence |
|--------|---------|-------------|
| `prometheus_data` | Prometheus TSDB (metrics history) | Survives restarts, destroyed on `just reset` |
| `grafana_data` | Grafana state (user settings, annotations) | Survives restarts, destroyed on `just reset` |
| `loki_data` | Loki chunks and indexes (72h of logs) | Survives restarts, destroyed on `just reset` |
| `data_node0` through `data_node3` | Validator persistent data (DKG keys, chain state) | Survives restarts, destroyed on `just reset` |
| `data_secondary0` | Secondary peer data | Survives restarts, destroyed on `just reset` |
| `shared_config` | Shared configuration (peers.json) | Survives restarts, destroyed on `just reset` |
