# Observability Stack Not Enabled on Live Devnet

**Category**: docker, metrics
**Severity**: medium

**Labels**: `bug`, `reliability`, `docker`, `metrics`

## Summary

The repository includes a comprehensive monitoring stack (Prometheus, Grafana, Loki, Promtail) with 27 alert rules, 6 Grafana dashboards, and centralized log aggregation, all gated behind a Docker Compose profile called `observability`. The live 10-node devnet at `65.21.232.29` does not have this profile enabled, so all alerting, dashboards, and log aggregation are dormant. Additionally, the Prometheus scrape configuration only targets 4 validators, which would miss nodes 4-9 even if enabled, and two of the four observability images use unpinned `latest` tags.

## Problem

### Observability services gated behind compose profile

All monitoring services require the `observability` profile to start:

```yaml
# /Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml
# Line 362-363:
prometheus:
  image: prom/prometheus:latest
  profiles: ["observability"]

# Line 387-388:
loki:
  image: grafana/loki:3.4.2
  profiles: ["observability"]

# Line 407-408:
promtail:
  image: grafana/promtail:3.4.2
  profiles: ["observability"]

# Line 431-432:
grafana:
  image: grafana/grafana:latest
  profiles: ["observability"]
```

The `devnet-run.sh` script only enables observability when `COMPOSE_PROFILES` includes it:

```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh:321-328
if [[ "${COMPOSE_PROFILES:-}" == *observability* ]]; then
    run_with_spinner "Launching validator, secondary, and observability containers..." docker compose -f compose/devnet.yaml --profile observability up -d \
        validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 \
        prometheus grafana loki promtail
else
    run_with_spinner "Launching validator and secondary containers..." docker compose -f compose/devnet.yaml up -d \
        validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0
fi
```

### Prometheus config only scrapes 4 validators

Even when enabled, the Prometheus configuration only targets 4 validators:

```yaml
# /Users/will/dev/nunchi/daeji/docker/config/prometheus.yml:14-25
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

The live 10-node devnet has nodes 0-9, so nodes 4-9 would not be scraped.

### Unpinned image tags

Two of four observability images use unpinned `latest` tags:

```yaml
prometheus: image: prom/prometheus:latest    # UNPINNED (line 362)
grafana:    image: grafana/grafana:latest    # UNPINNED (line 431)
loki:       image: grafana/loki:3.4.2       # PINNED (line 387)
promtail:   image: grafana/promtail:3.4.2   # PINNED (line 407)
```

### 27 alert rules exist but are dormant

The alert configuration at `/Users/will/dev/nunchi/daeji/docker/config/alerts.yml` defines 27 alert rules across 6 groups:

| Group | Rules | Examples |
|-------|-------|---------|
| `consensus_critical` | 3 | `ValidatorDown`, `ConsensusStall`, `VoterCrash` |
| `consensus_warnings` | 5 | `HeightDrift`, `HighNullificationRate`, `HighSkipRate`, `HighTimeoutRate`, `NodeLagging` |
| `resource_alerts` | 3 | `HighMemoryUsage`, `BroadcastFailures`, `ViewWithoutFinalization` |
| `performance_alerts` | 8 | `SlowBlockBuild`, `CriticalBlockBuild`, `HighFinalizationLatency`, `ThroughputDrop`, `LowConsensusEfficiency`, `ResolverPeersBlocked`, `MemoryLeakSuspected`, `StorageWriteStall` |
| `transaction_alerts` | 3 | `MempoolPoisoning`, `AllLeadersFailing`, `EfficiencyCliff` |
| `network_partition` | 5 | `PeerDisconnected`, `NetworkPartition`, `HighMessageDropRate`, `AsymmetricConnectivity`, `HighRateLimitedMessages` |

The `HighMemoryUsage` alert (lines 101-108) triggers when RSS exceeds 2GB for 5 minutes -- this would have detected the OOM issues on nodes 8/9 before they entered a restart loop.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml`, lines 361-363 (Prometheus), 386-388 (Loki), 406-408 (Promtail), 430-432 (Grafana)
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh`, lines 321-328 (observability profile conditional)
**File**: `/Users/will/dev/nunchi/daeji/docker/config/prometheus.yml`, lines 14-25 (scrape targets hardcoded to 4 validators)
**File**: `/Users/will/dev/nunchi/daeji/docker/config/alerts.yml`, lines 98-108 (`HighMemoryUsage` alert)

## Impact

- **No alerting**: 27 alert rules that would have caught OOM issues, memory leaks, consensus stalls, and network partitions are all dormant. Operators discover problems only through manual investigation via SSH and `curl`.
- **No dashboards**: Six Grafana dashboards providing real-time visibility into consensus, transactions, and resource usage are unavailable. Operators must manually query individual nodes' `/status` endpoints.
- **No log aggregation**: Diagnosing cross-node issues (e.g., network partitions, resolver errors) requires SSH-ing into each container individually, which is impractical with 10 nodes.
- **Image instability**: Enabling the stack could break if Prometheus or Grafana releases a breaking `latest` version.
- **Incomplete coverage**: Even if enabled, nodes 4-9 would not be scraped by Prometheus.

## Root Cause

1. The observability profile was designed as opt-in to reduce resource usage during development. The live devnet was deployed without enabling it.
2. The Prometheus config was written for the 4-node compose and was never updated for the 10-node deployment.
3. No deployment checklist exists that includes enabling observability.

## Suggested Fix

### 1. Enable the observability profile on the live devnet

```bash
cd /opt/kora/docker
COMPOSE_PROFILES=observability docker compose -f compose/devnet-10node.yaml up -d \
  prometheus grafana loki promtail
```

### 2. Pin all observability images

```yaml
# Before:
prometheus: image: prom/prometheus:latest
grafana:    image: grafana/grafana:latest

# After:
prometheus: image: prom/prometheus:v2.53.0
grafana:    image: grafana/grafana:11.0.0
```

### 3. Update Prometheus config for N validators

Either parameterize the scrape targets or use `dns_sd_configs` to auto-discover nodes:

```yaml
- job_name: 'kora-validators'
  dns_sd_configs:
    - names:
        - 'tasks.validator'
      type: 'A'
      port: 9002
```

Or generate the target list from the compose file at deploy time.

### 4. Add a deployment checklist

Document that production deployments must include `COMPOSE_PROFILES=observability` and verify alert delivery works.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml` -- Pin Prometheus image (line 362), pin Grafana image (line 431)
- `/Users/will/dev/nunchi/daeji/docker/config/prometheus.yml` -- Update scrape targets for N validators (lines 14-25)
- `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh` -- Consider making observability the default (lines 321-328)

## Related Issues

- `093-docker-10node-compose-not-in-vcs.md` -- Unversioned 10-node compose (Prometheus config needs to match deployment)
- `098-docker-oom-restart-loop.md` -- OOM restart loop (alerting would have detected this)
- `100-metrics-comprehensive-coverage.md` -- Missing metrics (alerting depends on metrics being emitted)
