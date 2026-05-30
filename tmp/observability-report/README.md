# Kora Devnet Observability Report

This report documents the full observability stack, current metrics, known issues,
and recommendations for improving diagnostics and performance of the Kora devnet.

Generated: 2026-05-20

## Quick Reference

| Tool | URL / Command |
|------|---------------|
| Grafana Dashboard | http://localhost:3000/d/kora-overview |
| Prometheus | http://localhost:9090 |
| Health Report CLI | `just devnet-health` |
| Live Stats TUI | `just devnet-stats` |
| Node0 Metrics | http://localhost:9000/metrics |
| Node1 Metrics | http://localhost:9001/metrics |
| Node2 Metrics | http://localhost:9002/metrics |
| Node3 Metrics | http://localhost:9003/metrics |

## Files in This Report

- `README.md` — This file
- `findings.md` — Detailed analysis of issues found
- `metrics-catalog.md` — All available metrics with descriptions
- `recommendations.md` — Actionable improvements
