# Observability stack: profile activation, wrong metric names, alert tuning, broken recording rules

## Summary

The observability stack (Prometheus, Grafana, Loki, Promtail) is fully configured but has seven categories of bugs that prevent it from working correctly. The containers are not started by default, recording rules silently overwrite each other due to duplicate names, alert thresholds fire constantly on healthy networks, the memory leak alert cannot fire before OOM, most alerts lack job label filters so they match non-Kora Prometheus targets, Ansible diagnostic playbooks reference metric names that do not exist, and a Grafana dashboard panel queries a non-existent metric.

## Context

Kora's devnet runs 4 validators and 1 secondary node in Docker. Each validator exposes a Prometheus metrics endpoint on port 9002 (mapped to host ports 9000-9003). The observability stack -- Prometheus, Grafana, Loki, and Promtail -- is defined in the same Docker Compose file (`docker/compose/devnet.yaml`) but gated behind a `profiles: ["observability"]` flag. All metrics come from the Commonware runtime framework; Kora application code does not emit any custom Prometheus metrics.

Prometheus scrapes three jobs (defined in `docker/config/prometheus.yml`):
- `prometheus` -- self-monitoring (localhost:9090)
- `kora-validators` -- the 4 validator nodes (validator-node0:9002 through validator-node3:9002)
- `kora-secondary` -- the secondary node (secondary-node0:9002)

Relevant files:

| File | Purpose |
|------|---------|
| `docker/compose/devnet.yaml` | Docker Compose with observability profile |
| `docker/config/prometheus.yml` | Scrape targets (3 jobs), rule file references |
| `docker/config/recording-rules.yml` | 41 recording rules in 3 groups |
| `docker/config/alerts.yml` | 22 alert rules in 5 groups |
| `docker/grafana/dashboards/kora-p2p.json` | P2P & Network dashboard |
| `docker/scripts/devnet-run.sh` | Local devnet startup script |
| `ansible/playbooks/observe.yml` | Ansible playbook to start observability on remote |
| `ansible/playbooks/diagnose.yml` | Ansible diagnostic snapshot playbook |
| `ansible/playbooks/query-metrics.yml` | Ansible quick metric query playbook |
| `ansible/roles/observe/tasks/main.yml` | Observe role (runs `docker compose --profile observability up -d`) |
| `Justfile` | Top-level commands including `remote-observe` |
| `docker/Justfile` | Docker commands including `down`/`reset` |
| `tmp/diagnostic-queries.md` | Query cookbook with wrong metric names |

---

## Sub-issue 1: Observability not started by default (Docker and local)

### Problem

The Prometheus, Grafana, Loki, and Promtail services are defined in `docker/compose/devnet.yaml` under `profiles: ["observability"]` (lines 312, 329, 341, 353):

```yaml
# docker/compose/devnet.yaml
prometheus:
  image: prom/prometheus:latest
  profiles: ["observability"]
  ...

loki:
  image: grafana/loki:3.4.2
  profiles: ["observability"]
  ...

promtail:
  image: grafana/promtail:3.4.2
  profiles: ["observability"]
  ...

grafana:
  image: grafana/grafana:latest
  profiles: ["observability"]
  ...
```

**Remote devnet**: The deploy playbook (`ansible/playbooks/deploy.yml`) starts only the validator containers. Observability requires a separate `just remote-observe` / `ansible-playbook playbooks/observe.yml` step. The observe role at `ansible/roles/observe/tasks/main.yml` correctly runs `docker compose --profile observability up -d prometheus loki promtail grafana`, but it is never called automatically by the deploy pipeline.

**Local devnet**: The `docker/scripts/devnet-run.sh` script (lines 321-328) checks the `COMPOSE_PROFILES` environment variable:

```bash
if [[ "${COMPOSE_PROFILES:-}" == *observability* ]]; then
    run_with_spinner "Launching validator, secondary, and observability containers..." docker compose -f compose/devnet.yaml --profile observability up -d \
        validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 \
        prometheus grafana loki promtail
else
    run_with_spinner "Launching validator and secondary containers..." docker compose -f compose/devnet.yaml up -d \
        validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0
fi
```

This means local observability works if you run `COMPOSE_PROFILES=observability just trusted-devnet`, but this is not documented anywhere. The `just trusted-devnet` and `just devnet` commands in the top-level `Justfile` do not mention this option.

### Impact

All 4 validator metrics endpoints are healthy and emitting ~685 metric lines per node, but nothing is scraping them by default. There are no dashboards, no alerts, and no log aggregation unless operators know the undocumented environment variable or manually run the separate observe step.

### Fix

**Option A -- Auto-start observability with the deploy**:
Add the observe role/playbook as a post-task in `ansible/playbooks/deploy.yml` for the remote path. For local, change `devnet-run.sh` to always start observability (it adds <200MB RAM).

**Option B -- Document the required extra step**:

1. In `Justfile`, add a comment to `trusted-devnet` and `devnet` recipes:
   ```just
   # Start devnet with trusted dealer DKG (fast, insecure, for local dev)
   # Add COMPOSE_PROFILES=observability to also start Prometheus + Grafana
   trusted-devnet:
       cd docker && just trusted-devnet
   ```

2. In `docker/scripts/devnet-run.sh`, add a post-startup message when observability is not running:
   ```bash
   if [[ "${COMPOSE_PROFILES:-}" != *observability* ]]; then
       echo -e "  ${DIM}Tip: run with COMPOSE_PROFILES=observability for Prometheus + Grafana${NC}"
   fi
   ```

3. Add a new Justfile recipe:
   ```just
   # Start local devnet with observability (Prometheus + Grafana)
   trusted-devnet-observe:
       COMPOSE_PROFILES=observability cd docker && just trusted-devnet
   ```

---

## Sub-issue 2: Recording rules silently overwrite each other (duplicate names)

### Problem

In `docker/config/recording-rules.yml`, the P2P channel recording rules (lines 88-184) define **5 separate rules with the same `record:` name**. Prometheus evaluates recording rules within a group sequentially, and when multiple rules produce the same metric name, the last evaluation wins and all previous results are overwritten.

The affected rules:

```yaml
# docker/config/recording-rules.yml, lines 91-120
# All five rules use the same record name:
- record: kora:p2p:channel_sent:rate1m   # data_0 = simplex_votes   -> OVERWRITTEN
  expr: ... network_spawner_messages_sent_total{message="data_0"} ...
- record: kora:p2p:channel_sent:rate1m   # data_1 = simplex_certs   -> OVERWRITTEN
  expr: ... network_spawner_messages_sent_total{message="data_1"} ...
- record: kora:p2p:channel_sent:rate1m   # data_2 = simplex_resolver -> OVERWRITTEN
  expr: ... network_spawner_messages_sent_total{message="data_2"} ...
- record: kora:p2p:channel_sent:rate1m   # data_3 = broadcast_blocks -> OVERWRITTEN
  expr: ... network_spawner_messages_sent_total{message="data_3"} ...
- record: kora:p2p:channel_sent:rate1m   # data_4 = marshal_backfill -> THIS ONE WINS
  expr: ... network_spawner_messages_sent_total{message="data_4"} ...
```

The same pattern repeats for `kora:p2p:channel_recv:rate1m` (lines 123-152) and `kora:p2p:channel_dropped:rate1m` (lines 155-184). That is 15 rules total, where only the last in each group of 5 takes effect.

The intent was for each rule to produce a distinct time series differentiated by a `channel` label added via `label_replace`. But because all 5 rules share the same `record:` name, Prometheus overwrites the output from earlier evaluations. Only the `data_4` (marshal_backfill) channel survives.

### Impact

The P2P dashboard (`docker/grafana/dashboards/kora-p2p.json`) shows channel breakdown panels using these recording rules. Only the `marshal_backfill` channel has data. The 4 other channels (simplex_votes, simplex_certs, simplex_resolver, broadcast_blocks) show as empty. This makes the per-channel traffic analysis useless.

Additionally, `kora:p2p:drop_ratio` (line 197) depends on `kora:p2p:channel_dropped:rate1m`, which is broken, making the drop ratio unreliable.

### Fix

Consolidate each set of 5 rules into a single rule that preserves the `message` label and uses `label_replace` to add human-readable channel names. The key insight is that a single PromQL expression can match all channels at once if you aggregate `by (message)` instead of filtering to a specific `message` value:

```yaml
# docker/config/recording-rules.yml
# Replace all 5 "sent" rules (lines 91-120) with one:
- record: kora:p2p:channel_sent:rate1m
  expr: >-
    label_replace(
      label_replace(
        label_replace(
          label_replace(
            label_replace(
              sum by (instance, message) (rate(network_spawner_messages_sent_total[1m])),
              "channel", "simplex_votes", "message", "data_0"),
            "channel", "simplex_certs", "message", "data_1"),
          "channel", "simplex_resolver", "message", "data_2"),
        "channel", "broadcast_blocks", "message", "data_3"),
      "channel", "marshal_backfill", "message", "data_4")

# Replace all 5 "recv" rules (lines 123-152) with one:
- record: kora:p2p:channel_recv:rate1m
  expr: >-
    label_replace(
      label_replace(
        label_replace(
          label_replace(
            label_replace(
              sum by (instance, message) (rate(network_spawner_messages_received_total[1m])),
              "channel", "simplex_votes", "message", "data_0"),
            "channel", "simplex_certs", "message", "data_1"),
          "channel", "simplex_resolver", "message", "data_2"),
        "channel", "broadcast_blocks", "message", "data_3"),
      "channel", "marshal_backfill", "message", "data_4")

# Replace all 5 "dropped" rules (lines 155-184) with one:
- record: kora:p2p:channel_dropped:rate1m
  expr: >-
    label_replace(
      label_replace(
        label_replace(
          label_replace(
            label_replace(
              sum by (instance, message) (rate(network_router_messages_dropped_total[1m])),
              "channel", "simplex_votes", "message", "data_0"),
            "channel", "simplex_certs", "message", "data_1"),
          "channel", "simplex_resolver", "message", "data_2"),
        "channel", "broadcast_blocks", "message", "data_3"),
      "channel", "marshal_backfill", "message", "data_4")
```

### Additional recording rule issues

**`kora:p2p:total_rate_limited:rate1m`** (line 193) references `network_spawner_messages_rate_limited_total`, which does not exist in any validator's `/metrics` output. This rule always returns NO_DATA. Either the metric needs to be added to the Commonware framework, or this rule should be removed.

**`kora:p2p:drop_ratio`** (lines 196-200) uses `clamp_min(..., 1)` as the denominator floor. When no messages are received, the denominator becomes 1 msg/s, which inflates the ratio. Use `clamp_min(..., 0.001)` or return 0 when the denominator is near zero:

```yaml
# docker/config/recording-rules.yml, line 196
# Before:
- record: kora:p2p:drop_ratio
  expr: >-
    sum(rate(network_router_messages_dropped_total[5m]))
    /
    clamp_min(sum(rate(network_spawner_messages_received_total[5m])), 1)

# After:
- record: kora:p2p:drop_ratio
  expr: >-
    sum(rate(network_router_messages_dropped_total[5m]))
    /
    clamp_min(sum(rate(network_spawner_messages_received_total[5m])), 0.001)
```

---

## Sub-issue 3: Alert thresholds fire on healthy idle networks

### Problem

Four alerts fire constantly on a healthy devnet where all 4 validators are up and the chain is finalizing blocks at ~105 blocks/sec. Alerts that fire during normal operation create alert fatigue and cause operators to ignore genuine failures.

| Alert | Threshold | Observed Value (healthy) | Why it fires |
|-------|-----------|------------------------|--------------|
| `HeightDrift` | `> 10 blocks` for 1m | 544 blocks | At 105 blocks/sec, 10 blocks is <100ms of lag. Minor scrape-interval timing differences cause apparent drift. |
| `HighNullificationRate` | `> 5/s` for 2m | ~43.6/s | The consensus protocol has a known ~33% idle nullification rate. At 143 views/s, this produces ~47 nullifications/s even when healthy. |
| `HighTimeoutRate` | `> 5/s` for 2m | ~45.7/s | Directly correlated with nullifications -- each nullified view involves a timeout. Same root cause. |
| `BroadcastFailures` | `> 1/s` for 2m | ~162/s | Broadcast `Get` failures occur when a block has not propagated yet. This is normal Commonware behavior, not a real failure. |

### Location

`docker/config/alerts.yml`:
- `HeightDrift`: line 41
- `HighNullificationRate`: line 51
- `HighTimeoutRate`: line 72
- `BroadcastFailures`: line 106

### Fix

**`HeightDrift`** (line 41): Raise threshold from 10 to 100 blocks, increase `for` to 2 minutes:
```yaml
# docker/config/alerts.yml, line 41
# Before:
expr: max(finalized_height{job="kora-validators"}) - min(finalized_height{job="kora-validators"}) > 10
for: 1m

# After:
expr: max(finalized_height{job="kora-validators"}) - min(finalized_height{job="kora-validators"}) > 100
for: 2m
```

**`HighNullificationRate`** (line 51): Change from absolute threshold to ratio-based:
```yaml
# docker/config/alerts.yml, line 51
# Before:
expr: sum(rate(engine_voter_state_nullifications_total[5m])) > 5

# After -- alert when nullification ratio exceeds 40%:
expr: >
  sum(rate(engine_voter_state_nullifications_total[5m]))
  / clamp_min(sum(rate(engine_voter_state_current_view[5m])), 0.001) > 0.4
```

**`HighTimeoutRate`** (line 72): Same approach -- use ratio instead of absolute count:
```yaml
# docker/config/alerts.yml, line 72
# Before:
expr: sum(rate(engine_voter_state_timeouts_total[5m])) > 5

# After:
expr: >
  sum(rate(engine_voter_state_timeouts_total[5m]))
  / clamp_min(sum(rate(engine_voter_state_current_view[5m])), 0.001) > 0.4
```

**`BroadcastFailures`** (line 106): Use failure ratio instead of absolute count, and raise the threshold:
```yaml
# docker/config/alerts.yml, line 106
# Before:
expr: rate(broadcast_get_total{status="Failure"}[5m]) > 1

# After -- alert when failure ratio exceeds 30%:
expr: >
  rate(broadcast_get_total{status="Failure"}[5m])
  / clamp_min(
      rate(broadcast_get_total{status="Success"}[5m])
      + rate(broadcast_get_total{status="Failure"}[5m]),
    0.001) > 0.3
```

---

## Sub-issue 4: Memory leak alert fires after OOM (useless)

### Problem

The `MemoryLeakSuspected` alert in `docker/config/alerts.yml` (line 192) is configured to fire only after sustained memory growth of `>10MB/s` for `10 minutes`:

```yaml
# docker/config/alerts.yml, line 192
- alert: MemoryLeakSuspected
  expr: deriv(runtime_process_rss[15m]) > 10e6
  for: 10m
```

Docker containers have a 4GB memory limit (`docker/compose/devnet.yaml`, line 40):
```yaml
deploy:
  resources:
    limits:
      memory: 4G
```

At 10MB/s growth rate over 10 minutes, the node accumulates ~6GB. Since the container limit is 4GB, the OOM killer would terminate the process **before the alert fires**. The alert is useless -- it can never trigger in time.

The companion `HighMemoryUsage` alert (line 96) fires at `runtime_process_rss > 2e9` (2GB) after 5 minutes, which gives 2GB of headroom before the 4GB limit. This one is reasonable.

### Fix

Lower the threshold and shorten the `for` duration so the alert fires while there is still time to investigate:

```yaml
# docker/config/alerts.yml, line 192
# Before:
- alert: MemoryLeakSuspected
  expr: deriv(runtime_process_rss[15m]) > 10e6
  for: 10m

# After -- alert at 1MB/s sustained for 5 minutes:
# At 1MB/s for 5min, that's ~300MB of growth. Starting from a typical 500MB baseline,
# the node would be at ~800MB, well below the 4GB limit. This gives operators time to
# investigate. At this rate, OOM would be ~1 hour away.
- alert: MemoryLeakSuspected
  expr: deriv(runtime_process_rss[15m]) > 1e6
  for: 5m
  labels:
    severity: warning
  annotations:
    summary: "Memory growing at {{ $value | humanize }}B/s on {{ $labels.instance }}"
    description: "Sustained memory growth >1MB/s for 5min. At 4GB container limit, investigate before OOM."
```

---

## Sub-issue 5: Missing job label filters on most alerts

### Problem

Prometheus scrapes 3 jobs (`docker/config/prometheus.yml`):
- `prometheus` (self-monitoring)
- `kora-validators` (4 validator nodes)
- `kora-secondary` (1 secondary node)

Many alert expressions query metrics like `finalized_height`, `runtime_process_rss`, and `broadcast_get_total` without a `{job="kora-validators"}` filter. Since the secondary node also emits some of these same metrics (it runs the same binary), alerts may fire incorrectly or produce unexpected results when secondary node data is included.

Alerts that correctly have `job` filters (4 of 22):

| Alert | Line | Filter |
|-------|------|--------|
| `ValidatorDown` | 6 | `up{job="kora-validators"}` |
| `ConsensusStall` | 16 | `up{job="kora-validators"}` |
| `VoterCrash` | 28 | `up{job="kora-validators"}` |
| `HeightDrift` | 41 | `finalized_height{job="kora-validators"}` |

Alerts **missing** `job` filters (18 of 22):

| Alert | Line | Metric(s) Needing Filter |
|-------|------|--------------------------|
| `HighNullificationRate` | 51 | `engine_voter_state_nullifications_total` |
| `HighSkipRate` | 62 | `finalized_height`, `engine_voter_state_current_view` |
| `HighTimeoutRate` | 72 | `engine_voter_state_timeouts_total` |
| `NodeLagging` | 83 | `finalized_height` |
| `HighMemoryUsage` | 96 | `runtime_process_rss` |
| `BroadcastFailures` | 106 | `broadcast_get_total` |
| `ViewWithoutFinalization` | 116 | `engine_voter_state_current_view`, `finalized_height` |
| `SlowBlockBuild` | 130 | Uses recording rule (no filter needed at alert level) |
| `CriticalBlockBuild` | 140 | Uses recording rule (no filter needed at alert level) |
| `HighFinalizationLatency` | 150 | Uses recording rule (no filter needed at alert level) |
| `ThroughputDrop` | 160 | Uses recording rule (no filter needed at alert level) |
| `LowConsensusEfficiency` | 172 | Uses recording rule (no filter needed at alert level) |
| `ResolverPeersBlocked` | 182 | `engine_resolver_resolver_peers_blocked` |
| `MemoryLeakSuspected` | 192 | `runtime_process_rss` |
| `StorageWriteStall` | 202 | `finalized_height`, `runtime_storage_writes_total` |
| `MempoolPoisoning` | 216 | `engine_voter_state_current_view`, `finalized_height`, `engine_voter_state_nullifications_total` |
| `AllLeadersFailing` | 229 | `engine_voter_state_nullifications_total`, `finalized_height` |
| `EfficiencyCliff` | 241 | Uses recording rule (no filter needed at alert level) |

Alerts that use recording rules (6 of the 18) get their data pre-aggregated, so they do not need `job` filters at the alert level. But the recording rules themselves in `docker/config/recording-rules.yml` also lack `job` filters on raw metrics (e.g., line 45: `avg(rate(finalized_height[1m]))` should be `avg(rate(finalized_height{job="kora-validators"}[1m]))`).

### Fix

Add `{job="kora-validators"}` to every raw metric selector in alert expressions and recording rules. Example for `HighNullificationRate`:

```yaml
# docker/config/alerts.yml, line 51
# Before:
expr: sum(rate(engine_voter_state_nullifications_total[5m])) > 5

# After:
expr: sum(rate(engine_voter_state_nullifications_total{job="kora-validators"}[5m])) > 5
```

The same filter should be added to every raw metric reference in `docker/config/recording-rules.yml`. Example:

```yaml
# docker/config/recording-rules.yml, line 45
# Before:
- record: kora:blocks_per_sec
  expr: avg(rate(finalized_height[1m]))

# After:
- record: kora:blocks_per_sec
  expr: avg(rate(finalized_height{job="kora-validators"}[1m]))
```

Full list of recording rules needing `{job="kora-validators"}` on their raw metric selectors:

| Line | Record Name | Raw Metrics Needing Filter |
|------|-------------|---------------------------|
| 7-12 | `kora:build_duration:*` | `marshaled_build_duration_bucket` |
| 15-20 | `kora:finalization_latency:*` | `engine_voter_finalization_latency_bucket` |
| 23-26 | `kora:notarization_latency:*` | `engine_voter_notarization_latency_bucket` |
| 29-32 | `kora:verify_latency:*` | `engine_batcher_verify_latency_bucket` |
| 35-38 | `kora:resolver_fetch:*` | `engine_resolver_resolver_fetch_duration_bucket` |
| 45 | `kora:blocks_per_sec` | `finalized_height` |
| 47 | `kora:views_per_sec` | `engine_voter_state_current_view` |
| 51 | `kora:block_time` | `finalized_height` |
| 55 | `kora:consensus_efficiency` | `finalized_height`, `engine_voter_state_current_view` |
| 59 | `kora:skip_rate` | `finalized_height`, `engine_voter_state_current_view` |
| 63 | `kora:height_drift` | `finalized_height` |
| 67 | `kora:nullification_rate` | `engine_voter_state_nullifications_total` |
| 71 | `kora:network_bytes_per_block` | `runtime_outbound_bandwidth_total`, `runtime_inbound_bandwidth_total`, `finalized_height` |
| 75 | `kora:storage_write_rate` | `runtime_storage_write_bytes_total` |
| 77 | `kora:storage_iops` | `runtime_storage_writes_total` |
| 91-184 | `kora:p2p:channel_*` | `network_spawner_messages_*_total`, `network_router_messages_dropped_total` |
| 189 | `kora:p2p:total_dropped:rate1m` | `network_router_messages_dropped_total` |
| 193 | `kora:p2p:total_rate_limited:rate1m` | `network_spawner_messages_rate_limited_total` |
| 196-200 | `kora:p2p:drop_ratio` | `network_router_messages_dropped_total`, `network_spawner_messages_received_total` |

---

## Sub-issue 6: Ansible diagnostic playbooks use wrong metric names

### Problem

Two Ansible playbooks reference Prometheus metric names that do not exist. The actual metric names emitted by validators differ from what the playbooks query. Running these playbooks produces empty or error results for the affected sections.

### `ansible/playbooks/diagnose.yml`

| Line | Queries | Actual Metric | Fix |
|------|---------|---------------|-----|
| 43 | `current_view` | `engine_voter_state_current_view` | Replace `current_view` with `engine_voter_state_current_view` |
| 75 | `process_resident_memory_bytes` | `runtime_process_rss` | Replace `process_resident_memory_bytes` with `runtime_process_rss` |

Line 43 (skip rate query):
```yaml
# ansible/playbooks/diagnose.yml, line 43
# Before:
body: "query=1 - avg(rate(finalized_height[1m])) / avg(rate(current_view[1m]))"
# After:
body: "query=1 - avg(rate(finalized_height[1m])) / avg(rate(engine_voter_state_current_view[1m]))"
```

Line 75 (memory query):
```yaml
# ansible/playbooks/diagnose.yml, line 75
# Before:
body: "query=process_resident_memory_bytes"
# After:
body: "query=runtime_process_rss"
```

### `ansible/playbooks/query-metrics.yml`

| Line | Query Name | Queries | Actual Metric | Fix |
|------|-----------|---------|---------------|-----|
| 22 | "Skip rate" | `current_view` | `engine_voter_state_current_view` | Replace metric name |
| 24 | "Nullifications/sec" | `simplex_voter_nullifications_total` | `engine_voter_state_nullifications_total` | Replace metric name |
| 28 | "Memory (MB)" | `process_resident_memory_bytes` | `runtime_process_rss` | Replace metric name |

```yaml
# ansible/playbooks/query-metrics.yml, lines 22-28
# Before:
- name: "Skip rate"
  query: "1 - rate(finalized_height[1m]) / rate(current_view[1m])"
- name: "Nullifications/sec"
  query: "rate(simplex_voter_nullifications_total[1m])"
- name: "Memory (MB)"
  query: "process_resident_memory_bytes / 1048576"

# After:
- name: "Skip rate"
  query: "1 - rate(finalized_height[1m]) / rate(engine_voter_state_current_view[1m])"
- name: "Nullifications/sec"
  query: "rate(engine_voter_state_nullifications_total[1m])"
- name: "Memory (MB)"
  query: "runtime_process_rss / 1048576"
```

### Impact

Running `ansible-playbook playbooks/diagnose.yml` produces a diagnostic report where the "Skip rate" and "Resources / Memory" sections show `N/A` or empty. Running `ansible-playbook playbooks/query-metrics.yml` returns empty results for 3 of 7 default queries (Skip rate, Nullifications/sec, Memory). This makes the primary debugging tools unreliable.

---

## Sub-issue 7: Dashboard panel references non-existent metric

### Problem

The P2P dashboard (`docker/grafana/dashboards/kora-p2p.json`) has a panel titled "Resolver Peer Performance (response EMA)" that queries the wrong metric name.

Line 286 of `kora-p2p.json`:
```json
{"expr": "engine_resolver_resolver_fetcher_peer_performance", "legendFormat": "{{peer}}", "refId": "A"}
```

The actual metric emitted by validators is `resolver_resolver_fetcher_peer_performance` (without the `engine_` prefix). The Commonware framework has two resolver metric registries: `engine_resolver_resolver_*` (used for fetch/serve/blocked metrics on the consensus engine resolver) and `resolver_resolver_*` (used for the standalone resolver's fetcher peer performance tracking). The `fetcher_peer_performance` metric is only in the standalone registry.

### Impact

The "Resolver Peer Performance" panel on the P2P dashboard always shows "No data", even when resolver activity is occurring.

### Fix

In `docker/grafana/dashboards/kora-p2p.json`, change line 286:

```json
// Before:
{"expr": "engine_resolver_resolver_fetcher_peer_performance", ...}

// After:
{"expr": "resolver_resolver_fetcher_peer_performance", ...}
```

Additionally, the same dashboard has a stat panel (line 84) querying `network_spawner_messages_rate_limited_total`:
```json
{"expr": "sum(rate(network_spawner_messages_rate_limited_total[1m]))", ...}
```

This metric does not exist in the current application. The "Msgs Rate Limited/s" stat panel will always show "No data". This is a lower-priority issue -- if rate limiting is added upstream, the metric will appear automatically. For now, consider adding a panel description noting that this metric only appears when P2P rate limiting is active.

---

## Sub-issue 8: Diagnostic query cookbook uses wrong metric names

### Problem

The diagnostic query cookbook (`tmp/diagnostic-queries.md`) contains five queries and a metrics reference section that use metric names from an older version of the Commonware framework. These metrics do not exist in the current validator builds.

| Location | Wrong Metric | Correct Metric |
|----------|-------------|----------------|
| Line 46 (skip rate query) | `current_view` | `engine_voter_state_current_view` |
| Line 58 (nullification rate query) | `simplex_voter_nullifications_total` | `engine_voter_state_nullifications_total` |
| Line 67 (timeout rate query) | `simplex_voter_timeouts_total` | `engine_voter_state_timeouts_total` |
| Line 103 (rate-limited query) | `network_router_messages_rate_limited_total` | Does not exist (see note) |
| Line 116 (memory query) | `process_resident_memory_bytes` | `runtime_process_rss` |
| Lines 234-244 (reference table) | Multiple wrong names in "From Commonware Framework" section | See mapping below |

Specific wrong entries in the reference table (lines 234-244):
```
# tmp/diagnostic-queries.md, lines 234-244
# Wrong:
- `current_view` - gauge: current consensus view number
- `simplex_voter_votes_total` - counter: votes cast
- `simplex_voter_nullifications_total` - counter: nullified views
- `simplex_voter_timeouts_total` - counter: leader timeouts
- `network_router_messages_rate_limited_total` - counter: rate-limited messages
- `network_tracker_tracked_peers` - gauge: known peers
- `process_resident_memory_bytes` - gauge: RSS memory

# Correct:
- `engine_voter_state_current_view` - gauge: current consensus view number
- `engine_batcher_votes_total` - counter: votes cast (if it exists)
- `engine_voter_state_nullifications_total` - counter: nullified views
- `engine_voter_state_timeouts_total` - counter: leader timeouts
- `network_spawner_messages_rate_limited_total` - counter: rate-limited messages (metric not yet instrumented)
- `network_tracker_directory_tracked` - gauge: known peers
- `runtime_process_rss` - gauge: RSS memory
```

Note: `network_router_messages_rate_limited_total` does not exist in any validator's metrics output. The recording rules reference `network_spawner_messages_rate_limited_total` (different prefix: `spawner` vs `router`), but that metric also does not exist. The rate-limited query should either be removed or annotated as "metric not yet instrumented".

### Impact

Copy-pasting queries from the cookbook into Prometheus produces empty results. The metrics reference section at the bottom of the file gives incorrect metric names, which would mislead anyone writing new dashboards or alerts.

---

## Complete metric name mapping

For reference, here is the full mapping of wrong names found across all files to their correct counterparts:

| Wrong Name | Correct Name | Files Affected |
|-----------|-------------|----------------|
| `current_view` | `engine_voter_state_current_view` | `diagnose.yml`, `query-metrics.yml`, `diagnostic-queries.md` |
| `process_resident_memory_bytes` | `runtime_process_rss` | `diagnose.yml`, `query-metrics.yml`, `diagnostic-queries.md` |
| `simplex_voter_nullifications_total` | `engine_voter_state_nullifications_total` | `query-metrics.yml`, `diagnostic-queries.md` |
| `simplex_voter_timeouts_total` | `engine_voter_state_timeouts_total` | `diagnostic-queries.md` |
| `simplex_voter_votes_total` | `engine_batcher_votes_total` (unverified) | `diagnostic-queries.md` |
| `network_tracker_tracked_peers` | `network_tracker_directory_tracked` | `diagnostic-queries.md` |
| `engine_resolver_resolver_fetcher_peer_performance` | `resolver_resolver_fetcher_peer_performance` | `kora-p2p.json` |
| `network_spawner_messages_rate_limited_total` | Does not exist | `recording-rules.yml`, `kora-p2p.json` |
| `network_router_messages_rate_limited_total` | Does not exist | `diagnostic-queries.md` |

---

## Priority and ordering

| Sub-issue | Severity | Effort | Recommendation |
|-----------|----------|--------|----------------|
| 2. Duplicate recording rules | High | Small | Fix first -- single file change, biggest data quality impact |
| 5. Missing job label filters | High | Medium | Fix with sub-issue 2 -- same files, prevents false alerts |
| 4. Memory leak alert OOM | High | Small | Single line change, prevents silent container kills |
| 3. Alert thresholds | Medium | Small | Fix with sub-issue 5 -- same file |
| 6. Ansible wrong metrics | Medium | Small | 5 line changes across 2 files |
| 7. Dashboard wrong metric | Low | Small | 1 line change in JSON |
| 8. Cookbook wrong metrics | Low | Small | Documentation-only, 7 line changes |
| 1. Profile activation | Low | Small | Document the COMPOSE_PROFILES env var |
