# Prometheus Retention Not Explicitly Configured -- Disk Exhaustion Risk on Long-Running Devnets

**Category:** Bug -- Docker/Monitoring
**Severity:** Medium

## Summary

The Prometheus service in the Docker Compose devnet configuration does not set any explicit retention limits (`--storage.tsdb.retention.time` or `--storage.tsdb.retention.size`). While Prometheus defaults to 15 days of retention, this is not explicitly controlled and can grow to several gigabytes on the devnet server. On a long-running devnet with constrained disk space, the `prometheus_data` Docker volume will eventually exhaust available disk, causing Prometheus to crash and taking down the entire monitoring stack (dashboards, alerts, metric queries).

## Problem

Kora deploys Prometheus as part of its optional observability stack (activated via the `observability` Docker Compose profile). The Prometheus service is defined in `docker/compose/devnet.yaml`, lines 361-384. Its command-line arguments (lines 377-380) configure the config file path, TSDB storage path, and lifecycle API, but do not include any retention configuration:

```yaml
command:
  - '--config.file=/etc/prometheus/prometheus.yml'
  - '--storage.tsdb.path=/prometheus'
  - '--web.enable-lifecycle'
```

The Prometheus configuration (`docker/config/prometheus.yml`) defines three scrape jobs:
- `prometheus` (self-scrape): 1 target
- `kora-validators`: 4 targets (`validator-node0:9002` through `validator-node3:9002`)
- `kora-secondary`: 1 target (`secondary-node0:9002`)

At a 10-second scrape interval (line 2 of `prometheus.yml`) with 6 targets and 200+ metrics per Kora node, Prometheus ingests roughly 120+ samples per second. Additionally, the configuration loads two rule files (`alerts.yml` with 31 alert rules and `recording-rules.yml`), which create derived time series that also consume storage.

Prometheus's default retention is 15 days. At the observed ingestion rate, 15 days of data can consume 2-5 GB depending on metric cardinality and churn. The devnet runs on a Hetzner server with shared disk resources (Docker volumes for 4 validator nodes, a secondary node, QMDB data, Loki logs, and Grafana data all compete for the same disk).

The `prometheus_data` volume is defined at line 20 of the compose file with no size limit.

**File:** `docker/compose/devnet.yaml`, lines 377-380

## Code Reference

Prometheus service definition:

```yaml
# docker/compose/devnet.yaml:361-384
  prometheus:
    image: prom/prometheus:latest
    profiles: ["observability"]
    restart: unless-stopped
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    tmpfs:
      - /tmp:size=64m,mode=1777
    volumes:
      - prometheus_data:/prometheus
      - ../config/prometheus.yml:/etc/prometheus/prometheus.yml:ro
      - ../config/alerts.yml:/etc/prometheus/alerts.yml:ro
      - ../config/recording-rules.yml:/etc/prometheus/recording-rules.yml:ro
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'
      - '--web.enable-lifecycle'
      # NOTE: No --storage.tsdb.retention.time or --storage.tsdb.retention.size
    ports:
      - "127.0.0.1:9090:9090"
    networks:
      - kora-net
```

Prometheus scrape configuration showing all scrape targets:

```yaml
# docker/config/prometheus.yml:1-4
global:
  scrape_interval: 10s
  evaluation_interval: 10s

# docker/config/prometheus.yml:9-35
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
    # ...

  - job_name: 'kora-secondary'
    static_configs:
      - targets:
        - 'secondary-node0:9002'
```

## Impact

1. **Disk exhaustion on long-running devnets.** The `prometheus_data` volume grows continuously with no upper bound. On a server with constrained disk, this eventually fills the disk and causes Prometheus to crash with write errors.
2. **Cascading monitoring failure.** When Prometheus crashes, all dashboards in Grafana go blank, all alert rules stop evaluating, and all historical metrics become inaccessible until Prometheus recovers.
3. **Disk pressure affects other services.** Prometheus shares the same disk as QMDB data volumes, Loki log storage, and Grafana data. Disk exhaustion from Prometheus can cause QMDB commit failures (see issue 020) or Loki data loss.
4. **Manual recovery required.** Recovery requires manually pruning the `prometheus_data` volume or recreating it, during which monitoring data is lost and the monitoring stack is unavailable.

## Root Cause

No retention configuration was added when the Prometheus service was set up. The default 15-day retention was implicitly accepted without considering the disk budget for the devnet server or the interaction with other storage-consuming services on the same host.

## Suggested Fix

Add explicit retention limits to the Prometheus command:

```yaml
# BEFORE (docker/compose/devnet.yaml:377-380)
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'
      - '--web.enable-lifecycle'

# AFTER
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'
      - '--web.enable-lifecycle'
      - '--storage.tsdb.retention.time=7d'
      - '--storage.tsdb.retention.size=2GB'
```

The `retention.size` parameter provides a hard upper bound on disk usage regardless of time-based retention. Adjust values based on the server's available disk space. On the current Hetzner server with 64GB RAM and a standard NVMe disk, 2GB for Prometheus data is a reasonable allocation that provides approximately 5-7 days of history.

## Files to Modify

- `docker/compose/devnet.yaml` -- add `--storage.tsdb.retention.time` and `--storage.tsdb.retention.size` to the Prometheus command (lines 377-380)

## Related Issues

- [094 - No QMDB backup or snapshot strategy](/Users/will/dev/nunchi/daeji/tmp/kora/issues/094-docker-no-qmdb-backup.md) -- another disk exhaustion risk from unbounded data growth
- [097 - Observability stack not enabled on live devnet](/Users/will/dev/nunchi/daeji/tmp/kora/issues/097-docker-observability-not-enabled.md) -- the observability stack (including Prometheus) uses an opt-in profile; this issue applies when the profile is activated

## Labels

`bug`, `docker`, `metrics`, `reliability`
