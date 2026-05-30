# Logging and Error Handling Analysis

## 1. Logging Infrastructure

### Setup

**File:** `bin/kora/src/main.rs` (Lines 12-18)

```rust
kora_cli::Backtracing::enable();
kora_cli::SigsegvHandler::install();

tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer())
    .with(tracing_subscriber::EnvFilter::from_default_env())
    .init();
```

- Uses `tracing` + `tracing_subscriber`
- `RUST_LOG` environment variable controls filtering (default: `info`)
- No structured JSON output (text format only)
- No file-based logging (stdout only, Docker captures)
- Backtrace support enabled
- SIGSEGV handler installed for crash diagnostics

### Key Logging Locations

#### Runner (`crates/node/runner/src/runner.rs`)
| Level | Line | Message |
|-------|------|---------|
| info | 300 | Runtime startup with storage directory |
| info | 332 | Validator startup with chain ID |
| info | 338-341 | Primary/secondary peers registered |
| info | 441 | RPC server startup |
| info | 472-474 | Prometheus metrics startup/error |
| info | 576 | "Validator started successfully" |
| debug | 419 | Transaction inserted into mempool |
| warn | 415 | RPC validator rejected transaction |
| trace | 136 | Transaction accepted into mempool |
| trace | 139 | Seed cache refreshed |
| trace | 142 | Snapshot persisted |

#### Application (`crates/node/runner/src/app.rs`)
| Level | Line | Message |
|-------|------|---------|
| warn | 103-108 | Mempool has unincluded txs but empty block |
| warn | 128-134 | Block execution failed |
| warn | 177 | Missing parent snapshot (verify) |
| warn | 190 | Verification execution failed |
| warn | 204 | Root computation failed |
| warn | 211-216 | State root mismatch |
| debug | 153-162 | Block built (with timing breakdown) |
| debug | 235-244 | Block verified (with timing breakdown) |
| debug | 307-313 | Proposal complete (with timing) |
| trace | 110-116 | Mempool drain stats |
| trace | 172 | Block already verified |

#### FinalizedReporter (`crates/node/reporters/src/lib.rs`)
| Level | Line | Message |
|-------|------|---------|
| error | 143 | Finalized block execution failed |
| error | 155 | QMDB root computation failed |
| error | 197 | Parent snapshot missing |
| error | 211 | Persist task failed |
| error | 217 | Persist failed |
| warn | 161-166 | State root mismatch for finalized block |
| warn | 191-195 | Parent snapshot missing (cached block) |
| trace | 126-129 | Re-executing finalized block |
| trace | 202 | Using cached snapshot |

---

## 2. Logging Gaps

### Critical Missing Logs

1. **No log when finalization completes successfully**: The `FinalizedReporter` processes blocks but has no `info!` or `debug!` confirming successful finalization + persistence + pruning. You only see errors/warnings.

2. **No info-level log when block is built**: Block building only logs at `debug` level (with timing). Under normal `RUST_LOG=info`, you can't see block production.

3. **No log for mempool pruning**: When `prune_mempool()` is called, there's no log indicating how many transactions were pruned.

4. **No structured entry/exit logging**: Major phases (propose, verify, finalize) lack structured span-based logging. No `tracing::instrument` annotations found in production code.

5. **No RPC request logging**: Individual RPC calls aren't logged. There's no middleware for request/response logging.

6. **No P2P message logging**: No application-level logging of P2P message exchange (only Commonware internal logging).

### Recommended Additions

```rust
// After successful finalization:
info!(?digest, height, txs = block.txs.len(), "block finalized and persisted");

// After mempool pruning:
debug!(pruned = tx_ids.len(), remaining = mempool.len(), "mempool pruned");

// On successful block build:
info!(height, txs = block.txs.len(), exec_ms, "block built");

// RPC middleware:
debug!(method, duration_ms, "RPC request");
```

---

## 3. Error Handling Patterns

### Dangerous Patterns Found

#### Silent Error Swallowing

1. **Bootstrap transaction failures** — `runner.rs:545`
```rust
let _ = ledger.submit_tx(tx.clone()).await;
```
Silently ignores bootstrap transaction submission failures. Validators could start with incomplete genesis state.

2. **Metrics server crash** — `runner.rs:473`
```rust
if let Err(e) = axum::serve(listener, app).await {
    error!("metrics server error: {}", e);
}
```
Logs error but metrics become permanently unavailable.

#### Unwrap/Expect in Production

Found 1 dangerous `expect` in production path:

- `runner.rs:399`: `.expect("block index is initialized with RPC")` — panics if RPC is configured but block index initialization fails. All other `unwrap()`/`expect()` calls are in tests or config defaults (hard-coded string parsing).

#### Config Default Parsing

Multiple `.parse().unwrap()` calls in config defaults (rpc/config.rs:65-66, 159-160, 167-168, 179). These parse hardcoded IP addresses and should always succeed, but it's a fragile pattern.

### Error Type Hierarchy

#### ExecutionError (`crates/node/executor/src/error.rs`)
```rust
pub enum ExecutionError {
    State(StateDbError),        // Database read failure
    TxDecode(String),           // RLP/envelope decode failure
    TxExecution(String),        // REVM execution failure
    Revert(Bytes),              // EVM revert with data
    InvalidTx(String),          // Transaction validation
    BlockValidation(String),    // Block-level validation
    CodeNotFound(B256),         // Missing contract code
}
```

**Problem:** All variants are treated as equally fatal (abort block). No distinction between "skip this tx" vs "system failure".

#### ConfigError (`crates/node/config/src/error.rs`)
```rust
pub enum ConfigError {
    Read { path, error },              // File I/O
    TomlParse(toml::de::Error),        // TOML parse
    JsonParse(serde_json::Error),      // JSON parse
    TomlSerialize(toml::ser::Error),   // TOML serialize
    InvalidKeyLength { got, expected }, // Key validation
    Write { path, error },             // File write
    CreateDir { path, error },         // Dir creation
    InvalidParticipantKeyLength,       // Participant key
    InvalidParticipantKey(String),     // Key parse
}
```

Well-structured config errors with specific variants and context.

#### TxPoolError (`crates/node/txpool/src/`)
```rust
pub enum TxPoolError {
    AlreadyExists,
    SenderFull(Address),
    TxTooLarge { size, max },
    DecodeError(String),
    InvalidChainId { got, expected },
    InvalidSignature,
    GasPriceTooLow { price, min },
    IntrinsicGasTooLow { limit, intrinsic },
    NonceTooLow { got, expected },
    NonceGap { got, expected },
    InsufficientBalance { need, have },
    StateError(String),
}
```

Good error variants with context. Used by `TransactionValidator`.

---

## 4. Error Handling Strengths

1. **Config errors are properly typed** with specific variants and file path context
2. **Runner wraps errors** via `RunnerError` → `anyhow::Error`
3. **Most production code uses `Result<>`** with proper propagation
4. **Tests use explicit `.unwrap()`/`.expect()`** — clean separation from production
5. **LedgerResult** type alias provides consistent error handling in ledger layer

---

## 5. Benchmark Infrastructure

### Current State: MINIMAL

**What exists:**
- `Cargo.toml` has `[profile.maxperf]` for optimized builds
- Timing logs in `app.rs` (snapshot_ms, exec_ms, root_ms, total_ms)
- `bin/loadgen/` for external load testing
- Commonware runtime metrics for basic performance visibility

**What's missing:**
- No criterion.rs benchmarks
- No flamegraph configuration
- No microbenchmarks for critical paths (ECDSA recovery, state root computation, REVM execution)
- No integration benchmarks (end-to-end block production throughput)
- No profiling infrastructure

### Recommended Benchmarks

| Benchmark | Target | Purpose |
|-----------|--------|---------|
| `bench_ecdsa_recovery` | `tx_order_key()` | Measure ECDSA cost per tx in build() |
| `bench_block_execution` | `RevmExecutor::execute()` | Execution throughput |
| `bench_state_root` | `QmdbStateRoot::transition()` | Root computation scaling |
| `bench_mempool_build` | `InMemoryMempool::build()` | Mempool bottleneck |
| `bench_overlay_read` | `OverlayState::nonce/balance/storage` | State access latency |
| `bench_changeset_merge` | `ChangeSet::merge()` | Merge overhead |
| `bench_qmdb_commit` | `QmdbLedger::commit_changes()` | Persistence throughput |
| `bench_e2e_block` | Full pipeline | End-to-end block production |
