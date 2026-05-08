# 21 -- Reconciliation: Impl Plans vs PR #42 / PR #41

> **Created:** 2026-05-08
> **Purpose:** Catalog every known divergence between the implementation plans
> (docs 01-20) and the actual code in PR #42 (`kora-precompiles`) and PR #41
> (Merkle proofs plan). For each divergence, document what the plan says, what
> the PR does, which approach is recommended, and what action items remain.

---

## Table of Contents

1. [Divergence Inventory](#1-divergence-inventory)
2. [Decision Matrix](#2-decision-matrix)
3. [Detailed Divergence Analysis](#3-detailed-divergence-analysis)
4. [Reconciliation Order](#4-reconciliation-order)
5. [Impact Assessment](#5-impact-assessment)
6. [Reconciliation Checklist](#6-reconciliation-checklist)

---

## 1. Divergence Inventory

| # | Area | Plan Says | PR #42 / #41 Does | Severity |
|---|------|-----------|-------------------|----------|
| D01 | Precompile address | `0x09` | `0xA0C` (HDC), `0xA0D` (Stigmergy) | HIGH |
| D02 | Crate structure | Two crates: `kora-hdc` + `kora-hdc-chain` | Three crates: `kora-hdc` + `kora-hdc-chain` + `kora-precompiles` | MEDIUM |
| D03 | Stigmergy | Solidity `PheromoneRegistry.sol` | Rust `StigmergyPrecompile` at `0xA0D` | HIGH |
| D04 | Gas costs | 1,500 (hamming), 500 (bind/bundle/permute) | 50,000 flat for all operations | MEDIUM |
| D05 | Selectors | Raw 1-byte opcode (`input[0]`) | 4-byte function selectors (keccak of signature) | HIGH |
| D06 | On-chain search | Tiered search (brute-force + HNSW + local) | Brute-force only | LOW |
| D07 | Event replay | Deferred (event.rs stubbed with B256::ZERO) | Implemented via `on_finalize_extend` | HIGH |
| D08 | Alpha paradox | Spec'd in PheromoneRegistry.sol + docs | Not implemented in kora-precompiles | MEDIUM |
| D09 | Half-life values | 48h for Warning kind (432,000 blocks) | 3 minutes in tests (450 blocks at 400ms) | LOW |
| D10 | Merkle proofs | Not addressed in hdc branch | Stubbed in PR #42 + full plan in PR #41 | MEDIUM |

---

## 2. Decision Matrix

| # | Divergence | Adopt Plan | Adopt PR #42 | Hybrid | RECOMMENDED |
|---|-----------|------------|-------------|--------|-------------|
| D01 | Precompile address | NO -- `0x09` collides with BLAKE2F (EIP-152) | **YES** -- `0xA0C` is in unallocated space | -- | **PR #42** |
| D02 | Crate structure | Simpler, fewer boundaries | Better separation of concerns | Merge precompiles into chain crate | **PR #42** (with review) |
| D03 | Stigmergy | Solidity is portable across chains | Rust is faster, consensus-safer | Keep Solidity as fallback interface | **PR #42** |
| D04 | Gas costs | More granular, reflects actual cost differences | Conservative, prevents DoS | Per-opcode costs with higher floor | **HYBRID** |
| D05 | Selectors | Simpler dispatch, lower overhead | Standard EVM tooling compatible | -- | **PR #42** |
| D06 | Search | HNSW provides O(log n) search | Simpler, correct, auditable | HNSW as optional optimization | **HYBRID** |
| D07 | Event replay | -- | Functional event replay exists | -- | **PR #42** |
| D08 | Alpha paradox | Core to stigmergy design | Missing from precompile | -- | **PLAN** (must add to PR #42) |
| D09 | Half-life values | Production values | Test-only values | Tests use short values, production uses plan values | **HYBRID** |
| D10 | Merkle proofs | No plan yet | Concrete plan exists (PR #41) | -- | **PR #41** |

---

## 3. Detailed Divergence Analysis

### D01: Precompile Address (`0x09` -> `0xA0C`)

**What the plan says:**
Docs 07 (shared-substrate), 15 (wiring-guide), and 19 (solidity-specs) all
specify the HDC precompile at address `0x09`. The `HdcLib.sol` library hardcodes
`address(0x09)`.

**What PR #42 does:**
Uses `0xA0C` for the HDC precompile and `0xA0D` for the Stigmergy precompile.
These addresses are in the "custom precompile" range (above `0x0A00`) and do not
collide with any EIP-specified precompile.

**Which is RECOMMENDED and why:**
**PR #42 (`0xA0C`).** Address `0x09` is allocated to BLAKE2F by EIP-152 (Istanbul
hardfork, 2019). Using `0x09` for HDC means:
1. Standard EVM tooling assumes BLAKE2F is at `0x09` and may send BLAKE2F inputs.
2. Any contract or library that depends on BLAKE2F (e.g., Zcash verification,
   Filecoin proofs) will silently call the HDC precompile instead.
3. Cross-chain compatibility is impossible if another chain expects BLAKE2F at
   `0x09`.

`0xA0C` avoids all of these issues.

**ACTION ITEMS:**
- [ ] Update `crates/hdc/chain/src/precompile.rs` line 14-17: change address
  to `0xA0C`
- [ ] Update `contracts/src/HdcPrecompile.sol`: change
  `HDC_PRECOMPILE = address(0x09)` to `address(0xA0C)`
- [ ] Update `crates/node/executor/src/hdc_precompiles.rs`: precompile
  registration address
- [ ] Update docs 07, 15, 19: all references to `0x09`
- [ ] Add `0xA0D` for StigmergyPrecompile if adopted

---

### D02: Crate Structure (Two-Crate -> Three-Crate)

**What the plan says:**
Two crates: `kora-hdc` (pure HDC algebra, no chain deps) and `kora-hdc-chain`
(precompile, index, event, RPC, wisdom). This is documented in docs 02, 10, and
15.

**What PR #42 does:**
Adds a third crate: `kora-precompiles` which implements the precompile dispatch,
gas metering, and selector-based routing. This crate sits between `kora-hdc` and
`kora-executor`.

**Which is RECOMMENDED and why:**
**PR #42 (three crates).** The `kora-precompiles` crate cleanly separates
"precompile dispatch and gas" from "index, event, RPC" concerns. The two-crate
model forced `kora-hdc-chain` to handle both precompile mechanics and node
integration, making it a kitchen-sink crate.

However, the orphaned `crates/kora-hdc/` crate (finding F01) must be deleted
to avoid confusion. After reconciliation, the crate structure should be:

```
kora-hdc (crates/hdc/core)          -- pure HDC algebra
kora-precompiles (crates/hdc/precompiles) -- precompile dispatch + gas
kora-hdc-chain (crates/hdc/chain)   -- index, event, RPC, wisdom
```

**ACTION ITEMS:**
- [ ] Delete `crates/kora-hdc/` (orphan)
- [ ] Create or adopt `kora-precompiles` crate from PR #42
- [ ] Move precompile dispatch logic out of `kora-hdc-chain`
- [ ] Update workspace `Cargo.toml` with new crate paths
- [ ] Update dependency graph in doc 15

---

### D03: Stigmergy (Solidity -> Rust Precompile)

**What the plan says:**
Stigmergy is implemented as a Solidity contract (`PheromoneRegistry.sol`) with
on-chain storage, exponential decay computed at read time, SINR interference
modeling, and alpha paradox via half-life reduction on confirmation. See docs
09 (optimal-design) and 19 (solidity-specs).

**What PR #42 does:**
Moves stigmergy to a Rust precompile at address `0xA0D`. Pheromone deposit,
decay, and read operations are handled natively in Rust. The Solidity contract
is bypassed entirely.

**Which is RECOMMENDED and why:**
**PR #42 (Rust precompile).** Reasons:
1. **Gas efficiency:** Fixed-point integer decay in Rust avoids the ~50k gas
   overhead of Solidity's emulated fixed-point arithmetic per read.
2. **Consensus safety:** Rust implementation can use `BTreeMap` for deterministic
   iteration. Solidity's `_locationPheromones` array manipulation is O(n) and
   the swap-and-pop removal pattern is subtle to get right.
3. **Consistency:** Keeps both HDC and stigmergy in the precompile layer, making
   the architecture more uniform.

However, the alpha paradox (finding D08) must be ported to the Rust
implementation. It is currently missing from `kora-precompiles`.

**ACTION ITEMS:**
- [ ] Adopt `StigmergyPrecompile` from PR #42
- [ ] Port alpha paradox logic (half-life reduction on confirmation)
- [ ] Decide: delete `PheromoneRegistry.sol` or keep as reference
- [ ] Add `IStigmergyPrecompile.sol` Solidity interface
- [ ] Update docs 09 and 19

---

### D04: Gas Costs (1.5k/500 -> 50k Flat)

**What the plan says:**
Per-operation gas costs based on computational complexity:
- Hamming distance: 1,500 gas
- Bind: 500 gas
- Bundle: 500 gas base + per-vector
- Permute: 500 gas

**What PR #42 does:**
Flat 50,000 gas for all operations regardless of type.

**What the `hdc` branch does:**
100-200 gas per operation (dramatically underpriced).

**Which is RECOMMENDED and why:**
**HYBRID.** The plan's values (500-1,500) are too low for production but reflect
real cost differences between operations. PR #42's flat 50,000 is conservative
but loses cost granularity. The `hdc` branch's 100-200 is a DoS vector.

Recommended approach:
- Hamming distance: 5,000 gas (160-word XOR + popcount)
- Bind: 3,000 gas (160-word XOR)
- Bundle: 5,000 + 2,000 per vector (accumulate + finalize)
- Permute: 3,000 gas (160-word rotation)
- Search: 50,000 gas (brute-force over index)
- ProjectBytes/ProjectTokens: 10,000 gas (keccak + PRNG)

These should be benchmarked before production and adjustable via chain config.

**ACTION ITEMS:**
- [ ] Benchmark actual CPU cost of each operation
- [ ] Implement per-opcode gas with a minimum floor of 3,000
- [ ] Make gas costs configurable via `HdcConfig`
- [ ] Update docs 07 and 18 with final gas schedule

---

### D05: Selectors (Raw Opcode -> 4-Byte)

**What the plan says:**
Raw 1-byte opcode dispatch: `input[0]` is the opcode byte (0x01-0x06), followed
by raw concatenated arguments.

**What PR #42 does:**
4-byte function selector dispatch: `input[0..4]` is `keccak256(signature)[0:4]`,
followed by standard ABI-encoded arguments.

**Which is RECOMMENDED and why:**
**PR #42 (4-byte selectors).** Reasons:
1. **Standard tooling:** ethers.js, cast, foundry, and all EVM tooling understand
   4-byte selectors. Raw opcodes require custom encoding/decoding.
2. **Solidity compatibility:** Standard Solidity interface calls work directly.
   No need for `abi.encodePacked(uint8(opcode), ...)` hacks.
3. **Extensibility:** 2^32 selector space vs. 256 opcodes.
4. **Debugging:** Selector databases (4byte.directory, openchain.xyz) can
   identify HDC precompile calls in transaction traces.

The 3-byte overhead (4 vs 1) is negligible compared to the 1,280-byte vector
payloads.

**ACTION ITEMS:**
- [ ] Adopt 4-byte selector dispatch from PR #42
- [ ] Update `HdcLib.sol` to use `abi.encodeWithSelector(...)` or interface calls
- [ ] Update `precompile.rs` dispatch to match
- [ ] Publish selector table in doc 18 (done -- see section 10.2)
- [ ] Add cross-language selector verification test

---

### D06: On-Chain Search (Tiered -> Brute-Force Only)

**What the plan says:**
Tiered search with three backends: brute-force for small indexes, HNSW for
medium indexes, and local search for per-agent knowledge. The search module in
`crates/hdc/core/src/search/` implements all three.

**What PR #42 does:**
Brute-force only. The precompile's `search` function iterates all stored vectors
and computes Hamming distance against each one.

**Which is RECOMMENDED and why:**
**HYBRID.** Brute-force is correct, simple, and auditable. For the initial
testnet with <10k vectors, brute-force at ~1ms per 1k vectors is fast enough.
HNSW should remain as an optional optimization that can be enabled when the
index exceeds a threshold (e.g., 10k vectors).

The HNSW implementation already exists and is consensus-safe (uses `BTreeMap`,
deterministic level assignment). It should not be deleted, just not activated
by default.

**ACTION ITEMS:**
- [ ] Keep brute-force as default search backend
- [ ] Keep HNSW code in `crates/hdc/core/src/search/hnsw.rs`
- [ ] Add a configurable threshold in `HdcConfig` for HNSW activation
- [ ] Document search backend selection in doc 03

---

### D07: Event Replay (Deferred -> Implemented)

**What the plan says:**
Event replay is described in docs 10, 15, and 20 but the `hdc` branch's
`event.rs` is entirely stubbed. All topic hashes are `B256::ZERO` and
`process_log()` is a no-op.

**What PR #42 does:**
Implements event replay via `on_finalize_extend` hook. Event decoders exist for
InsightPublished, InsightConfirmed, and PheromoneDeposited events. The index is
updated during finalized block processing.

**Which is RECOMMENDED and why:**
**PR #42.** Event replay is critical for node operation: without it, the
on-chain index is always empty after restart. PR #42's implementation is
functional and should be adopted.

**ACTION ITEMS:**
- [ ] Adopt PR #42's event replay mechanism
- [ ] Delete or replace the stubbed `event.rs` in `kora-hdc-chain`
- [ ] Verify event topic hashes match current Solidity ABI
- [ ] Wire `FinalizedReporter` HDC event sink in `runner.rs`
- [ ] Add event replay unit tests with fixture logs

---

### D08: Alpha Paradox (Spec'd -> Not Implemented)

**What the plan says:**
The alpha paradox is a core stigmergy mechanic: when a pheromone is confirmed by
another agent, its half-life is *reduced* (not extended). Formula:
`new_half_life = base_half_life / (1 + confirmation_count)`. This creates a
harmonic reduction where popular pheromones die faster, preventing signal
monopolization. Documented in docs 09 and 19.

**What PR #42 does:**
The `StigmergyPrecompile` handles deposit and read operations but does not
implement the alpha paradox. Confirmation does not affect half-life.

**Which is RECOMMENDED and why:**
**PLAN.** The alpha paradox is essential to the stigmergy design. Without it,
a single high-intensity pheromone can dominate a region indefinitely. The
half-life reduction ensures that well-known information fades faster, making
room for novel signals.

**ACTION ITEMS:**
- [ ] Add `confirm()` method to `StigmergyPrecompile`
- [ ] Implement half-life reduction: `effective_hl = base_hl / (1 + confs)`
- [ ] Reset deposit block on confirmation (new decay baseline)
- [ ] Add integration test verifying decay rate increases after confirmation
- [ ] Update docs 09 and 19 with Rust implementation details

---

### D09: Half-Life Values (48h Warning -> 3 min in Tests)

**What the plan says:**
Warning kind has a base half-life of 48 hours = 432,000 blocks (at 400ms/block).
Other kinds range from 72h (Insight) to 336h (AntiKnowledge).

**What PR #42 does:**
Tests use a 3-minute Warning half-life (~450 blocks) for fast test execution.

**Which is RECOMMENDED and why:**
**HYBRID.** Production code must use the plan's half-life values. Test code
should use short values for fast execution. The solution is configuration:

```rust
// Production config
half_lives:
  warning: 432_000    # 48h at 400ms/block
  insight: 648_000    # 72h
  # ...

// Test config
half_lives:
  warning: 450        # ~3 min at 400ms/block
  insight: 675        # ~4.5 min
```

**ACTION ITEMS:**
- [ ] Make half-life values configurable in `HdcConfig` (not hardcoded)
- [ ] Set production defaults to plan values (48h Warning, etc.)
- [ ] Set test defaults to short values (3 min Warning, etc.)
- [ ] Add validation: half-life must be >= minimum (e.g., 100 blocks)
- [ ] Update doc 09 with configurable half-life design

---

### D10: Merkle Proofs (Not Addressed -> Stubbed + Plan)

**What the plan says:**
The impl plans do not address Merkle proofs. The `hdc` branch has no proof
infrastructure.

**What PR #41 does:**
Provides a concrete follow-up plan for Merkle proofs via `QmdbProvable` trait
injection. Defines:
- A trait for generating proofs from QMDB storage
- Integration points in the precompile for proof verification
- A phased rollout (stub -> basic proofs -> full verification)

**What PR #42 does:**
Stubs Merkle proof support with placeholder types but no implementation.

**Which is RECOMMENDED and why:**
**PR #41 (plan).** PR #41 provides the most concrete path forward. PR #42's
stubs align with PR #41's Phase 1 (stub). The implementation should follow
PR #41's plan when Merkle proofs become a priority.

**ACTION ITEMS:**
- [ ] Adopt PR #42's proof stubs as the current baseline
- [ ] Follow PR #41's plan for phased implementation
- [ ] Track in milestone planning (not current testnet scope)
- [ ] No immediate code changes needed

---

## 4. Reconciliation Order

Execute reconciliation in this order to minimize cascading changes:

### Phase 1: Foundation (Do First)

| Priority | Divergence | Rationale |
|----------|-----------|-----------|
| 1 | D01: Precompile address | All other changes depend on the correct address |
| 2 | D05: Selectors | Dispatch format affects every precompile call |
| 3 | D02: Crate structure | Clean crate layout before modifying code |

### Phase 2: Core Integration

| Priority | Divergence | Rationale |
|----------|-----------|-----------|
| 4 | D07: Event replay | Required for functional node operation |
| 5 | D12: Solidity opcode mismatch (F12) | Contracts non-functional without this |
| 6 | D03: Stigmergy | Blocks pheromone functionality |

### Phase 3: Tuning and Polish

| Priority | Divergence | Rationale |
|----------|-----------|-----------|
| 7 | D04: Gas costs | Important but not blocking |
| 8 | D08: Alpha paradox | Important for stigmergy design, not for testnet launch |
| 9 | D09: Half-life values | Configuration change, low risk |
| 10 | D06: Search | Brute-force works for testnet scale |

### Phase 4: Deferred

| Priority | Divergence | Rationale |
|----------|-----------|-----------|
| 11 | D10: Merkle proofs | Explicitly deferred per PR #41 |

---

## 5. Impact Assessment

### What Breaks If We Adopt PR #42's Approach Wholesale?

| Component | Impact | Severity | Mitigation |
|-----------|--------|----------|------------|
| `HdcLib.sol` | **Breaks.** Address, selectors, and ABI all change. | HIGH | Rewrite `HdcLib.sol` to use `IHDCPrecompile` interface at `0xA0C` |
| `InsightBoard.sol` | **Breaks.** Calls to `HdcLib` fail. `submit()`, `searchSimilar()`, `purge()` all affected. | HIGH | Update all `HdcLib` calls to new interface |
| `PheromoneRegistry.sol` | **Superseded.** Entire contract replaced by `StigmergyPrecompile`. | HIGH | Delete or keep as fallback; must decide |
| `InsightBoard.t.sol` | **Breaks.** `TestableInsightBoard` bypasses precompile, but address/selector changes affect setup. | MEDIUM | Rewrite tests with mock precompile at `0xA0C` |
| `crates/hdc/chain/src/precompile.rs` | **Replaced.** `kora-precompiles` takes over dispatch. | HIGH | Delete precompile.rs or keep as dead code |
| `crates/hdc/chain/src/event.rs` | **Replaced.** PR #42 has its own event handling. | MEDIUM | Delete stubbed event.rs |
| `crates/hdc/chain/src/index.rs` | **Mostly compatible.** Index data structures may need adjustment. | LOW | Update ID derivation and state enum |
| `crates/hdc/chain/src/rpc.rs` | **Mostly compatible.** RPC methods wrap index queries. | LOW | Minor type updates |
| `crates/hdc/chain/src/wisdom.rs` | **Unaffected.** WisdomGate is independent of precompile surface. | NONE | No changes needed |
| `crates/hdc/core/*` | **Unaffected.** Pure algebra is independent of dispatch format. | NONE | No changes needed |
| E2E tests (`crates/e2e/`) | **Break.** Raw opcode calls no longer work. | MEDIUM | Update to use 4-byte selectors |
| `runner.rs` | **Minor update.** Precompile registration changes. | LOW | Update `.with_hdc_precompile()` call |

### What Breaks If We Keep the Plan's Approach?

| Component | Impact | Severity |
|-----------|--------|----------|
| BLAKE2F compatibility | **Broken.** `0x09` collides with EIP-152. | HIGH |
| EVM tooling | **Limited.** Raw opcodes not recognized by standard tools. | MEDIUM |
| Gas security | **Vulnerable.** 100-200 gas enables DoS attacks. | HIGH |
| Event replay | **Non-functional.** B256::ZERO topics, stubbed decoder. | HIGH |
| Stigmergy | **Functional but slow.** Solidity fixed-point is expensive. | MEDIUM |

**Conclusion:** Adopting PR #42 wholesale is the better path. The breakage is
concentrated in the Solidity contracts and test harnesses, which need updates
regardless. The core Rust algebra (`kora-hdc`) is unaffected.

---

## 6. Reconciliation Checklist

### Foundation

- [ ] Change precompile address from `0x09` to `0xA0C` in all files
- [ ] Change dispatch from raw opcode to 4-byte selectors
- [ ] Delete orphaned `crates/kora-hdc/` crate
- [ ] Adopt `kora-precompiles` crate from PR #42
- [ ] Update workspace `Cargo.toml` dependencies

### Solidity Contracts

- [ ] Rewrite `HdcLib.sol` as `IHDCPrecompile.sol` with 4-byte selectors
- [ ] Update `InsightBoard.sol` to use new interface at `0xA0C`
- [ ] Decide: delete `PheromoneRegistry.sol` or convert to precompile wrapper
- [ ] Add `IStigmergyPrecompile.sol` interface for `0xA0D`
- [ ] Update `DeployHDC.s.sol` with new addresses

### Rust Integration

- [ ] Wire `FinalizedReporter` HDC event sink in `runner.rs`
- [ ] Adopt PR #42's event replay mechanism
- [ ] Delete stubbed `event.rs` or replace with PR #42's implementation
- [ ] Update `hdc_precompiles.rs` registration in executor
- [ ] Implement alpha paradox in `StigmergyPrecompile`

### Gas and Configuration

- [ ] Benchmark actual CPU costs per operation
- [ ] Implement per-opcode gas with configurable values
- [ ] Make half-life values configurable in `HdcConfig`
- [ ] Set production defaults from plan, test defaults from PR #42

### Testing

- [ ] Rewrite `TestableInsightBoard` to use mock precompile
- [ ] Add cross-language selector verification tests
- [ ] Add event replay unit tests with fixture logs
- [ ] Update E2E tests to use 4-byte selectors
- [ ] Add decoded output verification to all precompile tests
- [ ] Add `hdc_enabled` flag to `TestApplication`

### Documentation

- [ ] Update docs 07, 15, 18, 19 with new addresses and selectors
- [ ] Mark `PheromoneRegistry.sol` as superseded in doc 09
- [ ] Update crate structure diagram in doc 15
- [ ] Update gas schedule in docs 07 and 18
- [ ] Add Merkle proof plan reference to doc overview

### Verification

- [ ] `cargo check --workspace --all-features` passes
- [ ] `cargo test --workspace --all-features` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean
- [ ] `forge test` passes
- [ ] E2E tests pass with real precompile
- [ ] RPC search returns results after finalized block with InsightPublished event

---

*End of reconciliation document.*
