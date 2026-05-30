# Findings: Kora Devnet Issues

Snapshot taken ~20 minutes after devnet boot, idle (no load generator running).

## Validator Identity Mapping

The `leader` label in metrics uses the ed25519 public key. Here is the mapping:

| Validator Index | Hostname | Public Key (truncated) | Bootstrap Address |
|----------------|----------|----------------------|-------------------|
| 0 | node0 | `36249a90f910dff0...` | node0:30303 |
| 1 | node1 | `d0e2a51fb6836ce4...` | node1:30303 |
| 2 | node2 | `75645516dbea77db...` | node2:30303 |
| 3 | node3 | `d18314631c60e4d2...` | node3:30303 |

Secondary peer: `03415a85a1aad15f...`

---

## Issue 1: ~27% Skip Rate (Wasted Consensus Views)

**Severity: High**

The devnet is wasting approximately 27% of all consensus views. For every 100 views
advanced, only ~73 produce a finalized block. The remaining ~27 views are nullified
(leader failed to produce, timed out, or was inactive).

**Data:**
```
finalized_height: ~71,000
current_view:     ~97,000
skip_rate:        ~0.268 (consistent across all 4 nodes)
```

This means the effective throughput is ~59 blocks/sec instead of a theoretical ~80+.

**Prometheus query to monitor:**
```promql
1 - (rate(finalized_height[5m]) / rate(engine_voter_state_current_view[5m]))
```

---

## Issue 2: Asymmetric Nullifications — Nodes 2 & 3 Are Worse Leaders

**Severity: Medium**

Nullifications are NOT evenly distributed. When nodes 2 and 3 are leaders, they
cause ~65% more nullifications than when nodes 0 and 1 lead:

```
Leader node0 (36249a90...): 19,590 nullifications
Leader node1 (d0e2a51f...): 19,337 nullifications
Leader node2 (75645516...): 32,332 nullifications  <- 65% higher
Leader node3 (d1831463...): 32,460 nullifications  <- 66% higher
```

**Prometheus query:**
```promql
sum by (leader)(engine_voter_state_nullifications_total)
```

This is NOT a "some nodes are slower" issue — the skip rate is uniform across all
reporters. This means when node2 or node3 is elected leader, ALL nodes skip that view.

---

## Issue 3: Broadcast Failures Concentrated on Nodes 2 & 3

**Severity: Medium — correlates with Issue 2**

Broadcast get failures are dramatically asymmetric:

```
node0: 2,501 failures,     0 drops,  396,968 successes  (0.6% failure rate)
node1: 2,492 failures,     0 drops,  369,835 successes  (0.7% failure rate)
node2: 53,033 failures, 12,280 drops, 306,029 successes  (17.3% failure rate)
node3: 53,324 failures, 12,328 drops, 305,053 successes  (17.6% failure rate)
```

Nodes 2 & 3 have a **25x higher broadcast failure rate** and are the only nodes
with `Dropped` broadcasts. This directly explains why they are worse leaders — when
they propose a block, the broadcast mechanism fails to deliver it ~17% of the time.

**Prometheus queries:**
```promql
broadcast_get_total{status="Failure"}
broadcast_get_total{status="Dropped"}
broadcast_get_total{status="Success"}
```

**Possible causes:**
- Network contention within Docker networking (node2/node3 start later, may get
  slower network paths)
- Resource contention — all 4 validators run on the same machine
- The Docker compose startup order: node0 starts first, then nodes 1-3 start after
  node0 is healthy. Nodes 2 and 3 may have suboptimal peer connections

---

## Issue 4: Timeout Breakdown Reveals Root Causes

**Severity: Informational**

The timeout reasons tell a story:

```
LeaderNullify:   61,485  (45%) — Leader proposed but then nullified (changed mind)
Inactivity:      48,676  (36%) — Peer marked as inactive (not participating)
MissingProposal: 25,964  (19%) — Leader was elected but never proposed
LeaderTimeout:        3  (<1%) — Leader proposed too slowly (rare, good)
```

**Inactivity timeouts are concentrated on leaders node2 and node3:**
```
Inactivity from leader node2 (75645516...): 24,329
Inactivity from leader node3 (d1831463...): 24,347
Inactivity from leader node0: 0
Inactivity from leader node1: 0
```

This confirms that node2 and node3 periodically appear "inactive" to other validators.

**MissingProposal is also skewed toward nodes 2 & 3:**
```
MissingProposal for node0: 4,888
MissingProposal for node1: 4,810
MissingProposal for node2: 8,125   <- 67% higher
MissingProposal for node3: 8,141   <- 69% higher
```

---

## Issue 5: Resolver Cancellations Are Very High

**Severity: Medium**

All nodes have extremely high resolver cancellation counts:

```
node0: 167,601 dropped resolver fetches
node1: 167,033 dropped resolver fetches
node2: 162,624 dropped resolver fetches
node3: 162,088 dropped resolver fetches
```

These are `engine_resolver_resolver_cancel_total{status="Dropped"}`. The resolver
tries to fetch missed consensus data from peers, but these fetches are being dropped.
Combined with zero successful fetches (`fetch_duration_count = 0`), the resolver
is essentially non-functional.

**Prometheus queries:**
```promql
engine_resolver_resolver_cancel_total
engine_resolver_resolver_fetch_duration_count  # should be > 0 but is 0
```

---

## Issue 6: Height Drift Between Nodes

**Severity: Low-Medium**

At any given time, nodes differ in finalized height by 500-700 blocks:

```
node2: 71,332  (highest)
node3: 71,173
node0: 70,868
node1: 70,630  (lowest)
max drift: 702 blocks
```

While all nodes eventually converge (same finalization rate over time), a persistent
drift of hundreds of blocks means the "view" of the chain is inconsistent across
nodes at any instant. This could cause issues for RPC consumers expecting consistent
reads.

**Prometheus query:**
```promql
max(finalized_height) - min(finalized_height)
```

---

## Issue 7: Memory Usage ~1 GiB Per Validator (Idle)

**Severity: Informational**

Each validator uses approximately 1 GiB RSS with no transaction load:

```
node0: 1,054 MiB
node1: 1,050 MiB
node2: 1,051 MiB
node3: 1,047 MiB
```

With 4 validators on one machine, that is ~4 GiB for the devnet. This should be
profiled to understand what is consuming memory (QMDB state, consensus caches,
network buffers, etc.).

**Prometheus query:**
```promql
runtime_process_rss
```

---

## Issue 8: Disk Write Volume Is Disproportionately High

**Severity: Informational**

After ~20 minutes of idle operation, total disk writes across all nodes:

```
Total written: ~78 GiB
Total read:    ~47 MiB
```

That is a write:read ratio of ~1700:1. At ~59 blocks/sec with empty blocks, the
write amplification is extremely high. This will be a bottleneck under real load
and on machines with slower storage.

**Prometheus query:**
```promql
rate(runtime_storage_write_bytes_total[1m])
rate(runtime_storage_read_bytes_total[1m])
```

---

## Summary Table

| # | Issue | Severity | Key Metric |
|---|-------|----------|------------|
| 1 | 27% skip rate | High | `1 - (rate(finalized_height) / rate(current_view))` |
| 2 | Asymmetric nullifications (node2/3) | Medium | `sum by (leader)(nullifications_total)` |
| 3 | Broadcast failures on node2/3 | Medium | `broadcast_get_total{status="Failure"}` |
| 4 | Inactivity timeouts on node2/3 | Medium | `timeouts_total{reason="Inactivity"}` |
| 5 | Resolver completely non-functional | Medium | `resolver_cancel_total` vs `fetch_duration_count` |
| 6 | 500-700 block height drift | Low-Med | `max(finalized_height) - min(finalized_height)` |
| 7 | ~1 GiB memory per validator (idle) | Info | `runtime_process_rss` |
| 8 | ~78 GiB disk writes in 20 min | Info | `runtime_storage_write_bytes_total` |
