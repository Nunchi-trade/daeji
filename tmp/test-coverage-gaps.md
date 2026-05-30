# Kora Test Coverage Gaps Analysis

## What Is Kora?

Kora is a high-performance EVM-compatible blockchain built in Rust. It uses BFT (Byzantine Fault Tolerant) consensus via the Commonware Simplex protocol, threshold BLS signatures for VRF-based leader election, and REVM for EVM execution. The architecture includes a transaction pool, DKG (Distributed Key Generation) ceremony for validator key setup, an RPC server compatible with Ethereum JSON-RPC standards, and a persistent ledger backed by QMDB (a custom Merkle database).

Test coverage is critical for blockchain systems because:

- **Consensus correctness**: A single bug can cause chain splits or liveness failures across the entire network.
- **Financial safety**: EVM execution errors can lead to lost or incorrectly attributed funds.
- **State machine determinism**: All validators must produce identical state transitions; divergence halts the chain.
- **Adversarial environment**: Blockchain nodes are exposed to malicious inputs (invalid transactions, network attacks, mempool poisoning).
- **Recovery resilience**: Nodes must restart cleanly and catch up without data loss.

---

## Test Infrastructure Overview

### How to Run Tests

```bash
# Run all tests (preferred, uses cargo-nextest)
just test
# Equivalent to:
cargo nextest run --workspace --all-features

# Run tests for a specific crate
cargo nextest run -p kora-txpool

# Run e2e tests (many are #[ignore]'d due to flakiness in parallel)
cargo nextest run -p kora-e2e -- --ignored --test-threads=1

# Run the full CI suite (format, lint, test, license check)
just ci
```

### Test Framework

- **Unit tests**: Inline `#[cfg(test)] mod tests` in source files
- **Integration tests**: `tests/` directories (e.g., `crates/node/executor/tests/executor.rs`)
- **End-to-end tests**: `crates/e2e/` with a full multi-node harness using simulated networking
- **Test dependencies**: `rstest` for parameterized tests, `tempfile` for temp directories, `tokio` for async tests
- **Load testing**: `bin/loadgen/` binary (not automated in CI, requires a running devnet)

### Test Harness Architecture

The e2e harness (`crates/e2e/`) spins up multiple in-process validator nodes connected via `kora-transport-sim` (simulated P2P network with configurable latency, jitter, and packet loss). It runs DKG, starts consensus, submits transactions, and verifies state convergence across all nodes.

---

## Per-Crate Test Coverage Assessment

### `crates/node/executor/` -- EVM Execution

**Files with `#[cfg(test)]`:**
- `src/revm.rs` -- Tests for `RevmExecutor` creation, header validation (gas limit bounds, parent validation, timestamp, gas delta), base fee calculation, receipt building (success/revert/halt), and change extraction (touched/created/selfdestructed accounts, storage changes)
- `src/validation.rs` -- Tests for intrinsic gas calculation (transfer, with data, create, with access list)
- `src/context.rs`, `src/config.rs`, `src/error.rs`, `src/adapter.rs`, `src/outcome.rs` -- Basic struct tests

**Integration tests (`tests/executor.rs`):**
- RevmExecutor creation with various chain IDs
- Empty transaction execution
- Header validation (valid gas, below minimum)
- BlockContext creation
- MockStateDb infrastructure validation (extensive)

**What IS tested:**
- Executor instantiation and configuration
- Empty block execution
- Header validation rules
- Receipt generation for success/revert/halt outcomes
- EVM state change extraction (accounts, storage, self-destruct, contract creation)
- Base fee calculation (at target, above target, below target)
- Parent header validation (sequential numbers, timestamps, gas limit delta)

**What is NOT tested (Critical Gaps):**
- Actual transaction execution (no test submits a real signed EIP-1559 transfer through the executor)
- Invalid/malformed transaction handling within the executor
- Transaction abort behavior (out-of-gas, stack overflow, revert propagation)
- Multi-transaction block execution (nonce ordering, gas accumulation)
- Contract deployment execution
- Contract call execution
- State rollback on transaction failure (partial block execution)
- Maximum gas limit enforcement per block
- Coinbase/beneficiary fee collection

**Severity: HIGH** -- The executor is the core of state transitions. No test verifies that a real signed transaction produces correct balance changes.

---

### `crates/node/consensus/` -- Consensus Logic

**Files with `#[cfg(test)]`:**
- `src/proposal.rs` -- Tests for ProposalBuilder: creation, missing parent, empty block, with transactions, max_txs limit, state root computation, gas used, parent tx exclusion (10 tests)
- `src/application.rs` -- Mock application trait tests (propose, verify, finalize, on_seed)
- `src/ledger.rs` -- LedgerView tests: creation, accessors, build_proposal_txs, snapshot insert/get/persist (chain and single), merged changes, seed operations (12 tests)
- `src/components/mempool.rs` -- InMemoryMempool: insert, duplicate rejection, prune, build with exclusions
- `src/components/snapshot.rs` -- InMemorySnapshotStore: insert/get, persisted flag, persisting guard
- `src/components/seed.rs` -- InMemorySeedTracker: insert/get, genesis seed
- `src/error.rs` -- Error display formatting
- `src/traits.rs` -- Trait mock tests

**What IS tested:**
- Proposal building with mocked executor
- Block exclusion logic (parent txs not re-included)
- Snapshot persistence chain walks
- Memory pool insert/prune/build operations
- Error formatting

**What is NOT tested (Critical Gaps):**
- Nullification scenarios (what happens when a round is nullified)
- Timeout handling (consensus stall detection and recovery)
- View change mechanics
- Conflicting proposals from different leaders
- Fork choice logic
- Re-execution of finalized blocks for catchup
- Consensus with byzantine validators (conflicting votes)
- Block verification with actual execution (real state transition verification)
- Race condition: proposal arrives before parent is persisted

**Severity: HIGH** -- No test exercises the actual consensus protocol behavior under adversarial or failure conditions.

---

### `crates/node/txpool/` -- Transaction Pool

**Files with `#[cfg(test)]`:**
- `src/pool.rs` -- TransactionPool tests: add/pending, duplicate rejection, sender limit, remove, clear, prune (nonce advancement), build with exclusions, queued promotion after gap fill (8 tests)
- `src/validator.rs` -- TransactionValidator tests: valid EIP-1559/legacy tx, reject too large, invalid chain ID, gas price too low, intrinsic gas too low, nonce too low, far future nonce, insufficient balance, accept future nonce, 39-tx burst from single sender, invalid RLP, effective gas price, intrinsic gas calculations, max tx cost, sender recovery (18 tests)
- `src/ordering.rs` -- SenderQueue tests: sequential insert, insert with gap, ordering comparison
- `src/config.rs`, `src/error.rs` -- Basic tests

**What IS tested:**
- Transaction validation (decode, chain ID, gas price, intrinsic gas, nonce, balance)
- Pool add/remove/clear/prune operations
- Sender queue ordering (pending vs queued)
- Nonce gap handling (queued transactions promoted when gap fills)
- Per-sender limit enforcement
- Burst submission (39 sequential txs from one sender)
- Build with exclusion sets

**What is NOT tested (Critical Gaps):**
- Pool eviction under global capacity limits (what happens when pool is full?)
- Priority-based eviction (which transactions get dropped?)
- Mempool poisoning: sender submits tx with nonce N, then submits a conflicting tx with same nonce
- Nonce replacement (same nonce, higher gas price)
- Transaction expiration/TTL
- Concurrent access patterns (multiple goroutines adding/removing simultaneously)
- Pool recovery after node restart (are pooled txs lost?)
- Gas price bumping
- Pool statistics/metrics accuracy

**Severity: MEDIUM** -- Core validation is well-tested, but pool management under pressure (full pool, eviction, replacement) is not.

---

### `crates/node/rpc/` -- JSON-RPC Server

**Files with `#[cfg(test)]`:**
- `src/eth.rs` -- Tests for: web3_clientVersion, eth_chainId, net_version, eth_blockNumber, web3_sha3, eth_sendRawTransaction (with callback, without callback, bytes verification), eth_getTransactionByHash (pending tx) (9 tests)
- `src/server.rs` -- CORS layer tests (empty origins, specific origins, wildcard)
- `src/state.rs` -- NodeStatus serde roundtrip, camelCase field names, NodeState operations (view, counters, peer count)
- `src/indexed_provider.rs` -- Tests for balance, nonce, block_by_number, block_by_hash, full transactions, code for missing account, transaction_by_hash, receipt_by_hash, block_number, block tag resolution (11 tests)
- `src/config.rs`, `src/error.rs`, `src/types.rs` -- Basic tests

**What IS tested:**
- Core Ethereum RPC methods (chain_id, block_number, send_raw_transaction, get_transaction_by_hash)
- Web3 namespace (client_version, sha3)
- Net namespace (version)
- State provider queries (balance, nonce, code, storage via index)
- Block/transaction/receipt retrieval from index
- Block tag resolution (latest, earliest)
- CORS configuration

**What is NOT tested (Critical Gaps):**
- `kora_nodeStatus` RPC method (no test in `kora.rs`)
- `eth_getBalance` via the actual RPC server (only tested through indexed provider)
- `eth_getTransactionCount` via RPC
- `eth_getCode` via RPC
- `eth_getStorageAt` via RPC
- `eth_call` (if implemented)
- `eth_estimateGas` (if implemented)
- `eth_getBlockByNumber` via the full RPC path
- `eth_getTransactionReceipt` via the full RPC path
- Error responses for invalid inputs (invalid hex, invalid block numbers)
- Rate limiting behavior
- Concurrent RPC request handling
- WebSocket subscriptions (if any)
- Large response pagination

**Severity: MEDIUM** -- The indexed_provider covers query logic well, but the full RPC request-response path through jsonrpsee is only tested for `send_raw_transaction`.

---

### `crates/node/dkg/` -- Distributed Key Generation

**Files with `#[cfg(test)]`:**
- `src/tests.rs` -- Integration tests: participant creation, dealer message generation, message serialization (with session binding), legacy message serialization, full local DKG simulation (4 nodes), quorum calculation for various validator counts, session mismatch rejection, duplicate message rejection (8 tests)
- `src/error.rs` -- Error display formatting (15 tests)
- `src/config.rs`, `src/transport.rs` -- Basic tests

**What IS tested:**
- Full local DKG ceremony (4 participants, all phases: deal, ack, finalize, verify group key)
- Message serialization/deserialization
- Session mismatch detection
- Duplicate message handling
- Quorum calculation for n=4,7,10,13
- Transport message encoding

**What is NOT tested (Critical Gaps):**
- DKG ceremony timeout (what if a participant never responds?)
- Ceremony with a byzantine dealer (invalid share)
- Ceremony with insufficient participants (n-f fails to respond)
- Network partition during DKG
- DKG restart/recovery after partial completion
- Concurrent ceremony attempts
- Large validator sets (n=100+)
- Dealer failure mid-ceremony
- Key share persistence and reload
- Key resharing/rotation

**Severity: HIGH** -- DKG is a one-time critical ceremony; failure means the chain cannot start. No failure-path testing exists.

---

### `crates/node/reporters/` -- Finalization Callbacks

**No `#[cfg(test)]` modules found.**

**What IS tested:** Nothing (zero unit tests).

**What is NOT tested (Critical Gaps):**
- `FinalizedReporter::report` -- the entire finalization pipeline:
  - Block re-execution for catchup
  - State root verification
  - Snapshot persistence
  - Mempool pruning after finalization
  - Block indexing for RPC
- `SeedReporter::report` -- seed storage from notarization/finalization
- `NodeStateReporter::report` -- view/counter updates
- Error handling (execution failure, missing parent, root mismatch)
- `index_finalized_block` -- transaction decoding and indexing
- `decode_tx_metadata` -- envelope parsing and sender recovery

**Severity: HIGH** -- This is the "glue" that connects consensus finalization to state persistence and RPC indexing. A bug here causes finalized blocks to not persist or RPC to show stale data.

---

### `crates/node/ledger/` -- State Management & Persistence

**Files with `#[cfg(test)]`:**
- `src/lib.rs` -- Tests for LedgerView/LedgerService: snapshot persistence (merges unpersisted ancestors), empty child inherits parent state root, duplicate persistence is noop, overlay merges, unrelated branch merges, snapshot state updates (6 integration tests using real QMDB + REVM execution)

**What IS tested:**
- Full execution pipeline: sign tx -> execute -> compute root -> persist (using real RevmExecutor and QMDB)
- Multi-block chains with state accumulation
- Fork handling (unrelated branches)
- Duplicate persistence safety
- Empty block state root inheritance

**What is NOT tested (Critical Gaps):**
- Ledger initialization with genesis allocations (error cases)
- Snapshot pruning/GC (old snapshots consuming memory)
- Concurrent persist calls (race conditions)
- Persistence failure recovery (disk full, I/O error)
- State root query for non-existent digest
- Large state trees (performance under load)
- Ledger restore from disk after restart

**Severity: MEDIUM** -- Core persistence logic is tested with real execution, but failure/recovery paths are not.

---

### `crates/node/runner/` -- Full Node Startup

**Files with `#[cfg(test)]`:**
- `src/runner.rs` -- Tests for `runtime_storage_directory_from`: default path, empty override, explicit override (3 tests)
- `src/error.rs` -- Basic error tests

**What IS tested:**
- Runtime storage directory path resolution

**What is NOT tested (Critical Gaps):**
- Full node startup sequence
- Configuration parsing and validation
- Component wiring (executor + pool + RPC + consensus)
- Graceful shutdown
- Signal handling (SIGTERM, SIGINT)
- Node restart and state recovery
- Invalid configuration rejection
- Port conflict handling
- Network bootstrap (connecting to peers)
- `RevmApplication` propose/verify/finalize cycle

**Severity: HIGH** -- The runner orchestrates all components. No test verifies the wiring is correct (e.g., the production bug where `tx_submit` was not wired to the mempool).

---

### `crates/node/simplex/` -- Consensus Engine Configuration

**Files with `#[cfg(test)]`:**
- `src/engine.rs` -- Tests for DefaultEngine debug/copy traits
- `src/pool.rs` -- Tests for DefaultPool debug/copy/constants
- `src/quota.rs` -- Tests for DefaultQuota debug/copy/init
- `src/config.rs` -- Configuration defaults

**What IS tested:**
- Struct trait implementations (Debug, Copy, Clone)
- Default configuration constants

**What is NOT tested:**
- Actual consensus engine behavior (delegated to Commonware)
- Mailbox overflow handling
- Rate limiting behavior

**Severity: LOW** -- This crate is mostly configuration and wrappers around Commonware; the actual logic lives upstream.

---

### `crates/node/config/` -- Node Configuration

**Files with `#[cfg(test)]`:**
- All config files have tests for default values and builder patterns

**What IS tested:**
- Default configuration values
- Builder pattern correctness
- Serialization/deserialization

**Severity: LOW** -- Adequate for a configuration crate.

---

## End-to-End Test Coverage

The `crates/e2e/` crate provides a comprehensive multi-node test harness:

**Active tests (not ignored):**
- 4-validator consensus (3 blocks)
- State root convergence (5 blocks)
- Seed/prevrandao convergence
- Sequential block production (10 blocks)
- Empty blocks
- Gas limit enforcement
- Network jitter tolerance
- Sustained throughput (10 blocks, 15 transfers)

**Ignored tests (flaky in parallel):**
- 7-validator consensus
- Simple/multiple transfers
- Sequential nonces
- Deterministic execution
- Balance updates after finalization
- Varying validator counts (4-7)
- High latency network
- Longer chain (20 blocks)
- Different chain IDs
- Stress tests (50 txs, 50 blocks)

**What is NOT tested in e2e:**
- Node crash and recovery (no test stops and restarts a node)
- Validator set changes
- DKG ceremony as part of the e2e flow (harness uses pre-generated keys)
- Message loss/corruption (success_rate < 1.0 is available but not used)
- Leader failure (current leader crashes mid-proposal)
- State divergence detection and halting
- RPC correctness during consensus (query while blocks finalize)
- Transaction submission via RPC during consensus (loadgen-style)
- Mempool interaction during active consensus
- Chain reorg scenarios

---

## Critical Untested Scenarios

### 1. Invalid Transaction Handling in Executor
**Risk**: A malformed transaction that passes pool validation but fails during EVM execution could panic or produce inconsistent state.
**What's missing**: No test submits a transaction with invalid bytecode, out-of-gas during execution, or a revert that should still consume gas.

### 2. Mempool Poisoning
**Risk**: An attacker submits many low-fee transactions to fill the pool, blocking legitimate transactions.
**What's missing**: No test verifies pool eviction under capacity pressure or priority-based ordering under attack.

### 3. Nonce Gap Handling
**Risk**: A sender submits nonces 0, 2, 3 (skipping 1). The pool must queue 2 and 3 until 1 arrives.
**What IS partially tested**: The `pool.rs` test `pool_prune_promotes_queued_transactions_after_gap_fills` covers the basic case.
**What's missing**: What happens if the gap is never filled? Is there a timeout? What about nonce gaps across multiple blocks?

### 4. Node Restart and Catch-Up
**Risk**: A node crashes and restarts. It must reload persisted state and catch up to the current chain tip.
**What's missing**: Zero tests for restart scenarios. The `FinalizedReporter` has catchup logic (re-executes blocks when snapshot is missing) but it's never tested.

### 5. DKG Ceremony Timeout
**Risk**: A validator is offline during DKG. The ceremony must either timeout or proceed with available participants.
**What's missing**: No test exercises the `timeout` field in `DkgConfig` or verifies timeout behavior.

### 6. Concurrent Transaction Submission
**Risk**: Multiple clients submit transactions simultaneously. The pool and RPC must handle this without data races.
**What's missing**: No concurrent/parallel test for the transaction pool or RPC server. The pool uses `parking_lot::RwLock` but no test stresses it.

### 7. Consensus Stall Recovery
**Risk**: If consensus stalls (no blocks finalize for N rounds), the system should detect this and potentially trigger view changes or alerts.
**What's missing**: No test for prolonged nullification periods or stall detection.

### 8. State Root Mismatch
**Risk**: If a validator computes a different state root than the proposer, the block should be rejected.
**What's missing**: While the `FinalizedReporter` checks state roots, no test verifies the rejection path (what happens when a state root mismatch occurs).

---

## Recommendations (Prioritized)

### Priority 1: Critical Safety (implement immediately)

1. **Real transaction execution test** (`crates/node/executor/`)
   - Sign an EIP-1559 transfer, execute it through `RevmExecutor`, verify balance changes.
   - Test a failing transaction (insufficient gas) and verify receipt status + gas consumption.
   - Test multi-transaction block with correct nonce ordering.

2. **Reporters unit tests** (`crates/node/reporters/`)
   - Test `FinalizedReporter` with a mock LedgerService: verify persist is called, mempool is pruned, index is updated.
   - Test the re-execution catchup path (snapshot missing for finalized block).
   - Test state root mismatch handling.

3. **Runner wiring test** (`crates/node/runner/`)
   - Verify that `tx_submit` callback is correctly wired to the mempool (regression for the silent-drop bug).
   - Test that a transaction submitted via RPC actually reaches the mempool.

### Priority 2: Resilience (implement before production)

4. **DKG timeout test** (`crates/node/dkg/`)
   - Simulate a participant that never responds; verify the ceremony either times out or proceeds with quorum.
   - Test invalid dealer share detection.

5. **Node restart e2e test** (`crates/e2e/`)
   - Run consensus for 5 blocks, stop a node, restart it, verify it catches up.
   - This requires persistence of DKG keys and ledger state.

6. **Pool capacity/eviction test** (`crates/node/txpool/`)
   - Fill the pool to capacity, submit one more transaction, verify the lowest-priority tx is evicted.
   - Test nonce replacement (same sender, same nonce, higher gas).

7. **Consensus under message loss** (`crates/e2e/`)
   - Use `SimLinkConfig { success_rate: 0.8 }` and verify consensus still makes progress (may require nullification + view change).

### Priority 3: Robustness (implement for hardening)

8. **Concurrent pool access test** (`crates/node/txpool/`)
   - Spawn multiple tasks adding/removing/building from the pool simultaneously.
   - Verify no panics, no duplicate inclusions, correct final state.

9. **RPC edge cases** (`crates/node/rpc/`)
   - Invalid hex parameters, out-of-range block numbers, queries for non-existent transactions.
   - Rate limiting verification.

10. **Ledger recovery test** (`crates/node/ledger/`)
    - Simulate a crash mid-persist (e.g., persist half a changeset).
    - Verify the ledger can recover on next startup.

11. **Full RPC integration test**
    - Start the RPC server, submit a transaction via HTTP, query balance via HTTP, verify the complete roundtrip.

---

## How to Run Existing Tests

```bash
# All workspace tests
just test

# Specific crate tests
cargo nextest run -p kora-executor
cargo nextest run -p kora-txpool
cargo nextest run -p kora-consensus
cargo nextest run -p kora-rpc
cargo nextest run -p kora-dkg
cargo nextest run -p kora-ledger
cargo nextest run -p kora-e2e

# E2E tests (ignored tests need single-threaded execution)
cargo nextest run -p kora-e2e -- --ignored --test-threads=1

# Stress tests only
cargo nextest run -p kora-e2e -- --ignored -k stress

# Run with output (for debugging)
cargo nextest run -p kora-executor --nocapture

# Load testing (requires running devnet)
just loadtest       # 1000 txs
just stresstest     # 10000 txs with 50 accounts
```

---

## Integration Test Infrastructure Summary

| Layer | Exists? | Notes |
|-------|---------|-------|
| Unit tests (per-function) | Yes | Good coverage for validation and data structures |
| Integration tests (per-crate) | Partial | Only `executor` has a `tests/` directory; DKG has `src/tests.rs` |
| E2E multi-node tests | Yes | `crates/e2e/` with simulated networking; many marked `#[ignore]` |
| Load testing | Yes | `bin/loadgen/` binary (manual, not in CI) |
| Fuzzing | No | No fuzz targets found |
| Property-based testing | No | No `proptest` or `quickcheck` usage |
| Benchmark tests | No | No `#[bench]` or criterion benchmarks |
| Mutation testing | No | Not configured |

### Key Infrastructure Gaps

1. **No fuzzing** -- Critical for a blockchain processing untrusted inputs (transactions, P2P messages). Transaction decoding, RLP parsing, and message deserialization are prime fuzz targets.

2. **No benchmarks** -- No way to detect performance regressions in execution, pool operations, or consensus throughput.

3. **Flaky e2e tests** -- Many e2e tests are `#[ignore]`'d due to parallelism issues. This means they are not run in CI by default, reducing the safety net.

4. **No CI load testing** -- The loadgen exists but is manual. Automated performance regression detection is missing.

5. **No coverage tooling** -- No `cargo-llvm-cov` or `tarpaulin` configuration to track coverage metrics over time.

---

## Test Count Summary

| Crate | Unit Tests | Integration Tests | Critical Gaps |
|-------|-----------|-------------------|---------------|
| `executor` | ~25 | ~20 (tests/executor.rs) | No real tx execution |
| `consensus` | ~35 | 0 | No failure scenarios |
| `txpool` | ~30 | 0 | No eviction/replacement |
| `rpc` | ~30 | 0 | Limited RPC path testing |
| `dkg` | ~25 | 8 (src/tests.rs) | No timeout/failure |
| `reporters` | 0 | 0 | Entirely untested |
| `ledger` | ~6 | 0 | No recovery testing |
| `runner` | 3 | 0 | No wiring/startup tests |
| `simplex` | ~12 | 0 | Config-only tests |
| `e2e` | 0 | ~20 (many ignored) | No crash/recovery |

**Estimated total: ~160 unit tests, ~48 integration/e2e tests**

The most dangerous gap is the `reporters` crate having zero tests -- it is responsible for the entire finalization pipeline (persisting state, pruning mempool, indexing for RPC). A bug there caused the production issue where transactions were accepted but never included in blocks.
