# Recovery Options for a Stalled Kora Chain

## What Recovery Means

Recovery is the process of restoring a Kora blockchain network from a state where no blocks are being finalized to a state where blocks are produced and finalized continuously. A healthy chain produces approximately 100+ blocks per second at 73% consensus efficiency.

A "stalled" chain is one where:
- Views may or may not still advance (consensus rounds still occur)
- No blocks are finalized (the finalized height is frozen)
- The nullification rate is 100% (every proposal fails or times out)
- The stall is permanent without intervention

This document covers recovery from the specific failure mode where mempool poisoning causes all block proposals to abort. For background on how this failure occurs, see `production-failure-timeline.md`.

---

## Summary of Recovery Options

| Option | Works? | Data Loss | Downtime | Notes |
|--------|--------|-----------|----------|-------|
| 1. Full cluster reset | YES | All chain history lost | ~2 minutes | Only currently viable option |
| 2. Clear mempools only | NOT POSSIBLE | None | N/A | No mechanism to flush mempool without code change |
| 3. Restart individual nodes | NO | None | N/A | Resolver bug prevents catch-up |
| 4. Wait it out | NO | None | N/A | Stall is permanent; no expiration mechanism |
| 5. Rolling restart all nodes | MAYBE | Journals lost | ~30 seconds | Works only if all 4 restart within quorum timeout |

---

## Option 1: Full Cluster Reset (Nuclear Option)

This is the only recovery method that is guaranteed to work in the current codebase.

### What It Does

1. Stops all Docker containers (validators, secondary, observability)
2. Removes all Docker volumes (QMDB state, consensus journals, DKG keys, Prometheus data)
3. Re-generates cryptographic keys and DKG shares
4. Starts a brand-new chain from genesis block 0

### What Is Lost

- All finalized blocks (entire chain history)
- All account state (balances, nonces, contract storage)
- All DKG key shares (new key ceremony required)
- All Prometheus metrics history
- All Grafana dashboard state

### Commands

#### Local Development (Docker Compose)

```bash
cd docker

# Stop everything and destroy all volumes
just reset

# Start fresh
just devnet
```

The `just reset` command expands to:
```bash
docker compose -f compose/devnet.yaml --profile observability --profile interactive-dkg down -v
```

The `-v` flag removes all named volumes defined in the compose file:
- `data_node0` through `data_node3` (validator state: QMDB, keys, runtime)
- `data_secondary0` (secondary node state)
- `shared_config` (peers.json, shared configuration)
- `prometheus_data`, `grafana_data`, `loki_data` (observability state)

#### Remote Server (Ansible)

```bash
cd ansible

# Reset: stops containers, removes volumes
ansible-playbook playbooks/reset.yml

# Deploy: builds image, generates keys, runs DKG, starts validators
ansible-playbook playbooks/deploy.yml
```

Or using the Justfile shortcuts (from repository root):
```bash
just remote-reset
just remote-deploy
```

### Why This Works

The mempool is entirely in-memory (`InMemoryMempool` is a `BTreeMap` stored in process memory). Destroying the container destroys the mempool. Destroying volumes removes any persisted state that could trigger re-contamination. Starting fresh means no stale transactions exist anywhere.

---

## Option 2: Clear Mempools Only

### Concept

If we could flush only the mempool contents without destroying chain state, the chain could resume from its current height with all historical blocks intact.

### Why This Is NOT Currently Possible

The `InMemoryMempool` has no external flush mechanism:
- No RPC method exposes mempool clearing (`kora_clearMempool` does not exist)
- No admin API allows mempool manipulation
- No signal handler triggers mempool reset
- The mempool is embedded inside `LedgerService` behind a `Mutex<LedgerInner>` -- not accessible from outside the process

### What Would Make This Work (Code Changes Required)

1. **Add an admin RPC endpoint**: `kora_flushMempool` that calls `inner.mempool.clear()`
2. **Add a signal handler**: Send `SIGUSR1` to trigger mempool flush
3. **Add a TTL to mempool entries**: Transactions older than N seconds are automatically evicted
4. **Add a startup flag**: `--clear-mempool` that empties the mempool at boot before joining consensus

Any of these would allow recovery without data loss. Until one is implemented, Option 1 remains the only viable path.

---

## Option 3: Restart Individual Nodes

### Concept

Restart crashed or stalled validators one at a time, allowing them to rejoin the running cluster.

### Why This Does NOT Work

When a validator restarts:

1. **Consensus journal is lost** (stored on tmpfs at `/runtime`): The node initializes consensus at view=1.
2. **Mempool is cleared** (in-memory): The restarted node has a clean mempool.
3. **But catch-up fails**: The Commonware resolver attempts to fetch missed blocks from peers. Block verification requires sequential parent state snapshots. A freshly restarted node has no parent snapshots. Verification fails, and the resolver permanently blocks those peers.

```
Node restart → clean mempool, consensus at view=1
    → Resolver requests block at height H from peer
    → verify_block(H) fails: missing parent snapshot for H-1
    → Resolver blocks that peer permanently
    → Tries remaining peers → same failure → all peers blocked
    → Node cannot catch up → cannot participate in consensus
```

Evidence from production: Node 0 ran for 19 hours after restart and only finalized 2 blocks. Node 3 finalized 1 block. Both showed continuous `"invalid data received"` warnings from the resolver.

### Additional Problem: Quorum Loss

Even if catch-up succeeded, restarting nodes one-by-one risks breaking quorum:
- BFT requires 3 of 4 validators for finalization
- If 2 nodes are down simultaneously (one being restarted, one already crashed), quorum is lost
- Once quorum is lost, the remaining nodes cannot finalize, and the restarted node has nothing to catch up to

---

## Option 4: Wait It Out

### Why This Does NOT Work

The stall is permanent because the stale transactions in the mempool will never be removed:

1. **No TTL**: `InMemoryMempool` has no time-based expiration. Transactions live forever until pruned.
2. **No pruning trigger**: `prune_mempool()` only runs after successful block finalization. Since no blocks finalize, no pruning occurs. This is a circular dependency.
3. **No size limit**: The mempool has no maximum capacity that would trigger eviction of old entries.
4. **No periodic cleanup**: There is no background task that validates mempool contents against current state.
5. **View advancement does not help**: The consensus protocol advances views (trying new leaders), but every leader draws from the same poisoned mempool.

The chain will remain stalled indefinitely -- minutes, hours, days, weeks -- until external intervention occurs.

---

## Option 5: Synchronized Restart of All Nodes

### Concept

Restart all 4 validators simultaneously so they all begin at view=1 with empty mempools and no need for catch-up.

### How It Might Work

If all nodes restart at the same time:
1. All consensus journals are lost (tmpfs cleared)
2. All mempools are cleared (in-memory, lost on restart)
3. All nodes initialize consensus at view=1
4. No node needs to catch up (all are at the same point)
5. QMDB state is preserved (persistent volumes survive container restart)
6. DKG keys are preserved (stored in persistent data volumes)
7. `recover_finalized_state()` replays from the archive to restore ledger head

### Why This Is Risky

1. **Archive availability**: Recovery depends on `finalized_blocks` and `finalizations_by_height` archives being intact. These are stored in Commonware's journal system. If the runtime directory is on tmpfs (devnet default), the archives are lost on restart and the ledger starts from genesis.

2. **Timing sensitivity**: If nodes do not start within the same consensus timeout window, some may begin proposing before others are ready, leading to immediate nullifications.

3. **Same vulnerability**: Without fixing the underlying bugs, the chain will stall again on the next load test. Recovery without code fixes is just delaying the next failure.

### Commands (If Attempting)

```bash
# Stop all validators (do NOT use 'down -v' -- that destroys volumes)
docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0

# Clear runtime journals only (preserves QMDB and keys)
for vol in kora-devnet_data_node{0,1,2,3} kora-devnet_data_secondary0; do
    docker run --rm -v "${vol}:/data" alpine rm -rf /data/runtime
done

# Start all validators simultaneously
docker compose -f compose/devnet.yaml up -d \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0
```

Note: The `devnet-run.sh` script already performs this pattern (`clear_runtime_state` then start), but it also rebuilds the Docker image and checks DKG state. For a faster recovery, the manual commands above skip the rebuild.

---

## Why the Stall Is Permanent (Technical Detail)

The stall is a fixed-point state -- every pathway that could lead to recovery is blocked:

```
To remove bad txs from mempool → need prune_mempool() to run
To run prune_mempool() → need a block to be finalized
To finalize a block → need a successful proposal
To make a successful proposal → need executor to not abort
To prevent executor abort → need no bad txs in proposal
To have no bad txs in proposal → need bad txs removed from mempool
→ CIRCULAR DEPENDENCY
```

This circular dependency has no timeout, no circuit breaker, and no escape hatch in the current code.

---

## Recovery Procedure: Step-by-Step (Full Reset)

This is the recommended recovery procedure for the current codebase.

### Step 1: Stop All Containers

```bash
# On the server or locally:
cd docker
docker compose -f compose/devnet.yaml --profile observability --profile interactive-dkg down
```

Verify no containers remain running:
```bash
docker compose -f compose/devnet.yaml ps
# Should show no containers
```

### Step 2: Remove Volumes

```bash
docker compose -f compose/devnet.yaml --profile observability --profile interactive-dkg down -v
```

The `-v` flag removes all named volumes. Verify:
```bash
docker volume ls | grep kora-devnet
# Should show nothing
```

### Step 3: DKG Key Generation

The `devnet-run.sh` script handles this automatically. For the default trusted-dealer mode:
```bash
just trusted-devnet
```

This will:
1. Build the Docker image (`docker buildx bake`)
2. Run `keygen setup` (generates validator keypairs, peers.json)
3. Run `keygen dkg-deal` (generates BLS threshold shares via trusted dealer)
4. Set file permissions (UID 1000)

For production-like interactive DKG:
```bash
just devnet
```

This runs a full distributed key generation ceremony where all 4 nodes participate in generating shares without any single party knowing the full secret.

### Step 4: Start All Validators Simultaneously

The startup script handles ordering:
1. `validator-node0` starts first (bootstrap node with `IS_BOOTSTRAP=true`)
2. Nodes 1-3 wait for node0 to be healthy (`depends_on: condition: service_healthy`)
3. `secondary-node0` starts after node0 is healthy
4. Observability stack (Prometheus, Grafana, Loki) starts in parallel

Validators use a 10-second health check interval with a 30-second start period. Full cluster readiness takes approximately 30-60 seconds.

### Step 5: Verify Health

```bash
# Check all containers are healthy
just status

# Or manually:
docker compose -f compose/devnet.yaml ps
```

All validators should show `healthy` status.

Query node status via RPC:
```bash
curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","id":1}' | jq .
```

Expected response fields:
- `currentView`: Should be advancing (check twice with 5-second gap)
- `finalizedCount`: Should be increasing
- `nullifiedCount`: Should be low relative to view count

### Step 6: Verify Block Production

```bash
# Check blocks/sec via Prometheus (if observability is running)
curl -s "http://localhost:9090/api/v1/query?query=kora:blocks_per_sec" | jq '.data.result[0].value[1]'
# Should show ~100+

# Or use the devnet stats script
just stats
```

---

## Ansible Commands for Automated Recovery

### Full Reset and Redeploy

```bash
cd ansible

# Complete reset (destroys all state, regenerates keys, deploys fresh)
ansible-playbook playbooks/reset.yml
ansible-playbook playbooks/deploy.yml

# Enable observability
ansible-playbook playbooks/observe.yml
```

### Targeted Recovery (If Supported by Playbooks)

```bash
# Restart validators only (preserves observability)
ansible-playbook playbooks/restart-validators.yml

# Redeploy binary only (rebuild + restart, preserves state if possible)
ansible-playbook playbooks/deploy.yml --tags restart
```

### Remote Execution via Justfile

From the repository root:
```bash
just remote-reset    # ansible-playbook playbooks/reset.yml
just remote-deploy   # ansible-playbook playbooks/deploy.yml
just remote-stats    # Check health of remote devnet
```

---

## Monitoring After Recovery

After a successful recovery, monitor these metrics to confirm stable operation:

### Immediate (First 60 Seconds)

| Metric | Healthy Value | Alert If |
|--------|---------------|----------|
| `kora:blocks_per_sec` | > 90 | < 50 |
| `kora:consensus_efficiency` | > 0.65 | < 0.50 |
| `kora:nullification_rate` | < 50/sec | > 60/sec |
| `kora:height_drift` | 0 | > 10 |
| `up{job="kora-validators"}` | 4 (all up) | < 4 |

### Short-Term (First 10 Minutes)

| Check | How | Expected |
|-------|-----|----------|
| View advancing | Query `kora_nodeStatus` twice, 10s apart | `currentView` increases by ~1400 |
| Blocks finalizing | Compare `finalizedCount` | Increases by ~1000 in 10 seconds |
| No height drift | `max(finalized_height) - min(finalized_height)` | 0 |
| Memory stable | `runtime_process_rss` | < 500MB per node, not growing |
| No resolver blocks | `engine_resolver_resolver_peers_blocked` | 0 |

### Before Next Load Test

Verify these conditions before applying transaction load:

1. Chain has been running stable for at least 5 minutes
2. Baseline nullification rate is at expected ~26% (idle background rate)
3. No alerts are firing
4. All 4 validators report healthy
5. Memory usage is stable (no upward trend)

---

## Prevention: Code Changes That Would Make Recovery Unnecessary

These changes would eliminate the permanent stall failure mode entirely:

### Priority 1: Executor Skip-and-Continue

**File**: `crates/node/executor/src/revm.rs`, lines 391 and 395

Replace `?` with `match` + `continue`. Bad transactions are skipped instead of aborting the entire block. This single change transforms "permanent stall" into "slightly reduced throughput."

**Impact**: Blocks always finalize (even with poisoned mempool), pruning always runs, stale txs are eventually removed.

### Priority 2: Mempool TTL / Pruning on Failure

**File**: `crates/node/ledger/src/components/mempool.rs`

Add time-based expiration to mempool entries. Transactions older than N seconds (suggested: 60s) are evicted regardless of whether they were included in a finalized block.

**Impact**: Even without the executor fix, stale transactions would eventually expire and the chain would self-heal.

### Priority 3: Always Prune on Finalization Error Paths

**File**: `crates/node/reporters/src/lib.rs`, line 219

Move `prune_mempool()` before the persistence step, or call it on all error paths. A consensus-finalized block's transactions must be pruned regardless of local persistence success.

**Impact**: Prevents re-proposal of already-finalized transactions that would fail with NonceTooLow.

### Priority 4: Wire TransactionPool (Replace InMemoryMempool)

**File**: `crates/node/txpool/src/pool.rs` (exists but unused)

The codebase already contains a `TransactionPool` with per-sender nonce tracking, ordering, and stale transaction rejection. Wire it into `LedgerService` to replace `InMemoryMempool`.

**Impact**: Stale transactions rejected at ingress. Mempool always contains only valid-at-tip transactions.

### Priority 5: Fix Resolver Catch-Up

**File**: Commonware framework (external dependency)

The resolver should not permanently block peers when application-level verification fails during catch-up. Options:
- Trust BLS finality certificates instead of re-executing blocks during catch-up
- Use exponential backoff instead of permanent blocking
- Fetch blocks sequentially so parent state is always available

**Impact**: Crashed nodes can rejoin consensus, preventing quorum loss after individual node failures.

---

## Decision Matrix: When to Use Which Recovery Method

| Situation | Recommended Action |
|-----------|-------------------|
| Devnet stalled after load test | Full reset (`just reset && just devnet`) |
| Single node crashed, others running | Full reset (catch-up is broken) |
| All nodes running but no finalization | Full reset (mempool poison is permanent) |
| Need to preserve chain history | NOT POSSIBLE with current code; must fix bugs first |
| Production chain stall (future) | Apply executor fix, deploy new binary, restart |

Until the executor skip-and-continue fix is deployed, **all stall scenarios require a full reset**. There is no partial recovery path in the current codebase.
