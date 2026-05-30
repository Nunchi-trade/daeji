# Recommendations — Prioritized Fixes and Improvements

## Critical Fixes (Stall Prevention)

### 1. Make Executor Skip Failed Transactions

**Files:** `crates/node/executor/src/revm.rs:388-407`

**Current:** `?` operator aborts entire block on any transaction failure.

**Fix:** Replace `?` with `match` + `continue`:

```rust
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
        Ok(env) => env,
        Err(err) => {
            warn!(?tx_hash, error = ?err, "skipping failed tx decode");
            skipped_txs.push(tx_hash);
            continue;
        }
    };
    evm.set_tx(tx_env);

    let result_and_state = match evm.replay() {
        Ok(result) => result,
        Err(err) => {
            warn!(?tx_hash, error = ?err, "skipping failed tx execution");
            skipped_txs.push(tx_hash);
            continue;
        }
    };

    // ... process successful tx
}
```

Return skipped transaction IDs so they can be pruned from the mempool.

**Impact:** Prevents single bad transaction from stalling the entire network.

---

### 2. Wire TransactionPool into Production

**Files:** `crates/node/ledger/src/lib.rs`, `crates/node/runner/src/runner.rs`

**Current:** `LedgerView` hardcodes `InMemoryMempool::new()`.

**Fix:**
1. Make `LedgerView` generic over mempool type (or use trait object)
2. Instantiate `TransactionPool` with `PoolConfig::default()` in runner
3. Store `OrderedTransaction` to avoid double ECDSA recovery
4. Add P2P transaction validation (currently only RPC txs are validated)

**Impact:** Nonce ordering, size limits, per-sender caps, fee prioritization.

---

### 3. Fix FinalizedReporter Early Return Bug

**File:** `crates/node/reporters/src/lib.rs:216-219`

**Current:** Persist failure returns without calling `prune_mempool()`.

**Fix:**
```rust
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    // Still prune — block is consensus-final regardless of persistence
    state.prune_mempool(&block.txs).await;
    ack.acknowledge();
    return;
}
```

**Impact:** Prevents mempool poisoning when persistence fails.

---

### 4. Validate Nonces Against Pending State

**File:** `crates/node/txpool/src/validator.rs:91-100`

**Current:** Checks nonces against QMDB persisted state only.

**Fix:** Pass pending transaction pool state to validator so it can check against the highest known nonce (persisted + pending):

```rust
let effective_nonce = max(state_nonce, pool_nonce_for_sender);
if nonce < effective_nonce {
    return Err(TxPoolError::NonceTooLow { got: nonce, expected: effective_nonce });
}
```

**Impact:** Rejects stale transactions at ingress before they reach the mempool.

---

## High Priority Improvements

### 5. Register Prometheus Metrics

**Files:** `crates/node/runner/src/runner.rs`, new metrics module

**Current:** Infrastructure exists (`prometheus-client`, `/metrics` endpoint) but no metrics registered.

**Fix:** Create a `KoraMetrics` struct with counters/histograms for:
- Block execution success/failure/duration
- Transaction pool size/additions/pruning
- Proposal success/failure count
- Finalization latency (application-level)
- RPC request counts

Register via `context.register()` during runner initialization.

**Impact:** Full observability of application behavior.

---

### 6. Enforce RPC Rate Limiting

**File:** `crates/node/rpc/src/server.rs`

**Current:** `RateLimitConfig` defined but never applied.

**Fix:** Add rate limiting middleware to the axum/jsonrpsee server:

```rust
use tower::limit::RateLimitLayer;
// Or use governor crate for per-IP rate limiting
```

**Impact:** Prevents RPC flooding that can overwhelm the mempool.

---

### 7. Add Prometheus Alert Rules

**File:** `docker/config/prometheus.yml` or new `alerts.yml`

Add rules for:
- Node down (no heartbeat for 30s)
- Height divergence > 10 blocks for 1 minute
- Nullification rate > 5/s
- Consensus stall (zero finalized blocks for 5 minutes)

**Impact:** Early detection of issues before they cascade.

---

## Medium Priority Improvements

### 8. Add Missing Logging

- `info!` when finalization completes successfully
- `debug!` when mempool is pruned (count + remaining)
- `info!` when block is built (height + tx count)
- RPC request middleware logging

### 9. Fix Loadgen Race Condition

**File:** `bin/loadgen/src/main.rs`

- Use `Ordering::SeqCst` for nonce operations
- Ensure per-account transaction ordering (await send before incrementing nonce)
- Or: use per-account send queues

### 10. Reduce Prometheus Scrape Interval

Change from 15s to 10s in `docker/config/prometheus.yml` for better consensus event resolution.

### 11. Add Criterion Benchmarks

Create `benches/` directory with microbenchmarks for:
- ECDSA recovery throughput
- Block execution throughput
- State root computation
- Mempool operations

### 12. Scrape Secondary Node in Prometheus

Add `secondary-node0:9002` (if metrics port exposed) to Prometheus scrape targets.

---

## Low Priority / Nice-to-Have

### 13. Add Missing Grafana Panels

- Mempool size over time
- Block execution error rate
- Transactions per block
- RPC request rate
- Catch-up progress for restarted nodes

### 14. Structured JSON Logging

Add option for JSON log output (useful for log aggregation systems).

### 15. Request Body Size Limit

Set explicit `max_payload_size` on the jsonrpsee server builder.

### 16. Add Tracing Spans

Use `#[tracing::instrument]` on critical functions for structured span-based tracing.

### 17. Per-Account Loadgen Queues

Redesign loadgen to maintain per-account ordered send queues instead of round-robin with fire-and-forget.

---

## Implementation Order

```
Phase 1: Stall Prevention (items 1-4)
  → Unblocks: Network stability under load
  → Effort: 2-3 days

Phase 2: Observability (items 5-7)
  → Unblocks: Debugging and monitoring
  → Effort: 1-2 days

Phase 3: Quality (items 8-12)
  → Unblocks: Performance optimization, dev workflow
  → Effort: 2-3 days

Phase 4: Polish (items 13-17)
  → Unblocks: Production readiness
  → Effort: 1-2 days
```
