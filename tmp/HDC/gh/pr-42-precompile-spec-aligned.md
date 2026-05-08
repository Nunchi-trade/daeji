# PR #42 — feat(precompiles): HDC precompile spec-aligned (D-PR1-v2)

**Branch:** `jl/precompile-registry-spec-aligned`
**State:** OPEN
**Author:** Jae Lee (@JaeLeex)
**Base:** main
**Stats:** +3,465 / -9 across 16 files (4 commits)

---

## What This PR Does

PR #42 delivers two chain-native Rust precompiles for the daeji/kora blockchain node, superseding the earlier PR #38 (closed). It is the implementation of the D-PR1 item from Jae's 6-PR plan.

### Precompile 1: `0xA0C` — HDCPrecompile (similarity search)

A REVM precompile providing native Hamming-distance similarity search over 10,240-bit binary hypervectors. Contracts call `staticcall(0xA0C, ...)` from Solidity to get:

- **Similarity** — Hamming distance between two HDC vectors
- **Search** — Top-K nearest-neighbor search against an in-memory index
- **Projection** — `projectBytes` and `projectTokens` for encoding raw data into HDC vectors
- **Bind** — XOR binding of two vectors
- **Bundle** — Majority-vote bundling of N vectors

**Spec-alignment fixes (vs. closed PR #38):**

| Issue | Old (PR #38) | New (PR #42) |
|-------|-------------|-------------|
| Gas cost | 5,000 flat | 50,000 flat (per spec L165) |
| External `insert`/`remove` | Allowed (consensus-breaking) | Reverts with "hdc: insert/remove are node-internal" |
| Event-replay rebuild | Missing | `HDCState::on_finalize_extend` decodes `InsightPosted` logs, inserts, and evicts |
| Eviction | Missing | `HdcIndex::evict_expired(now_ts)` drops entries past 7× effective_half_life |
| Merkle proof | Missing | Stubbed empty bytes (deferred to PR #41) |

The HDC index is now a **deterministic function of `InsightPosted` events**. No external caller can mutate it — only the `on_finalize` hook processes canonical event logs to update the index. This is the key consensus-safety property: two validators processing the same block logs produce identical HDC index states.

### Precompile 2: `0xA0D` — StigmergyPrecompile (NEW)

A chain-native stigmergic state store, moving pheromone/tier/decay logic out of Solidity (`InsightBoard.sol`) into Rust. Driven by Will Pankiewicz's 2026-05-06 standup directive:

> "Those don't need to go into any contracts necessarily... an agent would post things like this storage layer that's kind of shared between everyone... earn rewards based on how many other people maybe like query this or back this with conviction."

**Owns:**
- Pheromone counter per insight (permanent, monotonic)
- Tier tracking (Transient / Working / Consolidated / Persistent)
- Distinct context tags + cardinality
- `currentWeight` decay formula per spec L417-425 (weight × pheromone-boost, tier-multiplied half-life)
- `earnedTier` criteria: pheromone ≥ 2 → Working; distinct_contexts ≥ 3 → Consolidated
- Eviction at 7× effective_half_life

**Read-only selectors (flat 5k gas):**

| Selector | Signature | Returns |
|----------|-----------|---------|
| `0x3c5fa72b` | `currentWeight(uint256)` | `uint256` (1e6 fixed-point) |
| `0x53f96df2` | `tierOf(uint256)` | `uint8` |
| `0x90a4493b` | `pheromoneOf(uint256)` | `uint64` |
| `0x1d5a9ccc` | `distinctContextsCount(uint256)` | `uint64` |
| `0xadf2ef27` | `earnedTier(uint256)` | `uint8` |

**Mutation via event-replay only** — same consensus-safety discipline as HDCState:
- `InsightPosted` → `apply_posted` (initial state, Tier=Transient)
- `InsightConfirmed` → `apply_confirmed` (pheromone++, distinct context tracking)
- `InsightPromoted` → `apply_promoted` (tier change)

Foreign emitters filtered via `set_insight_board(canonical)`.

---

## Files Changed

| File | Change | Lines |
|------|--------|-------|
| `Cargo.lock` | Updated | +14 |
| `Cargo.toml` | Add `kora-precompiles` workspace member | +3/-1 |
| `crates/e2e/src/harness.rs` | Update for non-const `RevmExecutor::new` | +1/-1 |
| `crates/node/executor/Cargo.toml` | Add `kora-precompiles` dep | +1 |
| `crates/node/executor/src/revm.rs` | Wire HDCState + precompiles into RevmExecutor | +34/-7 |
| **`crates/precompiles/` (NEW CRATE)** | | |
| `Cargo.toml` | New crate definition | +22 |
| `src/agent_ns.rs` | Reserved 0xA10–0xA1F for agent namespace | +35 |
| `src/hdc.rs` | 8-selector HDC dispatcher + spec-aligned logic | +914 |
| `src/hdc_index.rs` | Brute-force top-K Hamming index with decay | +275 |
| `src/hdc_vector.rs` | 10,240-bit HdcVector (vendored from roko-primitives) | +686 |
| `src/insight_event.rs` | InsightPosted/Confirmed/Promoted event decoders | +243 |
| `src/insight_id.rs` | InsightId (FNV-1a128) + KnowledgeKind enum | +131 |
| `src/lib.rs` | Module declarations + KoraPrecompiles wrapper | +48 |
| `src/projection.rs` | project_bytes, project_tokens, ProjectionMatrix | +239 |
| `src/stigmergy.rs` | StigmergyState + StigmergyPrecompile at 0xA0D | +650 |
| `tests/on_finalize_extend.rs` | 6 integration tests for event-replay | +169 |

---

## Commits (4)

1. **`161823b`** — `feat(precompiles): chain-specific precompile registry + HDC at 0xA0C`
   Initial implementation: vendored HdcVector, 8-selector dispatcher, brute-force index, projection, InsightId. 52 tests passing.

2. **`e18b86d`** — `feat(precompiles): HDC precompile spec-aligned (D-PR1-v2)`
   Spec-alignment rework: gas 5k→50k, external insert/remove reverted, on_finalize event-replay, eviction, Merkle proof stub. 62 tests (6 new integration).

3. **`2a0abac`** — `feat(precompiles): chain-native StigmergyPrecompile at 0xA0D`
   New precompile for pheromone/tier/decay state. 11 new tests, 73 total.

4. **`afe2440`** — `fix(precompiles/stigmergy): correct dispatch selector constants`
   Fixes keccak256 selector mismatches (all 5 selectors were wrong). Adds `selectors_match_solidity_signatures` regression test.

---

## Tests (73 total)

- 56 lib tests (HDC vector ops, decoders, integration)
- 11 stigmergy tests (apply_*, earned_tier, decay golden values, pheromone boost asymptote, eviction, deterministic-across-replays)
- 6 integration tests in `tests/on_finalize_extend.rs`:
  - `on_finalize_extend_indexes_synthetic_logs`
  - `on_finalize_extend_filters_logs_from_other_emitters`
  - `on_finalize_extend_evicts_expired_warning_after_21_minutes`
  - `on_finalize_extend_keeps_long_lived_kinds_alive`
  - `on_finalize_extend_is_deterministic_across_replays` (THE consensus-correctness test)
  - `on_finalize_extend_skips_malformed_events_gracefully`

All pass: `cargo test -p kora-precompiles`, `cargo build --workspace`, `cargo clippy -- -D warnings`, `cargo deny check`.

---

## Cross-Repo Coordination

Paired with:
- **contracts-core#123** — slim InsightBoard.sol (provenance + token economics only)
- **contracts-core#124** — `postGuarded` with AntiKnowledge gating

---

## Deferred Work

- **FinalizedReporter wiring** in `crates/node/reporters/` — next stacked PR
- **Merkle proof in HDC Hit output** — tracked by PR #41
- **InsightBoard address registration** at chain bootstrap — currently set via `set_insight_board()`; needs config wiring

---

## Key Architectural Decisions

1. **Event-replay consensus model.** The HDC index and stigmergy state are deterministic functions of on-chain event logs. No external callers can mutate state directly. Two validators processing the same logs produce identical state.

2. **Precompile address: `0xA0C` not `0x09`.** Follows mirage-rs precedent. Does not collide with Ethereum standard precompiles.

3. **Stigmergy in Rust, not Solidity.** Per Will's standup direction, pheromone/tier/decay computation is chain-native Rust state, not contract state. InsightBoard.sol becomes thin (provenance + token economics only).

4. **Vendored HdcVector.** The vector implementation is vendored from `roko-primitives` into `kora-precompiles` with modifications: removed non-deterministic `random()`, removed `rkyv` feature blocks.

5. **Fixed-point decay.** `currentWeight` uses 1e6 fixed-point arithmetic (no floating-point in consensus paths).
