# Load Test Results

## Overview

This document records all load test runs performed against the Kora devnet, including test parameters, results, interpretation, and operational guidance. The tests were conducted to determine the chain's throughput ceiling, identify the stall threshold, and establish safe operating parameters.

---

## Test Environment

| Component | Description |
|-----------|-------------|
| **Server** | Hetzner dedicated server (65.21.232.29), Arch Linux |
| **Hardware** | AMD/Intel multi-core CPU, 64-128 GB RAM, NVMe SSDs |
| **Network topology** | 4 validators + 1 secondary, all on same machine (Docker) |
| **Consensus** | Simplex BFT with BLS12-381 threshold VRF |
| **EVM** | REVM (Cancun spec), 250M gas limit per block |
| **Storage** | QMDB (persistent state), Commonware journals (tmpfs) |
| **Mempool** | Validator-local (no cross-validator transaction gossip) |
| **Observability** | Prometheus (10s scrape), Grafana, Loki, Promtail |
| **Load generator** | `bin/loadgen` (Rust, runs on the same server) |
| **Chain state** | Fresh deploy, clean state, genesis-funded loadgen accounts |
| **Chain ID** | 1337 |

### Important Architectural Notes

- Mempools are **validator-local**: a transaction submitted to validator A is NOT available to validator B unless explicitly broadcast to B's RPC endpoint.
- The loadgen's `--broadcast-rpc-urls` flag sends each transaction to multiple validators to work around this limitation.
- All validators share the same physical CPU and memory, so resource contention between validators is possible under heavy load.

---

## Test Results Summary

| Test | Total TXs | Accounts | Concurrency | RPC Mode | Success Rate | Acceptance TPS | Chain Outcome |
|------|-----------|----------|-------------|----------|--------------|----------------|---------------|
| Idle baseline | 0 | - | - | - | - | - | 0.735 blocks/sec, 53% efficiency |
| Light load | 5,000 | 20 | 50 | Single | 100% | ~2,933 | Healthy, no degradation |
| Medium load | 10,000 | 30 | 100 | Single | 100% | ~2,873 | Healthy, no degradation |
| Heavy load | 50,000 | 50 | 200 | Single | Starts OK, then stalls | - | Permanent chain stall |
| Heavy + broadcast | 50,000 | 50 | 200 | All validators | Stalls faster | - | Permanent chain stall (accelerated) |

---

## Detailed Test Results

### Test 0: Idle Baseline (No Transactions)

**Purpose**: Establish consensus performance without any transaction load.

**Duration**: 30+ seconds of observation after chain starts.

| Metric | Value |
|--------|-------|
| Block rate | 0.735 blocks/sec |
| Consensus efficiency | 53% |
| Nullification rate | 3.5/sec |
| Drift | 1 view |
| Block build time (p50) | ~5ms |
| Block build time (p95) | ~15ms |
| Block build time (p99) | ~30ms |
| Finalization latency (p50) | ~10ms |
| Finalization latency (p95) | ~25ms |
| Memory (RSS) per validator | ~200 MB |

**Observation**: Even with zero external transactions, 47% of consensus views are nullified. This represents inherent waste in the current consensus timing configuration. The chain is healthy but operating at roughly half its theoretical capacity.

---

### Test 1: Light Load

**Command**:
```bash
cargo run --release --bin loadgen -- \
  --total-txs 5000 \
  --accounts 20 \
  --concurrency 50 \
  --rpc-url http://127.0.0.1:8545
```

**Parameters**:
| Parameter | Value |
|-----------|-------|
| Total transactions | 5,000 |
| Sender accounts | 20 |
| Max concurrency | 50 |
| TXs per account | 250 |
| RPC mode | Single validator |
| Transaction type | EIP-1559 transfer (21,000 gas, value=1 wei) |

**Results**:
| Metric | Value |
|--------|-------|
| Transactions sent | 5,000 |
| Success | 5,000 (100%) |
| Failed | 0 |
| Elapsed time | ~1.70s |
| Acceptance TPS | ~2,933 |

**Chain performance during test**:
| Metric | Before Load | During Load | After Load |
|--------|-------------|-------------|------------|
| Block rate | 0.735/sec | ~0.735/sec | 0.735/sec |
| Efficiency | 53% | ~53% | 53% |
| Build time p95 | ~15ms | ~15ms | ~15ms |
| Finalization p95 | ~25ms | ~25ms | ~25ms |

**Interpretation**: The chain handled 5,000 transactions with zero failures and no measurable degradation. At 250 transactions per account with 20 accounts, no sender exceeded the `max_txs_per_sender` limit of 256. The concurrency of 50 kept nonce ordering intact (sequential sends per account). This represents a safe operating point.

---

### Test 2: Medium Load

**Command**:
```bash
cargo run --release --bin loadgen -- \
  --total-txs 10000 \
  --accounts 30 \
  --concurrency 100 \
  --rpc-url http://127.0.0.1:8545
```

**Parameters**:
| Parameter | Value |
|-----------|-------|
| Total transactions | 10,000 |
| Sender accounts | 30 |
| Max concurrency | 100 |
| TXs per account | ~333 |
| RPC mode | Single validator |
| Transaction type | EIP-1559 transfer (21,000 gas, value=1 wei) |

**Results**:
| Metric | Value |
|--------|-------|
| Transactions sent | 10,000 |
| Success | 10,000 (100%) |
| Failed | 0 |
| Elapsed time | ~3.48s |
| Acceptance TPS | ~2,873 |

**Chain performance during test**:
| Metric | Before Load | During Load | After Load |
|--------|-------------|-------------|------------|
| Block rate | 0.735/sec | ~0.735/sec | 0.735/sec |
| Efficiency | 53% | ~53% | 53% |
| Build time p95 | ~15ms | ~15ms | ~15ms |
| Finalization p95 | ~25ms | ~25ms | ~25ms |

**Interpretation**: Even doubling the transaction count and concurrency produced 100% success with no degradation. The key factors keeping this stable: each account sends ~333 transactions which exceeds `max_txs_per_sender` (256), but the loadgen's per-account sequential sending with retry backoff handles pool rejections gracefully. The chain continues to finalize blocks faster than the pool drains, preventing accumulation.

---

### Test 3: Heavy Load (Single RPC)

**Command**:
```bash
cargo run --release --bin loadgen -- \
  --total-txs 50000 \
  --accounts 50 \
  --concurrency 200 \
  --rpc-url http://127.0.0.1:8545
```

**Parameters**:
| Parameter | Value |
|-----------|-------|
| Total transactions | 50,000 |
| Sender accounts | 50 |
| Max concurrency | 200 |
| TXs per account | 1,000 |
| RPC mode | Single validator |
| Transaction type | EIP-1559 transfer (21,000 gas, value=1 wei) |

**Results**:
| Metric | Value |
|--------|-------|
| Initial acceptance | Normal (~2,500+ TPS) |
| Eventual outcome | Chain stalls permanently |
| Failure mode | Mempool poisoning leading to executor abort loop |

**Chain performance timeline**:
| Phase | Duration | Block Rate | Efficiency |
|-------|----------|------------|------------|
| Normal operation | First 10-20s | ~0.735/sec | ~53% |
| Degradation onset | Next 5-10s | Dropping | Dropping |
| Full stall | Permanent | 0 blocks/sec | 0% |

**Interpretation**: The chain initially accepts transactions at normal rates, but eventually enters a permanent stall state. The failure sequence:
1. With 200 concurrency across 50 accounts, multiple transactions per account are in-flight simultaneously
2. Although loadgen sends sequentially per account, the high volume overwhelms the chain's ability to drain the mempool
3. Stale transactions accumulate (transactions with nonces that have already been executed on-chain)
4. The executor encounters a stale transaction during block building
5. The executor aborts with an error (uses `?` operator, which propagates the error up)
6. The aborted block proposal results in a nullification
7. The stale transaction is never pruned from the mempool
8. Every subsequent block proposal hits the same stale transaction and aborts
9. Chain is permanently stalled

---

### Test 4: Heavy Load with Broadcast to All Validators

**Command**:
```bash
cargo run --release --bin loadgen -- \
  --total-txs 50000 \
  --accounts 50 \
  --concurrency 200 \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

**Parameters**:
| Parameter | Value |
|-----------|-------|
| Total transactions | 50,000 |
| Sender accounts | 50 |
| Max concurrency | 200 |
| TXs per account | 1,000 |
| RPC mode | Broadcast to all 4 validators |
| Transaction type | EIP-1559 transfer (21,000 gas, value=1 wei) |

**Results**:
| Metric | Value |
|--------|-------|
| Outcome | Stalls faster than single-RPC test |
| Time to stall | Shorter than Test 3 |

**Interpretation**: Broadcasting to all validators accelerates the stall because:
1. Each validator receives the same transactions in its local mempool
2. Only the current leader builds blocks; non-leaders accumulate stale copies
3. When leadership rotates, the new leader has a mempool full of already-executed transactions
4. This creates more opportunities for the executor to hit stale transactions and abort
5. The stall condition is reached earlier because stale transaction density is higher in each validator's mempool

---

## The Stall Threshold

### When Does the Chain Die?

The chain enters a permanent stall when ALL of these conditions are met simultaneously:

1. **A stale transaction exists in the mempool** (nonce already executed on-chain)
2. **The executor aborts on error** (uses `?` instead of skipping bad transactions)
3. **No pruning removes the stale transaction** (pruning is not implemented for this case)

### What Triggers It?

| Factor | Safe | Dangerous |
|--------|------|-----------|
| TXs per account | < 256 | > 256 (exceeds pool limit, causes retries and timing gaps) |
| Concurrency | < 100 | > 150 (increases chance of nonce disorder at mempool level) |
| Load duration | Short bursts | Sustained (accumulates stale transactions over time) |
| Broadcast mode | Single RPC | All validators (multiplies stale copies) |
| Total volume | < 10k | > 20k sustained (overwhelms drain rate) |

### The Precise Failure Point

There is no single threshold. The stall is probabilistic and depends on the interaction between:
- Transaction submission rate vs block finalization rate
- Mempool drain speed vs stale transaction accumulation
- Which validator is the leader when stale transactions reach critical mass

**Empirical observation**: 50,000 transactions at concurrency 200 reliably stalls the chain within 20-30 seconds of sustained load. 10,000 transactions at concurrency 100 does not stall even with sustained submission.

---

## Safe Operating Parameters

Based on test results, the following parameters keep the chain healthy:

| Parameter | Safe Value | Rationale |
|-----------|-----------|-----------|
| `--total-txs` | Up to 10,000 | Proven to work at 100% success |
| `--accounts` | 20-30 | Distributes nonces across enough senders |
| `--concurrency` | 50-100 | Keeps per-account in-flight manageable |
| `--broadcast-rpc-urls` | None (single RPC) | Avoids stale copy multiplication |
| TXs per account | < 256 | Stays within `max_txs_per_sender` limit |
| Submission rate | < 3,000 TPS | Below the chain's acceptance ceiling |

### Conservative Recommendation

For routine testing where chain health must be preserved:

```bash
cargo run --release --bin loadgen -- \
  --total-txs 5000 \
  --accounts 20 \
  --concurrency 50 \
  --rpc-url http://127.0.0.1:8545
```

This is the largest proven-safe configuration that achieves 100% success with zero chain degradation.

---

## How to Run Load Tests Safely

### Before Testing

1. **Deploy a fresh devnet** (clean state eliminates variables from prior runs):
   ```bash
   just trusted-devnet
   ```

2. **Verify chain is healthy** (wait 30 seconds, check blocks are advancing):
   ```bash
   curl -s http://127.0.0.1:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
   ```

3. **Check Prometheus is scraping** (metrics must be available for post-test analysis):
   ```bash
   curl -s http://localhost:9090/api/v1/targets | jq '.data.activeTargets | length'
   ```

### During Testing

Monitor these in real-time (Grafana or Prometheus queries):

```bash
# Watch block rate (should stay > 0.5/sec)
watch -n 5 'curl -s "http://localhost:9090/api/v1/query?query=rate(kora_finalized_blocks_total[30s])" | jq ".data.result[0].value[1]"'

# Watch nullification rate (spike > 10/sec = warning)
watch -n 5 'curl -s "http://localhost:9090/api/v1/query?query=rate(kora_nullifications_total[30s])" | jq ".data.result[0].value[1]"'
```

### After Testing

1. **Verify chain is still alive** (blocks still advancing):
   ```bash
   # Query block number twice, 5 seconds apart — should increase
   curl -s http://127.0.0.1:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
   sleep 5
   curl -s http://127.0.0.1:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
   ```

2. **Check all transactions were executed** (verify nonce counts):
   ```bash
   # For each loadgen account, check final nonce matches expected
   curl -s http://127.0.0.1:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"eth_getTransactionCount","params":["<address>","latest"],"id":1}'
   ```

3. **If chain stalled**: the only recovery is redeploying:
   ```bash
   just trusted-devnet
   ```

---

## What to Monitor During Load Tests

### Primary Health Indicators

| Metric | Normal | Warning | Critical |
|--------|--------|---------|----------|
| `rate(kora_finalized_blocks_total[30s])` | > 0.5 | < 0.3 | 0 |
| `rate(kora_nullifications_total[30s])` | < 5 | 5-20 | > 50 |
| Consensus efficiency (blocks/views) | > 40% | 20-40% | < 10% |
| Block build time p95 | < 50ms | 50-200ms | > 500ms |
| Memory RSS | < 500 MB | 500 MB - 1 GB | > 1 GB |

### Loadgen Output Indicators

| Loadgen Output | Meaning |
|----------------|---------|
| `success = N, failed = 0` | All transactions accepted by RPC |
| `failed > 0` with retries | Pool full or nonce conflicts (may be benign) |
| `tx failed after retries` | 10 consecutive failures — likely chain stalling |
| TPS dropping over time | Chain is backing up, approaching stall |

### Stall Detection

The chain stall manifests as:
1. Loadgen stops making progress (no new successes, all retries failing)
2. `eth_blockNumber` returns the same value indefinitely
3. Prometheus shows `rate(kora_finalized_blocks_total[1m]) == 0`
4. Nullification rate spikes then drops to zero (all actors have halted)

---

## The Loadgen Tool

### What It Does

The loadgen (`bin/loadgen`) is a Rust-based load generator that creates and submits signed EIP-1559 transactions at high throughput. It sends simple ETH transfers (21,000 gas each, value = 1 wei) to a fixed receiver address from multiple sender accounts.

### Building

```bash
cd /path/to/kora
cargo build --release --bin loadgen
```

The binary is produced at `target/release/loadgen`.

### All Parameters

| Flag | Default | Description |
|------|---------|-------------|
| `--rpc-url` | `http://127.0.0.1:8545` | Primary RPC endpoint URL |
| `--broadcast-rpc-urls` | (none) | Comma-separated additional RPC endpoints to broadcast each transaction to |
| `--accounts` | `10` | Number of sender accounts to generate |
| `--total-txs` | `1000` | Total number of transactions to send |
| `--concurrency` | `50` | Maximum concurrent in-flight HTTP requests (global semaphore) |
| `--chain-id` | `1337` | Chain ID for EIP-155 signature |
| `--dry-run` | `false` | Sign transactions without sending (benchmarks signing speed) |
| `--verbose` | `false` | Print each transaction hash as it is sent |

### How It Works Internally

1. **Account generation**: Creates N accounts from deterministic seeds (`[0,0,...,0,1]` through `[0,0,...,0,N]`). These addresses must be pre-funded in the genesis configuration.

2. **Nonce initialization**: Queries `eth_getTransactionCount` for each account to get current on-chain nonce, then tracks nonces locally with atomic counters.

3. **Transaction distribution**: Divides `total_txs` evenly across accounts. Remainder transactions are assigned to the first accounts.

4. **Per-account sequential sending**: Each account runs in its own tokio task and sends transactions sequentially (nonce N must complete before nonce N+1 is sent). This preserves nonce ordering per account.

5. **Global concurrency control**: A tokio `Semaphore` with `--concurrency` permits bounds the total number of in-flight HTTP requests across all accounts.

6. **Retry with backoff**: If an RPC endpoint rejects a transaction, the loadgen retries up to 10 times with linear backoff (100ms, 200ms, 300ms, ... up to 1000ms). After 10 failures, the transaction is marked as failed and skipped.

7. **Validator pinning**: Each account is pinned to one validator (by index modulo number of clients). This avoids submitting the same nonce to multiple validators' local mempools.

8. **Fallback on rejection**: If the pinned validator rejects a transaction, the loadgen falls back to trying other validators.

### Transaction Details

| Field | Value |
|-------|-------|
| Type | EIP-1559 (type 2) |
| To | `0xBBBB...BBBB` (fixed receiver) |
| Value | 1 wei |
| Gas limit | 21,000 |
| Max fee per gas | 0 |
| Max priority fee per gas | 0 |
| Input data | Empty |
| Signature | ECDSA (secp256k1) |

### Example Commands

```bash
# Quick smoke test (default parameters)
cargo run --release --bin loadgen

# Light load test (proven safe)
cargo run --release --bin loadgen -- \
  --total-txs 5000 --accounts 20 --concurrency 50

# Medium load test
cargo run --release --bin loadgen -- \
  --total-txs 10000 --accounts 30 --concurrency 100

# Stress test (WARNING: may stall the chain)
cargo run --release --bin loadgen -- \
  --total-txs 50000 --accounts 50 --concurrency 200

# Broadcast to all validators (WARNING: stalls faster)
cargo run --release --bin loadgen -- \
  --total-txs 50000 --accounts 50 --concurrency 200 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548

# Dry run to benchmark signing throughput
cargo run --release --bin loadgen -- --total-txs 100000 --dry-run

# Verbose output for debugging
cargo run --release --bin loadgen -- \
  --total-txs 100 --accounts 5 --verbose
```

---

## Known Issues Affecting Test Results

### 1. Executor Abort on Invalid Transaction (Critical)

**Impact**: Causes permanent chain stall under sustained load.

**Description**: When the executor encounters a transaction it cannot execute (e.g., nonce already used, insufficient balance), it returns an error via the `?` operator. This propagates up and aborts the entire block proposal. The block is never produced, the view is nullified, and the offending transaction remains in the mempool. The next proposal hits the same transaction and aborts again, creating an infinite loop.

**Status**: Known bug. The executor should skip invalid transactions and continue building the block with remaining valid transactions.

### 2. No Mempool Pruning After Finalization (Critical)

**Impact**: Stale transactions accumulate and eventually poison the mempool.

**Description**: When a block is finalized and transactions are confirmed on-chain, the mempool does not remove transactions with nonces that are now below the on-chain nonce. These stale transactions remain in the `BTreeMap` and are returned by `mempool.build()` on subsequent calls.

**Status**: Known bug. The mempool should call `remove_confirmed(sender, confirmed_nonce)` after each finalized block to prune executed transactions.

### 3. Validator-Local Mempools (Design Limitation)

**Impact**: Transactions submitted to non-leader validators are not available for block building until leadership rotates.

**Description**: There is no transaction gossip between validators. Each validator's mempool is completely independent. The loadgen must either target the current leader or broadcast to all validators.

**Status**: By design for the current devnet phase. Transaction gossip is planned for a future release.

### 4. No Backpressure from Mempool to RPC

**Impact**: RPC continues accepting transactions even when the mempool is at capacity, leading to silent drops.

**Description**: The RPC layer accepts transactions and returns a hash immediately. If the mempool is full, the transaction may be rejected internally but the RPC response is already sent.

### 5. Pool Size Limits Create Artificial Throughput Cap

**Impact**: With few accounts, `max_txs_per_sender = 256` limits total pending transactions.

**Description**: With 10 accounts, maximum pending transactions = 10 x 256 = 2,560. With 50 accounts = 12,800 (exceeds `max_pending_txs` of 4,096, so global limit applies). The loadgen's retry backoff handles rejections gracefully but reduces effective submission rate.

---

## Recommended Test Plan for Validating Fixes

When fixes for the executor abort and mempool pruning issues are deployed, run the following test plan to validate:

### Phase 1: Verify Fix Does Not Regress Baseline

```bash
# Deploy fresh devnet with fixes
just trusted-devnet

# Wait for stabilization
sleep 30

# Run light load (must still achieve 100% success)
cargo run --release --bin loadgen -- \
  --total-txs 5000 --accounts 20 --concurrency 50 \
  --rpc-url http://127.0.0.1:8545
```

**Expected**: 100% success, ~2,900+ TPS acceptance, chain healthy after.

### Phase 2: Verify Stall Is Fixed

```bash
# Deploy fresh devnet
just trusted-devnet
sleep 30

# Run the previously-stalling configuration
cargo run --release --bin loadgen -- \
  --total-txs 50000 --accounts 50 --concurrency 200 \
  --rpc-url http://127.0.0.1:8545
```

**Expected**: Chain does NOT stall. Some transactions may fail (pool full, nonce rejection) but the chain continues producing blocks. Block rate should remain > 0.5/sec throughout.

### Phase 3: Verify Broadcast Mode Is Fixed

```bash
# Deploy fresh devnet
just trusted-devnet
sleep 30

# Run with broadcast to all validators
cargo run --release --bin loadgen -- \
  --total-txs 50000 --accounts 50 --concurrency 200 \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

**Expected**: Chain does NOT stall. Stale transactions in non-leader mempools should be pruned after finalization without causing executor aborts.

### Phase 4: Sustained Load (Soak Test)

```bash
# Deploy fresh devnet
just trusted-devnet
sleep 30

# Run moderate load for extended period (multiple rounds)
for i in $(seq 1 10); do
  echo "Round $i"
  cargo run --release --bin loadgen -- \
    --total-txs 10000 --accounts 30 --concurrency 100 \
    --rpc-url http://127.0.0.1:8545
  sleep 10
done
```

**Expected**: Chain survives all 10 rounds (100,000 total transactions) without stalling. Memory should not grow unboundedly (pruning working). Block rate should remain stable across all rounds.

### Phase 5: Find New Throughput Ceiling

Once the stall bug is fixed, incrementally increase load to find the new performance ceiling:

```bash
# Increase concurrency gradually
for conc in 100 200 300 500; do
  echo "Testing concurrency=$conc"
  just trusted-devnet
  sleep 30
  cargo run --release --bin loadgen -- \
    --total-txs 50000 --accounts 50 --concurrency $conc \
    --rpc-url http://127.0.0.1:8545
  echo "Check chain health, then continue"
  sleep 10
done
```

**Goal**: Determine the maximum sustainable concurrency where the chain degrades gracefully (some rejections, slower block production) rather than stalling completely.

### Success Criteria

| Criterion | Requirement |
|-----------|-------------|
| Chain liveness | Block rate > 0 at all times during load |
| Graceful degradation | Performance may slow but never stops |
| Memory stability | RSS does not grow without bound |
| Recovery | Chain returns to idle performance within 60s of load ending |
| No executor panics | Zero `executor abort` or `fatal` messages in logs |
| Pruning working | Mempool size returns to 0 within 60s of load ending |
