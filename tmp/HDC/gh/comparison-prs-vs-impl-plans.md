# Comparison: PRs #41/#42 vs. HDC Implementation Plans

This document compares what the open PRs actually implement against the HDC design documents (00-09) and the implementation plans (impl/ 00-19).

---

## Executive Summary

PRs #41 and #42 implement a **subset** of the HDC vision, focused on the chain-native precompile layer. They diverge from the impl plans in several significant ways — some intentional (design improvements), some consequential (different architecture). The PRs are closer to the *spirit* of the design docs than the impl plans are, particularly regarding consensus safety and the Solidity-vs-Rust boundary.

### Coverage at a Glance

| Impl Plan | Description | PR Coverage |
|-----------|-------------|-------------|
| 00 | Overview | N/A (meta) |
| 01 | Block time migration | Not in these PRs |
| 02 | kora-hdc core algebra | Partially (vendored into `kora-precompiles`) |
| **03** | **Vector search** | **Partially (brute-force only, no HNSW, no SIMD dispatch, no tiered search)** |
| 04 | Knowledge store | Not in these PRs |
| 05 | Context assembly | Not in these PRs |
| 06 | Cognitive architecture | Not in these PRs |
| **07** | **Precompile integration** | **YES — but at `0xA0C`/`0xA0D`, not `0x09`** |
| **08** | **InsightBoard contract** | **Partially (event decoding, not the contract itself)** |
| **09** | **Pheromone registry** | **YES — but as chain-native Rust (0xA0D), not Solidity** |
| 10 | kora-hdc-chain crate | Partially (event replay, decay, but different crate structure) |
| 11 | Trust pipeline | Not in these PRs |
| 12 | RPC extensions | Not in these PRs |
| 13 | E2E tests | Partially (73 tests in kora-precompiles) |
| 14 | Railway deployment | Not in these PRs |
| 15 | Wiring guide | N/A (docs) |
| 16 | Checklist | N/A (meta) |
| 17 | Anti-patterns | N/A (reference) |
| 18 | Encoding specs | Partially (ABI encoding implemented) |
| 19 | Solidity specs | Partially (event ABIs, not full contracts) |
| N/A | Merkle proof plan | PR #41 (plan only) |

---

## Detailed Divergences

### 1. Precompile Address: `0xA0C` (PR) vs `0x09` (impl plans)

**Impl plans (07, 08, 18, 19):** Register the HDC precompile at `0x09`, explicitly replacing BLAKE2F. The plans note this is intentional since daeji doesn't need BLAKE2F.

**PR #42:** Uses `0xA0C`, following mirage-rs precedent. Avoids colliding with any Ethereum standard precompile address.

**Assessment:** The PR's choice is **safer**. Colliding with `0x09` (BLAKE2F) could cause unexpected behavior if any EVM tooling or dependency assumes standard precompile availability. `0xA0C` is in unallocated space.

### 2. Crate Structure: `kora-precompiles` (PR) vs `kora-hdc` + `kora-hdc-chain` (impl plans)

**Impl plans (00, 02, 10):** Define a two-crate architecture:
- `crates/hdc/core/` (`kora-hdc`) — pure algebra, zero chain deps
- `crates/hdc/chain/` (`kora-hdc-chain`) — precompile, event replay, chain integration

**PR #42:** Creates a single `crates/precompiles/` (`kora-precompiles`) crate containing everything — vector algebra, index, projection, event decoding, HDC precompile, and stigmergy precompile.

**Assessment:** The PR's monolithic crate is **simpler for now** but sacrifices the clean separation the impl plans designed. The pure-algebra types (`HdcVector`, `BundleAccumulator`, projection) are coupled to chain dependencies. If the off-chain cognitive components (impl plans 04-06) ever need the same algebra, they'd either depend on `kora-precompiles` (pulling in chain deps) or re-vendor the code. The impl plans' two-crate split was specifically designed to avoid this.

### 3. Stigmergy: Chain-Native Rust (PR) vs Solidity Contract (impl plans)

**Impl plans (09, 19):** `PheromoneRegistry.sol` — a standalone Solidity contract managing stigmergic state. Features include the alpha paradox (confirmation reduces half-life), SINR interference model, `cleanup()` with gas refunds, and lazy decay evaluation.

**PR #42:** `StigmergyPrecompile` at `0xA0D` — chain-native Rust state. Driven by Will's standup directive to move this out of Solidity. Features include pheromone counting, tier tracking, distinct context tracking, `currentWeight` decay, and `earnedTier` criteria.

**Key differences:**

| Feature | Impl Plan (Solidity) | PR (Rust precompile) |
|---------|---------------------|---------------------|
| Alpha paradox (confirmation reduces HL) | Yes | No — pheromone is monotonic, no HL reduction |
| SINR interference | Yes (with caller-provided interferers) | Not implemented |
| 3 pheromone types (Threat/Opportunity/Wisdom) | Yes (100/250/1000 block HLs) | No — tied to insight kinds instead |
| Cleanup/purge incentive | Yes (gas refunds) | Eviction via `evict_expired()` |
| Deposit intensity | Clamped [100, 10000] | Not applicable (event-driven) |
| Self-confirmation prevention | Yes | Not explicitly mentioned |
| Mutation model | Direct contract calls | Event-replay only |

**Assessment:** These are **substantially different designs**. The PR's version is tightly coupled to the InsightBoard event lifecycle (InsightPosted/Confirmed/Promoted), while the impl plan's PheromoneRegistry is a self-contained standalone contract. The PR follows Will's standup direction to move stigmergy out of Solidity, which the impl plans hadn't incorporated yet. The alpha paradox (arguably the most novel game-theoretic feature) is absent from the PR.

### 4. Selector Interface: ABI Function Selectors (PR) vs Opcode Byte (impl plans)

**Impl plans (07):** The precompile dispatches on `input[0]` as a raw opcode byte (`0x01` = hamming, `0x02` = bind, `0x03` = bundle, etc.).

**PR #42:** Uses Solidity-standard 4-byte function selectors (keccak256 of the function signature). This matches how contracts naturally call precompiles via interface definitions.

**Assessment:** The PR's approach is **better for composability**. Standard ABI selectors mean any Solidity contract can call the precompile through a normal interface (`IHDCPrecompile`, `IStigmergyPrecompile`) without special encoding. The opcode-byte approach in the impl plans would require custom ABI encoding from Solidity callers.

### 5. Gas Costs

**Impl plans (07):** 1,500 gas for hamming, 500 gas for bind/bundle/permute.

**PR #42:** 50,000 gas flat for HDC (per spec L165), 5,000 gas flat for stigmergy.

**Assessment:** The PR aligns with the spec. The impl plans used much lower gas costs that may have been placeholder values.

### 6. Event-Replay Consensus Model

**Impl plans (07, 10):** Acknowledge that stateful operations (store, search) need EVM journal/DB access and mark them as stubs. The `NeuroChainSync` component in plan 10 handles event replay from `InsightPublished` logs.

**PR #42:** Fully implements `HDCState::on_finalize_extend` for event-replay rebuilding the HDC index from `InsightPosted` logs. Also implements `StigmergyState::apply_*` for stigmergy state derivation from events. Includes a critical determinism test (`on_finalize_extend_is_deterministic_across_replays`).

**Assessment:** The PR is **ahead of the impl plans** on this critical consensus-safety feature. The impl plans deferred this; the PR implemented it.

### 7. InsightBoard: Event Decoding (PR) vs Full Contract (impl plans)

**Impl plans (08, 19):** Full `InsightBoard.sol` specification with 7-state FSM, staking, confirmation, challenge, purge, and duplicate detection via precompile.

**PR #42:** Implements the event *decoder* side — `decode_insight_posted`, `decode_insight_confirmed`, `decode_insight_promoted` — but not the contract itself. The PR expects `InsightBoard.sol` to be deployed separately (via contracts-core#123/#124).

**Assessment:** These are **complementary**, not competing. The PR implements the chain-node side that processes InsightBoard events; the contract itself lives in a separate repo.

### 8. Knowledge Kinds and Half-Lives

**Design docs (04):** Six kinds — Insight (72h), Heuristic (168h), AntiKnowledge (336h), Warning (48h), CausalLink (240h), StrategyFragment (120h).

**Impl plans (08):** Same six kinds with block-based half-lives at 0.4s/block.

**PR #42:** References `KnowledgeKind` with six variants. Uses `decode_warning_kind_uses_three_minute_half_life` test (Warning at 3 minutes = 180s, not 48h). The tier multiplier tests reference Transient default.

**Assessment:** The half-life values in the PR **may differ** from both the design docs and impl plans. The 3-minute Warning half-life in the PR is dramatically shorter than the 48-hour value in the design docs. This may be intentional for testing or may reflect a different spec source. Needs review.

### 9. Merkle Proof

**Design docs (06, 09):** Mention that search results should include Merkle proofs for third-party verification.

**Impl plans (07):** Not addressed (stateful ops were deferred).

**PR #42:** Explicitly stubs to empty bytes.

**PR #41:** Provides the full design plan for implementing proofs via `QmdbProvable` trait, recommending Option A (inject `Arc<dyn QmdbProvable>` at construction).

**Assessment:** PR #41 provides a **clearer path** than the impl plans, which don't mention Merkle proofs at all. This is a net addition from the PRs.

---

## What the PRs Cover That the Impl Plans Don't

1. **StigmergyPrecompile (0xA0D)** — entirely new, driven by Will's standup directive. The impl plans have pheromone as Solidity, not Rust.

2. **Event decoder infrastructure** — `insight_event.rs` with `InsightPosted`, `InsightConfirmed`, `InsightPromoted` decoders using alloy-sol-types. The impl plans mention event replay but don't spec the decoder.

3. **Agent namespace reservation** — `agent_ns.rs` reserves `0xA10-0xA1F` for future agent precompiles (passport, capability, tier, reputation). Not in any impl plan.

4. **Selector correctness fix** — Commit 4 (`afe2440`) fixes all 5 stigmergy selectors that were wrong, with a regression test. This kind of bug-and-fix cycle isn't captured in impl plans.

5. **Merkle proof design plan** — PR #41 provides a concrete follow-up plan that doesn't exist in any impl doc.

---

## What the Impl Plans Cover That the PRs Don't

1. **Off-chain cognitive substrate** (impl plans 04-06) — Knowledge store, context assembly (VCG auction, 9-layer prompt), cognitive architecture (ALMA affect model, behavioral state machine, dream cycle). These are the bulk of the impl plans by volume and are entirely outside PR scope.

2. **SIMD optimization** (impl plan 03) — AVX2 Harley-Seal, AVX-512 VPOPCNTDQ, NEON paths for Hamming distance. PR #42 uses scalar `count_ones()` only.

3. **HNSW index** (impl plan 03) — Approximate nearest-neighbor for >100K vectors. PR #42 has brute-force only.

4. **Tiered on-chain search** (impl plan 03) — 3-tier gas optimization (first-word → sample-16 → full). PR #42 does full brute-force.

5. **Trust pipeline** (impl plan 11) — 5-stage multiplicative trust with immune system. Not in PRs.

6. **RPC extensions** (impl plan 12) — `hdc_*` JSON-RPC namespace. Not in PRs.

7. **Block time migration** (impl plan 01) — `block_time` → `block_time_ms`. Not in these PRs.

8. **Railway deployment** (impl plan 14) — 3-validator testnet. Not in PRs.

9. **Full Solidity contracts** (impl plans 08, 09, 19) — InsightBoard.sol and PheromoneRegistry.sol. The PRs implement the Rust-side counterparts only; the Solidity contracts are tracked in contracts-core PRs.

10. **Anti-knowledge subspace** (design doc 04, impl plan 04) — `ANTI_SUBSPACE` binding, contrarian retrieval, 3-tier anti-knowledge response. Not in PRs (off-chain concern).

11. **Alpha paradox** (impl plan 09) — confirmation reducing half-life. Not in PR's StigmergyPrecompile.

12. **SINR interference model** (impl plan 09) — signal-to-interference ratio preventing Sybil amplification. Not in PRs.

---

## Alignment with Design Docs (00-09)

The design docs describe a much larger system. Here's how the PRs map to each:

| Design Doc | PR Alignment |
|-----------|-------------|
| 00-index | PRs implement a slice of the "shared substrate" layer |
| 01-context | PRs implement the precompile infrastructure that enables the hybrid storage model described here |
| 02-hdc-foundations | PR #42 vendors the core BSC algebra (HdcVector, bind, bundle, permute, hamming). Does NOT implement GHRR, resonator networks, or SBDR |
| 03-roko-analysis | PR #42 directly addresses the P0 consensus-safety bugs identified here: no floating-point in consensus paths, deterministic event-replay, eviction. Does NOT fix HashMap ordering (uses RwLock<HdcIndex> with Vec storage) |
| 04-knowledge | PR #42 implements KnowledgeKind enum and tier system. Does NOT implement demurrage economics, anti-knowledge subspace, or the full knowledge lifecycle |
| 05-context-assembly | Not addressed by PRs (off-chain) |
| 06-vector-search | PR #42 implements brute-force search. Does NOT implement SIMD paths, HNSW, or tiered search pipeline |
| 07-shared-substrate | PR #42 implements the precompile side of the hybrid storage model (in-memory index rebuilt from events). InsightBoard and ReputationRegistry contracts are in contracts-core |
| 08-cognitive-architecture | Not addressed by PRs (off-chain) |
| 09-optimal-design | PR #42 follows the determinism-is-non-negotiable principle from Layer 1. Implements parts of Layer 3 (storage/search) and Layer 6 (chain integration) |

---

## Recommendations

1. **Reconcile precompile address** — the impl plans say `0x09`, the PR uses `0xA0C`. If `0xA0C` is the decision, update all impl plans.

2. **Reconcile crate structure** — decide whether `kora-precompiles` stays monolithic or splits into `kora-hdc` (pure) + `kora-hdc-chain` per the impl plans. The split matters when building off-chain components.

3. **Reconcile stigmergy design** — the PR's StigmergyPrecompile and the impl plan's PheromoneRegistry are fundamentally different. Decide whether the alpha paradox, SINR, and 3-type pheromone model are still desired.

4. **Verify half-life values** — Warning half-life appears to be 3 minutes in PR tests vs. 48 hours in design docs. Clarify which is canonical.

5. **Plan SIMD and HNSW follow-ups** — the brute-force search in PR #42 will not scale beyond ~10K vectors. The impl plans' tiered search and HNSW are needed for production.

6. **Update impl plans to reflect Rust-native stigmergy** — Will's standup directive invalidates impl plans 09 and 19 (Solidity PheromoneRegistry). These should be updated or marked superseded.
