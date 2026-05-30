# Kora Consolidated Issues Index

**Generated**: 2026-05-22 (updated with issues 23-26)
**Source**: 48 devnet test reports consolidated into 22 actionable issues + 4 audit gap issues
**Total content**: ~450 KB across 26 issue files

---

## Priority Classification

### P0 — Stability & Correctness (fix first)

| # | Issue | Impact | Effort |
|---|-------|--------|--------|
| 01 | [SIGTERM Graceful Shutdown](issue-01-sigterm-graceful-shutdown.md) | Every `docker stop` = SIGKILL after 30s, all state lost | 2-4 hours |
| 02 | [Consensus Timing Asymmetry](issue-02-consensus-timing-asymmetry.md) | 99.4% throughput drop on single node failure (714:1 penalty ratio) | Config change + 1 day |
| 03 | [State Sync & Crash Recovery](issue-03-state-sync-crash-recovery.md) | Restarted nodes can NEVER rejoin the chain | 1-3 weeks (phased) |
| 04 | [DKG Init-Config Race](issue-04-dkg-init-config-race.md) | `docker compose stop/start` = permanent consensus deadlock | 4-8 hours |
| 09 | [Snapshot Store Race Condition](issue-09-snapshot-store-race-condition.md) | TOCTOU race causes "missing parent snapshot" → state loss | 2-4 hours |
| 10 | [Consensus-Execution Backpressure](issue-10-consensus-execution-backpressure.md) | Unbounded memory growth from consensus outpacing execution | 1-2 days |
| 20 | [Finalization Error Handling](issue-20-finalization-error-handling.md) | 5 failure paths return `Err(())` with no retry → silent state loss | 1-2 days |
| 23 | [KORA_RUNTIME_DIR Persistence](issue-23-runtime-dir-persistence.md) | Runtime dir on tmpfs -- ALL consensus state lost on container restart (root cause of Issues 03, 18) | 30 minutes |

### P1 — Operational Reliability

| # | Issue | Impact | Effort |
|---|-------|--------|--------|
| 05 | [P2P Channel Backpressure](issue-05-p2p-channel-backpressure.md) | 18.5% message drop rate degrades consensus | 2-4 hours |
| 07 | [RPC Spec Compliance](issue-07-rpc-spec-compliance.md) | Truncated logsBloom breaks Foundry/alloy; gasUsed always 0 | 4-8 hours |
| 08 | [Observability Stack Fixes](issue-08-observability-stack-fixes.md) | 6 sub-issues: profile never activated, wrong metric names, bad alerts | 4-8 hours |
| 11 | [Docker Container Hardening](issue-11-docker-container-hardening.md) | No init process, no zombie reaping, override.yml never applied | 2-4 hours |
| 12 | [Ansible Deployment Fixes](issue-12-ansible-deployment-fixes.md) | Wrong metric names in playbooks, barrier files persist, override skipped | 2-4 hours |
| 14 | [Error Handling & Logging](issue-14-error-handling-logging.md) | 18 findings: silent errors, wrong log levels, missing structured fields | 2-3 days |
| 15 | [Network Partition Detection](issue-15-network-partition-detection.md) | No partition detection, health endpoint always returns 200 OK | 2-3 days |
| 16 | [Block Timestamps & Gas Accounting](issue-16-block-timestamps-gas-accounting.md) | Synthetic timestamps drift 107x, gas never charged | 1-2 days |
| 18 | [Entrypoint Bootstrap Wait](issue-18-entrypoint-bootstrap-wait.md) | Unconditional bootstrap wait on every restart → crash loops | 30 minutes |
| 24 | [Duplicate Transaction Risk](issue-24-duplicate-transaction-risk.md) | Excluded-tx set stops at persisted snapshot -- finalized txs can be re-proposed | 2-4 hours |
| 25 | [Network Security Hardening](issue-25-network-security-hardening.md) | All ports (RPC, metrics, Grafana) publicly exposed on 0.0.0.0 | 1-2 hours |
| 26 | [Mempool Finalization Pruning](issue-26-mempool-finalization-pruning.md) | Pruning skips txs not in local pool -- sender nonces may be stale after finalization | 2-4 hours |

### P2 — Features & Enhancements

| # | Issue | Impact | Effort |
|---|-------|--------|--------|
| 06 | [Global Mempool & Tx Gossip](issue-06-global-mempool-tx-gossip.md) | Transactions are validator-local, lost on crash, no gossip | 1-2 weeks (phased) |
| 13 | [Secondary Node Implementation](issue-13-secondary-node-implementation.md) | CLI exists but is a stub (`pending().await` forever) | 3-5 days |
| 17 | [Loadgen Resilience](issue-17-loadgen-resilience.md) | No progress reporting, no nonce recovery, no tx confirmation | 1-2 days |
| 19 | [Application-Level Metrics](issue-19-application-level-metrics.md) | Zero Kora metrics exposed; all are Commonware framework metrics | 2-3 days |
| 21 | [Genesis Hash Determinism](issue-21-genesis-hash-determinism.md) | Genesis hash changes every deployment (wall-clock timestamp) | 1-2 hours |
| 22 | [eth_subscribe Error Code](issue-22-eth-subscribe-error-code.md) | Returns -32603 (internal) instead of -32601 (not found) over HTTP | 1-2 hours |

---

## Suggested Implementation Order

### Week 1: Quick Wins + Stability Foundation
1. **Issue 23** — Remove tmpfs for runtime dir, use persistent volume (30 min, root cause fix)
2. **Issue 18** — Conditional bootstrap wait (30 min, shell only)
3. **Issue 01** — SIGTERM handler + `init: true` (2-4 hours)
4. **Issue 25** — Bind RPC/metrics ports to 127.0.0.1 (1-2 hours)
5. **Issue 02** — Reduce leader_timeout from 5s to 500ms (config change, 30 min)
6. **Issue 04** — DKG init-config idempotency (4-8 hours)
7. **Issue 09** — Snapshot store: move eviction inside mutex + increase capacity (2-4 hours)
8. **Issue 05** — Increase P2P channel buffers (2-4 hours)
9. **Issue 11** — Docker hardening (init: true, restart policy, security) (2-4 hours)
10. **Issue 21** — Fix genesis timestamp to 0 (1-2 hours)
11. **Issue 22** — Fix eth_subscribe error code (1-2 hours)

### Week 2: Core Reliability
12. **Issue 10** — Consensus-execution backpressure (1-2 days)
13. **Issue 20** — Finalization retry logic + typed errors (1-2 days)
14. **Issue 24** — Fix excluded-tx set to include persisted snapshot (2-4 hours)
15. **Issue 26** — Mempool pruning nonce advancement for unknown txs (2-4 hours)
16. **Issue 07** — RPC spec compliance: logsBloom, gasUsed (4-8 hours)
17. **Issue 08** — Observability stack fixes (4-8 hours)
18. **Issue 12** — Ansible deployment fixes (2-4 hours)
19. **Issue 16** — Block timestamps + gas accounting (1-2 days)

### Week 3: Operational Excellence
20. **Issue 14** — Error handling & logging improvements (2-3 days)
21. **Issue 15** — Network partition detection (2-3 days)
22. **Issue 19** — Application-level Prometheus metrics (2-3 days)
23. **Issue 17** — Loadgen resilience (1-2 days)

### Week 4+: Features
24. **Issue 03** — State sync protocol (phased, 1-3 weeks)
25. **Issue 13** — Secondary node implementation (3-5 days)
26. **Issue 06** — Global mempool & tx gossip (1-2 weeks)

---

## Report Coverage Matrix

All 48 test reports in `/tmp/*.md` are covered by one or more issues:

| Report | Covered By Issues |
|--------|-------------------|
| alert-tuning-issues.md | 08 |
| ansible-and-infrastructure.md | 12 |
| ansible-audit.md | 12 |
| bootstrap-node-failure-test.md | 05, 18 |
| code-level-issues.md | 14 |
| commonware-integration.md | 02, 05 |
| consensus-analysis.md | 02, 16 |
| contract-deployment-test.md | 07, 16 |
| controlled-single-node-failure.md | 03, 15 |
| devnet-test-summary-2026-05-22.md | All (master summary) |
| diagnostic-queries.md | 08 |
| dkg-key-management.md | 04 |
| docker-container-lifecycle.md | 01, 04, 11 |
| docker-infrastructure-audit.md | 11 |
| error-handling-analysis.md | 14 |
| evm-state-consistency-test.md | 03, 16 |
| fresh-deploy-log-analysis.md | 14 |
| full-cluster-restart-test.md | 01, 04, 11 |
| graceful-shutdown-test.md | 01 |
| idle-nullification-analysis.md | 02 |
| load-test-2026-05-22.md | 06, 17 |
| load-test-during-failure.md | 06, 17 |
| log-analysis-comprehensive.md | 14 |
| long-running-stability-test.md | 09, 10 |
| memory-growth-analysis.md | 09, 10 |
| mempool-architecture-and-gaps.md | 06 |
| multi-pattern-load-test.md | 06, 17 |
| network-partition-test.md | 15 |
| node-failure-recovery-test.md | 03, 15 |
| observability-gaps-and-recommendations.md | 08, 19 |
| observability-infrastructure.md | 08 |
| observability-stack-test.md | 08 |
| p2p-networking-gaps.md | 05 |
| performance-baseline.md | 02 |
| prometheus-grafana-validation.md | 08 |
| prometheus-metrics-inventory.md | 08, 19 |
| recording-rules-bugs.md | 08 |
| resolver-catchup-failure.md | 03 |
| resource-exhaustion-timeline.md | 09, 10 |
| rolling-restart-cascading-failure.md | 03, 15, 18 |
| rpc-consistency-test.md | 07, 16 |
| rpc-server-issues.md | 07, 22 |
| secondary-node-analysis.md | 13 |
| snapshot-store-deep-dive.md | 09 |
| solutions-roadmap.md | All (cross-reference) |
| startup-and-restart-issues.md | 01, 04, 18 |
| storage-persistence-architecture.md | 03 |
| txpool-behavior-test.md | 06 |

---

## Cross-References to Existing PRs

Several issues overlap with PRs already created from earlier analysis:

| Issue | Related PRs | Status |
|-------|------------|--------|
| 09 | PR #125 (snapshot store bounded eviction) | Merged — issue covers remaining race |
| 14 | PR #129 (silent error → warn logging) | Merged — issue covers remaining gaps |
| 05 | PR #135 (P2P channel metrics) | Merged — issue covers buffer sizing |
| 03 | PR #131 (resolver peer blocking fix) | Merged — issue covers broader state sync |
| 07 | PR #121 (reject historical state queries) | Merged — issue covers other RPC gaps |
| 07 | PR #120 (block gas limit) | Merged — issue covers gas accounting |
| 07 | PR #123 (BLOCKHASH opcode) | Merged — issue covers other RPC gaps |
