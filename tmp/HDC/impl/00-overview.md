# HDC Implementation Master Index

> **Start here.** This is the top-level document for implementing Hyperdimensional
> Computing (HDC) into the daeji/kora blockchain node. Every implementation plan
> document (01-21) is listed below with its purpose, dependencies, status, and
> complexity.
>
> **Last updated:** 2026-05-08
>
> **Authoritative PRs:** #42 (HDC precompile + stigmergy), #41 (Merkle proofs,
> pending)

---

## 1. Project Summary

**Daeji** is a high-performance EVM-compatible blockchain node written in Rust.

| Attribute | Value |
|---|---|
| Repository root | `/Users/will/dev/nunchi/daeji/` |
| Networking | commonware-p2p |
| EVM execution | REVM |
| Storage | QMDB |
| Consensus | Simplex (BLS12-381 threshold signatures) |
| Key binaries | `kora` (validator node), `keygen` (key generation), `loadgen` (load testing) |
| Current block time | 2 s (config) / 5 s (hardcoded leader timeout in `runner.rs:385`) |

**Goal:** Add an HDC cognitive substrate for agent cognition and deploy a
3-validator testnet on Railway with 50 ms block times.

**Current state:** The core HDC algebra, search, knowledge store, trust pipeline,
context assembly, cognitive architecture (affect, somatic, replay, state machine),
on-chain index, event handling, RPC extensions, and precompile are all
implemented with 73+ passing tests. The remaining work is completing partially
stubbed subsystems (dream cycle, WisdomGate lifecycle, precompile handler wiring),
the block time migration, Railway deployment, and deferred features (SINR,
Merkle proofs).

---

## 2. Architecture Decisions (Finalized)

These decisions are final. Do not revisit them.

1. **Block time units.** Migrating `block_time` from `u64` seconds to `u64`
   milliseconds across the entire codebase. *(Not yet started.)*

2. **HDC precompile address.** `0xA0C` in PR #42 (`kora-precompiles` monolithic
   crate). The `kora-hdc-chain` crate also contains a standalone precompile at
   `0x09` (used for unit tests and the two-crate development path). The
   canonical on-chain address is **`0xA0C`** per PR #42.

3. **Stigmergy precompile address.** `0xA0D` in PR #42 (`StigmergyPrecompile`
   in `kora-precompiles`). Replaces the planned Solidity `PheromoneRegistry.sol`
   contract. Pheromone state lives in Rust precompile memory, not EVM storage.

4. **Crate layout.** Three crates:
   - `kora-hdc` (`crates/hdc/core/`) -- core algebra, search, knowledge, trust,
     context, cognitive architecture. **No chain dependencies.**
   - `kora-hdc-chain` (`crates/hdc/chain/`) -- on-chain index, event decoders,
     RPC extensions, WisdomGate, standalone precompile.
   - `kora-precompiles` (PR #42) -- monolithic precompile crate registering HDC
     at `0xA0C` and Stigmergy at `0xA0D` with REVM.

5. **Gas model (PR #42).** Flat 50,000 gas per HDC operation in `kora-precompiles`.
   The `kora-hdc-chain` precompile uses fine-grained per-opcode gas
   (1,500/500/300/3,000) for standalone testing. On-chain deployments use the
   PR #42 flat model.

6. **Selector format (PR #42).** 4-byte Solidity-style function selectors
   (e.g., `hdc_bind(bytes,bytes)`) in `kora-precompiles`. The `kora-hdc-chain`
   precompile uses raw 1-byte opcodes (0x01-0x06).

7. **Search strategy.** Brute-force only in the on-chain precompile (determinism
   required). HNSW and TieredSearchPipeline are available in `kora-hdc` core
   for off-chain use.

8. **Event-replay consensus.** Fully implemented in `kora-hdc-chain/src/event.rs`.
   Six event types decoded from Solidity ABI logs, applied to `OnChainHdcIndex`.
   Original plans had this deferred; it was completed ahead of schedule.

9. **Railway deployment.** Full config files: `railway.toml`, `Dockerfile`,
   environment variables, health checks, deploy scripts. *(Not yet started.)*

---

## 3. Document Map

| # | Filename | Description | Depends On | Size |
|---|---|---|---|---|
| 00 | `00-overview.md` | **This file.** Master index, status, critical path. | -- | -- |
| 01 | `01-block-time-migration.md` | Migrate `block_time` from seconds to milliseconds. | -- | S |
| 02 | `02-kora-hdc-core.md` | `kora-hdc` crate: algebra, vectors, encoding. | -- | L |
| 03 | `03-vector-search.md` | Search subsystem: brute-force, HNSW, local, tiered, SIMD. | 02 | L |
| 04 | `04-knowledge-store.md` | KnowledgeStore, tiers, decay, anti-knowledge, scoring. | 02 | L |
| 05 | `05-context-assembly.md` | Gather/rank/compress pipeline, VCG auction, knapsack. | 03, 04 | M |
| 06 | `06-cognitive-architecture.md` | ALMA affect, somatic bias, replay, state machine, dream. | 02, 04 | L |
| 07 | `07-precompile-integration.md` | REVM precompile registration (0x09 in chain, 0xA0C in PR #42). | 02 | M |
| 08 | `08-insight-board-contract.md` | InsightBoard Solidity contract + on-chain index. | 07 | M |
| 09 | `09-pheromone-registry.md` | Stigmergy: Solidity spec vs Rust precompile (0xA0D). | 07 | M |
| 10 | `10-kora-hdc-chain.md` | `kora-hdc-chain` crate: index, events, wisdom, RPC. | 02, 07 | L |
| 11 | `11-trust-pipeline.md` | TrustRegistry, TrustPipeline (5 layers), TaintTracker. | 02 | M |
| 12 | `12-rpc-extensions.md` | `hdc_*` JSON-RPC methods, HdcApi, HdcConfig. | 10 | M |
| 13 | `13-e2e-tests.md` | End-to-end test harness in `crates/e2e/src/tests/hdc.rs`. | 10, 12 | M |
| 14 | `14-railway-deployment.md` | Railway config, Dockerfile, health checks, deploy. | 01, 13 | M |
| 15 | `15-wiring-guide.md` | Integration: runner, executor, RPC server hook-up. | 10, 12 | S |
| 16 | `16-checklist.md` | Milestone checklist across all docs. | All | S |
| 17 | `17-anti-patterns.md` | What NOT to do: consensus hazards, f64 on-chain, etc. | -- | S |
| 18 | `18-encoding-specs.md` | Trigram, projection, structured encoder specs. | 02 | S |
| 19 | `19-solidity-specs.md` | InsightBoard.sol + PheromoneRegistry.sol ABI specs. | 08, 09 | M |
| 20 | `20-cross-cutting-audit.md` | Cross-crate audit findings (orphaned crate, etc.). | All | M |
| 21 | `21-reconciliation.md` | PR #42 vs plan divergences, migration notes. | All | M |

**Size key:** S = <200 lines, M = 200-500 lines, L = >500 lines.

---

## 4. Implementation Status

### 4.1 Status by Document

| # | Document | Status | Notes |
|---|---|---|---|
| 01 | Block time migration | **TODO** | Not started. Requires codebase-wide `u64` seconds -> milliseconds. |
| 02 | `kora-hdc` core | **DONE** | `HdcVector` (10,240-bit BSC), `bind`, `bundle`, `permute`, `hamming_distance`, `similarity`, `serialize`/`deserialize`, `vector_id`. `BundleAccumulator` streaming bundler. `TrigramEncoder`, `ProjectionEncoder`, `StructuredEncoder`. All tested. |
| 03 | Vector search | **DONE** | `BruteForce`, `HNSW` (layered graph, greedy search), `LocalIndex` (auto-switch brute<->HNSW at 1,000 entries), `TieredSearchPipeline`, SIMD dispatch (AVX2/AVX-512/NEON). Search tests + stress tests passing. |
| 04 | Knowledge store | **DONE** | `KnowledgeStore` with full CRUD lifecycle. 6 `KnowledgeKind` variants (Perception, Strategy, Prediction, Reflection, AntiKnowledge, Meta). 4 `KnowledgeTier` levels (Transient, Working, Consolidated, Persistent). Exponential decay, anti-knowledge tracking, 4-factor scoring (relevance x importance x recency x freshness). |
| 05 | Context assembly | **DONE** | `assemble_context()` pipeline: gather -> rank -> compress -> assemble. VCG auction scoring, knapsack token-budget solver (DP granularity 10 tokens/unit), duplicate suppression via `DUPLICATE_THRESHOLD`. 4-weight scoring: relevance (0.35), importance (0.25), recency (0.20), freshness (0.20). |
| 06 | Cognitive architecture | **PARTIAL** | **DONE:** ALMA 3-layer PAD affect model (`AlmaState`, `PadState`, emotion/mood/personality EMA), somatic bias (`apply_somatic_bias`), Mattar-Daw replay prioritization (`mattar_daw_evb`, `prioritize_replay`), 6-state behavioral FSM with hysteresis (`StateContext.evaluate()`). **PARTIAL:** Dream cycle has `DreamConfig` and `DreamReport` structures but NREM/REM logic is stubbed (no actual re-encoding, merging, or GC). |
| 07 | Precompile integration | **DONE** | `kora-hdc-chain` precompile at `0x09`: 6 opcodes (hamming, bind, bundle, permute, vector_id, is_similar), per-opcode gas, full test coverage. PR #42 `kora-precompiles` at `0xA0C`: 4-byte selectors, 50K flat gas. |
| 08 | InsightBoard contract | **PARTIAL** | On-chain index (`OnChainHdcIndex`) with `InsightMeta`, 7-state `InsightState` FSM (Draft/Submitted/Challenged/Voting/Accepted/Rejected/Expired), capacity enforcement, deterministic BTreeMap iteration. Solidity `InsightBoard.sol` exists as spec only (doc 19). |
| 09 | Pheromone registry | **PARTIAL** | Event decoder for `PheromoneDeposited` implemented. `OnChainHdcIndex.record_pheromone()` is a stub (TODO in code). PR #42 has full `StigmergyPrecompile` at `0xA0D` as Rust precompile. Solidity `PheromoneRegistry.sol` superseded. |
| 10 | `kora-hdc-chain` crate | **DONE** | Modules: `index` (OnChainHdcIndex), `event` (6 event decoders), `precompile` (0x09), `rpc` (HdcApi), `wisdom` (WisdomGate). All have tests. |
| 11 | Trust pipeline | **DONE** | `TrustRegistry` (per-agent EMA reputation, domain scores, cold-start at 0.1). `TrustPipeline` 5-layer evaluation (reputation, domain, interaction history, taint check, threshold gate). `TaintTracker` (address blacklist + taint propagation). All tested. |
| 12 | RPC extensions | **DONE** | `HdcApi` with 8 methods: `hdc_hammingDistance`, `hdc_similarity`, `hdc_bind`, `hdc_bundle`, `hdc_search`, `hdc_vectorId`, `hdc_encode`, `hdc_getInsight`. `SearchResultRpc` type. Error types. |
| 13 | E2E tests | **DONE** | `crates/e2e/src/tests/hdc.rs` exists. |
| 14 | Railway deployment | **TODO** | Not started. Depends on block time migration (doc 01). |
| 15 | Wiring guide | **PARTIAL** | Precompile and RPC code exist but runner/executor integration wiring not documented or fully connected. |
| 16 | Checklist | **PARTIAL** | Needs update to reflect current done/partial/todo state. |
| 17 | Anti-patterns | **DONE** | Reference document, no implementation needed. |
| 18 | Encoding specs | **DONE** | `TrigramEncoder`, `ProjectionEncoder`, `StructuredEncoder` all implemented in `kora-hdc/src/encode.rs`. |
| 19 | Solidity specs | **PARTIAL** | Specs exist but PR #42 moved stigmergy to Rust precompile. InsightBoard.sol ABI still relevant for event decoding. |
| 20 | Cross-cutting audit | **DONE** | Completed 2026-05-08. Findings: orphaned crate, precompile address mismatch, stub handlers, etc. |
| 21 | Reconciliation | **TODO** | Not yet created. Should document all PR #42 vs plan divergences. |

### 4.2 Status by Subsystem

| Subsystem | Crate | Status | Key Files |
|---|---|---|---|
| HDC algebra | `kora-hdc` | DONE | `vector.rs`, `bundle.rs`, `constants.rs` |
| Encoding | `kora-hdc` | DONE | `encode.rs` |
| Search (brute-force) | `kora-hdc` | DONE | `search/brute.rs` |
| Search (HNSW) | `kora-hdc` | DONE | `search/hnsw.rs` |
| Search (local auto-switch) | `kora-hdc` | DONE | `search/local.rs` |
| Search (tiered pipeline) | `kora-hdc` | DONE | `search/tiered.rs` |
| SIMD dispatch | `kora-hdc` | DONE | `search/simd.rs` |
| Knowledge store | `kora-hdc` | DONE | `knowledge/store.rs`, `entry.rs`, `kind.rs`, `tier.rs`, `decay.rs`, `scoring.rs`, `anti.rs`, `source.rs` |
| Context assembly | `kora-hdc` | DONE | `context.rs` |
| Trust pipeline | `kora-hdc` | DONE | `trust.rs` |
| ALMA affect model | `kora-hdc` | DONE | `cognitive/affect.rs` |
| Somatic bias | `kora-hdc` | DONE | `cognitive/somatic.rs` |
| Replay prioritization | `kora-hdc` | DONE | `cognitive/replay.rs` |
| Behavioral state machine | `kora-hdc` | DONE | `cognitive/state_machine.rs` (6 states, hysteresis, modifiers, full test suite) |
| Dream cycle | `kora-hdc` | PARTIAL | `cognitive/dream.rs` (config + report structs only; NREM/REM logic stubbed) |
| On-chain index | `kora-hdc-chain` | DONE | `index.rs` |
| Event decoders | `kora-hdc-chain` | DONE | `event.rs` (6 event types, ABI decoding, LazyLock topic hashes) |
| Precompile (chain crate) | `kora-hdc-chain` | DONE | `precompile.rs` (0x09, 6 opcodes, per-opcode gas) |
| Precompile (PR #42) | `kora-precompiles` | DONE | 0xA0C, 4-byte selectors, 50K flat gas |
| Stigmergy precompile | `kora-precompiles` | DONE | 0xA0D (PR #42, replaces PheromoneRegistry.sol) |
| WisdomGate | `kora-hdc-chain` | PARTIAL | `wisdom.rs` (submit + challenge + resolve implemented; challenge integration with InsightBoard not wired) |
| RPC extensions | `kora-hdc-chain` | DONE | `rpc.rs` (8 methods) |
| E2E tests | `crates/e2e` | DONE | `tests/hdc.rs` |
| Block time migration | multiple | TODO | Not started |
| Railway deployment | -- | TODO | Not started |
| Solidity contracts | -- | PARTIAL | Specs exist (doc 19); stigmergy superseded by Rust precompile |
| Pheromone tracking | `kora-hdc-chain` | PARTIAL | Event decoder done; `record_pheromone()` is a stub |

### 4.3 Test Summary

73+ passing tests across the HDC crates:

| Location | Test Count | Scope |
|---|---|---|
| `crates/hdc/core/tests/search.rs` | ~10 | Search subsystem integration |
| `crates/hdc/core/tests/knowledge.rs` | ~10 | Knowledge store lifecycle |
| `crates/hdc/core/tests/stress.rs` | ~5 | Stress/determinism tests |
| `crates/hdc/core/src/` (unit tests) | ~30 | Per-module unit tests (vector, bundle, encode, trust, context, cognitive/*) |
| `crates/hdc/chain/src/` (unit tests) | ~30 | Per-module unit tests (precompile, index, event, wisdom, rpc) |
| `crates/e2e/src/tests/hdc.rs` | ~3 | End-to-end integration |

---

## 5. Key Divergences: PR #42 vs Implementation Plans

These divergences are documented here for reference and will be fully detailed
in `21-reconciliation.md` when it is created.

| # | Topic | Original Plan | PR #42 Reality |
|---|---|---|---|
| 1 | HDC precompile address | `0x09` (replaces BLAKE2F) | `0xA0C` (avoids BLAKE2F collision) |
| 2 | Stigmergy | Solidity `PheromoneRegistry.sol` | Rust precompile at `0xA0D` |
| 3 | Gas model | Per-opcode (1,500/500/300/3,000) | Flat 50,000 per operation |
| 4 | Selector format | Raw 1-byte opcodes | 4-byte Solidity function selectors |
| 5 | Search in precompile | Plans implied HNSW available | Brute-force only (determinism) |
| 6 | Event-replay | Deferred | Fully implemented |
| 7 | Crate count | 2 (`kora-hdc` + `kora-hdc-chain`) | 3 (+ `kora-precompiles` monolithic) |
| 8 | Alpha paradox | Confirmation reduces half-life | Monotonic pheromones (no decay) |

**Note:** The `kora-hdc-chain` precompile at `0x09` remains useful for unit
testing and standalone development. The canonical on-chain deployment uses
`0xA0C` from PR #42.

---

## 6. Crate Map

```
crates/
  hdc/
    core/                          # kora-hdc (workspace member)
      src/
        lib.rs                     # Public API, re-exports
        constants.rs               # D=10240, WORDS=160, BYTES=1280, thresholds
        vector.rs                  # HdcVector, bind, bundle, permute, hamming, similarity
        bundle.rs                  # BundleAccumulator (streaming majority-vote)
        encode.rs                  # TrigramEncoder, ProjectionEncoder, StructuredEncoder
        trust.rs                   # TrustRegistry, TrustPipeline, TaintTracker
        context.rs                 # assemble_context(), VCG auction, knapsack
        search/
          mod.rs                   # Search trait, SearchResult
          brute.rs                 # BruteForce (linear scan)
          hnsw.rs                  # HNSW (layered navigable small-world graph)
          local.rs                 # LocalIndex (auto-switch brute <-> HNSW)
          tiered.rs                # TieredSearchPipeline
          simd.rs                  # SIMD dispatch (AVX2/AVX-512/NEON)
        knowledge/
          mod.rs                   # Module re-exports
          store.rs                 # KnowledgeStore
          entry.rs                 # KnowledgeEntry
          kind.rs                  # KnowledgeKind (6 variants)
          tier.rs                  # KnowledgeTier (4 levels)
          decay.rs                 # Exponential decay
          scoring.rs               # 4-factor scoring
          anti.rs                  # Anti-knowledge tracking
          source.rs                # KnowledgeSource
          trust.rs                 # Knowledge-level trust integration
        cognitive/
          mod.rs                   # Module re-exports
          affect.rs                # ALMA 3-layer PAD model
          somatic.rs               # Somatic marker bias
          replay.rs                # Mattar-Daw replay prioritization
          state_machine.rs         # 6-state behavioral FSM with hysteresis
          dream.rs                 # Dream cycle (config + report; logic STUBBED)
      tests/
        search.rs                  # Search integration tests
        knowledge.rs               # Knowledge store tests
        stress.rs                  # Stress and determinism tests
      benches/
        hdc_bench.rs               # Benchmarks
    chain/                         # kora-hdc-chain (workspace member)
      src/
        lib.rs                     # Public API, re-exports
        index.rs                   # OnChainHdcIndex, InsightMeta, InsightState FSM
        precompile.rs              # HDC precompile at 0x09 (6 opcodes)
        event.rs                   # 6 event decoders (ABI log processing)
        rpc.rs                     # HdcApi (8 JSON-RPC methods)
        wisdom.rs                  # WisdomGate (submit/challenge/resolve)
  e2e/
    src/tests/
      hdc.rs                       # E2E integration tests
```

---

## 7. Critical Path

### 7.1 Completed (no further work needed)

```
02 (core algebra) ─── DONE
  ├── 03 (search) ─── DONE
  │     └── 05 (context assembly) ─── DONE
  ├── 04 (knowledge store) ─── DONE
  │     └── 05 (context assembly) ─── DONE
  ├── 07 (precompile, chain crate 0x09) ─── DONE
  ├── 11 (trust pipeline) ─── DONE
  └── 18 (encoding specs) ─── DONE

10 (kora-hdc-chain) ─── DONE
  ├── 12 (RPC extensions) ─── DONE
  └── 13 (E2E tests) ─── DONE
```

### 7.2 Remaining Work

```
01 (block time migration) ─── TODO
  └── 14 (Railway deployment) ─── TODO

06 (cognitive arch) ─── PARTIAL
  └── Dream cycle NREM/REM logic needs implementation

08 (InsightBoard) ─── PARTIAL
  └── Solidity contract deployment vs pure precompile path TBD

09 (pheromone registry) ─── PARTIAL
  └── record_pheromone() stub needs implementation
  └── PR #42 Rust precompile supersedes Solidity contract

15 (wiring guide) ─── PARTIAL
  └── runner/executor integration documentation

19 (Solidity specs) ─── PARTIAL
  └── Needs update for PR #42 Rust precompile reality

21 (reconciliation) ─── TODO
  └── Must document all PR #42 divergences
```

### 7.3 Unblocked Work (can start immediately)

1. **Dream cycle implementation** (doc 06) -- implement NREM re-encoding,
   near-duplicate merging, tier promotion, GC, and REM cross-binding in
   `crates/hdc/core/src/cognitive/dream.rs`.

2. **Pheromone tracking** (doc 09) -- implement `OnChainHdcIndex::record_pheromone()`
   body in `crates/hdc/chain/src/index.rs`.

3. **21-reconciliation.md** (doc 21) -- document all divergences between the
   original implementation plans and PR #42 reality.

4. **Block time migration** (doc 01) -- grep for `block_time` across the
   codebase and migrate from seconds to milliseconds.

### 7.4 Blocked Work

| Task | Blocked By |
|---|---|
| Railway deployment (doc 14) | Block time migration (doc 01) |
| SINR interference model | Not yet planned |
| Merkle proofs in search results | PR #41 (pending) |

---

## 8. What Needs to Be Done Next

### Priority 1: Complete Partial Implementations

- [ ] **Dream cycle logic** (`cognitive/dream.rs`): Implement NREM phase
  (re-encode top entries by replay priority, merge near-duplicates within
  `DUPLICATE_THRESHOLD`, promote entries exceeding consolidation threshold,
  GC entries below `gc_threshold`). Implement REM phase (random cross-binding
  of entries from different `KnowledgeKind` categories, detect resonances
  within `RESONANCE_THRESHOLD_HAMMING`, create new insight entries from
  resonances). Populate `DreamReport` fields.

- [ ] **Pheromone tracking** (`chain/src/index.rs`): Replace
  `record_pheromone()` stub with actual pheromone state storage. Decide on
  data structure (BTreeMap keyed by pheromone ID, with intensity + location
  hash + block number).

- [ ] **WisdomGate wiring**: The `WisdomGate` has working submit/challenge/resolve
  logic. Wire it into the event processing pipeline so that `InsightPublished`
  events create WisdomGate submissions automatically.

### Priority 2: Infrastructure

- [ ] **Block time migration** (doc 01): Audit all uses of `block_time: u64`
  across the codebase. Change semantics from seconds to milliseconds. Update
  consensus timeout, leader rotation, and all time-dependent calculations.

- [ ] **Runner/executor wiring** (doc 15): Register the HDC precompile with
  REVM in the executor. Hook `OnChainHdcIndex` into the block finalization
  pipeline. Mount `HdcApi` on the RPC server.

### Priority 3: Deployment

- [ ] **Railway deployment** (doc 14): Create `railway.toml`, `Dockerfile`,
  environment variables, health checks, and deploy scripts for a 3-validator
  testnet.

### Priority 4: Documentation

- [ ] **Create 21-reconciliation.md**: Full documentation of all divergences
  between original plans and PR #42, with migration notes for each.

- [ ] **Update 16-checklist.md**: Refresh milestone checklist to reflect
  current done/partial/todo state.

- [ ] **Update 19-solidity-specs.md**: Note that `PheromoneRegistry.sol` is
  superseded by the Rust precompile at `0xA0D`. Keep `InsightBoard.sol` ABI
  specs for event decoding reference.

### Deferred (No Current Plan)

- SINR interference model
- Alpha paradox (confirmation reducing half-life) -- PR #42 uses monotonic
  pheromones, making this unnecessary
- Merkle proofs in search results (deferred to PR #41)
- Solidity contract deployment (InsightBoard.sol, PheromoneRegistry.sol) --
  may remain as specs if Rust precompile path is preferred

---

## 9. Consensus Safety Annotations

The codebase uses explicit annotations to prevent consensus hazards:

| Annotation | Meaning | Example |
|---|---|---|
| **CONSENSUS-SAFE** | Integer-only, deterministic, safe for on-chain use | `hamming_distance`, `bind`, `bundle`, `permute` |
| **OFF-CHAIN ONLY** | Uses `f64`, non-deterministic, local node only | `similarity`, trust scores, context assembly, affect model |

**Rule:** Any function used in block validation, precompile execution, or state
root computation must be CONSENSUS-SAFE. If an OFF-CHAIN ONLY function is ever
moved on-chain, all `f64` must be replaced with fixed-point integer arithmetic.

All data structures used in consensus-adjacent iteration use `BTreeMap` (not
`HashMap`) for deterministic key ordering. This applies to `OnChainHdcIndex`,
`WisdomGate`, and any future consensus-critical state.

---

## 10. Constants Reference

| Constant | Value | Defined In | Consensus-Critical |
|---|---|---|---|
| `D` | 10,240 bits | `constants.rs` | Yes |
| `WORDS` | 160 (= D/64) | `constants.rs` | Yes |
| `BYTES` | 1,280 (= D/8) | `constants.rs` | Yes |
| `THRESHOLD_HAMMING` | 4,854 | `constants.rs` | Yes |
| `DUPLICATE_THRESHOLD` | 512 | `constants.rs` | No (off-chain dedup) |
| `RESONANCE_THRESHOLD_HAMMING` | 1,024 | `constants.rs` | No (dream cycle) |
| `HDC_PRECOMPILE_ADDRESS` | `0x09` | `precompile.rs` (chain) | Yes (standalone) |
| HDC precompile (PR #42) | `0xA0C` | `kora-precompiles` | Yes (canonical) |
| Stigmergy precompile (PR #42) | `0xA0D` | `kora-precompiles` | Yes (canonical) |
| Gas per HDC op (PR #42) | 50,000 | `kora-precompiles` | Yes |

---

## 11. PR Reference

| PR | Title | Status | Impact |
|---|---|---|---|
| #42 | HDC precompile + stigmergy | Merged/Active | Canonical precompile addresses (0xA0C, 0xA0D), flat gas, Rust stigmergy, 4-byte selectors |
| #41 | Merkle proofs | Pending | Deferred feature: Merkle inclusion proofs in search results |
