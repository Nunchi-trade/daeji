# Comprehensive error handling and logging improvements

## Summary

Kora has multiple categories of error handling and logging deficiencies that degrade observability, mask failures, and in some cases risk silent data loss or cascading crashes. PR #129 addressed three specific silent error paths (see "Relationship to existing PRs" below), but significant gaps remain across the codebase.

This issue catalogs every identified deficiency, grouped by severity, with exact file locations and specific fixes.

---

## Severity: CRITICAL

These issues can cause data loss, cascading crashes, or permanent state divergence.

---

### C1. Finalization failures return `Err(())` with no retry and no recovery

**Files:**
- `crates/node/reporters/src/lib.rs` (lines 169-279) -- `finalize_block()` function
- `crates/node/reporters/src/lib.rs` (lines 112-161) -- `handle_finalized_update()` caller

**Current behavior:**

Six separate failure paths in `finalize_block()` all return `Err(())`, a type-erased unit error that discards all context:

```rust
// Line 204-206: execution failure
Err(err) => {
    error!(?digest, error = ?err, "failed to execute finalized block");
    return Err(());
}

// Line 215-217: root computation failure
Err(err) => {
    error!(?digest, error = ?err, "failed to compute qmdb root");
    return Err(());
}

// Line 220-228: state root mismatch (logged at warn, not error -- see C4)
if state_root != block.state_root {
    warn!(?digest, expected = ?block.state_root, computed = ?state_root,
          "state root mismatch for finalized block");
    return Err(());
}

// Line 254-256: missing parent snapshot
error!(?digest, ?parent_digest, "missing parent snapshot for finalized block");
return Err(());

// Line 268-271: persist task join failure
Err(err) => {
    error!(?digest, error = ?err, "persist task failed");
    return Err(());
}

// Line 273-275: persist result error
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    return Err(());
}
```

The caller in `handle_finalized_update()` (lines 128-158) calls `finalize_block()` and checks `if let Ok((Some(outcome), Some(block_context))) = result.as_ref()` (line 138) to conditionally run indexing and GC. However, the mempool prune (`state.prune_mempool`) and marshal ack (`ack.acknowledge()`) on lines 154 and 158 run unconditionally regardless of whether `finalize_block` returned `Ok` or `Err`. This means when finalization fails (e.g., execution error, missing parent snapshot, or state root mismatch), the node acknowledges the block to the marshal without having successfully persisted it. The QMDB state silently diverges from the network. Once one finalization fails, all subsequent blocks also fail because each depends on the previous one's snapshot.

**Problems:**
1. `Err(())` erases all error context -- callers cannot distinguish transient failures (I/O, OOM) from permanent ones (missing snapshot, state root mismatch).
2. No retry logic for transient failures.
3. No metrics counter for finalization failures.
4. State root mismatch is logged at `warn!` but is arguably more severe than the `error!`-level I/O failures because it indicates state corruption or a consensus bug (see C4).
5. The marshal `ack.acknowledge()` at line 158 runs unconditionally (outside the `if let Ok` branch), telling consensus "I processed this block" even when the node failed to persist it.

**Fix:**
1. Replace `Err(())` with a typed error enum (`FinalizationError { Transient(e), Permanent(e), StateRootMismatch { expected, actual } }`).
2. Add retry logic (3 attempts with exponential backoff) for transient failures (execution, root computation, persist).
3. For permanent failures (missing parent snapshot, state root mismatch), log at `error!` and emit a Prometheus counter (`kora_finalization_failures_total{reason="..."}`).
4. Consider triggering a graceful shutdown on repeated permanent failures rather than silently diverging.

---

### C3. `std::process::abort()` with no graceful shutdown

**File:** `crates/node/runner/src/runner.rs` (lines 373-395) -- `spawn_task_watchdog()`

When any critical consensus task (engine, marshal, broadcast) terminates, the watchdog calls `std::process::abort()`:

```rust
fn spawn_task_watchdog(context: &cw_tokio::Context, name: &'static str, handle: RuntimeHandle<()>) {
    context.with_label(name).shared(true).spawn(move |_| async move {
        match handle.await {
            Ok(()) => {
                error!(task = name, "critical task exited cleanly — this should never happen for a long-lived consensus actor");
            }
            Err(commonware_runtime::Error::Exited) => {
                error!(task = name, "critical task panicked (runtime caught panic and returned Error::Exited)");
            }
            Err(commonware_runtime::Error::Closed) => {
                warn!(task = name, "critical task terminated because the runtime context was shut down");
            }
            Err(ref e) => {
                error!(task = name, error = %e, error_debug = ?e, "critical task failed with unexpected error");
            }
        }
        error!(
            task = name,
            "consensus infrastructure is dead, aborting process for supervisor restart"
        );
        std::process::abort();
    });
}
```

`abort()` terminates the process immediately -- no destructors run, no QMDB flush, no RPC shutdown, no tracing flush. Combined with the resolver catch-up bug (PR #131), this creates a crash-restart-fail cycle: abort -> Docker restart -> catch-up attempt -> resolver blocks peers -> permanent degradation.

**Fix:**
1. Before aborting, attempt a graceful shutdown: close the RPC server, flush any pending QMDB writes, and flush the tracing subscriber so all buffered logs are written.
2. Consider using `std::process::exit(1)` instead of `abort()` so destructors can run, or use a shutdown signal that the main async runtime can handle cooperatively.
3. Add a timeout (e.g., 5 seconds) on the graceful shutdown path, then hard-abort if it does not complete.

---

### C4. State root mismatch logged at `warn` instead of `error` in both verify and finalize paths

**Files:**
- `crates/node/runner/src/app.rs` (lines 238-245) -- `verify_block()`
- `crates/node/reporters/src/lib.rs` (lines 220-227) -- `finalize_block()`

**Current behavior:**

A state root mismatch means the node re-executed a block and computed a different state root than the proposer committed. This indicates one of three things: (a) a determinism bug in the executor, (b) state corruption in QMDB, or (c) a byzantine proposer. All three are critical conditions.

In `verify_block()`:
```rust
// crates/node/runner/src/app.rs, lines 238-245
if state_root != block.state_root {
    warn!(
        ?digest,
        expected = ?block.state_root,
        computed = ?state_root,
        "state root mismatch"
    );
    return false;
}
```

In `finalize_block()`:
```rust
// crates/node/reporters/src/lib.rs, lines 220-227
if state_root != block.state_root {
    warn!(
        ?digest,
        expected = ?block.state_root,
        computed = ?state_root,
        "state root mismatch for finalized block"
    );
    return Err(());
}
```

Both use `warn!`. The execution and root computation failures immediately above these lines use `warn!` in verify and `error!` in finalize, but a state root mismatch is arguably more severe than either -- it means the node's state has diverged from the network. A node operator scanning for `error` level logs would miss this entirely.

**Fix:**
1. Change both `warn!` calls to `error!`:
   ```rust
   // crates/node/runner/src/app.rs, line 239
   error!(
       ?digest,
       expected = ?block.state_root,
       computed = ?state_root,
       "state root mismatch — possible state corruption or byzantine proposer"
   );

   // crates/node/reporters/src/lib.rs, line 221
   error!(
       ?digest,
       expected = ?block.state_root,
       computed = ?state_root,
       "state root mismatch for finalized block — possible state corruption"
   );
   ```
2. Emit a Prometheus counter: `kora_state_root_mismatches_total{path="verify|finalize"}`.

---

### C5. `verify_block` logs `warn` for missing parent snapshot during catch-up

**File:** `crates/node/runner/src/app.rs` (lines 204-206) -- `verify_block()`

**Current behavior:**

When a node is catching up (e.g., after a restart or joining a running network), consensus delivers blocks for verification before the node has processed their parents. The `verify_block` method logs at `warn` for each such block:

```rust
// crates/node/runner/src/app.rs, lines 204-206
let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
    warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
    return false;
};
```

During catch-up, this fires on every block at every consensus view, producing the same log volume problem as H1 (the `build_block` path). In devnet testing, a single restarting node generated thousands of these warnings per minute. The message is indistinguishable from a genuine problem (e.g., a snapshot store bug that drops entries) because it fires for both expected catch-up and unexpected missing snapshots.

This is closely related to H1 (the `propose()` path) but is a different code path -- `verify_block` is called when the node is a non-proposer validator, while H1 covers the proposer path.

**Fix:**
1. Downgrade from `warn!` to `debug!` for the missing parent snapshot case in `verify_block`:
   ```rust
   let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
       debug!(?digest, ?parent_digest, height = block.height, "missing parent snapshot during verify — node may be catching up");
       return false;
   };
   ```
2. Ideally, check whether the node is in a known catch-up state (e.g., current finalized height is significantly behind the block being verified) and use `trace!` during catch-up, `warn!` otherwise. This would require threading the current finalized height into the application context.
3. Add a periodic summary counter (similar to the fix proposed for H1) so operators can see the rate of verification failures without per-event log spam.

---

### C6. `RpcServerHandle::stopped()` discards `JoinError`s from spawned server tasks

**File:** `crates/node/rpc/src/server.rs` (lines 544-559)

**Current behavior:**

`RpcServerHandle` holds `JoinHandle`s for the HTTP and JSON-RPC server tasks. Both the `stopped()` method and the production usage discard task failures:

```rust
// crates/node/rpc/src/server.rs, lines 544-559
pub struct RpcServerHandle {
    http_handle: tokio::task::JoinHandle<()>,
    jsonrpc_handle: tokio::task::JoinHandle<Option<()>>,
}

impl RpcServerHandle {
    pub async fn stopped(self) {
        let _ = tokio::join!(self.http_handle, self.jsonrpc_handle);
    }
}
```

The `let _ = tokio::join!(...)` on line 558 discards `JoinError`s from both tasks. A `tokio::task::JoinError` occurs when a spawned task panics or is cancelled. If the HTTP server task panics (e.g., due to a bug in axum middleware), this is silently swallowed.

Additionally, in production the handle is immediately dropped:

```rust
// crates/node/runner/src/runner.rs, line 622
drop(rpc.start());
```

This detaches both `JoinHandle`s entirely, meaning panics in either server task are completely invisible -- no log, no metric, no crash propagation.

**Fix:**
1. In `stopped()`, log `JoinError`s:
   ```rust
   pub async fn stopped(self) {
       let (http_result, jsonrpc_result) = tokio::join!(self.http_handle, self.jsonrpc_handle);
       if let Err(err) = http_result {
           error!(error = %err, "HTTP server task failed");
       }
       if let Err(err) = jsonrpc_result {
           error!(error = %err, "JSON-RPC server task failed");
       }
   }
   ```
2. In the runner, instead of `drop(rpc.start())`, spawn a watchdog that calls `stopped()` and handles errors:
   ```rust
   let rpc_handle = rpc.start();
   tokio::spawn(async move {
       rpc_handle.stopped().await;
       warn!("RPC server has stopped");
   });
   ```

---

## Severity: HIGH

These issues degrade operational visibility or silently discard important information.

---

### H1. Log level misclassification: steady-state warnings flood logs

**File:** `crates/node/runner/src/app.rs`

Two `warn!` calls fire on every consensus view where the node has not yet processed the parent block. In production this is the normal steady-state condition (consensus advances faster than execution), producing approximately 35 warnings per second across a 4-node cluster:

```rust
// Line 99-104: inside build_block()
warn!(
    parent_height = parent.height,
    ?parent_digest,
    "build_block: parent snapshot not found — \
     node has not yet processed this parent block"
);

// Line 358-364: inside propose()
warn!(
    parent_height = parent.height,
    parent_digest = ?parent.commitment(),
    build_ms = build_elapsed.as_millis(),
    "propose failed: build_block returned None \
     (likely missing parent snapshot — node may still be catching up)"
);
```

In fresh-deploy testing, over 99.5% of all log output was these two warnings. The 23 useful INFO startup lines were buried under approximately 8,000 warnings per node in 7 minutes.

**Problems:**
1. Signal-to-noise ratio is catastrophically inverted -- real warnings are invisible.
2. The two messages always fire as a pair for the same event, doubling log volume.
3. `build_ms=0` confirms the node returns instantly (not an actual build attempt), so the "failed" framing is misleading.

**Fix:**
1. Downgrade both `warn!` calls to `debug!`.
2. Consolidate into a single log line at the `propose()` callsite.
3. Add a periodic INFO-level summary: `info!(proposed = X, skipped_no_snapshot = Y, "proposal stats (last 60s)")`.

---

### H2. Block build failures are undifferentiated -- `None` for three different causes

**File:** `crates/node/runner/src/app.rs` (lines 91-192) -- `build_block()`

Three distinct failure modes all return `None`:

| Line | Cause | Severity |
|------|-------|----------|
| 98-106 | Missing parent snapshot | Expected during normal operation |
| 143-154 | Execution failure | Could indicate OOM, bad transaction, or state corruption |
| 159-171 | Root computation failure | Indicates QMDB issue |

The caller in `propose()` (line 343) treats all three identically:

```rust
match block {
    Some(ref b) => { /* success */ }
    None => {
        warn!(/* ... */ "propose failed: build_block returned None (likely missing parent snapshot...)");
    }
}
```

The message says "likely missing parent snapshot" but the actual cause might be execution failure or root computation failure, which are far more serious.

**Fix:**
1. Return a typed error enum from `build_block()` instead of `Option`:
   ```rust
   enum BuildBlockError {
       ParentSnapshotMissing,
       ExecutionFailed(ExecutionError),
       RootComputationFailed(StateDbError),
   }
   ```
2. Log at different levels based on cause: `debug!` for `ParentSnapshotMissing`, `warn!` for `ExecutionFailed`, `error!` for `RootComputationFailed`.
3. Emit per-cause Prometheus counters.

---

### H3. `ConsensusError::Execution` erases structured error context

**Files:**
- `crates/node/consensus/src/proposal.rs` (lines 106, 151)
- `crates/node/consensus/src/execution.rs` (line 35)

Three call sites convert structured `ExecutionError` enum variants into strings:

```rust
.map_err(|e| ConsensusError::Execution(e.to_string()))?;
```

The `ConsensusError::Execution` variant stores a `String`:

```rust
// crates/node/consensus/src/error.rs, line 18-19
#[error("execution failed: {0}")]
Execution(String),
```

This means downstream code cannot match on the specific `ExecutionError` variant to decide whether the error is transient (e.g., OOM) or permanent (e.g., invalid transaction), or to emit variant-specific metrics.

**Fix:**
Change `ConsensusError::Execution(String)` to `ConsensusError::Execution(ExecutionError)` (or `Box<ExecutionError>` if the type is large). This preserves the structured error for programmatic handling while still providing a human-readable `Display` implementation through the `#[error(...)]` attribute.

---

### H4. Silently discarded broadcast results in transaction pool and RPC

**Files:**
- `crates/node/txpool/src/pool.rs` (lines 249-257)
- `crates/node/rpc/src/eth.rs` (lines 894, 901)
- `crates/node/reporters/src/lib.rs` (line 288)

Multiple `let _ = sender.send(...)` calls silently discard broadcast channel errors:

```rust
// txpool/src/pool.rs lines 249-257
if let Some(events) = &self.events {
    if let Some(hash) = replaced_hash {
        let _ = events.send(MempoolEvent::TxEvicted { hash, reason: "replaced".to_string() });
    }
    if !inserted_evicted {
        let _ = events.send(added_event);
    }
    for hash in &evicted_hashes {
        let _ = events.send(MempoolEvent::TxEvicted { hash: *hash, reason: "evicted".to_string() });
    }
}

// txpool/src/pool.rs line 375
let _ = events.send(MempoolEvent::TxEvicted { hash: *hash, reason: reason.to_string() });

// rpc/src/eth.rs line 894
let _ = sender.send(PendingTxEvent::Added(PendingTxInfo { ... }));

// rpc/src/eth.rs line 901
let _ = sender.send(MempoolEvent::TxAdded { ... });

// reporters/src/lib.rs line 288
let _ = sender.send(MempoolEvent::TxIncluded { ... });
```

**Current behavior:** If the broadcast channel has no active receivers (all subscribers disconnected) or the channel is closed, these sends fail silently. This means subscription clients may miss transaction lifecycle events (added, evicted, included) with no indication that events were lost.

**Context:** For `tokio::sync::broadcast`, a send failure means either (a) no receivers exist, or (b) the channel is closed. In the first case, the silent discard is arguably acceptable since there is nobody to notify. However, the `let _ =` pattern provides no diagnostic signal at all.

**Fix:**
Replace `let _ =` with:
```rust
if sender.send(event).is_err() {
    debug!("no active subscribers for mempool event");
}
```
This preserves the non-blocking behavior but provides a diagnostic trace. For the higher-traffic paths (txpool), use `trace!` to avoid noise. For the finalization path (reporters), use `debug!` since it fires once per finalized block.

---

### H5. CORS configuration silently drops invalid origins, methods, and headers

**File:** `crates/node/rpc/src/server.rs` (lines 71, 79, 87)

Three `filter_map(...parse().ok())` calls silently discard CORS configuration entries that fail to parse:

```rust
let origins: Vec<_> = config.allowed_origins.iter().filter_map(|o| o.parse().ok()).collect();
let methods: Vec<_> = config.allowed_methods.iter().filter_map(|m| m.parse().ok()).collect();
let headers: Vec<_> = config.allowed_headers.iter().filter_map(|h| h.parse().ok()).collect();
```

If an operator configures `allowed_origins: ["https://example.com", "not a valid origin"]`, the invalid entry is silently dropped. The operator has no way to know their CORS configuration is partially broken.

**Fix:**
```rust
let origins: Vec<_> = config.allowed_origins.iter().filter_map(|o| {
    match o.parse() {
        Ok(origin) => Some(origin),
        Err(err) => {
            warn!(origin = %o, error = %err, "invalid CORS origin in config, skipping");
            None
        }
    }
}).collect();
```

---

## Severity: MEDIUM

These issues reduce operational visibility but do not risk data loss.

---

### M1. Subscription lag drops events without client notification

**File:** `crates/node/rpc/src/subscription.rs` (lines 209-222) -- `recv_broadcast()`

```rust
async fn recv_broadcast<T>(receiver: &mut broadcast::Receiver<T>, subscription: &str) -> Option<T>
where
    T: Clone,
{
    loop {
        match receiver.recv().await {
            Ok(event) => return Some(event),
            Err(RecvError::Lagged(skipped)) => {
                warn!(subscription, skipped, "subscription receiver lagged; skipping events");
            }
            Err(RecvError::Closed) => return None,
        }
    }
}
```

When a subscriber cannot keep up with the broadcast channel, events are silently dropped. The `warn!` is logged server-side but the client is never notified that events were missed. This means WebSocket subscribers under high load will have gaps in their event streams (missing blocks, missing pending transactions) with no way to detect the gap.

**Fix:**
1. After detecting a lag, send a special "events_dropped" notification to the subscriber so it knows to re-sync.
2. Emit a Prometheus counter `kora_subscription_events_dropped_total{subscription="..."}` for monitoring.

---

### M2. Transaction decode in pool silently returns `None` on failure

**File:** `crates/node/txpool/src/pool.rs` (lines 549-551) -- `tx_to_ordered()`

```rust
fn tx_to_ordered(tx: &Tx) -> Option<OrderedTransaction> {
    let envelope = TxEnvelope::decode_2718(&mut tx.bytes.as_ref()).ok()?;
    let sender = recover_sender_from_envelope(&envelope).ok()?;
    // ...
}
```

Two `.ok()?` calls silently discard decode and signature recovery errors. If a malformed transaction somehow reaches the pool, it is silently ignored with no log. This makes debugging pool behavior difficult -- if an operator sees a transaction in the pool that never gets included, there is no log indicating that decode or sender recovery failed.

**Fix:**
```rust
fn tx_to_ordered(tx: &Tx) -> Option<OrderedTransaction> {
    let envelope = match TxEnvelope::decode_2718(&mut tx.bytes.as_ref()) {
        Ok(env) => env,
        Err(err) => {
            debug!(error = %err, "failed to decode transaction envelope in pool");
            return None;
        }
    };
    let sender = match recover_sender_from_envelope(&envelope) {
        Ok(s) => s,
        Err(err) => {
            debug!(error = %err, "failed to recover sender from transaction in pool");
            return None;
        }
    };
    // ...
}
```

---

### M3. Balance query silently discards state database errors

**File:** `crates/node/ledger/src/lib.rs` (line 292)

```rust
pub async fn query_balance(&self, digest: ConsensusDigest, address: Address) -> Option<U256> {
    let snapshot = { /* ... */ }?;
    snapshot.state.balance(&address).await.ok()
}
```

The `.ok()` on the balance query converts any `StateDbError` (lock poisoning, I/O failure) into `None`, which is indistinguishable from "snapshot not found." An operator seeing `None` for a balance query cannot tell whether the state was actually empty or whether the query failed due to a backend error.

**Fix:**
Return `Result<Option<U256>, StateDbError>` instead of `Option<U256>`, or at minimum log the error before converting:
```rust
snapshot.state.balance(&address).await.map_err(|err| {
    warn!(?digest, ?address, error = ?err, "balance query failed");
    err
}).ok()
```

---

### M4. Missing structured fields in log messages

Several log messages lack the structured fields needed for filtering and correlation in log aggregation systems (Loki, Datadog, etc.).

| File | Line | Message | Missing Field |
|------|------|---------|---------------|
| `runner/src/runner.rs` | 86 | `NoOpBlocker: ignoring block request` | No `view` or `height` |
| `runner/src/runner.rs` | 751 | `failed to submit bootstrap transaction to mempool` | No `tx_id`, no error reason |
| `reporters/src/lib.rs` | 762 | `failed to decode finalized transaction for indexing` | No `block_height`, no `tx_index` |
| `reporters/src/lib.rs` | 769 | `failed to recover finalized transaction sender` | No `block_height`, no `tx_hash` |
| `rpc/src/subscription.rs` | 98 | `failed to accept pending transaction subscription` | No client identifier |
| `dkg/src/protocol.rs` | 597 | `Failed to process player ack` | No `dealer` identity, no `player` index |

**Fix:**
Add the missing fields to each log macro invocation. Example for line 751:
```rust
// Before
warn!("failed to submit bootstrap transaction to mempool");

// After
warn!(?tx_id, "failed to submit bootstrap transaction to mempool");
```

---

### M5. No correlation IDs for tracing request flows

**Files:**
- `crates/node/rpc/src/eth.rs` -- RPC handler methods
- `crates/node/runner/src/runner.rs` -- transaction submission path (lines 583-605)
- `crates/node/reporters/src/lib.rs` -- finalization path

There is no correlation ID or request ID threaded through the system. When an RPC transaction is submitted, it generates log entries in three different modules (RPC -> validator -> mempool -> proposer -> executor -> finalizer), but there is no shared identifier to correlate these entries. For a production deployment handling hundreds of transactions per second, tracing a single transaction's lifecycle through the logs requires manually matching transaction hashes across timestamps.

**Fix:**
1. Generate a `tracing::Span` with a `request_id` field at the RPC entry point.
2. Pass the span (or enter it) through the transaction submission, validation, and mempool insertion paths.
3. For block-level operations (propose, verify, finalize), use `block_digest` as the correlation ID and include it in every log line in the path. Most log lines already include `?digest`, but the propose and verify paths do not carry this through their helper functions consistently.

---

### M6. Metrics server bind failure is logged but does not prevent node from starting

**File:** `crates/node/runner/src/runner.rs` (lines 646-657)

```rust
let listener = match tokio::net::TcpListener::bind(metrics_addr).await {
    Ok(l) => l,
    Err(e) => {
        error!(addr = %metrics_addr, error = %e, "Failed to bind metrics server");
        return;  // Silently returns from the spawned task, node continues without metrics
    }
};
```

If the metrics server fails to bind (port conflict, permission denied), the error is logged but the node continues running without any metrics. For a production deployment, running without metrics is a serious operational gap. There is no periodic retry and no health indicator that metrics are unavailable.

**Fix:**
1. Make the metrics server bind a startup prerequisite -- if configured, failure to bind should be a startup error that prevents the node from proceeding.
2. Alternatively, if the metrics server is optional, add a health field to `NodeState` indicating whether metrics are available.

---

## Severity: LOW

Code quality issues that do not affect production behavior but reduce maintainability.

---

### L1. `unwrap_or(0)` for system time could mask clock issues

**Files:**
- `crates/node/runner/src/app.rs` (line 29)
- `crates/node/txpool/src/pool.rs` (line 536)

```rust
// app.rs
fn unix_timestamp_secs<Env: Clock>(env: &Env) -> u64 {
    env.current().duration_since(UNIX_EPOCH).map(|duration| duration.as_secs()).unwrap_or(0)
}

// pool.rs
fn current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
```

If the system clock is before the Unix epoch (e.g., misconfigured NTP), these silently return 0. Timestamp 0 would cause blocks to have incorrect timestamps and transactions to have incorrect insertion times, leading to premature eviction or incorrect TTL calculations.

**Fix:**
Log a warning when the system clock returns a pre-epoch time:
```rust
fn unix_timestamp_secs<Env: Clock>(env: &Env) -> u64 {
    match env.current().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(err) => {
            warn!(error = %err, "system clock is before Unix epoch");
            0
        }
    }
}
```

---

### L2. `seed_for_parent` silently falls back to zero

**File:** `crates/node/runner/src/app.rs` (line 88)

```rust
async fn get_prevrandao(&self, parent_digest: ConsensusDigest) -> B256 {
    self.ledger.seed_for_parent(parent_digest).await.unwrap_or(B256::ZERO)
}
```

If the seed cache does not have an entry for the parent, `prevrandao` silently falls back to `B256::ZERO`. This is deterministic (all validators will get the same zero value) but means EVM contracts relying on `PREVRANDAO` for randomness will get zero, which is a predictable value.

**Fix:**
Add a `debug!` log when falling back to the zero seed, so operators can monitor whether the seed cache is working correctly:
```rust
async fn get_prevrandao(&self, parent_digest: ConsensusDigest) -> B256 {
    match self.ledger.seed_for_parent(parent_digest).await {
        Some(seed) => seed,
        None => {
            debug!(?parent_digest, "seed cache miss, falling back to zero prevrandao");
            B256::ZERO
        }
    }
}
```

---

### L3. Snapshot chain traversal silently produces incomplete excluded sets

**File:** `crates/node/runner/src/app.rs` (lines 276-296) -- `collect_pending_tx_ids()`

```rust
fn collect_pending_tx_ids(
    &self,
    snapshots: &InMemorySnapshotStore<OverlayState<QmdbState>>,
    from: ConsensusDigest,
) -> BTreeSet<kora_consensus::TxId> {
    let mut excluded = BTreeSet::new();
    let mut current = Some(from);

    while let Some(digest) = current {
        if snapshots.is_persisted(&digest) {
            break;
        }
        let Some(snapshot) = snapshots.get(&digest) else {
            break;  // Silently stops traversal on missing snapshot
        };
        excluded.extend(snapshot.tx_ids.iter().copied());
        current = snapshot.parent;
    }

    excluded
}
```

When the snapshot chain has a gap (a snapshot digest is missing), the traversal silently stops and returns an incomplete excluded set. This means transactions from earlier (untraversed) snapshots may be included in the new block as duplicates.

**Fix:**
Add a `debug!` log when the chain traversal encounters a gap:
```rust
let Some(snapshot) = snapshots.get(&digest) else {
    debug!(?digest, "snapshot chain gap during pending tx collection, excluded set may be incomplete");
    break;
};
```

---

### L4. DKG player ack processing logs at `warn` but discards the error details

**File:** `crates/node/dkg/src/protocol.rs` (lines 596-598)

```rust
if let Err(e) = our_dealer.receive_player_ack(player, ack) {
    warn!(?e, "Failed to process player ack");
}
```

The error is logged but there is no `?player` field and no indication of which dealer is affected. During a DKG ceremony with multiple participants, this makes it impossible to diagnose which player-dealer pair had the ack failure.

**Fix:**
```rust
if let Err(e) = our_dealer.receive_player_ack(player.clone(), ack) {
    warn!(?player, error = ?e, "failed to process player ack for our dealer");
}
```

---

## Cross-cutting: missing Prometheus metrics

The following operations have no Prometheus counter or histogram, making it impossible to set up alerts or dashboards for failure rates:

| Operation | Suggested Metric |
|-----------|-----------------|
| Finalization failure (by cause) | `kora_finalization_failures_total{reason="execution\|root\|snapshot\|persist\|mismatch"}` |
| Block proposal skip (by cause) | `kora_proposal_skips_total{reason="no_snapshot\|execution\|root"}` |
| State root mismatch | `kora_state_root_mismatches_total{path="verify\|finalize"}` |
| Subscription events dropped | `kora_subscription_events_dropped_total{subscription="..."}` |
| Transaction pool operations | `kora_txpool_inserts_total`, `kora_txpool_evictions_total`, `kora_txpool_rejections_total{reason="..."}` |
| DKG message failures | `kora_dkg_message_failures_total{kind="..."}` |
| RPC server task failures | `kora_rpc_task_failures_total{server="http\|jsonrpc"}` |

---

## Summary table

| ID | Severity | File | Issue | Effort |
|----|----------|------|-------|--------|
| C1 | Critical | `reporters/src/lib.rs` | Finalization `Err(())` no retry, no typed error | 4h |
| C3 | Critical | `runner/src/runner.rs` | `abort()` with no graceful shutdown | 4h |
| C4 | Critical | `runner/src/app.rs:238`, `reporters/src/lib.rs:220` | State root mismatch logged at `warn` instead of `error` | 15m |
| C5 | Critical | `runner/src/app.rs:205` | `verify_block` warn-level log floods during catch-up | 15m |
| C6 | Critical | `rpc/src/server.rs:558` | `RpcServerHandle` discards `JoinError`s from spawned tasks | 30m |
| H1 | High | `runner/src/app.rs` | Steady-state `warn!` floods (35/sec) in propose path | 30m |
| H2 | High | `runner/src/app.rs` | Undifferentiated `build_block` failures | 2h |
| H3 | High | `consensus/src/error.rs` | `ConsensusError::Execution(String)` erases type | 1h |
| H4 | High | `txpool/src/pool.rs`, `rpc/src/eth.rs`, `reporters/src/lib.rs` | Silent `let _ =` on broadcast sends | 1h |
| H5 | High | `rpc/src/server.rs` | CORS parse failures silently dropped | 30m |
| M1 | Medium | `rpc/src/subscription.rs` | Subscription lag drops without client notification | 2h |
| M2 | Medium | `txpool/src/pool.rs` | `.ok()?` on decode/recovery in pool | 30m |
| M3 | Medium | `ledger/src/lib.rs` | `.ok()` on balance query erases DB errors | 30m |
| M4 | Medium | Multiple | Missing structured fields in log messages | 1h |
| M5 | Medium | Multiple | No correlation IDs for request tracing | 4h |
| M6 | Medium | `runner/src/runner.rs` | Metrics server bind failure is non-fatal | 1h |
| L1 | Low | `runner/src/app.rs`, `txpool/src/pool.rs` | `unwrap_or(0)` for system time | 15m |
| L2 | Low | `runner/src/app.rs` | Silent zero seed fallback | 15m |
| L3 | Low | `runner/src/app.rs` | Incomplete excluded set on chain gap | 15m |
| L4 | Low | `dkg/src/protocol.rs` | Player ack error log missing context fields | 15m |

---

## Relationship to existing PRs

### PR #129 -- "log errors in critical paths instead of silently discarding"

PR #129 (commit `4372409`, merged 2026-05-22) addressed three specific silent error paths:

1. **`crates/node/runner/src/app.rs`**: `compute_root_from_store()` in `build_block()` previously used `.ok()?` to silently discard root computation errors. PR #129 replaced this with a `match` that logs at `warn!` with structured fields (`parent`, `height`, `error`). This is the fix reflected in the current code at lines 158-172.

2. **`crates/node/runner/src/runner.rs`**: Bootstrap transaction submission previously used `let _ = ledger.submit_tx(tx.clone()).await`. PR #129 replaced this with `if !ledger.submit_tx(...).await { warn!(...) }`. This is the fix reflected in the current code at line 547.

3. **`crates/node/rpc/src/eth.rs`**: `block_by_number_or_none()` in the gas oracle fee history helper previously used `.ok().flatten()` to silently discard block fetch errors. PR #129 replaced this with a `match` that logs at `warn!`.

**Remaining gaps not covered by PR #129:**
- The `let _ = sender.send(...)` patterns in txpool, RPC, and reporters (H4)
- Transaction decode `.ok()?` in pool (M2)
- Balance query `.ok()` in ledger (M3)
- CORS parse `.ok()` in server (H5)
- `RpcServerHandle` `JoinError` discarding (C6)
- All log level misclassifications (C4, C5, H1)

### PR #131 -- "prevent resolver from permanently blocking peers after restart"

PR #131 fixed the resolver peer-blocking bug, but the interaction between C3 (abort with no cleanup) and the resolver catch-up failure remains: if the watchdog triggers, the restart puts the node in a state that may not recover without the full cluster restarting.

### PR #122 -- "skip invalid transactions instead of aborting block"

PR #122 added the `warn!` for NonceTooHigh skips, which is correctly at `warn` level since it fires infrequently per block.

---

## Removed findings

### ~~C2. RwLock poisoning risk in consensus snapshot store~~ (INVALID)

This finding was removed after audit. The original claim was that `InMemorySnapshotStore` in `crates/node/consensus/src/components/snapshot.rs` uses `std::sync::RwLock`, which can poison on panic. **This is incorrect.** The production code uses `parking_lot::RwLock` (imported on line 10 of `snapshot.rs`), which does not have poisoning semantics. All production consensus components (`snapshot.rs`, `mempool.rs`, `seed.rs`) use `parking_lot::RwLock`. The only usage of `std::sync::RwLock` in the consensus module is in the test mocks in `proposal.rs` (line 199), where poisoning is irrelevant. No code change is needed.

---

## Suggested implementation order

1. **C4 + C5 + H1** (log level fixes) -- highest impact, lowest effort, immediately fixes log usability and alert quality
2. **C6** (RpcServerHandle JoinError logging) -- small change, prevents silent task panics
3. **C1** (finalization retry + typed errors) -- highest risk, prevents silent state divergence
4. **H2 + H3** (typed build_block errors + ConsensusError fix) -- improves all error paths
5. **H4 + H5 + M2 + M3** (silent discard fixes) -- batch of small changes
6. **C3** (graceful shutdown) -- significant refactor but important for production resilience
7. **M4 + M5** (structured logging + correlation IDs) -- observability polish
8. **Metrics** -- add counters for all failure paths identified above
