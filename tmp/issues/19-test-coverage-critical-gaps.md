# Testing: Critical coverage gaps across multiple crates, including zero tests for finalization pipeline

**Severity:** High
**Components:** `kora-reporters`, `kora-executor`, `kora-runner`, `kora-e2e`
**Affects:** Production reliability, regression detection, developer confidence

---

## Summary

Multiple crates in Kora have critical test coverage gaps. Most notably, the `kora-reporters` crate -- which is responsible for the entire block finalization pipeline including snapshot persistence, mempool pruning, and RPC indexing -- has **zero tests**. The executor has no test that signs and submits a real EIP-1559 transaction to verify balance changes end-to-end. The runner has no wiring verification test to confirm that the RPC `tx_submit` callback is actually connected to the mempool. Several known production bugs (transaction silent drop, resolver catch-up failure) existed in code paths that had no test coverage.

---

## Background

Kora is an EVM-compatible blockchain built on the Commonware consensus framework (Simplex BFT). It uses REVM for EVM execution, a custom transaction pool, and a QMDB-backed state database. The project is organized as a Cargo workspace with crates under `crates/node/` (executor, consensus, txpool, rpc, reporters, ledger, runner, dkg), `crates/e2e/` (end-to-end tests), and supporting crates under `crates/storage/` and `crates/network/`.

### Test Infrastructure

Kora has three testing layers:

1. **Unit tests** (`#[test]` / `#[tokio::test]` in `mod tests` blocks within source files): Test individual functions and types in isolation using mock implementations.
2. **Integration tests** (files under `crates/*/tests/`): Test a crate's public API with more realistic setups. Currently only `kora-executor` has integration tests (`crates/node/executor/tests/executor.rs`).
3. **End-to-end tests** (`crates/e2e/`): Spin up a multi-validator network using `kora-transport-sim` (simulated network transport) and run consensus rounds with transaction execution. These use a `TestHarness` that configures validators, link conditions, and test scenarios.

### How to Run Tests

```bash
# Run all tests (default method)
just test
# Which expands to:
cargo nextest run --workspace --all-features

# Run tests for a specific crate
cargo nextest run -p kora-executor

# Run ignored e2e tests (must be single-threaded)
cargo nextest run -p kora-e2e --run-ignored all -- --test-threads=1

# Run a specific test
cargo nextest run -p kora-executor -- test_execute_empty_transactions
```

---

## Current Test Count by Crate

Counts verified by grepping for `#[test]`, `#[tokio::test]`, and `#[rstest]` (rstest cases counted individually) on 2026-05-21.

| Crate | Unit Tests | Integration Tests | Notes |
|-------|-----------|------------------|-------|
| `kora-executor` | 40 | 40 | No real signed tx test; integration tests in `tests/executor.rs` only test empty blocks, mock state DB infra, header validation, and block context creation. Uses `rstest` for parameterized cases (4 rstests expanding to 17 cases). |
| `kora-consensus` | ~43 | 0 | Good coverage of proposal builder, ledger, error types |
| `kora-txpool` | ~59 | 0 | Pool, validator, ordering, config, error all covered |
| `kora-rpc` | ~91 | 0 | State, provider, eth, server, config, types, error |
| `kora-dkg` | ~35 | 8 (in `src/tests.rs`) | Config, transport, error well-tested; core DKG ceremony has integration-style tests in `src/tests.rs` |
| **`kora-reporters`** | **0** | **0** | **ZERO tests for finalization pipeline** |
| `kora-ledger` | ~6 | 0 | Minimal coverage |
| `kora-runner` | 8 | 0 | 3 tests for `runtime_storage_directory_from()` in `runner.rs`; 5 tests for error formatting in `error.rs` |
| `kora-e2e` | 0 | 26 | 18 are `#[ignore]`'d (15 "flaky when run in parallel", 1 "requires investigation", 2 "slow stress test") |

**Breakdown of executor unit tests (40 total):** `revm.rs` (18), `error.rs` (9), `config.rs` (4), `validation.rs` (4), `context.rs` (3), `adapter.rs` (1), `outcome.rs` (1).

**Breakdown of executor integration tests (40 total in `tests/executor.rs`):** 23 plain `#[test]`/`#[tokio::test]` + 4 `#[rstest]` expanding to 17 parameterized cases (6+3+4+4). Categories: chain ID creation (7), empty tx execution (4), header validation (6), block context (6), mock state DB infrastructure (15), populated-state empty-tx execution (1), merge_changes (1).

**E2e test breakdown:** `consensus.rs` (9 tests, 4 ignored), `execution.rs` (9 tests, 8 ignored), `resilience.rs` (8 tests, 6 ignored). Active tests (8 total): 4-validator consensus, state root convergence, seed convergence, sequential block production, empty blocks, gas limit enforcement, network jitter, sustained throughput.

---

## CRITICAL GAP 1: `kora-reporters` -- ZERO Tests

**Location:** `crates/node/reporters/src/lib.rs` (476 lines, 0 tests)

Verified: `grep -r "#[test]" crates/node/reporters/` returns zero matches. There is no `#[cfg(test)]` module anywhere in this crate.

This crate contains the three reporters that are the backbone of the post-consensus pipeline:

### `FinalizedReporter` (struct at lines 363-405, `Reporter` impl at lines 407-424)
The struct holds `LedgerService`, `tokio::Context`, `BlockExecutor`, `BlockContextProvider`, and an optional `BlockIndex`. Its `report()` method (line 414) delegates to the free function `handle_finalized_update()` (lines 105-232), which contains all the logic:
1. Matches on `Update::Block` vs `Update::Tip` (line 116-118; Tip is a no-op)
2. Checks if a snapshot already exists for the finalized digest (line 120)
3. If snapshot is missing OR `block_index` is `Some`, re-executes the block against the parent snapshot using `BlockExecution::execute()` (lines 133-147)
4. Computes the QMDB state root via `compute_root_from_store()` and validates it matches `block.state_root` (lines 149-169)
5. If snapshot was missing, inserts the new snapshot with merged overlay state (lines 171-186)
6. Spawns `persist_snapshot()` on a shared task (lines 204-215) and handles errors (lines 216-220)
7. If `block_index`, `execution_outcome`, and `execution_context` are all present, calls `index_finalized_block()` (lines 221-225)
8. Prunes executed transactions from the mempool (line 226)
9. Acknowledges the update so Marshal can advance the delivery floor (line 229)

A bug in this code path caused a production issue where transactions were accepted via RPC but never included in finalized blocks -- the mempool was not being pruned because finalization was silently erroring.

### `SeedReporter` (struct at lines 65-89, `Reporter` impl at lines 91-103)
Stores VRF seed hashes from notarization/finalization activities into the `LedgerService`, which are later used as `prevrandao` in block headers. The actual logic is in the free function `seed_report_inner()` (lines 40-63).

### `NodeStateReporter` (struct at lines 432-451, `Reporter` impl at lines 453-475)
Updates RPC-visible node metrics (current view, finalized count, nullified count) from consensus activity. The `report()` method (lines 459-474) directly mutates `NodeState` -- `set_view()` on notarization and finalization, `inc_finalized()` on finalization, `inc_nullified()` on nullification.

### Helper Functions (also untested)
- `index_finalized_block()` (lines 245-324): Builds `IndexedBlock`, `IndexedTransaction`, and `IndexedReceipt` structs and inserts them into the `BlockIndex`.
- `decode_tx_metadata()` (lines 326-351): Decodes `TxEnvelope` from raw bytes, recovers the signer, and returns `TxMetadata`. Returns `None` on decode or recovery failure (with a `warn!` log).
- `effective_gas_price()` (lines 353-361): Extracts gas price from each `TxEnvelope` variant.

### Required Tests

Testing these reporters requires mock implementations of:
- `LedgerService` (or its underlying traits) -- provides `query_state_root()`, `parent_snapshot()`, `compute_root_from_store()`, `insert_snapshot()`, `persist_snapshot()`, `prune_mempool()`, `set_seed()`, `submit_tx()`
- `BlockExecutor` -- the `execute()` method; can use a mock that returns a predetermined `ExecutionOutcome`
- `BlockContextProvider` -- the `context()` method; trivial to mock
- `commonware_utils::acknowledgement::Acknowledgement` -- `acknowledge()` must be callable; need to verify it was called
- `BlockIndex` (from `kora-indexer`) -- `insert_block()` to verify indexed data
- `NodeState` (from `kora-rpc`) -- `set_view()`, `inc_finalized()`, `inc_nullified()`
- `tokio::Context` -- needed by `FinalizedReporter` for spawning persist tasks

```
1. handle_finalized_update() / FinalizedReporter::report()
   - Happy path: snapshot exists, block_index is None -> persist, prune, ack
   - Happy path with block_index: snapshot exists -> re-execute for indexing, persist, prune, ack
   - Re-execution catchup: snapshot missing, parent exists -> re-execute, insert snapshot, persist, prune, ack
   - State root mismatch: computed root != block.state_root -> warn, ack, NO persist
   - BlockExecution::execute() fails -> error log, ack, NO persist
   - compute_root_from_store() fails -> error log, ack, NO persist
   - Missing parent snapshot (and snapshot missing): error log, ack, NO persist
   - Missing parent snapshot (but snapshot exists): warn, skip indexing, persist, prune, ack
   - persist_snapshot task panics -> error log, ack
   - persist_snapshot returns Err -> error log, ack
   - Tip update: Update::Tip should be a no-op (immediate return)

2. index_finalized_block() (lines 245-324)
   - Verify IndexedBlock fields match block data (hash, number, parent_hash, state_root, timestamp, gas_limit, gas_used, base_fee_per_gas, transaction_hashes)
   - Verify IndexedTransaction recovery from signed tx bytes (from, to, value, gas_limit, gas_price, input, nonce)
   - Verify IndexedReceipt log indexing with sequential log_index across multiple receipts
   - Verify decode_tx_metadata handles malformed bytes gracefully (returns None, does not panic)
   - Verify effective_gas_price returns correct value for Legacy, EIP-2930, EIP-1559, EIP-4844, EIP-7702

3. SeedReporter::report() (via seed_report_inner)
   - Notarization activity stores seed hash via ledger.set_seed()
   - Finalization activity stores seed hash via ledger.set_seed()
   - Other activity types (Nullification, etc.) are no-ops

4. NodeStateReporter::report()
   - Notarization: calls set_view(n.proposal.round.view().get())
   - Finalization: calls set_view() AND inc_finalized()
   - Nullification: calls inc_nullified()
   - Other activity types: no-op
```

---

## CRITICAL GAP 2: No Real Transaction Execution Test

**Location:** `crates/node/executor/src/revm.rs` (lines 357-411) and `crates/node/executor/tests/executor.rs`

The executor has 40 unit tests and 40 integration tests (80 total), but **not a single test signs a real EIP-1559 (or any type) transfer transaction and verifies balance changes through the `BlockExecutor::execute()` method**. All existing tests either:

- Test with empty transaction lists (`test_execute_empty_transactions_returns_empty_outcome`, line 179)
- Test with populated state but empty transactions (`test_execute_with_populated_state`, line 562)
- Test mock infrastructure (`test_mock_state_db_*`)
- Test header validation and config (`validate_header_*`, `calculate_base_fee_*`)
- Test receipt building with synthetic `ExecutionResult` values (`build_receipt_*`)
- Test change extraction from synthetic `EvmState` (`extract_changes_*`)

No test exercises the full path: sign a transaction -> call `executor.execute()` -> verify sender balance decreased, recipient balance increased, nonce incremented, gas consumed, receipt generated.

### Implementation Guide for Real Transaction Test

The integration test in `crates/node/executor/tests/executor.rs` already has a `MockStateDb` implementation that supports reads and commits. To sign a real transaction, you need:

1. **Generate a signing key:** Use `alloy_signer_local::PrivateKeySigner` (or `alloy_signer::SignerSync`).
2. **Pre-fund the sender:** Insert a `MockAccount` with sufficient balance into the `MockStateDb` using `insert_account()`.
3. **Build and sign a transaction:** Use `alloy_consensus::TxEip1559` to construct the transaction, then sign it with the signer to get a `TxEnvelope`. Encode it with `alloy_eips::eip2718::Encodable2718::encode_2718()` to get raw bytes.
4. **Execute:** Call `executor.execute(&state, &context, &[Bytes::from(raw_tx)])`.
5. **Verify:** Check `outcome.changes` for sender nonce increment, sender balance decrease, recipient balance increase. Check `outcome.receipts[0].success()`.

**Required dependencies** (add to `[dev-dependencies]` in `crates/node/executor/Cargo.toml`):
- `alloy-signer-local` (for `PrivateKeySigner`)
- `alloy-consensus` (already a dependency)
- `alloy-eips` (already a dependency)

### Required Tests

```
1. Sign an EIP-1559 transfer, execute, verify:
   - Sender balance decreased by (value + gas_used * effective_gas_price)
   - Recipient balance increased by value
   - Sender nonce incremented from 0 to 1
   - ExecutionOutcome.gas_used >= 21000 (intrinsic gas for a transfer)
   - Receipt is successful with correct tx_hash (keccak256 of raw tx bytes)
   - outcome.changes contains both sender and recipient entries

2. Multi-tx block with nonce ordering:
   - Two transfers from same sender with sequential nonces (0, 1)
   - Verify both execute and state accumulates correctly
   - Verify cumulative_gas_used in second receipt > first receipt

3. Block with one invalid tx (bad signature or wrong chain_id):
   - Verify the executor returns Err(ExecutionError::TxDecode(...)) or Err(ExecutionError::TxExecution(...))
   - NOTE: Currently this aborts the ENTIRE block (see issue #1 / executor-fatal-abort)

4. Contract deployment:
   - Sign a CREATE transaction with bytecode in data field and to=None
   - Verify receipt.contract_address is Some(computed_address)

5. EIP-4844 blob transaction:
   - Blob gas fields are decoded (revm.rs lines 499-517) but blob gas is not tracked
   - Verify behavior with blob tx (may require setting blob_base_fee in BlockContext)
```

---

## CRITICAL GAP 3: No Node Crash/Recovery E2E Test

**Location:** `crates/e2e/src/tests/` (all test files)

There are zero tests for node restart or recovery scenarios. The e2e test harness (`crates/e2e/src/harness.rs`) uses `kora-transport-sim` to run a simulated network, but all tests run a clean network from genesis to some block height and then stop. No test:

- Starts consensus, stops a node, restarts it, and verifies it catches up
- Simulates a crash during finalization and verifies the reporter handles partial state
- Tests the Marshal resolver catch-up path (which had a known bug where peers were permanently blocked after restart)

The resolver catch-up failure was a production bug that was never caught by tests because no test exercises the restart path.

### Required Tests

```
1. Stop one validator after N blocks, restart, verify it catches up to tip
2. Stop and restart the leader, verify a new leader is elected and chain progresses
3. Kill all nodes, restart all from persisted state, verify chain resumes
4. Crash during finalization (between persist_snapshot and prune_mempool)
```

---

## CRITICAL GAP 4: No Runner Wiring Test

**Location:** `crates/node/runner/src/runner.rs` (lines 329-578 for `ProductionRunner::run()`)

The runner has 8 unit tests total (3 in `runner.rs` lines 581-613 testing `runtime_storage_directory_from()`, and 5 in `error.rs` testing error formatting). None test the critical wiring between components -- specifically, that the `tx_submit` callback set on the RPC server (line 438: `.with_tx_submit(tx_submit)`) actually routes transactions to the mempool via `ledger.submit_tx()`.

The `tx_submit` callback is constructed at lines 406-431 of `runner.rs`. It:
1. Wraps the raw bytes in a `Tx` struct
2. Validates the transaction using `TransactionValidator::new(chain_id, state, PoolConfig::default())`
3. Calls `ledger.submit_tx(tx).await`
4. Returns `Ok(())` on success or `Err(RpcError::InvalidTransaction(...))` on failure

This was a production bug: transactions submitted via RPC were silently dropped because the callback was not correctly wired, and there was no diagnostic logging to detect it. The `app.rs` code (line 102) now has a warning for this case (`"build_block: mempool has unincluded txs but produced empty block"`), but this was added after the fact.

### Required Tests

```
1. Construct a Runner with mock components, verify:
   - tx_submit callback is set on RPC server
   - Calling tx_submit with valid tx bytes results in mempool insertion
   - Calling tx_submit with invalid tx bytes returns RpcError::InvalidTransaction

2. Integration test: submit tx via RPC -> verify it appears in next proposed block
```

---

## Additional Coverage Gaps

### DKG Timeout Handling
- No test for DKG ceremony timeout (what happens when a participant is unreachable during key generation)
- No test for threshold signature failure when insufficient shares are collected

### Transaction Pool Eviction
- The pool has config for `max_pending_txs` (4096), `max_queued_txs` (1024), and `max_txs_per_sender` (256) but no test verifies eviction behavior when these limits are reached
- No test for concurrent pool access from multiple async tasks

### Consensus Failure Scenarios
- No test for message loss during consensus (e2e resilience tests use `success_rate: 1.0`)
- No test for Byzantine validator behavior
- No test for what happens when all validators nullify repeatedly

### Ledger Recovery
- `kora-ledger` has only 6 tests covering basic state operations
- No test for `persist_snapshot` failure and recovery
- No test for `compute_root_from_store` with large change sets

---

## Missing Infrastructure

### No Fuzzing Targets
No `fuzz/` directory or `cargo-fuzz` targets exist. High-value fuzzing targets would include:
- Transaction decoding (`decode_tx_env` in `revm.rs`)
- Block codec (encode/decode roundtrip)
- RPC request parsing

### No Benchmarks
No `benches/` directory or `criterion` benchmarks. Critical paths to benchmark:
- Block execution throughput (txs/second)
- State root computation time
- Transaction validation throughput

### No Coverage Tooling
No CI integration with `cargo-tarpaulin` or `cargo-llvm-cov`. Coverage metrics would immediately highlight the reporters gap.

### Many E2E Tests Are `#[ignore]`'d
Of the 26 e2e tests, exactly 18 are marked `#[ignore]` (15 with "flaky when run in parallel", 1 with "requires investigation - larger quorums time out", and 2 with "slow stress test"). Only 8 tests run in CI by default. The ignored tests only run when explicitly requested with `--run-ignored`, meaning they are effectively not part of CI.

---

## Priority

### Priority 1 -- Implement Immediately

These tests cover code paths where production bugs have already occurred:

1. **Real transaction execution test** (executor): Sign EIP-1559 transfer, execute, verify balance changes
2. **FinalizedReporter unit tests** (reporters): Mock LedgerService, test happy path and all error branches
3. **Runner wiring test**: Verify tx_submit -> mempool path
4. **SeedReporter / NodeStateReporter tests**: Simple state mutation verification

### Priority 2 -- Before Production

These tests cover likely failure modes:

1. **DKG timeout handling**: Ceremony timeout, insufficient shares
2. **Node restart e2e test**: Stop, restart, verify catch-up
3. **Pool eviction under pressure**: Max size reached, verify eviction policy
4. **Consensus under message loss**: Set `success_rate < 1.0` in SimLinkConfig
5. **Multi-tx block execution**: Nonce ordering, mixed valid/invalid

### Priority 3 -- Hardening

1. **Concurrent pool access**: Spawn multiple tasks submitting to pool simultaneously
2. **RPC edge cases**: Malformed requests, missing fields, boundary values
3. **Ledger recovery**: Crash during persist, restart, verify consistency
4. **Fuzzing targets**: Transaction decode, block codec, RPC parsing
5. **Benchmarks**: Execution throughput, state root computation

---

## How to Run Existing Tests

```bash
# All tests (excludes ignored)
just test

# All tests including ignored (single-threaded for e2e)
cargo nextest run --workspace --all-features --run-ignored all -- --test-threads=1

# Specific crate
cargo nextest run -p kora-reporters   # currently: 0 tests
cargo nextest run -p kora-executor    # currently: ~80 tests (40 unit + 40 integration)
cargo nextest run -p kora-runner      # currently: 8 tests (3 runner.rs + 5 error.rs)

# E2E tests only (8 active, 18 ignored)
cargo nextest run -p kora-e2e --run-ignored all -- --test-threads=1

# With output
cargo nextest run -p kora-executor --no-capture
```

---

## Verification Steps

After implementing the tests described in this issue, verify completeness with:

```bash
# 1. Confirm reporters crate now has tests
cargo nextest run -p kora-reporters --no-capture
# Should show > 0 tests passing

# 2. Confirm executor has a real signed-tx test
cargo nextest run -p kora-executor -- test_signed_eip1559_transfer
# Should pass with balance changes verified

# 3. Confirm runner wiring test exists
cargo nextest run -p kora-runner -- test_tx_submit
# Should show tx_submit callback routes to mempool

# 4. Verify no regressions
just test

# 5. Run full e2e suite including ignored tests
cargo nextest run -p kora-e2e --run-ignored all -- --test-threads=1
```

---

## Additional Coverage Gaps: Load Generator and RPC

### Load Generator Tests

The `bin/loadgen/` binary has zero tests. Critical scenarios to test:

1. **Nonce race condition**: The loadgen uses `AtomicU64::fetch_add(1, SeqCst)` for nonce assignment per sender. While `SeqCst` prevents ordering issues, each sender struct is shared across concurrent tasks. Test that nonce assignment under concurrency (200 tasks) produces no gaps or duplicates.

2. **Broadcast deduplication**: When `--broadcast-rpc-urls` is set, the same transaction is sent to multiple validators. Test that the overall acceptance rate matches expectations (should be ~25% with 4 validators for broadcast mode).

3. **Error handling**: The loadgen currently counts "accepted" vs "rejected" but does not distinguish rejection reasons. Test that nonce-too-low, nonce-too-high, insufficient-balance, and pool-full errors are counted separately.

### RPC Server Tests

The `crates/node/rpc/` crate has limited test coverage:

1. **Pending transaction memory**: `eth_sendRawTransaction` adds to `pending_txs` HashMap but entries are only removed when queried AND finalized. Test that the HashMap doesn't grow unbounded (see Issue #21).

2. **Historical state queries**: `eth_getBalance(addr, "0x5")` silently returns latest state. Test that specific block numbers either return correct historical state or return an explicit error (see Issue #30).

3. **Rate limiting**: `RateLimitConfig` exists but is never wired. Test that rate limiting is actually applied when configured (see Issue #22).
