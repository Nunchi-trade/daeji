# Kora Error Handling Patterns

## What is Kora?

Kora is an EVM-compatible blockchain node built in Rust. It uses a threshold-BLS consensus protocol (Simplex) with REVM for transaction execution, a custom state database (QMDB), and supports standard Ethereum JSON-RPC. The system is composed of several subsystems: executor, consensus, mempool (txpool), RPC, DKG (distributed key generation), storage, and a runner that wires everything together.

## Why Error Handling is Critical in Blockchain

In a blockchain validator node, error handling is existentially important:

- **One panic = chain death.** If a validator panics during block proposal or voting, it cannot produce or vote on blocks. In a BFT system with threshold `t`, losing validators beyond the fault tolerance causes the chain to halt entirely.
- **Silent errors = state divergence.** If an error is swallowed and execution proceeds with incorrect state, validators may disagree on state roots, causing consensus forks or permanent chain splits.
- **Unhandled execution errors = lost blocks.** If a single transaction error aborts an entire block proposal, the chain produces empty blocks even when there are valid transactions waiting.

The correct philosophy for a production blockchain is:
- Execution errors (single tx) should NEVER kill a block proposal. Skip the failing tx and continue.
- Consensus protocol errors should be logged and retried, not panicked on.
- Storage write failures are genuinely fatal -- if the database cannot be written, the node must halt gracefully.
- Network errors should be retried with backoff, never fatal.

---

## Error Handling Philosophy: What SHOULD Happen vs. What DOES Happen

| Scenario | Should Happen | What Actually Happens |
|----------|--------------|----------------------|
| Single tx fails during block execution | Skip tx, include remaining txs in block | `?` propagates error, entire block proposal fails |
| Parent snapshot not found during proposal | Return None (skip slot) | Returns `None` -- handled correctly |
| State root computation fails | Log error, skip slot | `.ok()?` silently discards error context |
| Network message send fails (DKG) | Log and retry | `let _ =` discards the result silently |
| Mempool tx decode fails | Reject tx gracefully | Returns `false` -- handled correctly |
| RPC request fails internally | Return JSON-RPC error | Mapped via `Into<RpcError>` -- handled correctly |
| Storage persist fails at finalization | Log fatal, halt | Logs error and skips block -- chain may diverge |

---

## Critical Error Paths (Can Kill the Chain)

### 1. Executor `?` Operator -- P0 Bug (Block Proposal Abort)

**File:** `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`, lines 391-395

```rust
let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;
evm.set_tx(tx_env);

let result_and_state =
    evm.replay().map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;
```

The `execute()` method iterates over all transactions in a block (`for tx_bytes in txs`). If **any single transaction** fails to decode or fails during EVM execution, the `?` operator propagates the error, aborting the **entire block**. This means:
- A single malformed transaction poisons the whole batch.
- The block proposal returns `Err(ExecutionError)`.
- The caller in the consensus application (`app.rs` line 125) catches this and returns `None`, meaning no block is proposed for that slot.

**Impact:** In a live network with adversarial or buggy transactions, a single bad tx can cause the chain to miss slots indefinitely.

**Where the error surfaces:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs`, lines 125-136:
```rust
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        warn!(/* ... */ "build_block: execution failed");
        return None;
    }
};
```

### 2. Silent State Root Failure in Block Proposal

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs`, line 145

```rust
let state_root = self
    .ledger
    .compute_root_from_store(parent_digest, outcome.changes.clone())
    .await
    .ok()?;
```

The `.ok()?` pattern converts the `Result` to `Option`, discarding the error entirely, then uses `?` on the `Option` to return `None` from `build_block`. No error context is logged. If the QMDB root computation fails (disk corruption, OOM), the node silently skips block production with zero diagnostic information.

### 3. Finalization Persist Failure

**File:** `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs`, lines 208-220

```rust
let persist_result = match persist_handle.await {
    Ok(result) => result,
    Err(err) => {
        error!(?digest, error = ?err, "persist task failed");
        ack.acknowledge();
        return;
    }
};
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    ack.acknowledge();
    return;
}
```

When a finalized block fails to persist, the error is logged and the block is acknowledged to consensus (so the delivery floor advances), but the state is never persisted. This means:
- The consensus layer believes the block is processed.
- The state database does NOT contain the finalized state.
- On restart, the node may not be able to recover this block's state.

### 4. DKG Ceremony -- `SystemTime::now().unwrap()`

**File:** `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs`, line 79

```rust
let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
```

If the system clock is set before the UNIX epoch (extremely rare but possible in containers or VMs with misconfigured clocks), this will panic and kill the DKG ceremony process.

---

## Error Handling by Subsystem

### Executor

**Pattern:** Uses `?` operator throughout `BlockExecutor::execute()`.

The REVM executor at `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` line 357-411 implements block execution. The critical problem is that the iteration over transactions uses `?` on both `decode_tx_env()` (line 391) and `evm.replay()` (line 395). Any single failure aborts the entire block.

The `simulate_call` and `estimate_gas` methods (lines 209-297) handle errors properly -- they map errors to `ExecutionError` variants and propagate them appropriately since those are single-operation RPCs where failure is expected.

**Verdict:** Broken for production block building. Correct for RPC simulation.

### Consensus

**Pattern:** Returns `Result<_, ConsensusError>` from all critical operations.

The consensus layer (`/Users/will/dev/nunchi/daeji/crates/node/consensus/src/`) uses proper error propagation:
- `proposal.rs` line 100-103: Maps execution errors via `.map_err(|e| ConsensusError::Execution(e.to_string()))`.
- `ledger.rs` line 176-191: `persist_snapshot` properly propagates state DB errors.
- `application.rs`: The trait defines `Result` returns for `propose`, `verify`, and `finalize`.

However, the actual runtime integration in `app.rs` converts all errors to `Option<Block>`:
- Proposal errors become `None` (no block proposed).
- Verification errors become `false` (block rejected).

**Verdict:** Error types are well-defined. The runtime integration swallows error details.

### Mempool (TxPool)

**Pattern:** Returns structured `TxPoolError` on validation failure. Never panics in production code.

The transaction validator at `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/validator.rs` returns detailed error variants:
- `TxPoolError::TxTooLarge`
- `TxPoolError::InvalidChainId`
- `TxPoolError::GasPriceTooLow`
- `TxPoolError::IntrinsicGasTooLow`
- `TxPoolError::NonceTooLow`
- `TxPoolError::NonceGap`
- `TxPoolError::InsufficientBalance`
- `TxPoolError::DecodeError`
- `TxPoolError::InvalidSignature`

The `Mempool` trait implementation in `pool.rs` (lines 301-313) gracefully handles failures:
```rust
fn insert(&self, tx: Tx) -> bool {
    let Some(ordered) = tx_to_ordered(&tx) else {
        trace!("failed to decode transaction for mempool insert");
        return false;
    };
    match self.add(ordered) {
        Ok(()) => true,
        Err(e) => {
            trace!(?e, "failed to insert transaction");
            false
        }
    }
}
```

**Verdict:** Well-designed. Errors are handled gracefully without crashing.

### RPC

**Pattern:** Uses `jsonrpsee::RpcResult<T>` with `RpcError` mapped to JSON-RPC error codes.

The RPC layer at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs` properly:
- Maps provider errors to `RpcResult` via `Into<jsonrpsee::types::ErrorObjectOwned>`.
- Returns structured JSON-RPC errors to clients.
- Never panics in request handlers.

The server startup at `server.rs` handles bind failures gracefully (logs error, returns from spawned task without crashing the main process).

**Verdict:** Correct for production. The one notable issue is that `send_raw_transaction` silently succeeds even when `tx_submit` is `None` (documented as a known bug in the test at line 666-689).

### DKG

**Pattern:** Returns `DkgError` from operations. Network failures are logged and tolerated. Timeouts are explicit.

Critical observations:
- **Silent `.ok()` on recv:** `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/transport.rs` line 250: `self.receiver.recv().await.ok().map(...)` -- if the receiver channel errors, the message is silently dropped. No log, no retry.
- **`let _ =` on send:** `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` line 346: `let _ = network.send_to(leader_pk, &request_msg);` -- failed log requests are silently discarded.
- **Timeout-based phase completion:** Phases 2 and 4 use hardcoded 120-second timeouts. If network conditions are poor, the DKG may timeout and fail permanently without a retry mechanism at the ceremony level.
- **`.ok()` on socket timeout:** `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/network.rs` line 124: `stream.set_read_timeout(Some(Duration::from_secs(5))).ok();` -- if the socket option fails to set, reads may block indefinitely.

**Verdict:** Network failures are tolerated (DKG is designed to retry within phases), but the ceremony-level failure has no auto-retry. A failed DKG requires manual operator intervention.

---

## Patterns Found

### Counts (Production Code vs. Tests)

Analysis of `unwrap()` calls in `crates/node/`:

| Location | unwrap() in production code | unwrap() in test code |
|----------|---------------------------|---------------------|
| `executor/src/revm.rs` | 0 | ~8 |
| `consensus/src/` | 0 | ~15 |
| `rpc/src/` | 0 | ~40 |
| `txpool/src/` | 0 | ~30 |
| `dkg/src/ceremony.rs` | 1 (SystemTime) | 0 |
| `dkg/src/protocol.rs` | 3 (byte slice conversions, NonZeroU32) | 0 |
| `runner/src/runner.rs` | 0 | 0 |
| `config/src/` | 0 | ~15 |

**Total production `unwrap()` calls in `crates/node/`:** ~4 (all in DKG module)

### Silent Error Discarding (`.ok()` usage in production)

| File | Line | Pattern | Risk |
|------|------|---------|------|
| `runner/src/app.rs` | 145 | `.ok()?` on state root computation | HIGH -- silent block skip with no diagnostics |
| `dkg/src/transport.rs` | 250 | `.ok().map(...)` on channel recv | MEDIUM -- lost DKG messages |
| `dkg/src/network.rs` | 124 | `.ok()` on set_read_timeout | LOW -- may cause indefinite blocking |
| `dkg/src/state.rs` | 158 | `.ok()` on hex decode | LOW -- skip corrupt persisted state |
| `dkg/src/state.rs` | 170 | `.ok()` on hex decode | LOW -- skip corrupt log entries |
| `txpool/src/pool.rs` | 278-279 | `.ok()?` on tx decode/sender recovery | LOW -- tx silently dropped from mempool build (traced at call site) |
| `rpc/src/server.rs` | 48,56,64 | `.ok()` inside filter_map on parse | LOW -- invalid CORS entries skipped |
| `ledger/src/lib.rs` | 184 | `.ok()` on balance query | MEDIUM -- RPC may return stale/missing data |

### `let _ =` Patterns (Intentionally Discarded Results in Production)

| File | Line | What is discarded | Risk |
|------|------|------------------|------|
| `runner/src/runner.rs` | 545 | `ledger.submit_tx()` for bootstrap txs | LOW -- bootstrap txs may already exist |
| `dkg/src/ceremony.rs` | 346 | `network.send_to()` for log requests | LOW -- will be retried on next loop |
| `rpc/src/server.rs` | 303 | `tokio::join!()` result in `stopped()` | LOW -- server is shutting down anyway |

---

## Specific Problematic Patterns with File References

### P0: Executor `?` Kills Entire Block

**Location:** `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs:391`

The `decode_tx_env` call uses `?` inside a `for tx_bytes in txs` loop. A single RLP-malformed transaction in the mempool batch will cause the entire `execute()` call to return `Err`. The caller loses all valid transactions that preceded and followed the bad one.

### P1: Silent State Root Failure

**Location:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:145`

```rust
.ok()?;
```

This discards the actual error from `compute_root_from_store`. Could be: disk full, QMDB corruption, serialization bug. The operator sees nothing in logs.

### P2: Finalization Persistence Failure Acks Without Persisting

**Location:** `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:216-220`

The error path calls `ack.acknowledge()` and returns. The consensus layer advances its delivery floor, believing the block was processed. But the state was never committed. On restart, this block's state may be unrecoverable.

### P3: DKG unwrap on SystemTime

**Location:** `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs:79`

```rust
SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
```

Panics if system clock is before epoch. While unlikely, this is in a production code path that runs during validator initialization.

### P4: DKG Protocol Byte Slice Unwraps

**Location:** `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs:76-77`

```rust
let chain_id = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
let round = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
```

These are safe in practice (the slices are exactly the right length) but a defense-in-depth approach would use `map_err` to return a protocol error instead of panicking.

---

## Errors That Can Kill the Chain vs. Errors Handled Safely

| Error | Severity | Can Kill Chain? | Current Handling |
|-------|----------|-----------------|-----------------|
| Single tx decode failure in executor | P0 | YES -- aborts block | `?` propagates, block proposal returns None |
| Single tx EVM execution error | P0 | YES -- aborts block | `?` propagates, block proposal returns None |
| State root computation failure | P1 | YES -- silent slot skip | `.ok()?` discards error, returns None |
| Finalization persist failure | P1 | YES -- state divergence on restart | Logged, acked, not persisted |
| Parent snapshot not found | P2 | Partial -- slot skipped | Returns None, logged as warning |
| DKG SystemTime panic | P2 | YES -- kills DKG process | `unwrap()` |
| Mempool tx validation failure | Safe | No | Returns error, tx rejected |
| RPC internal error | Safe | No | Returns JSON-RPC error |
| DKG network send failure | Safe | No | Logged, retried |
| DKG phase timeout | Safe | No | Returns `DkgError::Timeout` |
| Consensus verification mismatch | Safe | No | Returns false, block rejected |
| Storage overlay read miss | Safe | No | Falls through to base state |
| TxPool duplicate insert | Safe | No | Returns `TxPoolError::AlreadyExists` |

---

## Recommendations

### 1. Replace `?` in Executor with Skip-and-Continue (P0 Fix)

In `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`, the `execute()` method should change from:

```rust
for tx_bytes in txs {
    let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;
    let result_and_state = evm.replay().map_err(...)?;
    // ...
}
```

To:

```rust
for tx_bytes in txs {
    let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
        Ok(env) => env,
        Err(e) => {
            tracing::warn!(error = ?e, "skipping undecoded transaction");
            continue;
        }
    };
    evm.set_tx(tx_env);
    let result_and_state = match evm.replay() {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!(error = ?e, "skipping failed transaction");
            continue;
        }
    };
    // ... process receipt and changes
}
```

This ensures that a single bad transaction never poisons the entire block.

### 2. Add Logging for All Silently Discarded Errors

Replace every `.ok()?` in production code with explicit error logging:

```rust
// Before (app.rs:141-145):
let state_root = self.ledger
    .compute_root_from_store(parent_digest, outcome.changes.clone())
    .await
    .ok()?;

// After:
let state_root = match self.ledger
    .compute_root_from_store(parent_digest, outcome.changes.clone())
    .await
{
    Ok(root) => root,
    Err(err) => {
        warn!(?parent_digest, error = ?err, "failed to compute state root");
        return None;
    }
};
```

### 3. Add Panic Hooks for Actor Recovery

Install a panic hook at the top level that:
- Logs the panic with full backtrace to structured logging.
- Attempts to gracefully shut down the consensus engine before exit.
- Exits with a non-zero code so orchestrators (systemd, Kubernetes) restart the node.

```rust
std::panic::set_hook(Box::new(|info| {
    tracing::error!(panic = %info, "FATAL: unrecoverable panic in validator");
    std::process::exit(1);
}));
```

### 4. Do Not Ack Blocks That Fail to Persist

In the finalized reporter (`reporters/src/lib.rs`), if persistence fails, the block should NOT be acknowledged. This will cause the marshal layer to redeliver the block on the next attempt. The current behavior of acking then returning creates an irrecoverable gap.

### 5. Replace DKG `unwrap()` Calls with Proper Error Handling

- `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` should use `.unwrap_or_default()` or return `DkgError::Internal`.
- Byte slice conversions in `protocol.rs` should use `try_into().map_err(|_| DkgError::Protocol("malformed message"))`.

### 6. Add Structured Error Context to All Error Variants

Several error types use `String` for context (e.g., `ConsensusError::Execution(String)`). These should carry the original error via `#[source]` or use `anyhow` for the internal context while keeping typed variants for the public API.

---

## Summary

Kora's error handling is generally well-structured at the type level -- each subsystem defines its own error enum, and most boundaries properly map between error types. The critical gap is in the **executor loop** where the `?` operator makes single-transaction failures fatal to entire blocks. This is the highest-priority fix for production readiness. Secondary concerns are the silent error discarding via `.ok()` in block building and the finalization ack-before-persist pattern that can cause state divergence.
