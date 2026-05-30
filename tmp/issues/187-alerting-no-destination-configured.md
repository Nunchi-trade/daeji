# No Alerting Destination Configured -- 31 Alert Rules Fire but Nobody Receives Them

**Category:** Bug -- Docker/Monitoring
**Severity:** Medium

## Summary

The Kora devnet monitoring stack defines 31 comprehensive Prometheus alert rules covering consensus stalls, validator crashes, mempool poisoning, network partitions, memory leaks, and performance degradation. However, the Prometheus configuration has no `alerting` section, no Alertmanager is deployed, and no webhook or notification channel is configured. When alert rules fire, the alerts are evaluated and visible in the Prometheus UI but are never delivered to any operator. An operator would need to manually refresh the Prometheus or Grafana web UI to notice firing alerts, which defeats the purpose of automated alerting.

## Problem

The Kora observability stack consists of Prometheus, Grafana, Loki, and Promtail, deployed via the `observability` Docker Compose profile. The alert rules are defined in `docker/config/alerts.yml` and loaded by Prometheus via the `rule_files` directive in `docker/config/prometheus.yml` (line 6).

The alert rules are organized into 5 groups with 31 rules:

| Group                  | Rules | Examples                                              |
|------------------------|-------|-------------------------------------------------------|
| `consensus_critical`   | 3     | ValidatorDown, ConsensusStall, VoterCrash             |
| `consensus_warnings`   | 5     | HeightDrift, HighNullificationRate, HighSkipRate       |
| `resource_alerts`      | 4     | HighMemoryUsage, BroadcastFailures, StorageWriteStall |
| `performance_alerts`   | 8     | SlowBlockBuild, CriticalBlockBuild, ThroughputDrop    |
| `transaction_alerts`   | 3     | MempoolPoisoning, AllLeadersFailing, EfficiencyCliff   |
| `network_partition`    | 5     | PeerDisconnected, NetworkPartition, HighMessageDropRate |

However, the Prometheus configuration (`docker/config/prometheus.yml`) has no `alerting` section:

```yaml
# docker/config/prometheus.yml (complete file)
global:
  scrape_interval: 10s
  evaluation_interval: 10s

rule_files:
  - /etc/prometheus/alerts.yml
  - /etc/prometheus/recording-rules.yml

scrape_configs:
  # ... (scrape targets)
```

There is no `alerting:` block, no `alertmanagers:` target, and no Alertmanager service in the Docker Compose file. The alerts are evaluated every 10 seconds (`evaluation_interval: 10s`) but the results are only stored internally by Prometheus.

**File:** `docker/config/prometheus.yml` -- no `alerting` section
**File:** `docker/config/alerts.yml` -- 31 alert rules with no delivery mechanism
**File:** `docker/compose/devnet.yaml` -- no Alertmanager service defined

## Code Reference

The complete Prometheus configuration (no `alerting` section):

```yaml
# docker/config/prometheus.yml:1-35
global:
  scrape_interval: 10s
  evaluation_interval: 10s

rule_files:
  - /etc/prometheus/alerts.yml
  - /etc/prometheus/recording-rules.yml

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

  - job_name: 'kora-secondary'
    static_configs:
      - targets:
        - 'secondary-node0:9002'
    relabel_configs:
      - source_labels: [__address__]
        regex: 'secondary-node(\d+):.*'
        target_label: secondary_index
        replacement: '$1'
```

Example critical alert rules that fire silently:

```yaml
# docker/config/alerts.yml:4-12 (ValidatorDown)
      - alert: ValidatorDown
        expr: up{job="kora-validators"} == 0
        for: 30s
        labels:
          severity: critical
        annotations:
          summary: "Validator {{ $labels.instance }} is down"
          description: "Validator has been unreachable for 30 seconds. Check container health."

# docker/config/alerts.yml:222-234 (MempoolPoisoning)
      - alert: MempoolPoisoning
        expr: |
          sum(rate(engine_voter_state_current_view[2m])) > 0
          and sum(rate(finalized_height[2m])) < 0.001
          and sum(rate(engine_voter_state_nullifications_total[2m])) > 5
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Chain stalled with active nullifications -- likely mempool poisoning"
```

## Impact

1. **Critical alerts are silently discarded.** Alerts for validator crashes, consensus stalls, network partitions, and mempool poisoning fire into the void. These are exactly the scenarios where rapid operator response is essential.
2. **Operators have no automated notification.** The only way to notice a firing alert is to manually check the Prometheus web UI (`http://localhost:9090/alerts`) or query alert state in Grafana. This is impractical for 24/7 monitoring of a distributed system.
3. **Wasted engineering effort.** The 31 carefully-crafted alert rules (including tuned thresholds based on production observations, such as the nullification rate threshold of 60/s and the skip rate threshold of 45%) provide no operational value until a notification pipeline is configured.
4. **False sense of security.** The presence of alert rules in the codebase may give operators the impression that alerting is active, when in fact no notifications are being sent.

## Root Cause

The alert rules were written as part of the monitoring stack setup, but the notification pipeline (Alertmanager or equivalent webhook) was not deployed. The `prometheus.yml` configuration loads the alert rules via `rule_files` but has no `alerting` section to route fired alerts to a notification channel. No Alertmanager container is defined in the Docker Compose file.

## Suggested Fix

**Option A: Deploy Alertmanager** (recommended for production-grade alerting):

1. Add an Alertmanager service to the Docker Compose file:

   ```yaml
   # docker/compose/devnet.yaml (add to services section)
   alertmanager:
     image: prom/alertmanager:latest
     profiles: ["observability"]
     restart: unless-stopped
     volumes:
       - ../config/alertmanager.yml:/etc/alertmanager/alertmanager.yml:ro
     command:
       - '--config.file=/etc/alertmanager/alertmanager.yml'
     ports:
       - "127.0.0.1:9093:9093"
     networks:
       - kora-net
   ```

2. Create `docker/config/alertmanager.yml` with a notification channel:

   ```yaml
   route:
     receiver: 'default'
     group_wait: 30s
     group_interval: 5m
     repeat_interval: 4h

   receivers:
     - name: 'default'
       webhook_configs:
         - url: 'https://hooks.slack.com/services/YOUR/SLACK/WEBHOOK'
   ```

3. Add the `alerting` section to `docker/config/prometheus.yml`:

   ```yaml
   # Add after rule_files section
   alerting:
     alertmanagers:
       - static_configs:
           - targets: ['alertmanager:9093']
   ```

**Option B: Use Grafana alerting** (simpler, no additional container):

Configure Grafana to evaluate Prometheus alert rules and send notifications via its built-in contact points (email, Slack, webhook, PagerDuty). This avoids deploying Alertmanager but couples alerting to Grafana's availability.

## Files to Modify

- `docker/config/prometheus.yml` -- add `alerting` section with Alertmanager target
- `docker/compose/devnet.yaml` -- add Alertmanager service definition
- `docker/config/alertmanager.yml` -- new file: Alertmanager configuration with notification channel

## Related Issues

- [097 - Observability stack not enabled on live devnet](/Users/will/dev/nunchi/daeji/tmp/kora/issues/097-docker-observability-not-enabled.md) -- the observability stack (including alert rules) requires the `observability` profile to be active; on the live devnet, this profile may not be enabled
- [182 - Grafana admin password hardcoded](/Users/will/dev/nunchi/daeji/tmp/kora/issues/182-grafana-admin-password-hardcoded.md) -- if Grafana alerting is used instead of Alertmanager, the Grafana security issues become more impactful

## Labels

`bug`, `docker`, `metrics`, `reliability`
