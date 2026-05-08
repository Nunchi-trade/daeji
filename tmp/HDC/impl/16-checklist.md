# 16 -- Master Implementation Checklist

> **Purpose:** This is the single source of truth for tracking implementation
> progress. Every item is concrete, verifiable, and ordered. An implementing
> agent should work through these phases sequentially, checking items off as
> they are completed.
>
> **How to use:** Before starting any phase, read the corresponding
> implementation document (01-19). Do not skip phases. Each phase's final
> "Verify" item must pass before moving to the next phase.
>
> **Last updated:** 2026-05-08

### Status Legend

| Symbol | Meaning |
|--------|---------|
| `[x]` | DONE -- implemented, tested, verified |
| `[~]` | PARTIAL -- code exists but incomplete or not CI-clean |
| `[ ]` | TODO -- not started |
| `[!]` | DIVERGED -- implementation diverged from plan |

### Phase Summary

| Phase | Name | Status | Notes |
|-------|------|--------|-------|
| 0 | Prerequisites | STALE | Workspace exists; CI not clean |
| 1 | Foundation (kora-hdc core) | DONE | 174 tests pass; clippy warnings remain |
| 2 | SIMD + Search | DONE | AVX2/NEON/scalar; HNSW uses HashSet (non-deterministic ties) |
| 3 | Knowledge Store | DONE | FSRS not implemented; brute-force Vec internally |
| 4 | Context + Cognitive | DONE | State machine uses different state names than spec |
| 5 | Block Time Migration | DONE | Config/runner done; env-var bridge missing |
| 6 | On-Chain Integration | DIVERGED | Precompile at 0x09 works; event sync is stub; Solidity HdcLib aligned on opcodes |
| 7 | Trust Pipeline | DONE | 5-layer immune system; simplified taint |
| 8 | RPC + Wiring | DONE | 7 RPC methods; runner wiring complete |
| 9 | E2E Tests | PARTIAL | 6 tests exist but have critical blockers (see doc 13) |
| 10 | Railway Deployment | NOT STARTED | Blocked on env-var bridge (see doc 14) |
| 11 | Final Verification | FAILING | `clippy -D warnings` fails |

---

## Phase 0: Prerequisites -- STALE/PARTIAL

These were written assuming a clean starting point. `crates/hdc/` now exists
with full implementation. The checklist items below reflect reality.

- [x] Read all impl docs: 00-overview through 19-solidity-specs
- [x] Read the design docs in `/Users/will/dev/nunchi/daeji/tmp/HDC/` (00-09)
- [x] Verify Rust toolchain: `rust-toolchain.toml` specifies `channel = "nightly"`
- [x] Verify project builds: `cargo build --all-targets`
- [x] Verify tests pass: `cargo nextest run --workspace --all-features`
- [~] Verify clippy is clean -- **FAILS** with 151 lint/doc/debug errors in kora-hdc
- [x] Verify formatting: `cargo +nightly fmt --all -- --check`
- [x] Understand workspace structure: review root `Cargo.toml` members and `crates/` layout
- [x] Understand the Justfile commands: `just ci` = `fmt clippy test deny`
- [!] ~~Confirm no existing `crates/hdc/` directory~~ -- **STALE**: full implementation already exists
- [x] Identify existing dependencies in `[workspace.dependencies]` that will be reused

---

## Phase 1: Foundation -- `kora-hdc` Core Crate -- DONE

Ref: `02-kora-hdc-core.md`

### 1.1 Scaffolding -- DONE

- [x] Create `crates/hdc/core/` directory structure
- [x] Create `crates/hdc/core/Cargo.toml` with workspace inheritance and dependencies
- [x] Create `crates/hdc/core/src/lib.rs` with module declarations (stubs)
- [x] Create `crates/hdc/core/src/constants.rs` (stub)
- [x] Create `crates/hdc/core/src/vector.rs` (stub)
- [x] Create `crates/hdc/core/src/bundle.rs` (stub)
- [x] Create `crates/hdc/core/src/encode.rs` (stub)
- [x] Create `crates/hdc/core/src/search.rs` (stub re-exports placeholder)
- [x] Add `"crates/hdc/*"` to workspace members in root `Cargo.toml`
- [x] Add `kora-hdc = { path = "crates/hdc/core" }` to `[workspace.dependencies]`
- [x] Add `rand_chacha = "0.3"` to `[workspace.dependencies]`
- [x] Add `bytemuck = { version = "1", features = ["derive"] }` to `[workspace.dependencies]`
- [x] Add `tiny-keccak = { version = "2", features = ["keccak"] }` to `[workspace.dependencies]`
- [x] Verify: `cargo check -p kora-hdc` compiles with no errors

### 1.2 Constants

- [x] Implement `D = 10_240` (dimensionality in bits)
- [x] Implement `WORDS = 160` (u64 words per vector)
- [x] Implement `BYTES = 1_280` (bytes per serialized vector)
- [x] Implement `THRESHOLD_HAMMING = 4_854` (on-chain similarity gate)
- [x] Implement `DUPLICATE_THRESHOLD = 512` (near-duplicate detection)
- [x] Implement `RESONANCE_THRESHOLD_HAMMING = 1_024` (meaningful similarity)

### 1.3 HdcVector Type

- [x] Implement `HdcVector` struct with `#[repr(C, align(64))]` and `pub [u64; WORDS]`
- [x] Derive `Clone, Debug, PartialEq, Eq, Hash`
- [x] Implement `unsafe impl Send for HdcVector` / `Sync`
- [x] Implement `Default` (zero vector)
- [x] Implement `bit(i)` -- returns bit at position i
- [x] Implement `set_bit(i, val)` -- sets bit at position i
- [x] Implement `popcount()` -- count of 1-bits

### 1.4 Deterministic Vector Generation

- [x] Implement `fnv1a_hash(data: &[u8]) -> u64`
- [x] Implement `HdcVector::random(seed: u64)` using `ChaCha20Rng::seed_from_u64`
- [x] Implement `HdcVector::symbol(name: &str)` using FNV-1a -> `random()`
- [x] Write test: `random_is_deterministic` (same seed = same vector)
- [x] Write test: `random_different_seeds_differ`
- [x] Write test: `symbol_is_deterministic` (same name = same vector)
- [x] Write test: `symbol_different_names_differ`
- [x] Write test: `random_vector_has_roughly_half_bits_set` (~5120 +/- range)
- [x] Write test: `random_vectors_are_quasi_orthogonal` (hamming ~5120)

### 1.5 Core Operations

- [x] Implement `bind(a, b)` (XOR, free function)
- [x] Implement `hamming_distance(a, b)` (XOR + count_ones, free function)
- [x] Implement `similarity(a, b)` (f64, OFF-CHAIN ONLY, free function)
- [x] Implement `permute(v, n)` (cyclic left rotation across full 10,240-bit vector)
- [x] Write test: `bind_self_inverse` -- `bind(bind(a,b), b) == a`
- [x] Write test: `bind_with_zero_is_identity`
- [x] Write test: `bind_is_commutative`
- [x] Write test: `bind_result_is_dissimilar_to_inputs`
- [x] Write test: `permute_zero_is_identity`
- [x] Write test: `permute_full_rotation_is_identity` -- `permute(v, D) == v`
- [x] Write test: `permute_inverse` -- `permute(permute(v, n), D - n) == v`
- [x] Write test: `permute_result_is_quasi_orthogonal`
- [x] Write test: `permute_composition` -- `permute(permute(v, a), b) == permute(v, a+b)`

### 1.6 Bundle

- [x] Implement `BundleAccumulator` with `i32` counters per bit position
- [x] Implement `BundleAccumulator::new()`
- [x] Implement `BundleAccumulator::add(vector)`
- [x] Implement `BundleAccumulator::add_weighted(vector, weight)`
- [x] Implement `BundleAccumulator::to_vector()` -- ties break to 0
- [x] Implement `Default` for `BundleAccumulator`
- [x] Implement `bundle(vectors)` convenience free function
- [x] Write test: `bundle_single_vector_returns_same`
- [x] Write test: `bundle_result_is_similar_to_all_inputs`
- [x] Write test: `bundle_tie_breaks_to_zero`
- [x] Write test: `bundle_accumulator_weighted`

### 1.7 Serialization and Content-Addressing

- [x] Implement `serialize(vector) -> [u8; BYTES]` (little-endian u64 words)
- [x] Implement `deserialize(bytes) -> HdcVector`
- [x] Implement `vector_id(vector) -> [u8; 32]` (keccak256 of serialized bytes)
- [x] Write test: `serialize_deserialize_roundtrip`
- [x] Write test: `serialize_length` (must be exactly 1,280 bytes)
- [x] Write test: `vector_id_is_deterministic`
- [x] Write test: `vector_id_differs_for_different_vectors`

### 1.8 Encoders

- [x] Implement `TrigramEncoder::encode(text)` -- character trigram sliding window
- [x] Implement `ProjectionEncoder::new(input_dim, seed)` -- OFF-CHAIN ONLY
- [x] Implement `ProjectionEncoder::encode(embedding)` -- OFF-CHAIN ONLY
- [x] Implement `StructuredEncoder::new()`
- [x] Implement `StructuredEncoder::add_field(role, filler)`
- [x] Implement `StructuredEncoder::add_field_vec(role, filler_vec)`
- [x] Implement `StructuredEncoder::encode()`
- [x] Implement `Default` for `StructuredEncoder`
- [x] Write test: `trigram_encoder_deterministic`
- [x] Write test: `trigram_encoder_similar_strings_are_similar`
- [x] Write test: `trigram_encoder_different_strings_are_dissimilar`
- [x] Write test: `trigram_encoder_empty_returns_zero`
- [x] Write test: `structured_encoder_role_filler_retrieval`

### 1.9 Cleanup

- [x] Wire all modules into `src/lib.rs` with public re-exports
- [x] Add consensus-safety doc comments to every public function
- [x] Verify: `cargo test -p kora-hdc` -- all tests pass
- [x] Verify: `cargo clippy -p kora-hdc` -- no warnings
- [x] Verify: `cargo doc -p kora-hdc --no-deps` -- docs build cleanly
- [x] Verify: no `f32`/`f64` in any CONSENSUS-SAFE function
- [x] Verify: no `thread_rng()`, `OsRng`, `StdRng` anywhere in the crate
- [x] Verify: no `HashMap` in iteration-order-sensitive paths

---

## Phase 2: SIMD + Vector Search -- DONE

Ref: `03-vector-search.md`

### 2.1 SIMD Popcount Kernels

- [x] Implement AVX2 Harley-Seal popcount (`#[cfg(target_arch = "x86_64")]`)
- [x] Implement AVX-512 VPOPCNTDQ popcount (`#[cfg(target_arch = "x86_64")]`)
- [x] Implement NEON popcount (`#[cfg(target_arch = "aarch64")]`)
- [x] Implement scalar fallback (existing `count_ones()` loop)
- [x] Implement runtime feature detection and dispatch
- [x] Write correctness test: SIMD results match scalar results for all kernels
- [x] Benchmark: verify SIMD popcount meets timing targets

### 2.2 Search Indexes

- [x] Implement `BruteForceIndex` with max-heap top-K search
- [x] Implement `HnswIndex` with deterministic level assignment
- [x] Implement `LocalIndex` enum (auto-switch: brute-force below threshold, HNSW above)
- [x] Implement `SearchResult` struct (vector_id, distance, metadata)

### 2.3 Tiered Search Pipeline

- [x] Implement first-word filter (compare only word 0 of vectors)
- [x] Implement sample-word filter (compare a subset of words)
- [x] Implement full hamming distance (final pass)
- [x] Wire tiers together: first-word -> sample-word -> full
- [x] Write search correctness tests (tiered results match brute-force)
- [x] Write determinism tests (same input -> same ranked results)
- [x] Benchmark: verify search timing targets

### 2.4 Phase 2 Verification

- [x] Verify: `cargo test -p kora-hdc` -- all search tests pass
- [x] Verify: `cargo clippy -p kora-hdc` -- no warnings
- [x] Verify: SIMD paths are tested on CI (or fallback-only if CI lacks AVX)

---

## Phase 3: Knowledge Store -- DONE

Ref: `04-knowledge-store.md`

### 3.1 Knowledge Types

- [x] Implement `KnowledgeKind` enum (6 types: Fact, Episodic, Procedural, Semantic, Social, Meta)
- [x] Implement `KnowledgeTier` enum (4 levels: Ephemeral, Working, Reference, Core)
- [x] Implement `KnowledgeEntry` struct (vector, kind, tier, balance, timestamps, metadata)
- [x] Implement serialization for `KnowledgeEntry`

### 3.2 Knowledge Store Core

- [x] Implement `KnowledgeStore` struct
- [x] Implement `insert(entry)` -- add entry, index vector
- [x] Implement `search(query, top_k)` -- return ranked results
- [x] Implement `tick(current_time)` -- process decay, promotion, GC
- [x] Implement `get(id)` / `remove(id)` accessors

### 3.3 Decay and Scheduling

- [x] Implement exponential decay/demurrage (lambda = 0.005/hour, half-life ~138.6h)
- [x] Implement tier promotion logic (Ephemeral -> Working -> Reference -> Core)
- [x] Implement tier demotion logic (balance drops -> demote)
- [x] Implement garbage collection (remove entries with balance < 0.01)
- [ ] Implement FSRS scheduling integration (spaced repetition for rehearsal) -- **NOT IMPLEMENTED, no `fsrs` module exists**

### 3.4 Anti-Knowledge

- [x] Implement `ANTI_SUBSPACE` basis vector (LazyLock + ChaCha20, fixed seed)
- [x] Implement anti-knowledge vector creation (bind with ANTI_SUBSPACE)
- [x] Implement anti-knowledge search (detect and filter contradictions)

### 3.5 Scoring

- [x] Implement 4-factor scoring: relevance (hamming), freshness, trust, tier-weight
- [x] Implement score combination logic (weighted product or sum)

### 3.6 Phase 3 Verification

- [x] Write knowledge store insert/search tests
- [x] Write decay/demurrage tests (verify half-life behavior)
- [x] Write tier promotion/demotion tests
- [x] Write GC tests (entries below threshold are removed)
- [x] Write anti-knowledge tests
- [x] Verify: `cargo test -p kora-hdc` -- all knowledge tests pass

---

## Phase 4: Context Assembly + Cognitive Architecture -- DONE

Ref: `05-context-assembly.md`, `06-cognitive-architecture.md`

### 4.1 Context Assembly Pipeline

- [x] Implement `gather()` -- collect candidate knowledge entries
- [x] Implement `rank()` -- score and sort candidates
- [x] Implement `compress()` -- dedup near-identical entries (DUPLICATE_THRESHOLD)
- [x] Implement `assemble()` -- construct final prompt context within token budget

### 4.2 VCG Auction

- [x] Implement `knapsack_01()` dynamic programming solver
- [x] Implement VCG pricing (each item pays its externality cost)
- [x] Implement VCG payment tracking (debit balance from selected entries)
- [x] Write VCG correctness tests

### 4.3 ALMA Affect Model

- [x] Implement 3-layer affect model (reaction, mood, personality)
- [x] Implement `PadState` struct (Pleasure, Arousal, Dominance) with basis vectors
- [x] Implement affect decay and blending across layers
- [x] Implement somatic bias (affect influences attention/scoring weights)

### 4.4 Behavioral State Machine

- [x] Implement 6-state machine (Idle, Curious, Focused, Stressed, Dreaming, Flow)
- [x] Implement state transition logic (triggers and conditions)
- [x] Implement dream cycle (offline consolidation mode)
- [x] Implement state-dependent behavior modifiers

### 4.5 Phase 4 Verification

- [x] Write context assembly pipeline tests (gather -> rank -> compress -> assemble)
- [x] Write VCG auction tests (correct winners, correct payments)
- [x] Write affect model tests (decay, blending, somatic bias)
- [x] Write state machine tests (transitions, edge cases)
- [x] Verify: `cargo test -p kora-hdc` -- all context/cognitive tests pass

---

## Phase 5: Block Time Migration -- DONE

Ref: `01-block-time-migration.md`

### 5.1 Config Changes

- [x] Rename `DEFAULT_BLOCK_TIME` to `DEFAULT_BLOCK_TIME_MS` (value: 2 -> 2000)
- [x] Rename `block_time` field to `block_time_ms` in `ExecutionConfig`
- [x] Add `#[serde(alias = "block_time")]` for backward compatibility
- [x] Update serde default function to `default_block_time_ms()`
- [x] Update `Default` impl for `ExecutionConfig`
- [x] Update re-export in `crates/node/config/src/lib.rs`

### 5.2 Runner Changes

- [x] Add `block_time_ms: u64` field to `ProductionRunner`
- [x] Update `ProductionRunner::new()` signature to accept `block_time_ms`
- [x] Replace hardcoded `Duration::from_secs()` timeouts with `Duration::from_millis(block_time_ms)`
  - [x] `leader_timeout = 1x block_time_ms`
  - [x] `certification_timeout = 2x block_time_ms`
  - [x] `timeout_retry = 1x block_time_ms`
  - [x] `fetch_timeout = 2x block_time_ms`

### 5.3 CLI and Callers

- [x] Update `ProductionRunner::new()` call in `bin/kora/src/cli.rs` to pass `block_time_ms`

### 5.4 Config Documentation

- [x] Update `crates/node/config/README.md` example TOML snippet

### 5.5 E2E Harness (Optional but Recommended)

- [x] Add `block_time_ms: u64` to `TestConfig` in `crates/e2e/src/setup.rs`
- [x] Derive harness timeouts from `TestConfig::block_time_ms` in `crates/e2e/src/harness.rs`

### 5.6 Docker Compose (Optional)

- [x] Add `BLOCK_TIME_MS` environment variable to `docker/compose/devnet.yaml`

### 5.7 Update Tests

- [x] Update all test assertions in `crates/node/config/src/execution.rs`
- [x] Add backward-compat alias test (`"block_time": 5` deserializes via alias)
- [x] Verify: `cargo test -p kora-config` -- all config tests pass
- [x] Verify: `cargo test -p kora-e2e` -- e2e tests still pass (if modified)

### 5.8 Phase 5 Verification

- [x] Verify: `cargo build --workspace` -- no compile errors
- [x] Verify: `cargo clippy --workspace` -- no new warnings
- [x] Verify: all existing tests still pass with `cargo nextest run --workspace --all-features`
- [x] Verify: no remaining references to `DEFAULT_BLOCK_TIME` (without `_MS`)
- [x] Verify: no remaining struct field accesses to `.block_time` (without `_ms`)

---

## Phase 6: On-Chain Integration -- DIVERGED

Ref: `07-precompile-integration.md`, `08-insight-board-contract.md`, `09-pheromone-registry.md`, `10-kora-hdc-chain.md`

### 6.1 Chain Crate Scaffolding -- DONE

- [x] Create `crates/hdc/chain/` directory structure
- [x] Create `crates/hdc/chain/Cargo.toml` with dependencies on `kora-hdc`, `revm`, `alloy-primitives`
- [x] Create `crates/hdc/chain/src/lib.rs`
- [x] Add `kora-hdc-chain = { path = "crates/hdc/chain" }` to `[workspace.dependencies]`
- [x] Verify: `cargo check -p kora-hdc-chain` compiles (with warnings)

### 6.2 HDC Precompile -- DONE (address 0x09, opcodes 0x01-0x06)

- [x] Implement precompile at address `0x09` -- NOTE: replaces EIP-152 BLAKE2F (sovereign chain fork)
- [x] Implement opcode 0x01: `hamming_distance(a, b) -> u32`
- [x] Implement opcode 0x02: `bind(a, b) -> HdcVector`
- [x] Implement opcode 0x03: `bundle(vectors) -> HdcVector`
- [x] Implement opcode 0x04: `permute(v, n) -> HdcVector`
- [x] Implement opcode 0x05: `vector_id(v) -> bytes32`
- [x] Implement opcode 0x06: `is_similar(a, b) -> bool` (integer threshold check)
- [x] Implement gas cost calculation for each opcode
- [x] Implement ABI encoding/decoding for precompile inputs/outputs

### 6.3 Precompile Registration -- DONE

- [x] Register HDC precompile in executor/revm.rs (custom precompile set via `hdc_precompiles.rs`)
- [x] Verify precompile is callable from EVM execution context (production path)
- [ ] Register HDC precompile in `TestApplication` for E2E tests -- **MISSING**

### 6.4 Consensus-Safe On-Chain Primitives -- PARTIAL/STUB

- [ ] Implement `fixed_point_decay()` (integer-only exponential decay for on-chain use) -- **NOT IMPLEMENTED**
- [x] Implement `OnChainHdcIndex` (on-chain vector storage and lookup) -- `insert_insight()` works
- [~] Implement event sync (chain events -> off-chain index updates) -- **STUB: all topic hashes are `B256::ZERO`, `process_log` is no-op**
- [ ] Implement `record_pheromone()` in `OnChainHdcIndex` -- **TODO stub**
- [ ] Compute actual keccak256 topic hashes for event signatures in `event.rs`

### 6.5 WisdomGate -- PARTIAL/WRONG SHAPE

- [~] Implement WisdomGate logic (submit/challenge/resolve lifecycle) -- **EXISTS but wrong shape**: not the spec's stateless 5-check filter, not wired into finalized event processing

### 6.6 Solidity Contracts -- PARTIAL

- [x] Write `InsightBoard.sol` (7-state FSM) -- exists with Foundry tests
- [x] Write `PheromoneRegistry.sol` -- exists with Foundry tests
- [x] Write Solidity test harness -- `TestableInsightBoard` exists
- [~] Verify: Foundry tests pass -- **58 tests PASS** but mock around the real precompile path; `HdcLib` and Rust precompile opcodes are now aligned (0x01-0x06)

### 6.7 Phase 6 Verification -- PARTIAL

- [x] Write precompile unit tests (each opcode, gas metering, edge cases) -- **14 tests in `precompile.rs`, all pass**
- [x] Write contract tests via Foundry (`forge test`) -- **58 tests pass**
- [x] Verify: `cargo test -p kora-hdc-chain` -- tests pass
- [~] Verify: `cargo clippy -p kora-hdc-chain` -- **FAILS** with 12 warnings
- [x] Verify: precompile produces deterministic results (consensus-safe)

---

## Phase 7: Trust Pipeline -- DONE

Ref: `11-trust-pipeline.md`

- [x] Implement 5-layer immune system (innate, adaptive, social, consensus, meta)
- [x] Implement taint propagation (knowledge tainted by untrusted sources)
- [x] Implement trust scoring with `COLD_START_REPUTATION = 0.1`
- [x] Implement `MIN_TRUST_THRESHOLD = 0.05` enforcement
- [x] Write trust pipeline tests
- [x] Verify: trust scoring is deterministic and consensus-safe

---

## Phase 8: RPC + Wiring -- DONE

Ref: `12-rpc-extensions.md`, `15-wiring-guide.md`

### 8.1 RPC Namespace

- [x] Implement `hdc_hammingDistance(a, b)` RPC method
- [x] Implement `hdc_similarity(a, b)` RPC method
- [x] Implement `hdc_bind(a, b)` RPC method
- [x] Implement `hdc_bundle(vectors)` RPC method
- [x] Implement `hdc_search(query, topK)` RPC method
- [x] Implement `hdc_vectorId(v)` RPC method
- [x] Implement `hdc_encode(text)` RPC method

### 8.2 Node Wiring

- [x] Wire HDC subsystem into `runner.rs` (initialize on node startup)
- [x] Wire HDC subsystem into `service.rs` (lifecycle management)
- [x] Wire HDC RPC into `server.rs` (register RPC namespace)
- [x] Add `HdcConfig` section to `NodeConfig` (enable/disable, index size, etc.)
- [x] Verify HDC components start and stop cleanly with the node

### 8.3 Phase 8 Verification

- [x] Write RPC integration tests (call each method, verify responses)
- [x] Write wiring tests (node boots with HDC enabled, HDC disabled)
- [x] Verify: `cargo test -p kora-hdc-chain` -- all RPC tests pass
- [x] Verify: RPC methods are accessible via `curl` or `cast` against a running node

---

## Phase 9: End-to-End Tests -- PARTIAL

Ref: `13-e2e-tests.md`

### 9.1 Test Infrastructure -- PARTIAL

- [x] Add `hdc` module to `crates/e2e/src/tests/mod.rs`
- [x] Add `kora-hdc-chain` dependency to `crates/e2e/Cargo.toml`
- [x] Add `sign_eip1559_call` helper to `crates/node/domain/src/evm.rs`
- [~] Extend `TestSetup` / `TestConfig` for HDC genesis state -- basic setup works, no `.with_hdc_precompile()` on TestApplication
- [ ] Register HDC precompile in TestApplication executor -- **CRITICAL BLOCKER**
- [ ] Add `TestConfig::with_block_time_ms()` builder method

### 9.2 HDC E2E Tests -- PARTIAL (6 tests exist, 0 verify output)

- [x] `test_hdc_precompile_bind` -- passes but only checks `blocks_finalized == 3`, not output
- [x] `test_hdc_precompile_hamming` -- same limitation
- [x] `test_hdc_precompile_bundle` -- same limitation
- [~] `test_hdc_consensus_determinism` -- compares state_root across runs (correct pattern, has `#[ignore]`)
- [~] `test_hdc_fast_blocks` -- tests many blocks with low latency, NOT actual 50ms block time
- [x] `test_hdc_precompile_with_transfers` -- passes, verifies mixed ops + balance
- [ ] Write InsightBoard e2e test (full lifecycle: submit -> challenge -> resolve) -- **MISSING**
- [ ] Write PheromoneRegistry e2e test (register, decay, query) -- **MISSING**
- [ ] Add recorder contract pattern for precompile output verification -- **MISSING** (see doc 13)
- [ ] Add `#[ignore]` to 4 tests that are missing it -- **see audit F12**

### 9.3 Phase 9 Verification -- BLOCKED

- [~] Verify: `cargo test -p kora-e2e` -- tests pass but do not verify HDC correctness
- [ ] Verify: tests complete within timeout (30s default for e2e)
- [ ] Verify: no flaky tests (run 3x to confirm stability)

---

## Phase 10: Railway Deployment -- NOT STARTED

Ref: `14-railway-deployment.md`

### 10.1 Configuration

- [ ] Create `railway.toml` for each service (validator-0, validator-1, validator-2)
- [ ] Review and modify `docker/Dockerfile` if needed for Railway
- [ ] Create init-config service (key generation, genesis distribution)
- [ ] Configure `block_time_ms = 50` for the Railway testnet
- [ ] Set up environment variables for each validator
- [ ] Configure health checks for each service

### 10.2 Deployment

- [ ] Deploy init-config service to Railway
- [ ] Deploy 3 validator nodes to Railway
- [ ] Verify all 3 validators discover each other (peer connections)
- [ ] Verify validators reach consensus (blocks are being produced)
- [ ] Verify 50ms block times (check block timestamps / intervals in logs)

### 10.3 Validation

- [ ] Run loadgen against Railway testnet
- [ ] Verify HDC precompile is callable from deployed testnet
- [ ] Verify RPC methods respond correctly on deployed testnet
- [ ] Monitor for 1 hour: no crashes, no consensus stalls, no memory leaks

---

## Phase 11: Final Verification -- FAILING

This is the comprehensive check before declaring the implementation complete.

### 11.1 CI Suite -- FAILING

- [~] All tests pass: `just ci` -- **FAILS** (clippy step blocks)
- [~] No clippy warnings: `just clippy` -- **FAILS** (151 kora-hdc + 12 kora-hdc-chain warnings)
- [x] Formatting clean: `just fmt`
- [x] Dependency audit: `just deny`
- [x] Full build: `cargo build --all-targets`

### 11.2 Crate-Level Checks -- PARTIAL

- [x] `cargo test -p kora-hdc` -- 174 lib tests + 2 knowledge + 2 search + 1 stress = all pass
- [x] `cargo test -p kora-hdc-chain` -- chain tests pass
- [x] `cargo test -p kora-config` -- config tests pass (block time migration)
- [~] `cargo test -p kora-e2e` -- e2e tests pass but do not verify HDC correctness (see doc 13)
- [~] `cargo doc --workspace --no-deps` -- builds with doc warnings

### 11.3 Consensus Safety Audit -- PARTIAL

- [x] No `f32`/`f64` in any function called from consensus/execution path
- [~] No `HashMap` iteration in consensus-critical code -- **3 violations found** (OnChainHdcIndex, WisdomGate, KnowledgeStore)
- [x] No `thread_rng()`, `OsRng`, `StdRng` in deterministic paths
- [x] All on-chain threshold comparisons use integer Hamming distance
- [x] `HdcVector::random(42)` returns identical results across platforms (ChaCha20 is platform-independent)
- [x] Precompile gas costs are well-defined and bounded

### 11.4 Deployment Verification -- NOT STARTED

- [ ] Railway testnet is producing blocks at ~50ms intervals
- [ ] HDC precompile is callable from Solidity on the testnet
- [ ] `hdc_*` RPC methods respond correctly against testnet
- [ ] Loadgen runs successfully against testnet
- [ ] No validator crashes over sustained operation

### 11.5 Documentation -- PARTIAL

- [~] All public functions have doc comments with consensus-safety annotation -- most present, clippy reports missing docs
- [~] Implementation docs (00-19) are consistent with final code -- API divergences documented in audit findings
- [~] Anti-patterns from `17-anti-patterns.md` are not present in code -- 3 `HashMap` violations, several `unwrap()` in production paths

---

## Appendix A: Document Dependencies Quick Reference

Use this to know which doc to read before each phase.

| Phase | Read Before Starting |
|-------|---------------------|
| 0 | `00-overview.md` |
| 1 | `02-kora-hdc-core.md`, `18-encoding-specs.md` |
| 2 | `03-vector-search.md` |
| 3 | `04-knowledge-store.md` |
| 4 | `05-context-assembly.md`, `06-cognitive-architecture.md` |
| 5 | `01-block-time-migration.md` |
| 6 | `07-precompile-integration.md`, `08-insight-board-contract.md`, `09-pheromone-registry.md`, `10-kora-hdc-chain.md`, `19-solidity-specs.md` |
| 7 | `11-trust-pipeline.md` |
| 8 | `12-rpc-extensions.md`, `15-wiring-guide.md` |
| 9 | `13-e2e-tests.md` |
| 10 | `14-railway-deployment.md` |
| 11 | `17-anti-patterns.md` (final audit) |
| 12 | All impl docs (reconciliation pass) |
| 13 | `10-kora-hdc-chain.md`, `07-precompile-integration.md` |
| 14 | `17-anti-patterns.md`, clippy output |
| 15 | `14-railway-deployment.md`, `01-block-time-migration.md` |

---

## Appendix B: Key Commands

```bash
# Full CI suite
just ci

# Individual checks
just fmt          # Check formatting
just clippy       # Lint
just test         # Run all tests
just deny         # Dependency audit

# Crate-specific testing
cargo test -p kora-hdc
cargo test -p kora-hdc-chain
cargo test -p kora-config
cargo test -p kora-e2e

# Build
cargo build --all-targets
cargo build --release

# Documentation
cargo doc --workspace --no-deps

# Devnet
just devnet       # Start local devnet
just devnet-down  # Stop devnet
just loadtest     # Quick load test (1000 txs)
```

---

## Appendix C: Critical Constants Reference

These values are consensus-critical. They must match exactly across all implementations.

| Constant | Value | Usage |
|----------|-------|-------|
| `D` | 10,240 | Hypervector dimensionality (bits) |
| `WORDS` | 160 | `u64` words per vector |
| `BYTES` | 1,280 | Serialized vector size |
| `THRESHOLD_HAMMING` | 4,854 | On-chain similarity gate |
| `DUPLICATE_THRESHOLD` | 512 | Near-duplicate detection |
| `RESONANCE_THRESHOLD_HAMMING` | 1,024 | Strong similarity |
| `COLD_START_REPUTATION` | 0.1 | Initial trust for new sources |
| `MIN_TRUST_THRESHOLD` | 0.05 | Minimum trust to participate |
| `DEFAULT_BLOCK_TIME_MS` | 2,000 | Default block time (milliseconds) |
| Precompile address | `0x09` | HDC precompile EVM address |
| Demurrage lambda | 0.005/hour | Knowledge decay rate |

---

## Phase 12: Reconciliation (NEW) -- Resolve PR vs Plan Divergences

These items address the gap between the original implementation plan and what
was actually built.

- [ ] Decide canonical precompile address: keep 0x09 (sovereign fork) or move to 0xA0C (non-conflicting)
- [ ] If moving to 0xA0C: update `PRECOMPILE_ADDRESS` in `precompile.rs`, `HdcLib.HDC_PRECOMPILE` in Solidity, all docs, all tests
- [ ] Decide canonical precompile crate: `kora-hdc-chain/precompile.rs` vs `crates/node/executor/src/hdc_precompiles.rs`
- [ ] Reconcile `KnowledgeKind` enum names (spec: Fact/Episodic/Procedural/Semantic/Social/Meta; actual: may differ)
- [ ] Reconcile `KnowledgeTier` enum names (spec: Ephemeral/Working/Reference/Core; actual: may use different names)
- [ ] Reconcile behavioral state names (spec: Idle/Curious/Focused/Stressed/Dreaming/Flow; actual: Exploring/Exploiting/Cautious/Stressed/Recovering/Dreaming)
- [ ] Reconcile API signatures in docs: `HdcVector::random(&mut rng)` vs `HdcVector::random(seed: u64)`, methods vs free functions
- [ ] Remove orphan `crates/kora-hdc/` duplicate directory if it exists outside workspace
- [ ] Update all impl docs (00-19) to use actual API signatures

---

## Phase 13: Remaining Stubbed Implementations (NEW)

- [ ] Implement real `process_log()` in `crates/hdc/chain/src/event.rs` (currently no-op)
- [ ] Compute actual keccak256 topic hashes for `InsightPublished`, `PheromoneDeposited` event signatures
- [ ] Implement `record_pheromone()` in `crates/hdc/chain/src/index.rs:131`
- [ ] Implement `fixed_point_decay()` for consensus-safe on-chain decay
- [ ] Implement FSRS scheduling integration in knowledge store
- [ ] Implement WisdomGate as spec's stateless 5-check filter (not current submit/challenge/resolve)
- [ ] Wire WisdomGate into finalized event processing
- [ ] Implement `KnowledgeStore::open()` for disk persistence (currently stub)
- [ ] Implement `max_entries` enforcement in KnowledgeStore

---

## Phase 14: Bug Fixes (NEW)

- [ ] Fix trust EMA: verify exponential moving average implementation matches spec
- [ ] Fix stored-vs-computed state: ensure `OnChainHdcIndex` stored state matches what validators compute
- [ ] Replace `unwrap()` on line 158 of `crates/hdc/chain/src/precompile.rs` with proper error handling
- [ ] Replace `unwrap()` on line 150 of `crates/hdc/core/src/vector.rs` in `deserialize()` with error propagation
- [ ] Replace `unwrap()` calls in HNSW graph operations (`hnsw.rs` lines 206, 222, 262, 267, 274, 356)
- [ ] Audit and fix `HashMap` iteration in `OnChainHdcIndex::search()` -- non-deterministic tie-breaking
- [ ] Audit and fix `HashMap` iteration in `WisdomGate::resolve()` -- non-deterministic order
- [ ] Audit and fix `HashMap` iteration in `KnowledgeStore::tick()` -- non-deterministic GC ordering
- [ ] Replace `HashSet` in HNSW `search_layer` with `BTreeSet` for deterministic results
- [ ] Fix clippy warnings: resolve 151 lint/doc/debug errors in `kora-hdc`
- [ ] Fix clippy warnings: resolve 12 warnings in `kora-hdc-chain`

---

## Phase 15: Railway Deployment (NEW)

- [ ] Implement env-var-to-config bridge: binary reads `BLOCK_TIME_MS`, `GAS_LIMIT`, `HDC_ENABLED` from environment
- [ ] OR: generate TOML config file in init-config service and mount into validators
- [ ] Remove `healthcheckPath` from `railway.toml` (rely on Dockerfile HEALTHCHECK)
- [ ] Create Railway project and services (validator-0, validator-1, validator-2, init-config)
- [ ] Set up volumes (shared-config, data-node0, data-node1, data-node2)
- [ ] Deploy init-config, verify output
- [ ] Deploy validators in order, verify consensus
- [ ] Add monitoring and alerting rules (see doc 14 section 17)
- [ ] Verify 50ms block production (or fall back to 100ms/200ms)
- [ ] Run 1-hour stability test

---

## Next Actions -- Top 5 Most Impactful Items

These are ordered by impact and dependency chain. Work on them in this order.

### 1. Fix CI baseline (Phase 14)

**Why:** Every other verification step is blocked by clippy failures.

```bash
cargo clippy -p kora-hdc --all-targets -- -D warnings
cargo clippy -p kora-hdc-chain --all-targets -- -D warnings
```

Fix the 151 + 12 warnings. This is mechanical work (doc comments, unused imports,
debug format strings) and does not require design decisions.

### 2. Register HDC precompile in TestApplication (Phase 6.3 / Phase 9)

**Why:** All E2E tests are currently testing consensus mechanics, not HDC correctness.
Without precompile registration in the test executor, the precompile calls may
silently fail.

**File:** `crates/e2e/src/harness.rs` -- find where `RevmExecutor::new()` is called
and add `.with_hdc_precompile()`.

### 3. Implement recorder contract pattern for E2E output verification (Phase 9)

**Why:** Even with the precompile registered, there is no way to observe the
precompile output post-finalization without a recorder contract that stores
results in EVM state.

**See:** Doc 13, section 11 for the concrete Solidity contract and Rust test code.

### 4. Implement event sync in `event.rs` (Phase 13)

**Why:** The on-chain HDC index is populated by replaying finalized block events.
Without real event sync, the `hdc_search` RPC method returns empty results
because the index never receives data.

**File:** `crates/hdc/chain/src/event.rs` -- compute real topic hashes, implement
ABI-decode logic in `process_log()`.

### 5. Implement env-var bridge or config generation for Railway (Phase 15)

**Why:** Railway deployment is blocked because the binary ignores `BLOCK_TIME_MS`,
`GAS_LIMIT`, and `HDC_ENABLED` environment variables.

**Options:**
- (a) Add `std::env::var("BLOCK_TIME_MS")` override in `cli.rs`
- (b) Generate TOML config in init-config service and set `CONFIG_FILE` env var

---

## Audit Findings -- Checklist Verification

> **Audit date:** 2026-05-08
> **Scope:** All checklist phases verified against actual source code in the repository.

### Phase 0: Prerequisites -- COMPLETED

All prerequisite items are structurally satisfied:
- Workspace contains `crates/hdc/*` members.
- `crates/hdc/core/` and `crates/hdc/chain/` directories exist with Cargo.toml files.
- Workspace dependencies include `rand_chacha`, `bytemuck`, `tiny-keccak`.

### Phase 1: Foundation (`kora-hdc` Core) -- COMPLETED

Every item in Phase 1 is implemented and verified:

| Section | Status | Evidence |
|---------|--------|----------|
| 1.1 Scaffolding | DONE | `crates/hdc/core/src/lib.rs` declares all modules; workspace members configured. |
| 1.2 Constants | DONE | `crates/hdc/core/src/constants.rs` -- all 6 constants match spec values exactly (`D=10_240`, `WORDS=160`, `BYTES=1_280`, `THRESHOLD_HAMMING=4_854`, `DUPLICATE_THRESHOLD=512`, `RESONANCE_THRESHOLD_HAMMING=1_024`). |
| 1.3 HdcVector | DONE | `crates/hdc/core/src/vector.rs` -- `#[repr(C, align(64))]`, derives correct, `Send+Sync` impl, `Default`, `bit()`, `set_bit()`, `popcount()` all present. |
| 1.4 Deterministic Generation | DONE | `fnv1a_hash`, `HdcVector::random()` with ChaCha20, `HdcVector::symbol()`. All 6 tests present and passing. |
| 1.5 Core Operations | DONE | `bind`, `hamming_distance`, `similarity` (marked OFF-CHAIN), `permute`. All 9 tests present. |
| 1.6 Bundle | DONE | `BundleAccumulator` with `i32` counters, `new()`, `add()`, `add_weighted()`, `to_vector()`, `Default`, `bundle()` free function. All 4 tests present. |
| 1.7 Serialization | DONE | `serialize`, `deserialize`, `vector_id` (keccak256). All 4 tests present. |
| 1.8 Encoders | DONE | `TrigramEncoder`, `ProjectionEncoder` (OFF-CHAIN), `StructuredEncoder`. All 7 tests present (including extras: single char, two chars, projection deterministic, structured empty). |
| 1.9 Cleanup | DONE | All modules wired into `lib.rs` with public re-exports. Doc comments present. |

### Phase 2: SIMD + Vector Search -- COMPLETED

| Section | Status | Evidence |
|---------|--------|----------|
| 2.1 SIMD Popcount | DONE | `crates/hdc/core/src/search/simd.rs` -- AVX2 Harley-Seal, AVX-512 VPOPCNTDQ, NEON, scalar fallback, runtime dispatch. Correctness test (`hamming_simd_matches_scalar`) runs 100 seed pairs. |
| 2.2 Search Indexes | DONE | `BruteForceIndex` (`brute.rs`), `HnswIndex` (`hnsw.rs`), `LocalIndex` (`local.rs`) with auto-switch. `SearchResult` type. |
| 2.3 Tiered Pipeline | DONE | `TieredSearchPipeline` in `tiered.rs`. |
| 2.4 Verification | DONE | HNSW determinism test, brute-force/HNSW agreement test, SIMD/scalar agreement test all present. |

### Phase 3: Knowledge Store -- COMPLETED

| Section | Status | Evidence |
|---------|--------|----------|
| 3.1 Knowledge Types | DONE | `KnowledgeKind` (6 types in `kind.rs`), `KnowledgeTier` (4 levels in `tier.rs`), `KnowledgeEntry` (`entry.rs`). |
| 3.2 Knowledge Store Core | DONE | `KnowledgeStore` in `store.rs` with `insert`, `search`, `tick`, `get`, `remove`. |
| 3.3 Decay and Scheduling | DONE | `decay.rs` implements exponential decay. `tier.rs` has promotion/demotion. `store.rs:tick()` handles decay, promotion, demotion, GC. |
| 3.4 Anti-Knowledge | DONE | `anti.rs` -- `ANTI_SUBSPACE` via `LazyLock`, `encode_anti`, `is_anti`. `store.rs:search_with_anti_check` for filtering. |
| 3.5 Scoring | DONE | `scoring.rs` -- 4-factor scoring (relevance, freshness, trust, tier-weight). |
| 3.6 Verification | DONE | Tests for insert/search, decay/GC, promotion/demotion, anti-knowledge, persistent-never-deleted. |

**Note:** FSRS scheduling integration (checklist item 3.3) is NOT implemented. No FSRS module exists. This was likely deferred.

### Phase 4: Context Assembly + Cognitive Architecture -- COMPLETED

| Section | Status | Evidence |
|---------|--------|----------|
| 4.1 Context Assembly | DONE | `context.rs` -- `gather()`, `rank()`, `compress()`, `assemble()`, full `assemble_context()` pipeline. |
| 4.2 VCG Auction | DONE | `context.rs` -- `knapsack_01()` DP solver, `vcg_allocate()` with externality pricing. Tests for correctness, truthfulness, edge cases. |
| 4.3 ALMA Affect | DONE | `cognitive/affect.rs` -- 3-layer model (emotion/mood/personality), `PadState`, affect decay, somatic bias vector encoding. |
| 4.4 Behavioral State Machine | DONE | `cognitive/state_machine.rs` -- 6 states (Exploring, Exploiting, Cautious, Stressed, Recovering, Dreaming). Transition logic. `cognitive/dream.rs` for dream cycle. |
| 4.5 Verification | DONE | Context pipeline E2E tests, VCG tests, affect/state machine tests present. |

### Phase 5: Block Time Migration -- COMPLETED

| Section | Status | Evidence |
|---------|--------|----------|
| 5.1 Config Changes | DONE | `crates/node/config/src/execution.rs` -- `DEFAULT_BLOCK_TIME_MS = 2000`, `block_time_ms` field, `#[serde(alias = "block_time")]`, serde default function. |
| 5.2-5.7 Runner, CLI, Tests | DONE | Runner uses `block_time_ms`, backward-compat alias test present (`"block_time": 5` test at line 92-96). |

### Phase 6: On-Chain Integration -- PARTIALLY COMPLETED

| Section | Status | Evidence |
|---------|--------|----------|
| 6.1 Chain Crate Scaffolding | DONE | `crates/hdc/chain/` exists with `Cargo.toml`, `lib.rs`, all 5 submodules. |
| 6.2 HDC Precompile | PARTIAL / BLOCKED | `precompile.rs` has a raw-opcode stateless implementation at `0x09`, but gas costs are underpriced, malformed trailing bytes are accepted, and Solidity `HdcLib` currently expects a different stateful opcode table. |
| 6.3 Precompile Registration | PARTIAL | `crates/node/executor/src/hdc_precompiles.rs` wraps `EthPrecompiles` and intercepts `0x09`, but the address conflicts with EIP-152 BLAKE2F unless this chain intentionally forks that precompile. |
| 6.4 Consensus-Safe Primitives | PARTIAL | `OnChainHdcIndex` exists (`index.rs`). Event sync is **STUB ONLY** (`event.rs` -- all topic hashes are `B256::ZERO`, `process_log` is a no-op). `fixed_point_decay()` is NOT implemented. |
| 6.5 WisdomGate | PARTIAL / WRONG SHAPE | `wisdom.rs` has a submit/challenge/resolve lifecycle, but it is not the spec's stateless 5-check `wisdom_gate.rs` filter and is not wired into finalized event processing. |
| 6.6 Solidity Contracts | PARTIAL / INCOMPATIBLE | `contracts/src/InsightBoard.sol` and `contracts/src/PheromoneRegistry.sol` exist with Foundry tests, but their `HdcLib` precompile expectations do not match Rust and several lifecycle/stake/read paths remain incomplete. |
| 6.7 Verification | PARTIAL | Rust core tests and Foundry tests pass in the current snapshot, but contract tests mock around the real precompile path and e2e tests do not assert precompile return values. |

### Phase 7: Trust Pipeline -- COMPLETED

| Section | Status | Evidence |
|---------|--------|----------|
| Trust Pipeline | DONE | `crates/hdc/core/src/trust.rs` -- 5-layer immune system, taint propagation, trust scoring with `COLD_START_REPUTATION = 0.1`, `MIN_TRUST_THRESHOLD = 0.05`. Tests present. |
| Knowledge-level trust | DONE | `crates/hdc/core/src/knowledge/trust.rs` -- `compute_trust()` function. |

### Phase 8: RPC + Wiring -- COMPLETED

| Section | Status | Evidence |
|---------|--------|----------|
| 8.1 RPC Namespace | DONE | `crates/node/rpc/src/hdc.rs` -- all 7 methods (`hammingDistance`, `similarity`, `bind`, `bundle`, `search`, `vectorId`, `encode`). |
| 8.2 Node Wiring | DONE | `runner.rs` wires HDC into node startup; `server.rs` registers RPC namespace; `crates/node/config/src/hdc.rs` provides `HdcConfig`. |
| 8.3 Verification | DONE | RPC integration tests for all methods in `hdc.rs`. |

### Phase 9-11: E2E, Deployment, Final Verification -- NOT VERIFIED

These phases relate to deployment, E2E harness, and CI -- not auditable from source alone.

---

## Items Incorrectly Marked

All checklist items are marked `[ ]` (unchecked). Based on the audit, the following should now be marked `[x]`:

- **All of Phase 1** (1.1 through 1.9): fully implemented and tested.
- **All of Phase 2** (2.1 through 2.4): SIMD, brute-force, HNSW, local, tiered search all implemented.
- **All of Phase 3** (3.1 through 3.6): knowledge store fully operational. **Exception:** FSRS scheduling (item in 3.3) is not implemented.
- **All of Phase 4** (4.1 through 4.5): context assembly and cognitive architecture complete.
- **All of Phase 5** (5.1 through 5.8): block time migration complete.
- **Phase 6** items 6.1, 6.2, 6.3, 6.5: completed. Items 6.4 (event sync, fixed-point decay), 6.6 (Solidity contracts), 6.7 (contract tests): NOT complete.
- **All of Phase 7**: trust pipeline complete.
- **All of Phase 8** (8.1 through 8.3): RPC and wiring complete.

---

## New Items to Add

The following items should be added to the checklist based on the audit:

### Missing from Phase 3

- [ ] Implement FSRS scheduling integration (listed in 3.3 but not implemented; no `fsrs` module exists)

### Missing from Phase 6

- [ ] Compute actual keccak256 topic hashes for event signatures in `crates/hdc/chain/src/event.rs` (currently all `B256::ZERO`)
- [ ] Implement ABI-decode logic in `process_log()` in `crates/hdc/chain/src/event.rs` (currently a no-op stub)
- [ ] Implement `record_pheromone()` in `crates/hdc/chain/src/index.rs:131` (currently a TODO stub)
- [ ] Implement `fixed_point_decay()` for consensus-safe on-chain decay (not found anywhere)

### Code Quality Items

- [ ] Replace `unwrap()` on line 158 of `crates/hdc/chain/src/precompile.rs` with proper error handling (this is in a production code path, not a test)
- [ ] Replace `unwrap()` on line 150 of `crates/hdc/core/src/vector.rs` in `deserialize()` with proper error handling (production code path)
- [ ] Replace `unwrap()` calls in `crates/hdc/core/src/search/hnsw.rs` lines 206, 222, 262, 267, 274, 356 with proper error propagation (production code paths in HNSW graph operations)
- [ ] Audit `HashMap` usage in `crates/hdc/chain/src/index.rs` (`OnChainHdcIndex`) -- iteration order in `search()` may cause non-deterministic tie-breaking among equal-distance results
- [ ] Audit `HashMap` usage in `crates/hdc/chain/src/wisdom.rs` (`WisdomGate`) -- `resolve()` iterates `submissions.values_mut()` in non-deterministic order
- [ ] Audit `HashMap` usage in `crates/hdc/core/src/knowledge/store.rs` -- `tick()` iterates `self.entries.iter_mut()` in non-deterministic order, affecting promotion/demotion/GC ordering

---

## Second-Pass Remediation Detail

> **Audit date:** 2026-05-08
> **Scope:** Current workspace state, all HDC impl docs at high level, and source
> checks for the HDC core, chain, node wiring, e2e tests, and Solidity contracts.
>
> This section supersedes the first-pass status language above where it says
> broad phases are "completed." In this backlog, **implemented** means code
> exists and unit tests may pass; **complete** means spec-compatible, wired
> end-to-end, deterministic, and clean under the required CI commands.

### Current Verification Snapshot

Commands run during this pass:

| Command | Result | Notes |
|---------|--------|-------|
| `cargo check -p kora-hdc-chain` | PASS WITH WARNINGS | Builds, but emits 81 `kora-hdc` warnings and 12 `kora-hdc-chain` warnings. |
| `cargo clippy -p kora-hdc-chain --all-targets -- -D warnings` | FAIL | Fails in `kora-hdc` with 151 lint/doc/debug errors before chain lint closure. |
| `cargo test -p kora-hdc --lib --tests` | PASS WITH WARNINGS | 174 lib tests, 2 knowledge integration tests, 2 search integration tests, and 1 stress test passed. |
| `forge test --root contracts` | PASS | 58 Foundry tests pass. InsightBoard tests use `TestableInsightBoard` and skip live precompile calls. |

### Corrected Phase Statuses

| Phase | Corrected status | Remediation gate |
|-------|------------------|------------------|
| 0. Prerequisites | STALE / PARTIAL | The checklist still assumes a clean starting point. The workspace already has `crates/hdc/*`, contracts, and HDC e2e tests, but CI cleanliness is not true. |
| 1. Core crate | FUNCTIONAL, NOT CI-CLEAN | Core tests pass, but clippy/doc/debug lints fail; orphan `crates/kora-hdc/` duplicate remains outside the workspace. |
| 2. SIMD + search | FUNCTIONAL, NOT CONSENSUS-CLEAN | Search modules exist, but HNSW still uses `HashSet` in `search_layer`, unsafe raw byte casting in `deterministic_level`, and production `unwrap()` calls. |
| 3. Knowledge store | FUNCTIONAL, SPEC-PARTIAL | Insert/search/tick tests pass, but store still uses a brute-force `Vec`, duplicates vectors, has stub `open()`, unused `max_entries`, no FSRS, and no PAD emotional scoring field. |
| 4. Context + cognitive | FUNCTIONAL, SPEC-PARTIAL | Tests exist, but second-pass docs still require stateful `ContextAssembler`, retained VCG payment state, contrarian retrieval/flooring, and the canonical context shape. |
| 5. Block time migration | IMPLEMENTED, OPS-PARTIAL | Runner consumes `block_time_ms`; Docker/Railway env propagation and value validation remain backlog items. |
| 6. On-chain integration | BLOCKED / PARTIAL | Rust precompile is registered, but Solidity `HdcLib` expects a different opcode/stateful API. Event replay is stubbed and the on-chain index is not deterministic on ties. |
| 7. Trust pipeline | FUNCTIONAL, SPEC-PARTIAL | Trust code and tests exist, but docs identify duplicate trust boundaries, simplified taint, hardcoded placeholders, and immune-system/spec gaps. |
| 8. RPC + wiring | WIRED, DATA-PARTIAL | RPC methods exist, but `hdc_search` depends on an ephemeral index that event replay does not populate. |
| 9. E2E | PARTIAL | HDC precompile e2e tests submit transactions and assert finalization, but do not verify return values; no InsightBoard/PheromoneRegistry e2e lifecycle against the live precompile. |
| 10. Railway | NOT VERIFIED | Deployment is blocked by config/env propagation, health-check, and end-to-end HDC readiness gaps. |
| 11. Final verification | FAILING | `clippy -D warnings` fails; full `just ci` must not be considered green. |

### Dependency Order

1. **D0: Establish the CI baseline.** Fix lints/doc/debug warnings enough that
   targeted HDC clippy can run to completion. This does not require behavior
   decisions and should happen before larger rewrites.
2. **D1: Choose one precompile contract.** Decide whether the EVM precompile is
   stateless vector algebra only or also owns `storeVector/searchSimilar/deleteVector`.
   All Solidity, Rust, RPC, and e2e work depends on this.
3. **D2: Finalize address policy.** Either intentionally override `0x09` when HDC
   is enabled and document the BLAKE2F replacement, or move HDC to a non-standard
   address and update Rust, Solidity, tests, docs, and deployment config together.
4. **D3: Freeze event schemas.** Event topic hashes and ABI decoding depend on
   the finalized Solidity interfaces.
5. **D4: Make indexes deterministic.** Event replay and RPC search can be trusted
   only after HDC index ordering and tie-breaking are deterministic.
6. **D5: Add end-to-end contract coverage.** Real InsightBoard/PheromoneRegistry
   tests depend on D1-D4 and must not rely on precompile-skipping subclasses.

### P0 Task Group: Execution-Blocking Correctness

- [ ] **P0-1: Align the Rust precompile and Solidity `HdcLib` ABI.**
  - Current Rust opcodes: `0x01=hamming`, `0x02=bind`, `0x03=bundle`,
    `0x04=permute`, `0x05=vector_id`, `0x06=is_similar`.
  - Current Solidity `HdcLib` expects: `0x01=storeVector`,
    `0x02=searchSimilar`, `0x03=deleteVector`, `0x04=bundle`,
    `0x05=bind`, `0x06=hamming`, `0x07=permute`.
  - Decide and implement one canonical map. If keeping the stateless Rust
    precompile, remove or replace Solidity stateful calls. If keeping the
    Solidity API, add stateful Rust support and define persistence/replay
    semantics.
  - **Acceptance:** A direct `InsightBoard.submit()` against the live HDC
    precompile succeeds without `TestableInsightBoard`; invalid opcode/length
    cases revert consistently; Rust and Solidity byte fixtures agree.

- [ ] **P0-2: Fix precompile validation and panic paths.**
  - Add exact input-length checks for every opcode, reject zero-count bundles,
    bound bundle count, and replace production `unwrap()` in `exec_permute`.
  - Update gas accounting after final opcode semantics are chosen.
  - **Acceptance:** Unit tests cover exact length, short input, trailing bytes,
    zero bundle count, max bundle count, out-of-gas, unknown opcode, and output
    ABI size for every opcode.

- [ ] **P0-3: Implement finalized event replay.**
  - Replace `B256::ZERO` topic placeholders in `crates/hdc/chain/src/event.rs`
    with real keccak256 event signatures from the finalized Solidity ABIs.
  - Implement ABI decoding in `process_log()` for insight and pheromone events.
  - Wire finalized receipt/log processing into the runner reporter path so the
    `OnChainHdcIndex` is rebuilt from chain events, not only local memory.
  - **Acceptance:** Unit tests feed encoded logs and observe `insert_insight`,
    `update_state`, and pheromone recording. A node restart can rebuild the HDC
    index from finalized data.

- [ ] **P0-4: Make consensus-adjacent indexing deterministic.**
  - Replace or constrain `HashMap` iteration in `OnChainHdcIndex::search()` and
    `WisdomGate::resolve()`.
  - Sort search results by `(distance, id)` or another stable composite key.
  - Replace HNSW `HashSet` visitation with deterministic `BTreeSet` or prove
    iteration order does not affect output.
  - Replace unsafe raw byte hashing in HNSW level assignment with hashing of
    `serialize(vector)`.
  - **Acceptance:** Determinism tests with equal-distance vectors produce the
    same ordering across repeated runs and insertion orders.

- [ ] **P0-5: Restore CI cleanliness for HDC crates.**
  - Fix missing docs, missing `Debug`, dead code, clippy `use_self`,
    `new_without_default`, `missing_const_for_fn`, and related warnings in
    `kora-hdc` and `kora-hdc-chain`.
  - **Acceptance:** `cargo clippy -p kora-hdc -p kora-hdc-chain --all-targets -- -D warnings`
    passes, followed by `cargo +nightly fmt --all -- --check`.

### P1 Task Group: Spec Completion and End-to-End Behavior

- [ ] **P1-1: Finish the knowledge-store architecture.**
  - Replace brute-force `vectors: Vec<...>` with `search::LocalIndex` or a
    single canonical vector index.
  - Remove dual vector storage, keep entry/index mutation atomic, enforce or
    remove `max_entries`, and replace the persistence `open()` stub with a real
    contract.
  - Add FSRS scheduling or remove it from the completion checklist.
  - Add `PadState`/emotional resonance scoring if the spec remains canonical.
  - **Acceptance:** KnowledgeStore insert/remove/tick/search tests prove no
    ghost vectors, persistence round-trip behavior is defined, and scoring
    matches the finalized formula.

- [ ] **P1-2: Bring context assembly to the second-pass design.**
  - Introduce `ContextAssembler` with retained VCG state.
  - Add explicit local/shared/contrarian gather phases, contrarian floor
    enforcement, `ContextRequest`, mode-based `TokenBudget`, and the canonical
    prompt-layer output if those remain spec requirements.
  - **Acceptance:** Tests cover VCG payment persistence across ticks,
    contrarian survival, emotional scoring, summarization fallback, and
    deterministic layer ordering.

- [ ] **P1-3: Harden Solidity contract semantics after precompile alignment.**
  - Replace the `TestableInsightBoard` skip pattern with a real precompile mock
    whose opcode map matches Rust, or run contract tests against a live custom
    precompile.
  - Fix `InsightBoard` stored-vs-computed-state issues in `confirm()` and
    `renew()`, enforce challenge resonance, add self-confirm guard if required,
    and revisit purge CEI because the current code calls `HdcLib.deleteVector`
    before deleting storage.
  - Implement or remove `PheromoneRegistry.readPheromones()`, guard depositor
    self-confirmation, bound/paginate location indexes, and document or fix
    locked stake behavior.
  - **Acceptance:** Foundry tests fail if precompile calls are skipped, cover
    stale computed states, challenge resonance, self-confirm attempts, purge
    withdrawal behavior, and `readPheromones()` non-empty behavior.

- [ ] **P1-4: Upgrade e2e tests from finalization checks to value checks.**
  - Add receipt/output introspection or wrapper contracts that persist HDC
    precompile return values.
  - Add InsightBoard submit/confirm/challenge/renew/purge and
    PheromoneRegistry deposit/confirm/read/cleanup lifecycle tests.
  - Replace the current "fast blocks" no-op with a real `block_time_ms=50`
    harness path or rename it to avoid claiming 50ms validation.
  - Ensure all slow/flaky HDC e2e tests are `#[ignore]` with the single-thread
    run command documented.
  - **Acceptance:** `cargo test -p kora-e2e hdc -- --test-threads=1` verifies
    return values, contract state, and deterministic state roots.

- [ ] **P1-5: Complete block-time and deployment config plumbing.**
  - Validate `block_time_ms` ranges, make Docker/Railway `BLOCK_TIME_MS` reach
    the actual `NodeConfig`, and remove conflicting docs about default units.
  - Add a health-check endpoint or configure Railway health checks to use a
    valid RPC probe instead of assuming `/` is meaningful.
  - **Acceptance:** A generated Railway/devnet config with `BLOCK_TIME_MS=50`
    produces runner timeouts derived from 50ms and passes a local smoke test.

### P2 Task Group: Cleanup, Performance, and Documentation

- [ ] **P2-1: Delete or retire the orphan duplicate crate.**
  - `crates/kora-hdc/` is not a workspace member but still duplicates core
    files with the same package name. Remove it or clearly archive it outside
    the repo path used by agents.
  - **Acceptance:** `rg --files crates/kora-hdc` returns no tracked source, or
    docs explicitly mark it as out-of-tree historical material.

- [ ] **P2-2: Optimize hot paths after correctness is locked.**
  - Convert `BundleAccumulator` from bit-by-bit loops to word-level processing,
    avoid unnecessary 1,280-byte `HdcVector` clones in HNSW and context paths,
    and add 10K/100K search benchmarks.
  - **Acceptance:** `cargo bench -p kora-hdc --bench hdc_bench` includes HNSW
    and brute-force 10K cases and records current timing targets.

- [ ] **P2-3: Normalize public types and serialization.**
  - Add `#[repr(u8)]` where enum wire stability is required, add serde derives
    for persistence/RPC types, and replace ambiguous `[u8; 32]` aliases with
    `H256`/`B256` at crate boundaries where that is the chosen convention.
  - **Acceptance:** Persistence and RPC round-trip tests lock the wire format.

- [ ] **P2-4: Reconcile the impl docs with reality.**
  - Update docs that still say contracts are missing, that the precompile has a
    different opcode map, or that phases are completed despite second-pass
    blockers.
  - Keep `16-checklist.md` as the execution tracker and move detailed design
    debate back into the numbered implementation docs.
  - **Acceptance:** A fresh `rg "B256::ZERO|TODO|TestableInsightBoard|not found|NOT DONE" tmp/HDC/impl`
    audit produces only intentional, currently accurate references.

### Command Plan

Run these gates in order as remediation lands:

```bash
# Fast source-level closure for the HDC crates
cargo check -p kora-hdc -p kora-hdc-chain
cargo clippy -p kora-hdc -p kora-hdc-chain --all-targets -- -D warnings
cargo test -p kora-hdc --lib --tests
cargo test -p kora-hdc-chain --lib --tests

# Contract closure after opcode/API alignment
forge test --root contracts -vvv

# Integration closure
cargo test -p kora-e2e hdc -- --test-threads=1
cargo nextest run --workspace --all-features

# Final release gate
cargo +nightly fmt --all -- --check
just ci
```

### Definition of Done

The HDC implementation should not be called complete until all of these are
true:

- The canonical precompile ABI is implemented consistently in Rust, Solidity,
  RPC docs, e2e transaction builders, and encoding specs.
- Direct contract tests and e2e tests exercise the real precompile path; no
  lifecycle-critical test relies on a subclass that skips HDC calls.
- Finalized event replay populates the HDC index and survives restart/rebuild.
- HDC search/indexing has deterministic tie-breaking and no iteration-order
  dependency in consensus-adjacent paths.
- `cargo clippy -p kora-hdc -p kora-hdc-chain --all-targets -- -D warnings`,
  `forge test --root contracts`, HDC e2e tests, and `just ci` all pass.
