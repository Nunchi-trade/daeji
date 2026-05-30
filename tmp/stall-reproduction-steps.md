# How to Reproduce the Permanent Chain Stall

## What Is a "Chain Stall"?

A chain stall in Kora means that consensus views may advance (the BFT engine rotates leaders and attempts new rounds), but **no blocks are ever finalized**. The `finalizedCount` metric stops incrementing permanently. The chain is not merely slow -- it is dead. No amount of waiting will recover it because the root cause (a poison transaction in the mempool) is never cleared.

Specifically:
- The consensus engine continues running (views advance, leaders rotate)
- Every leader proposes a block containing the same bad transaction(s)
- Every proposal fails during execution (the executor aborts the entire block)
- Every view is "nullified" (wasted) because no valid block is produced
- The mempool is never pruned because pruning only runs after successful finalization
- The same bad transactions are re-proposed in the next view
- This loop continues forever

---

## Prerequisites

### For Docker Devnet (Local)

1. Docker and Docker Compose installed
2. The Kora repository cloned and built:
   ```bash
   cd docker
   just build
   ```
3. The `loadgen` binary built:
   ```bash
   cargo build --release -p loadgen
   ```
4. A running devnet (instructions below will start one)

### For Remote Server (Ansible-Deployed)

1. Remote server with Kora deployed via Ansible
2. SSH access to the server
3. Loadgen binary built on the server (`cargo build --release -p loadgen`)

---

## Step-by-Step Reproduction (Docker Devnet)

### Step 1: Start a Clean Devnet

```bash
cd docker
just reset          # Wipe any previous state
just trusted-devnet # Fast start with trusted dealer DKG
```

Wait for the script to print the "Devnet ready" status table showing all 4 validators as "healthy".

**Expected output:**
```
[3/3] Devnet ready

  | Node       | Status     | Port    |
  | node0      | healthy    | 30400   |
  | node1      | healthy    | 30401   |
  | node2      | healthy    | 30402   |
  | node3      | healthy    | 30403   |
  | secondary0 | healthy    | 30500   |
```

### Step 2: Verify the Chain Is Healthy

Check that blocks are being finalized:

```bash
curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq .result
```

**Expected output (after ~10 seconds of uptime):**
```json
{
  "validatorIndex": 0,
  "uptimeSecs": 15,
  "currentView": 42,
  "finalizedCount": 38,
  "nullifiedCount": 0,
  "proposedCount": 10,
  "isLeader": false
}
```

Key indicators of a healthy chain:
- `currentView` is increasing (consensus is progressing)
- `finalizedCount` is increasing and close to `currentView` (blocks are being finalized efficiently)
- `nullifiedCount` is 0 or very low (proposals are succeeding)

Wait 5 seconds and query again to confirm `finalizedCount` has increased.

Alternatively, run the live monitor:
```bash
just stats
```

Confirm the "Blocks/s" column shows a non-zero rate (typically 50-150 b/s on a healthy devnet).

### Step 3: Run a High-Concurrency Load Test

From the repository root:

```bash
cargo run --release -p loadgen -- \
  --total-txs 50000 \
  --accounts 50 \
  --concurrency 200 \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

**What this does:**
- Creates 50 sender accounts (deterministic keys derived from seed bytes 1..50)
- Sends 50,000 EIP-1559 transfer transactions total (1,000 per account)
- Limits in-flight HTTP requests to 200 concurrent
- Sends each transaction to one primary validator and broadcasts to all others
- Each account sends sequentially (nonce N finishes before N+1 starts for that account)
- But transactions from DIFFERENT accounts fly in parallel across all validators

**What to observe during the load test:**

While `loadgen` is running (~30-90 seconds), query node status periodically:
```bash
watch -n 2 'curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"kora_nodeStatus\",\"params\":[],\"id\":1}" | jq ".result | {currentView, finalizedCount, nullifiedCount}"'
```

You will see:
1. Initially: `finalizedCount` climbs rapidly, `nullifiedCount` stays 0
2. Midway: `nullifiedCount` begins increasing, `finalizedCount` slows
3. Eventually: `finalizedCount` stops completely, `nullifiedCount` climbs with every view

### Step 4: Observe the Stall

After the loadgen completes, wait 10 seconds, then check:

```bash
curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result | {currentView, finalizedCount, nullifiedCount}'
```

**Expected output (stalled):**
```json
{
  "currentView": 1847,
  "finalizedCount": 312,
  "nullifiedCount": 1535
}
```

Wait another 10 seconds and query again:
```json
{
  "currentView": 1893,
  "finalizedCount": 312,
  "nullifiedCount": 1581
}
```

Notice:
- `currentView` is still advancing (consensus engine is running)
- `finalizedCount` is FROZEN (no blocks are being finalized)
- `nullifiedCount` increases by roughly the same amount as `currentView` (every view is wasted)

**The chain is permanently stalled.**

### Step 5: Verify the Stall Is Permanent

To confirm this is not a temporary slowdown but a permanent deadlock:

```bash
# Record current finalized count
BEFORE=$(curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq -r '.result.finalizedCount')

# Wait 60 seconds
sleep 60

# Check again
AFTER=$(curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq -r '.result.finalizedCount')

echo "Before: $BEFORE, After: $AFTER"
```

If `BEFORE == AFTER`, the stall is confirmed permanent. The chain will never produce another block without intervention.

Check all 4 validators to confirm they all agree:
```bash
for port in 8545 8546 8547 8548; do
  echo -n "Node (port $port): "
  curl -s http://localhost:$port -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq -r '.result | "\(.finalizedCount) finalized, \(.nullifiedCount) nullified"'
done
```

All nodes should show the same frozen `finalizedCount` with climbing `nullifiedCount`.

---

## Alternative Reproduction (Lighter Method)

If you do not want to run the full 50k-transaction loadgen, you can trigger the stall with a smaller, more targeted approach. The key insight is that any transaction that causes the executor to return an error will poison the entire block proposal.

### Method: Submit Same Nonce Twice to Different Validators

```bash
# Create a signed transaction (nonce 0, from a funded test account)
# Submit it to validator 0
curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_sendRawTransaction","params":["0x<SIGNED_TX_NONCE_0>"],"id":1}'

# Wait for it to be finalized
sleep 2

# Submit ANOTHER transaction with the SAME nonce 0 to validator 1
# (This validator's mempool does not know nonce 0 was already finalized)
curl -s http://localhost:8546 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_sendRawTransaction","params":["0x<DIFFERENT_TX_SAME_NONCE_0>"],"id":1}'
```

The second transaction has a stale nonce. When the leader next proposes a block containing this transaction, the executor will fail with a nonce error, the entire block execution aborts, the view is nullified, and the stale transaction remains in the mempool for the next proposal.

This method is harder to execute manually because you need to construct raw signed transactions, but it demonstrates the minimal trigger condition.

---

## What to Observe During Reproduction

### Validator Logs

Stream logs during the test:
```bash
just logs
```

Key log patterns indicating the stall:

**Before stall (healthy):**
```
[runner] finalized block height=312 txs=48 elapsed=2.1ms
[runner] finalized block height=313 txs=50 elapsed=2.3ms
```

**During stall onset:**
```
[app] WARN build_block: execution failed parent=0xabc... height=313 txs=50 error=TxExecution("...")
```

**After stall (repeating forever):**
```
[app] WARN build_block: execution failed parent=0xabc... height=313 txs=50 error=TxExecution("...")
[app] WARN build_block: execution failed parent=0xabc... height=313 txs=50 error=TxExecution("...")
```

The same error repeats every view because:
1. The leader drains the mempool (gets the same bad tx)
2. Passes it to the executor
3. Executor hits the bad tx, returns `Err(ExecutionError::TxExecution(...))`
4. `build_block()` returns `None`
5. `propose()` returns `None`
6. View is nullified
7. Mempool is NOT pruned (only pruned on successful finalization)
8. Next leader gets the same bad tx from its mempool

### Prometheus Metrics (If Observability Stack Is Running)

Start devnet with observability:
```bash
just devnet  # (not devnet-minimal)
```

Then query:
```bash
# Blocks per second (should drop to 0)
curl -s "http://localhost:9090/api/v1/query?query=rate(finalized_height[1m])" | jq '.data.result[0].value[1]'

# Nullification rate (should spike)
curl -s "http://localhost:9090/api/v1/query?query=sum(rate(engine_voter_state_nullifications_total[1m]))" | jq '.data.result[0].value[1]'

# Height drift between nodes (should be 0 since all are stuck at same height)
curl -s "http://localhost:9090/api/v1/query?query=max(finalized_height)-min(finalized_height)" | jq '.data.result[0].value[1]'
```

### Grafana Dashboard

If Grafana is running (http://localhost:3000, admin/admin):
- The "Kora Overview" dashboard will show finalization rate dropping to zero
- The "Kora Stall Diagnostics" dashboard will highlight the nullification spike

---

## Expected Results Summary

| Metric | Before Load Test | During Load Test | After Stall |
|--------|-----------------|-----------------|-------------|
| blocks/sec | 50-150 | Declining | 0 |
| currentView | Advancing | Advancing | Still advancing |
| finalizedCount | Advancing | Slowing | Frozen |
| nullifiedCount | 0 | Rising | Rising every view |
| Alerts | None | HighNullificationRate | HighNullificationRate, HighTimeoutRate |

---

## Why This Happens: The Failure Cascade

### Root Cause

The executor (`crates/node/executor/src/revm.rs`, line 395) uses the `?` operator on transaction execution results:

```rust
let result_and_state =
    evm.replay().map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;
```

This means if ANY single transaction in the batch fails (e.g., nonce too low, invalid signature, insufficient balance), the ENTIRE block execution aborts. The executor does not skip the bad transaction and continue with the rest.

### The Full Cascade

1. **Loadgen submits 50k txs across 50 accounts with concurrency=200**: Many transactions arrive at different validators' mempools
2. **Transactions are broadcast to all validators**: `--broadcast-rpc-urls` ensures all 4 validators have copies of all submitted transactions in their local mempools
3. **Some transactions become stale**: When the chain finalizes a block containing transaction N from account A, any earlier-nonce transaction from account A that has not yet been included becomes invalid (NonceTooLow)
4. **Stale transactions persist in mempool**: The `InMemoryMempool` is a simple BTreeMap keyed by transaction hash. It performs no nonce validation on storage -- it accepts whatever the RPC layer gives it
5. **Leader proposes block containing stale tx**: `mempool.build()` drains transactions for the next block proposal, including the stale one
6. **Executor hits stale tx and aborts**: The `?` operator propagates the error, returning `Err(ExecutionError::TxExecution(...))` for the entire batch
7. **`build_block()` returns `None`**: The block cannot be built
8. **`propose()` returns `None`**: No block is proposed for this view
9. **View is nullified**: Consensus wastes this round
10. **Mempool is NOT pruned**: `prune_mempool()` only runs in the `FinalizedReporter` after a block is successfully finalized and persisted. Since no block was finalized, nothing is pruned
11. **Next leader gets the same stale tx**: The cycle repeats for every subsequent view, forever

### Why This Is the Same Bug That Would Stall Production

This is not merely a devnet testing artifact. The core mechanism is:

1. A transaction that was valid when submitted becomes invalid before it is included in a block
2. The executor treats this as a fatal error instead of skipping the transaction
3. The mempool has no mechanism to evict transactions that become invalid after submission

In production, this could be triggered by:
- Network partitions that cause nonce gaps
- Transaction reordering during high-throughput periods
- Any scenario where a transaction's preconditions change between submission and execution

The loadgen merely accelerates the conditions that expose this bug.

---

## Why Moderate Load Does NOT Stall

With lower parameters (e.g., `--total-txs 5000 --accounts 50 --concurrency 50`):

- Each account sends sequentially (the loadgen assigns nonces with `SeqCst` ordering and sends one at a time per account)
- With lower total volume, transactions are finalized before they can become stale
- The chain processes transactions faster than new ones arrive
- No stale nonces accumulate in the mempool

The critical factors for reproduction:
- **High total transactions** (>= 10,000): enough volume to saturate the chain
- **Broadcast to all validators** (`--broadcast-rpc-urls`): ensures the stale tx exists in ALL mempools, not just one leader's
- **Enough accounts** (>= 50): distributes load so multiple accounts' transactions can interleave

---

## Recovery

Once stalled, the chain **cannot self-recover**. The poisoned mempool state is never cleared because:
- Pruning only happens after successful finalization
- No finalization can occur because of the poisoned mempool
- This is a permanent deadlock

### Docker Devnet Recovery

```bash
cd docker
just reset          # Wipes all volumes (DKG shares, ledger, everything)
just trusted-devnet # Fresh start from genesis
```

A simple `just restart` or `just restart-validators` will NOT fix the stall because:
- The mempool is in-memory and IS lost on restart (good)
- But if the stale transactions are re-submitted (e.g., by retrying clients), the stall recurs
- More importantly, some stale transactions may be persisted in the finalized blocks archive and re-indexed on restart, depending on the failure point

The only reliable fix is a full volume wipe (`just reset`).

### Remote Server Recovery (Ansible)

```bash
cd ansible
ansible-playbook playbooks/reset.yml    # Wipes everything on the server
ansible-playbook playbooks/deploy.yml   # Fresh deploy from scratch
```

---

## Quick Reference: Reproduction Commands

```bash
# Terminal 1: Start clean devnet
cd docker
just reset && just trusted-devnet

# Terminal 2: Monitor (after devnet is ready)
cd docker
just stats

# Terminal 3: Trigger stall (after confirming chain is healthy in stats)
cd /path/to/kora/repo
cargo run --release -p loadgen -- \
  --total-txs 50000 \
  --accounts 50 \
  --concurrency 200 \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548

# Terminal 3: Verify stall (after loadgen completes)
curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result | {currentView, finalizedCount, nullifiedCount}'

# Wait 30 seconds, run same curl again -- finalizedCount unchanged = stall confirmed
```

---

## Required Fixes to Prevent This

1. **Executor: skip bad transactions instead of aborting** (`crates/node/executor/src/revm.rs:395`): Replace `?` with `match` + `continue` so individual bad transactions are skipped and logged, but the rest of the block executes successfully.

2. **Mempool: validate nonces on insertion and re-validate before proposal** (`crates/node/consensus/src/components/mempool.rs`): Reject transactions with nonces that are already finalized. Before building a block, re-check nonces against current state and exclude stale entries.

3. **Pruning: always prune regardless of persistence outcome** (`crates/node/reporters/src/lib.rs:226`): Decouple mempool pruning from the persistence success path so that finalized transactions are always removed from the mempool.
