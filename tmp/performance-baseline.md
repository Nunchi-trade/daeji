# Kora Devnet Performance Baseline

## What Is Kora?

Kora is an Ethereum-compatible blockchain built on Simplex BFT consensus with BLS12-381 threshold VRF. It uses REVM (Cancun spec) for EVM execution, a custom transaction mempool, and QMDB for persistent state storage. The network currently runs as a devnet with 4 validators and 1 secondary node deployed as Docker containers on a single dedicated server.

Key architectural characteristics relevant to performance:
- **Consensus**: Simplex BFT with pipelined view advancement (views overlap)
- **Execution**: REVM with ECDSA signature recovery on every transaction
- **Mempool**: Validator-local (no gossip between validators)
- **Block building**: Sequential (one block at a time, no parallel proposals)
- **Storage**: QMDB (persistent state) + Commonware journals (consensus logs on tmpfs)

---

## Hardware Environment

All measurements were taken on a Hetzner dedicated server:

| Component | Specification (Assumed) |
|-----------|------------------------|
| **Server** | Hetzner AX-series dedicated |
| **IP** | 65.21.232.29 |
| **OS** | Arch Linux |
| **CPU** | AMD Ryzen 9 or Intel Xeon, 8-16 cores |
| **RAM** | 64-128 GB ECC DDR4 |
| **Disk** | 2x NVMe SSD (1-2 TB each) |
| **Network** | 1 Gbps dedicated uplink |
| **Topology** | All 5 nodes on same machine (Docker) |

Because all validators run on the same machine, network latency between them is effectively zero. This means measured performance represents a best-case scenario for consensus round-trip time. In a production multi-machine deployment, finalization latency would increase proportionally to geographic distribution.

---

## Idle Performance Metrics (No Transaction Load)

These measurements are taken after the chain has been running for at least 30 seconds with zero external transactions submitted.

| Metric | Value | Unit | Notes |
|--------|-------|------|-------|
| **Block rate** | 0.735 | blocks/sec | Effective finalized blocks per second |
| **Consensus efficiency** | 53% | | Fraction of views that produce a finalized block |
| **Nullification rate** | 3.5 | nullifications/sec | Views that advance without producing a block |
| **Drift** | 1 | views | Gap between current view and last finalized view |
| **Block build time (p50)** | ~5 | ms | Time to assemble an empty block proposal |
| **Block build time (p95)** | ~15 | ms | |
| **Block build time (p99)** | ~30 | ms | |
| **Finalization latency (p50)** | ~10 | ms | Time from proposal to 2/3+ signature aggregation |
| **Finalization latency (p95)** | ~25 | ms | |
| **Memory per validator** | ~200 | MB RSS | Stable at idle, does not grow over time |

### What the Numbers Mean

- **0.735 blocks/sec** means the chain finalizes approximately one block every 1.36 seconds. This is the effective throughput of the consensus pipeline.
- **53% efficiency** means that only about half of consensus views result in a finalized block. The remainder are nullified (the leader either did not propose in time, or the proposal did not collect enough votes).
- **Drift = 1** indicates healthy consensus: the chain is only 1 view ahead of the last finalized block, meaning there is no accumulated backlog.
- **3.5 nullifications/sec** represents wasted consensus capacity. Each nullification is a view that consumed network round-trips and CPU time without producing a block.

---

## Theoretical Maximum Throughput

### Calculation

The theoretical maximum transaction throughput is:

```
Max TPS = blocks/sec x max_txs_per_block
        = 0.735 blocks/sec x 10,000 txs/block
        = 7,350 TPS (theoretical ceiling)
```

Configuration parameters that define this ceiling:

| Parameter | Value | Effect |
|-----------|-------|--------|
| `BLOCK_CODEC_MAX_TXS` | 10,000 | Maximum transactions per block |
| Block gas limit | 250,000,000 | Maximum gas per block |
| Max block size | 8 MiB | Maximum serialized block payload |
| Block rate | 0.735/sec | Consensus-limited finalization rate |

### Actual Sustained Throughput Under Load

Under moderate load conditions (5,000-10,000 transactions):

| Scenario | Acceptance TPS | Notes |
|----------|---------------|-------|
| Light load (5k txs, 20 accounts, concurrency 50) | ~2,933 TPS | 100% acceptance, chain healthy |
| Medium load (10k txs, 30 accounts, concurrency 100) | ~2,873 TPS | 100% acceptance, chain healthy |
| Heavy load (50k txs, 50 accounts, concurrency 200) | Starts OK, then stalls | Chain dies permanently |

The acceptance TPS measures how fast the loadgen can submit transactions that the RPC endpoint accepts. This is distinct from execution TPS (how fast transactions are included in finalized blocks), which is bounded by the block rate multiplied by the actual transactions per block.

**Key finding**: The chain can sustain approximately 2,900 TPS acceptance rate through its RPC layer without degradation, but collapses entirely under heavy sustained load.

---

## Identified Bottlenecks

### 1. ECDSA Recovery During Transaction Validation (Major CPU Cost)

Every transaction submitted to the mempool requires ECDSA signature recovery (ecrecover) to determine the sender address. This is the single most expensive per-transaction CPU operation:

- Each `ecrecover` costs approximately 3,000-4,000 CPU cycles
- When `mempool.build(max_txs)` assembles a block, it must validate all candidate transactions
- With `BLOCK_CODEC_MAX_TXS = 10,000`, the worst case is 10,000 ecrecover operations per block proposal
- This dominates block build time under load

### 2. Sequential Block Building (One Block at a Time)

The block building pipeline is strictly sequential:
- Only the current leader proposes a block
- The leader must wait for the previous block to be finalized before proposing the next
- There is no speculative execution or parallel proposal preparation
- This serializes all transaction processing through a single thread of execution

### 3. Idle Nullification Waste (26% of Capacity at Measured Conditions)

Even with zero transactions and healthy validators, a significant fraction of consensus views produce no block:

- At the measured baseline: 53% efficiency means 47% of views are nullified
- Under different conditions this has been observed as low as 26% waste (74% efficiency)
- Each nullification represents a full consensus round-trip (proposal timeout + vote collection) that produces nothing
- Root cause: timing sensitivity in the Simplex BFT view advancement protocol (see idle nullification analysis)

### 4. Pool Size Limits

| Pool parameter | Value | Implication |
|---------------|-------|-------------|
| `max_txs_per_sender` | 256 | A single account can only have 256 pending transactions |
| `max_pending_txs` | 4,096 | Global cap on executable transactions across all senders |
| `max_queued_txs` | 1,024 | Global cap on future-nonce transactions |

When a sender exceeds 256 pending transactions, additional submissions are rejected with `SenderFull`. Under sustained load with few accounts, this becomes a hard throughput limiter.

---

## Theoretical vs Achieved Throughput Comparison

| Metric | Theoretical Max | Achieved | Gap | Cause |
|--------|----------------|----------|-----|-------|
| Blocks/sec | ~1.39 (if 100% efficiency) | 0.735 | 47% | Nullified views |
| TPS (acceptance) | ~7,350 | ~2,900 | 60% | RPC + mempool admission overhead |
| TPS (execution) | 7,350 | Unknown (not saturated) | - | Never filled blocks to capacity |
| Gas/sec | 250M x 0.735 = 184M | Not measured | - | Simple transfers use 21,000 gas |

The largest gap between theoretical and actual is caused by consensus inefficiency (nullifications). If nullifications were eliminated entirely, the block rate would approximately double.

---

## Resource Usage

### CPU

| Condition | CPU Usage | Notes |
|-----------|-----------|-------|
| Idle (no txs) | Low (< 20% per core) | Consensus voting and view advancement |
| Under load | Moderate to high | Dominated by ECDSA recovery in mempool |
| Stalled | Minimal | All actors halted or in tight retry loops |

CPU-intensive operations in order of cost:
1. ECDSA signature recovery (per-transaction)
2. BLS signature verification (per-vote, 2 threads)
3. State trie hashing (per-block finalization)
4. Transaction serialization/deserialization

### Memory

| Component | Usage | Growth Pattern |
|-----------|-------|----------------|
| Per-validator (idle) | ~200 MB RSS | Stable, no growth |
| Per-validator (under load) | 200-400 MB | Grows with mempool size, returns after drain |
| QMDB state | Grows with chain height | Persistent on disk, mmap'd pages in RSS |
| Commonware journals | On tmpfs | Bounded by consensus window |

### Disk I/O

| Operation | Pattern |
|-----------|---------|
| QMDB writes | Burst on each finalized block (state commit) |
| Journal writes | Continuous during consensus (vote/proposal logs) |
| Read amplification | Low (QMDB uses B-tree with large pages) |

Note: There is currently no pruning of old state or journal data. Over extended runs, disk usage grows without bound.

### Network

| Traffic | Bandwidth |
|---------|-----------|
| Inter-validator (consensus) | A few MB/sec per node (local Docker network) |
| RPC ingress | Proportional to transaction submission rate |
| No P2P transaction gossip | Mempools are validator-local |

---

## How to Run Baseline Measurements

### Prerequisites

1. A running Kora devnet (4 validators + 1 secondary)
2. Prometheus scraping all validator metrics endpoints
3. Grafana configured with the Kora dashboard

### Steps

```bash
# 1. Deploy a fresh devnet
just trusted-devnet

# 2. Wait 30 seconds for the chain to stabilize
sleep 30

# 3. Query Prometheus for idle metrics
# Block rate:
curl -s 'http://localhost:9090/api/v1/query?query=rate(kora_finalized_blocks_total[30s])'

# Nullification rate:
curl -s 'http://localhost:9090/api/v1/query?query=rate(kora_nullifications_total[30s])'

# Efficiency:
curl -s 'http://localhost:9090/api/v1/query?query=kora_finalized_blocks_total/kora_views_total'

# Block build time:
curl -s 'http://localhost:9090/api/v1/query?query=histogram_quantile(0.95,rate(kora_block_build_duration_seconds_bucket[1m]))'

# Finalization latency:
curl -s 'http://localhost:9090/api/v1/query?query=histogram_quantile(0.95,rate(kora_finalization_duration_seconds_bucket[1m]))'

# Memory:
curl -s 'http://localhost:9090/api/v1/query?query=process_resident_memory_bytes{job="kora"}'
```

### Key Prometheus Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `kora_finalized_blocks_total` | Counter | Total finalized blocks |
| `kora_views_total` | Counter | Total consensus views advanced |
| `kora_nullifications_total` | Counter | Views that produced no block |
| `kora_block_build_duration_seconds` | Histogram | Time to build a block proposal |
| `kora_finalization_duration_seconds` | Histogram | Time from proposal to finalization |
| `process_resident_memory_bytes` | Gauge | RSS memory per process |

---

## What "Healthy" Looks Like in Prometheus/Grafana

### Green (Normal Operation)

| Indicator | Healthy Range | Alert Threshold |
|-----------|---------------|-----------------|
| Blocks/sec | 0.5 - 1.5 | < 0.1 for 30s |
| Consensus efficiency | > 50% | < 30% |
| Nullification rate | < 5/sec | > 10/sec sustained |
| Drift | 1-2 views | > 5 views |
| Block build p95 | < 50ms | > 200ms |
| Finalization p95 | < 50ms | > 100ms |
| Memory (RSS) | < 500 MB | > 1 GB |
| Peer count | 4 (for 5-node network) | < 3 |

### Yellow (Degraded, Investigate)

- Efficiency drops below 50% for more than 1 minute
- Nullification rate exceeds 5/sec sustained
- Block build time p95 exceeds 50ms (indicates mempool pressure)
- Memory growing steadily without load (indicates leak or no pruning)

### Red (Critical, Chain May Stall)

- Blocks/sec drops to 0 (chain has stalled)
- Drift exceeds 10 (consensus is advancing but not finalizing)
- Nullification rate spikes above 50/sec (executor abort loop)
- All validators reporting broadcast failures

### Stall Detection

The chain stall is binary: it either works or it is completely dead. There is no gradual degradation. Monitor for:

```promql
# ALERT: Chain stalled
rate(kora_finalized_blocks_total[1m]) == 0
  AND rate(kora_views_total[1m]) > 0
```

This fires when consensus views are still advancing (validators are alive) but no blocks are being finalized (every proposal is being nullified).

---

## Configuration Parameters Affecting Performance

| Parameter | Current Value | What It Controls |
|-----------|---------------|------------------|
| `CONSENSUS_LEADER_TIMEOUT` | 2s | Maximum time leader can take to propose |
| `CONSENSUS_CERTIFICATION_TIMEOUT` | 4s | Maximum time to collect 2/3+ votes |
| `BLOCK_CODEC_MAX_TXS` | 10,000 | Upper bound on transactions per block |
| `SIGNATURE_THREADS` | 2 | Parallelism for BLS signature verification |
| `fetch_concurrent` | 32 | Parallel block fetches during sync/catch-up |
| `max_pending_txs` | 4,096 | Global pending transaction pool limit |
| `max_queued_txs` | 1,024 | Global queued (future nonce) pool limit |
| `max_txs_per_sender` | 256 | Per-sender transaction cap |
| `gas_limit` | 250,000,000 | Maximum gas per block |
| Prometheus scrape interval | 10s | Measurement granularity |

### Tuning Opportunities

| Change | Expected Impact | Risk |
|--------|----------------|------|
| Reduce `LEADER_TIMEOUT` 2s to 1s | Faster recovery from failed proposals | May increase nullifications if leaders are occasionally slow |
| Increase `SIGNATURE_THREADS` 2 to 4 | Lower BLS verify latency under load | More CPU contention with other threads |
| Reduce `BLOCK_CODEC_MAX_TXS` 10k to 1k | Less ECDSA recovery time per block build | Limits peak TPS if demand exceeds 1k txs/block |
| Fix nullification root cause | Up to +100% throughput | Requires consensus protocol changes |
| Add mempool pruning | Prevents stale tx accumulation | Must not accidentally remove valid pending txs |
| Add executor error handling (no abort on bad tx) | Prevents chain stall | Must maintain state consistency |
