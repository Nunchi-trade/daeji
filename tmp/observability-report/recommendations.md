# Recommendations

Actionable improvements organized by priority. Each recommendation references the
findings from `findings.md` and includes specific implementation guidance.

---

## Priority 1: Fix Broadcast Asymmetry (Issues 2, 3, 4)

The root cause of the 27% skip rate is that nodes 2 and 3 fail to deliver their
proposals via the broadcast system ~17% of the time. This cascades into inactivity
timeouts and nullifications when they are leaders.

### Investigation Steps

1. **Check if this is a Docker networking artifact:**
   - Run the devnet with only 2 validators (nodes 0 and 1) and measure skip rate.
     If it drops to near-zero, the issue is contention from 4 nodes on one host.
   - Try adding explicit Docker network bandwidth limits to all nodes equally to
     rule out asymmetric QoS.

2. **Check startup timing:**
   - In `docker/compose/devnet.yaml`, nodes 1-3 all start after node0's healthcheck.
     But nodes 2 and 3 may establish peer connections later and end up with worse
     network topology. Add logging to show when each node's peer connections are
     fully established.

3. **Inspect the broadcast implementation:**
   - Look at the `Dropped` status in the broadcast code. What triggers a drop vs
     a failure? A drop likely means the broadcast timed out waiting for the data to
     become available locally, while a failure means the peer didn't respond.
   - Key code paths to examine:
     ```
     crates/node/broadcast/  — broadcast get/subscribe/receive logic
     ```
   - Check if there's a queue depth or buffer size that could cause drops under
     contention.

4. **Add broadcast-level timing metrics:**
   - The broadcast module tracks success/failure/dropped counts but NOT latency.
     Adding a histogram for broadcast delivery latency would reveal whether nodes
     2 and 3 are consistently slower or occasionally very slow (tail latency).

### Specific Code Changes

- Add `broadcast_get_duration` histogram (successful gets only) to the broadcast module.
- Add `broadcast_get_queue_depth` gauge to track how many get requests are pending.
- Consider adding a `broadcast_get_timeout` configuration option if one doesn't exist.

---

## Priority 2: Investigate Non-Functional Resolver (Issue 5)

The resolver has ~165k dropped fetches per node and zero successful fetches. This
means when a node misses consensus data, it cannot recover it from peers.

### Investigation Steps

1. **Determine if this is expected for a devnet:**
   - The resolver may only be needed when nodes fall behind (e.g., after a restart).
     In a continuously-running devnet where all nodes start together, the resolver
     may correctly have nothing to fetch.
   - The "Dropped" cancellations might just be the resolver canceling stale fetch
     requests for views that have already been finalized.

2. **Test resolver functionality:**
   - Stop one validator for 30 seconds, then restart it. Check whether:
     - `engine_resolver_resolver_fetch_duration_count` increases (successful fetches)
     - The restarted node catches up to the current height
   - If fetch_duration_count stays at 0, the resolver is truly broken.

3. **Check resolver configuration:**
   ```
   crates/node/  — look for resolver config (timeout, max concurrent fetches, etc.)
   ```

### Metrics to Add

- `engine_resolver_resolver_fetch_attempts_total` — total fetch attempts (not just cancels)
- A gauge showing the "gap" between the node's finalized height and the network's
  known highest view — this would quantify how much data the resolver needs to fetch.

---

## Priority 3: Reduce Disk Write Amplification (Issue 8)

78 GiB written in 20 minutes with zero transaction load is excessive. At ~59 blocks/sec
with empty blocks, that's ~1.1 MiB/block written to disk.

### Investigation Steps

1. **Profile which storage layer is writing:**
   - QMDB (state tree) writes: `state_qmdb_*` metrics
   - Archive (finalized blocks) writes: `finalized_blocks_*` and `finalizations_by_height_*`
   - Consensus journal writes: `engine_voter_journal_*`
   - Compare the write volume from each subsystem.

2. **Check QMDB compaction/sync behavior:**
   - QMDB uses a journal-based approach. Check if the journal is being fsynced too
     aggressively or if compaction is running continuously.
   - Look for configuration options like `sync_interval` or `compaction_threshold`.

3. **Empty block optimization:**
   - If the block is empty (no transactions), the state tree shouldn't change. Check
     if QMDB is writing the full state root even when nothing changed.
   - Consider short-circuiting the state commitment when the block is empty.

### Metrics to Add

- Per-subsystem write byte counters (QMDB accounts vs code vs storage, archive, journal)
- `marshaled_block_tx_count` histogram — track transactions per block to correlate
  write volume with actual work done.
- `state_commit_duration` histogram — time to commit state changes after execution.

---

## Priority 4: Add Missing Application-Level Metrics

The current metrics cover consensus, networking, and storage well, but are missing
key application-layer visibility.

### Transaction Pipeline Metrics

| Metric | Type | Why |
|--------|------|-----|
| `txpool_pending` | gauge | Number of pending transactions in the mempool |
| `txpool_queued` | gauge | Queued (future nonce) transactions |
| `txpool_added_total` | counter | Transactions added to pool |
| `txpool_rejected_total` | counter | Transactions rejected (labels: `reason`) |
| `txpool_evicted_total` | counter | Transactions evicted from pool |

### Execution Metrics

| Metric | Type | Why |
|--------|------|-----|
| `execution_duration` | histogram | Time to execute a block via revm |
| `execution_tx_count` | histogram | Transactions per block |
| `execution_gas_used` | histogram | Gas used per block |
| `execution_gas_limit` | gauge | Current gas limit |
| `execution_revert_total` | counter | Reverted transactions |

### RPC Metrics

| Metric | Type | Why |
|--------|------|-----|
| `rpc_requests_total` | counter | RPC requests (labels: `method`, `status`) |
| `rpc_request_duration` | histogram | RPC response time (labels: `method`) |
| `rpc_active_connections` | gauge | Active WebSocket/HTTP connections |

### Where to Add These

- Transaction pool metrics: in the marshal/mempool module
- Execution metrics: in the execution/revm integration layer
- RPC metrics: in the RPC server handler (tower middleware or axum layer)

---

## Priority 5: Improve Height Drift Observability (Issue 6)

A 500-700 block drift is concerning for RPC consistency. Need to understand whether
this is a measurement artifact or a real divergence.

### Investigation Steps

1. **Distinguish processed vs finalized height:**
   - `processed_height` tracks blocks processed locally.
   - `finalized_height` tracks blocks with finalization certificates.
   - Check if the drift is in finalization certificate propagation rather than actual
     block processing.

2. **Check finalization certificate broadcast:**
   - If finalization certificates are propagated via the same broadcast mechanism
     as proposals, the same broadcast failures affecting nodes 2 and 3 could cause
     slower finalization delivery to some nodes.

3. **Add drift alerting:**
   ```promql
   # Alert if height drift exceeds 1000 blocks
   max(finalized_height) - min(finalized_height) > 1000
   ```

---

## Priority 6: Memory Profiling (Issue 7)

1 GiB per idle validator is high. Profile with:

### Approaches

1. **Heap profiling with jemalloc:**
   - Add `jemalloc` as the global allocator with profiling enabled.
   - Use `MALLOC_CONF="prof:true,prof_prefix:/tmp/heap"` to dump heap profiles.
   - Analyze with `jeprof` to find allocation hotspots.

2. **Runtime metric breakdown:**
   - The `runtime_tasks_running` gauge shows active async tasks. Correlate with memory.
   - `engine_voter_state_tracked_views` shows how many views are held in memory.
     If this grows unbounded, it's a memory leak.
   - QMDB `index_items` gauges show how much state is cached in memory.

3. **Quick check — cache sizes:**
   - `cache_cache_notarizations_items_tracked`
   - `cache_cache_finalizations_items_tracked`
   - If these grow linearly with height, there's a cache eviction bug.

---

## Benchmarking Approaches

### Load Generator Usage

The existing load generator (`just loadtest` or `just stresstest`) can be used to
stress-test the devnet:

```bash
# Quick sanity test: 1000 transactions
just loadtest

# Sustained load: 10000 txs with 50 concurrent accounts
just stresstest

# Custom load profile
cargo run --release -p loadgen --bin loadgen -- \
  --total-txs 50000 \
  --accounts 100 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

### Key Metrics to Watch Under Load

```promql
# TPS (transactions finalized per second)
# Requires adding execution_tx_count metric first
rate(execution_tx_count_sum[1m])

# Block build time should stay low
rate(marshaled_build_duration_sum[1m]) / rate(marshaled_build_duration_count[1m])

# Finalization latency should not degrade
rate(engine_voter_finalization_latency_sum[1m]) / rate(engine_voter_finalization_latency_count[1m])

# Memory should not grow unboundedly
runtime_process_rss

# Disk write rate will increase — track by how much
rate(runtime_storage_write_bytes_total[1m])
```

### Comparative Benchmarking

To isolate performance bottlenecks, run the devnet under different configurations:

1. **Baseline (current):** 4 validators, empty blocks, idle
2. **Light load:** 4 validators, 100 TPS sustained
3. **Heavy load:** 4 validators, max TPS until degradation
4. **Reduced validator count:** 2 validators, same load — isolates consensus overhead
5. **Single machine vs distributed:** Deploy across multiple machines to isolate
   resource contention from consensus issues

---

## Log-Based Debugging

### Useful Log Patterns

The validators use `tracing` for structured logging. Key log targets:

```bash
# View logs for a specific node
docker compose -f docker/compose/devnet.yaml logs validator-node0 --tail 100

# Filter for consensus events
docker compose -f docker/compose/devnet.yaml logs validator-node0 2>&1 | grep -i "nullif\|timeout\|skip"

# Filter for broadcast issues
docker compose -f docker/compose/devnet.yaml logs validator-node2 2>&1 | grep -i "broadcast\|dropped"

# Filter for resolver activity
docker compose -f docker/compose/devnet.yaml logs validator-node0 2>&1 | grep -i "resolver\|fetch"
```

### Recommended Log Level Changes

For debugging specific issues, adjust `RUST_LOG` in the Docker compose file:

```yaml
# In docker/compose/devnet.yaml, add to validator environment:
RUST_LOG: "info,broadcast=debug,resolver=debug"
```

Useful targets:
- `broadcast=debug` — shows individual get/subscribe/receive events
- `resolver=debug` — shows fetch attempts and cancellations
- `engine=debug` — shows consensus state transitions (very verbose)
- `network=debug` — shows peer connections and message routing

---

## Additional Grafana Dashboard Panels to Add

Based on the findings, these panels would be valuable additions:

1. **Skip Rate Gauge** — single stat panel showing `1 - (rate(finalized_height[5m]) / rate(current_view[5m]))` with thresholds (green < 5%, yellow < 15%, red > 15%)
2. **Broadcast Success Rate by Node** — bar gauge showing success/(success+failure+dropped) per node
3. **Resolver Health** — stat showing successful fetches vs cancellations
4. **Disk Write Rate** — time series of `rate(runtime_storage_write_bytes_total[1m])` per node
5. **Cache Growth** — time series of `cache_cache_*_items_tracked` to detect unbounded growth
6. **Height Drift Over Time** — time series of `max(finalized_height) - min(finalized_height)`

---

## Summary: Recommended Action Order

| # | Action | Effort | Impact |
|---|--------|--------|--------|
| 1 | Investigate broadcast failures on nodes 2/3 | Medium | High — directly fixes 27% skip rate |
| 2 | Add broadcast latency histogram | Low | Medium — enables root cause analysis |
| 3 | Test resolver with node restart | Low | Medium — verifies recovery path works |
| 4 | Add execution/txpool metrics | Medium | High — essential for production visibility |
| 5 | Profile disk write amplification | Medium | Medium — prevents future bottleneck |
| 6 | Profile memory with jemalloc | Low | Low — informational for now |
| 7 | Add RPC metrics | Low | Medium — needed for RPC SLA monitoring |
| 8 | Run load generator with metric capture | Low | High — validates system under real conditions |
