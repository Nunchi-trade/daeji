# Grafana Dashboards and Monitoring

## 1. Dashboard Overview

**File:** `docker/grafana/dashboards/kora-overview.json`

- **Title:** Kora Devnet Overview
- **UID:** kora-overview
- **Refresh:** 5 seconds
- **Time range:** Last 15 minutes
- **Panels:** 26 panels across 7 rows

---

## 2. Panel Inventory

### Row: OVERVIEW (ID 100)

| # | Panel | Query | Type | Thresholds |
|---|-------|-------|------|------------|
| 1 | Validators Up | `count(up{job="kora-validators"} == 1)` | Stat | R:null Y:3 G:4+ |
| 2 | Finalized Height | `max(finalized_height)` | Stat+area | — |
| 3 | Blocks/sec | `avg(rate(finalized_height[1m]))` | Stat+area | — |
| 4 | Avg Finalization Latency | `avg(rate(engine_voter_finalization_latency_sum[1m]) / rate(engine_voter_finalization_latency_count[1m]))` | Stat+area | G:null Y:0.05s R:0.2s+ |
| 5 | Nullifications/s | `sum(rate(engine_voter_state_nullifications_total[5m]))` | Stat+area | G:null Y:1 R:10+ |
| 6 | Timeouts/s | `sum(rate(engine_voter_state_timeouts_total[5m]))` | Stat+area | G:null Y:1 R:10+ |
| 7 | Avg Memory | `avg(runtime_process_rss)` | Stat+area | bytes |
| 8 | Height Drift | `max(finalized_height) - min(finalized_height)` | Stat | G:null Y:5 R:20+ |

### Row: CONSENSUS HEALTH (ID 101)

| # | Panel | Query | Type |
|---|-------|-------|------|
| 10 | Finalized Height | `finalized_height` per node | Time series |
| 11 | Finalization Rate | `rate(finalized_height[1m])` per node | Time series |
| 12 | Node Divergence | `max-min finalized_height` + `max-min current_view` | Time series |

### Row: LATENCY (ID 102)

| # | Panel | Query | Type |
|---|-------|-------|------|
| 20 | Notarization Latency | `rate(notarization_latency_sum[1m]) / rate(notarization_latency_count[1m])` | Time series (seconds) |
| 21 | Finalization Latency | `rate(finalization_latency_sum[1m]) / rate(finalization_latency_count[1m])` | Time series (seconds) |
| 22 | Block Build Duration | `rate(marshaled_build_duration_sum[1m]) / rate(marshaled_build_duration_count[1m])` | Time series (seconds) |

### Row: FAULTS & ANOMALIES (ID 103)

| # | Panel | Query | Type |
|---|-------|-------|------|
| 30 | Timeouts by Reason | `sum by (reason) (rate(timeouts_total[5m]))` | Stacked timeseries |
| 31 | Nullification Rate by Reporter | `sum by (instance) (rate(nullifications_total[5m]))` | Stacked timeseries |
| 32 | Skip Rate (wasted views) | `1 - (rate(finalized_height[5m]) / rate(current_view[5m]))` | Time series (%) |

Description for Skip Rate: "Ratio of nullified views to total views. Higher = more wasted consensus rounds."

Timeout reasons: MissingProposal, LeaderNullify, LeaderTimeout

### Row: NETWORK (ID 104)

| # | Panel | Query | Type |
|---|-------|-------|------|
| 40 | Network Bandwidth | `rate(runtime_outbound_bandwidth_total[1m])` + `rate(runtime_inbound_bandwidth_total[1m])` | Time series (Bps) |
| 41 | Message Rate | `rate(network_spawner_messages_sent_total[1m])` + `rate(network_spawner_messages_received_total[1m])` | Time series |
| 42 | Consensus Message Types | `sum by (message) (rate(engine_voter_outbound_messages_total[1m]))` | Stacked timeseries |

### Row: RESOURCES (ID 105)

| # | Panel | Query | Type |
|---|-------|-------|------|
| 50 | Memory (RSS) | `runtime_process_rss` per node | Time series (bytes) |
| 51 | Disk I/O | `rate(storage_write_bytes_total[1m])` + `rate(storage_read_bytes_total[1m])` | Time series (Bps) |
| 52 | Tasks | `runtime_tasks_running` + `rate(runtime_tasks_spawned_total[1m])` | Time series |

### Row: BROADCAST & RESOLVER (ID 106)

| # | Panel | Query | Type |
|---|-------|-------|------|
| 60 | Broadcast Gets | `rate(broadcast_get_total{status="Success"}[1m])` + `rate(broadcast_get_total{status="Failure"}[1m])` | Time series |
| 61 | Broadcast Receives | `rate(broadcast_receive_total[1m])` by status | Time series |
| 62 | Signature Verify Latency | `rate(engine_batcher_verify_latency_sum[1m]) / rate(engine_batcher_verify_latency_count[1m])` | Time series (seconds) |

---

## 3. Missing Dashboard Panels

| Panel | Query | Rationale |
|-------|-------|-----------|
| P2P Peer Count | `kora_p2p_peer_connections` | Track connectivity |
| Resolver Invalid Data Rate | `kora_p2p_resolver_invalid_total` | Detect catch-up issues |
| Mempool Size | `kora_txpool_size` | Monitor mempool health |
| Block Execution Errors | `kora_block_execution_errors_total` | Track executor failures |
| Transactions per Block | `kora_block_transactions_total` | Block utilization |
| RPC Request Rate | `kora_rpc_requests_total` | API load |
| Catch-up Progress | Height delta over time for lagging nodes | Recovery monitoring |
| Leader Election Distribution | Proposals by validator | Fairness check |

**Note:** Most missing panels require new application-level Prometheus metrics (see 04-metrics-and-observability.md).

---

## 4. Prometheus Configuration

**File:** `docker/config/prometheus.yml`

```yaml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: 'prometheus'
    static_configs:
      - targets: ['localhost:9090']

  - job_name: 'kora-validators'
    static_configs:
      - targets:
        - 'validator-node0:9002'
        - 'validator-node1:9002'
        - 'validator-node2:9002'
        - 'validator-node3:9002'
    relabel_configs:
      - source_labels: [__address__]
        regex: 'validator-node(\d+):.*'
        target_label: validator_index
        replacement: '$1'
```

**Issues:**
- 15s scrape interval is coarse for consensus monitoring (could be 10s)
- No scrape timeout configured (default 10s)
- **No alerting rules defined**
- Secondary node not scraped

### Missing Alert Rules

| Alert | Condition | Severity |
|-------|-----------|----------|
| NodeDown | `up{job="kora-validators"} == 0 for 30s` | Critical |
| HeightDrift | `max(finalized_height) - min(finalized_height) > 10 for 1m` | Warning |
| HighNullificationRate | `rate(nullifications_total[5m]) > 5` | Warning |
| ConsensusStall | `rate(finalized_height[5m]) == 0` | Critical |
| CatchUpFailure | Node height not advancing for 60s after restart | Warning |

---

## 5. Grafana Provisioning

### Datasource (`docker/grafana/provisioning/datasources/prometheus.yaml`)
```yaml
datasources:
  - name: Prometheus
    type: prometheus
    uid: prometheus
    access: proxy
    url: http://prometheus:9090
    isDefault: true
    editable: false
```

### Dashboard Provider (`docker/grafana/provisioning/dashboards/default.yaml`)
```yaml
providers:
  - name: 'default'
    type: file
    options:
      path: /var/lib/grafana/dashboards
```

Dashboards auto-reload from file changes.

---

## 6. Monitoring Scripts

### devnet-stats.sh — Real-Time CLI Dashboard

**File:** `docker/scripts/devnet-stats.sh`

- Queries all 4 validators + secondary via RPC every 0.3s
- Displays tabular output with color-coded health
- Calculates live blocks/sec from height deltas
- Shows: status, uptime, view, finalized, nullified, proposed, blocks/s, leader

### chaos_monitor.py — Chaos Testing

**File:** `repro-logs/chaos_monitor.py`

- Samples all nodes simultaneously
- CSV-style output with timestamps
- Calculates block rate between samples
- Used for node restart and partition testing

---

## 7. Previous Investigation Evidence

**Directory:** `repro-logs/commonware-evidence/`

### Test 1: One Validator Restart

**Scenario:** Restart `validator-node3` during healthy 4-node consensus.

**Results:**
- Baseline: 4.5 blocks/sec, all nodes synchronized
- Post-restart: node3 **stuck at height 25348 for 120+ seconds** (681 blocks behind)
- Healthy nodes continued at 4.5 blocks/sec
- node3 logs: repeated `"invalid data received"` from resolver
- node3 nullified count reset to 46 (vs 4365 on healthy nodes)

**Key evidence:**
```
validator-node3-1  | WARN commonware_resolver::p2p::engine: invalid data received
peer=fcd86abea94795b4d89b3332e1cedf937b55119d391e17bdeff8762a6ec54e8a
```

### Test 2: Two Validators Down + Restart

**Scenario:** Stop validator-node2 and node3 simultaneously, then restart both.

**Results:**
- During outage: consensus stalled (expected — below 3/4 threshold)
- Post-restart: **severe degradation** — 0.5-1.0 blocks/sec (vs baseline 4.5)
- node3 **stuck at height 25514** (680 blocks behind)
- Nullification rate spiked: +452 nullifications in 180 seconds
- Same "invalid data received" errors

### Timeline Evidence

```
# Test 1 - Baseline (healthy)
[baseline] t=1779298693 | net_rate=4.491 blk/s (~0.223s/blk)

# Test 1 - Post restart (node3 stalled)
[post-restart] t=1779298762 | node3: h=25348 (frozen)
[post-restart] t=1779298882 | node3: h=25348 (still frozen, 120s later)

# Test 2 - Two nodes restart (degraded)
[post-restart] t=1779298972 | node3: h=25514 | net_rate=0.000 blk/s
[post-restart] t=1779298976 | node3: h=25514 | net_rate=0.998 blk/s
[post-restart] t=1779299040 | node3: h=25514 | net_rate=0.990 blk/s
```

### Root Causes Identified

1. **Resolver peer blocking**: Restarted node's resolver rejects "invalid data" from peers and blocks them, preventing catch-up block fetching.

2. **Certificate rejection during catch-up**: Lower-view certificates needed for catch-up are rejected by the resolver, stalling finalization progress.

Both are Commonware library bugs, not Kora application bugs.
