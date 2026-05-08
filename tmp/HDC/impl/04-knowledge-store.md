# 04 -- Knowledge Store Implementation

> **STATUS: FULLY IMPLEMENTED** (all knowledge store components complete and tested)
>
> | Component | Status | Source File | Test Count |
> |-----------|--------|------------|------------|
> | KnowledgeKind enum (6 kinds) | DONE | `crates/hdc/core/src/knowledge/kind.rs` | 1 |
> | KnowledgeTier enum (4 tiers) | DONE | `crates/hdc/core/src/knowledge/tier.rs` | 5 |
> | KnowledgeSource enum | DONE | `crates/hdc/core/src/knowledge/source.rs` | -- |
> | KnowledgeEntry struct | DONE | `crates/hdc/core/src/knowledge/entry.rs` | -- |
> | Decay / demurrage | DONE | `crates/hdc/core/src/knowledge/decay.rs` | -- |
> | Anti-knowledge (ANTI_SUBSPACE, encode, classify) | DONE | `crates/hdc/core/src/knowledge/anti.rs` | 8 |
> | 4-factor scoring | DONE | `crates/hdc/core/src/knowledge/scoring.rs` | 4 |
> | Trust pipeline (5-stage) | DONE | `crates/hdc/core/src/knowledge/trust.rs` | -- |
> | KnowledgeStore (full) | DONE | `crates/hdc/core/src/knowledge/store.rs` | 9 |

---

## Verification Commands

```bash
# Run all knowledge module tests:
cargo test -p kora-hdc knowledge::

# Specific test names:
#   kind: test_half_life_values
#   tier: test_multiplier_values, test_weight_values, test_try_promote, test_demote, test_tier_ordering
#   anti: test_anti_subspace_deterministic, test_encode_anti_self_inverse, test_anti_orthogonal,
#         test_is_anti_true_positive, test_is_anti_true_negative, test_classify_strong,
#         test_classify_moderate, test_classify_weak, test_classify_none
#   scoring: test_score_weights_sum_to_one, test_score_perfect_entry, test_score_relevance_dominates,
#            test_emotional_resonance_boosts_score
#   store: test_insert_and_search, test_tick_gc, test_tick_promotion, test_tick_demotion,
#          test_persistent_never_deleted, test_promotion_beats_demotion, test_reinforce_resets_balance,
#          test_record_contradiction_halves_balance, test_search_with_anti_check,
#          test_anti_check_does_not_reject_unrelated

# Full crate tests:
cargo test -p kora-hdc
```

---

## CRITICAL: Actual Names vs Spec Names

The spec originally listed incorrect names. The implementation uses these **correct** names:

### KnowledgeKind (6 kinds -- NOT Fact/Episodic/Procedural)

| Implemented Name | Base Half-Life (hours) | Description |
|-----------------|----------------------|-------------|
| `Insight` | 72.0 | Derived conclusion from observations |
| `Heuristic` | 168.0 | Rule of thumb: "in situation X, do Y" |
| `CausalLink` | 240.0 | "X causes Y" -- directional causal relationship |
| `Warning` | 48.0 | Temporary caution signal |
| `StrategyFragment` | 96.0 | Partial plan or tactic -- composable building block |
| `AntiKnowledge` | 336.0 | "This is false/harmful" -- meta-cognitive negation |

### KnowledgeTier (4 tiers -- NOT Ephemeral/Working/Reference/Core)

| Implemented Name | Multiplier | Scoring Weight | Promotion Threshold |
|-----------------|-----------|---------------|-------------------|
| `Transient` | 0.1x | 0.2 | -- (starting tier) |
| `Working` | 0.5x | 0.4 | 3 confirmations |
| `Consolidated` | 1.0x | 0.7 | 10 confirmations |
| `Persistent` | 5.0x | 1.0 | 25 confirmations |

### Scoring Weights (actual, NOT default/equal weighting)

```
score = 0.35 * relevance + 0.20 * recency + 0.25 * importance + 0.20 * emotional_resonance
```

| Factor | Weight | Computation |
|--------|--------|-------------|
| Relevance | 0.35 | `1.0 - (hamming_distance / 10240.0)` |
| Recency | 0.20 | `exp(-age_ticks / RECENCY_DECAY)` where `RECENCY_DECAY = 100.0` |
| Importance | 0.25 | `balance * tier.weight() * ln(confirmations + 1)`, normalized by `IMPORTANCE_NORMALIZER = 7.0` |
| Emotional resonance | 0.20 | PAD Euclidean distance similarity (from `cognitive::affect::pad_similarity`) |

---

## Anti-Knowledge Implementation Details

### ANTI_SUBSPACE

- **Seed:** `0xAE71_5B8C_0000_0001` (consensus-critical, must never change)
- **Generation:** `LazyLock<HdcVector>` initialized via `HdcVector::random(ANTI_SUBSPACE_SEED)`
- **Self-inverse property:** `encode_anti(encode_anti(X)) == X` (because `XOR(XOR(X, A), A) == X`)

### Three-Tier Anti-Knowledge Classification

| Classification | Similarity Range | Action | Constant |
|---------------|-----------------|--------|----------|
| `Strong` | > 0.90 | Reject entirely (not included in results) | `ANTI_STRONG_THRESHOLD` |
| `Moderate` | 0.70 - 0.90 | Include with `confidence_modifier = 0.5`, `contradicted = true` | `ANTI_MODERATE_THRESHOLD` |
| `Weak` | 0.50 - 0.70 | Include with warning (informational only) | `ANTI_WEAK_THRESHOLD` |
| `None` | <= 0.50 | No concern (chance level) | -- |

### AntiCheckedResult struct (in `anti.rs`)

```rust
pub struct AntiCheckedResult {
    pub key: H256,
    pub similarity: f64,
    pub contradicted: bool,         // true only for Moderate
    pub confidence_modifier: f64,   // 1.0 normal, 0.5 moderate
    pub warnings: Vec<String>,
}
```

### AntiClassification enum (in `anti.rs`)

```rust
pub enum AntiClassification {
    Strong,    // > 0.90
    Moderate,  // 0.70 - 0.90
    Weak,      // 0.50 - 0.70
    None,      // <= 0.50
}
```

---

> **Crate:** `kora-hdc` at `crates/hdc/core/src/knowledge/`
>
> **Depends on:** 02 (core algebra -- `HdcVector`, `bind`, `bundle`, `permute`, `hamming_distance`) and 03 (vector search -- `LocalIndex`)
>
> **You do NOT need to read anything else.** This document is self-contained.

---

## 0. Orientation

The knowledge store is the agent's local memory. It holds knowledge entries --
each a 10,240-bit HDC vector plus metadata -- and supports insertion, vector
search with multi-factor scoring, exponential decay (demurrage), tier
promotion/demotion, garbage collection, and anti-knowledge filtering.

All operations in this module are **off-chain**. They run inside a single
agent's local process. `f64` arithmetic is acceptable everywhere.

### File Layout

Create the following files under `crates/hdc/core/src/knowledge/`:

```
knowledge/
  mod.rs           # re-exports
  kind.rs          # KnowledgeKind enum
  tier.rs          # KnowledgeTier enum
  entry.rs         # KnowledgeEntry struct
  source.rs        # KnowledgeSource enum
  store.rs         # KnowledgeStore (HashMap + LocalIndex + all methods)
  decay.rs         # Decay/demurrage math
  anti.rs          # ANTI_SUBSPACE, encode_anti, anti-check pipeline
  scoring.rs       # 4-factor scoring + ScoredEntry
  trust.rs         # 5-stage trust pipeline
```

Add `pub mod knowledge;` to `crates/hdc/core/src/lib.rs`.

### Imports From Sibling Modules

You will use these types from `crate::`:

| Type | Module | What It Is |
|------|--------|------------|
| `HdcVector` | `crate::algebra` | `[u64; 160]`, 10,240-bit vector |
| `bind(&self, &HdcVector) -> HdcVector` | `crate::algebra` | XOR binding |
| `hamming_distance(&self, &HdcVector) -> u32` | `crate::algebra` | Popcount distance |
| `LocalIndex` | `crate::search` | Vector index, provides `insert(H256, HdcVector)`, `search(&HdcVector, usize) -> Vec<(H256, u32)>`, `remove(&H256)`, `get(&H256) -> Option<&HdcVector>` |
| `PadState` | `crate::affect` (from doc 06) | `{ pleasure: f64, arousal: f64, dominance: f64 }` -- PAD emotion coordinates |

External crates: `ethereum_types::H256`, `rand_chacha::ChaCha20Rng`, `rand::Rng`, `rand::SeedableRng`.

---

## 1. KnowledgeKind

File: `kind.rs`

```rust
/// The six kinds of knowledge an agent can store.
/// Each kind has a different base half-life (in hours) that controls how
/// fast it decays via demurrage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KnowledgeKind {
    /// Derived conclusion from observations.
    Insight,
    /// Rule of thumb: "in situation X, do Y."
    Heuristic,
    /// "X causes Y" -- directional causal relationship.
    CausalLink,
    /// Temporary caution signal.
    Warning,
    /// Partial plan or tactic -- composable building block.
    StrategyFragment,
    /// "This is false/harmful" -- meta-cognitive negation.
    AntiKnowledge,
}

impl KnowledgeKind {
    /// Base half-life in hours. This is the half-life at the Consolidated
    /// tier (multiplier 1.0x). Other tiers scale this value.
    pub fn base_half_life_hours(&self) -> f64 {
        match self {
            KnowledgeKind::Insight          => 72.0,
            KnowledgeKind::Heuristic        => 168.0,
            KnowledgeKind::CausalLink       => 240.0,
            KnowledgeKind::Warning          => 48.0,
            KnowledgeKind::StrategyFragment => 96.0,
            KnowledgeKind::AntiKnowledge    => 336.0,
        }
    }
}
```

### Effective Half-Life Table (reference)

| Kind | Transient (0.1x) | Working (0.5x) | Consolidated (1.0x) | Persistent (5.0x) |
|------|-------------------|----------------|---------------------|--------------------|
| Insight | 7.2 h | 36 h | 72 h | 360 h (15 d) |
| Heuristic | 16.8 h | 84 h | 168 h | 840 h (35 d) |
| CausalLink | 24.0 h | 120 h | 240 h | 1,200 h (50 d) |
| Warning | 4.8 h | 24 h | 48 h | 240 h (10 d) |
| StrategyFragment | 9.6 h | 48 h | 96 h | 480 h (20 d) |
| AntiKnowledge | 33.6 h | 168 h | 336 h | 1,680 h (70 d) |

---

## 2. KnowledgeTier

File: `tier.rs`

```rust
/// Retention tier. Higher tiers decay more slowly (larger half-life
/// multiplier) and require more confirmations to reach.
///
/// Ordering: Transient < Working < Consolidated < Persistent.
/// The derived PartialOrd is correct because the variants are declared
/// in ascending order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KnowledgeTier {
    /// Multiplier: 0.1x. Default after creation. Promote at 3 confirmations.
    Transient,
    /// Multiplier: 0.5x. Promote at 10 confirmations.
    Working,
    /// Multiplier: 1.0x. Promote at 25 confirmations.
    Consolidated,
    /// Multiplier: 5.0x. Never deleted; demoted to Consolidated if balance < 0.01.
    Persistent,
}

impl KnowledgeTier {
    /// Half-life multiplier. Effective half-life = base_half_life * multiplier().
    pub fn multiplier(&self) -> f64 {
        match self {
            KnowledgeTier::Transient    => 0.1,
            KnowledgeTier::Working      => 0.5,
            KnowledgeTier::Consolidated => 1.0,
            KnowledgeTier::Persistent   => 5.0,
        }
    }

    /// Scoring weight used in the 4-factor importance calculation.
    /// This is separate from the half-life multiplier.
    pub fn weight(&self) -> f64 {
        match self {
            KnowledgeTier::Transient    => 0.2,
            KnowledgeTier::Working      => 0.4,
            KnowledgeTier::Consolidated => 0.7,
            KnowledgeTier::Persistent   => 1.0,
        }
    }

    /// Attempt promotion based on confirmation count.
    /// Returns `Some(new_tier)` if the confirmation threshold is met,
    /// `None` otherwise.
    pub fn try_promote(&self, confirmations: u32) -> Option<KnowledgeTier> {
        match self {
            KnowledgeTier::Transient    if confirmations >= 3  => Some(KnowledgeTier::Working),
            KnowledgeTier::Working      if confirmations >= 10 => Some(KnowledgeTier::Consolidated),
            KnowledgeTier::Consolidated if confirmations >= 25 => Some(KnowledgeTier::Persistent),
            _ => None,
        }
    }

    /// Demote one level. Persistent -> Consolidated, Working -> Transient, etc.
    /// Transient cannot be demoted further; returns Transient.
    pub fn demote(&self) -> KnowledgeTier {
        match self {
            KnowledgeTier::Persistent   => KnowledgeTier::Consolidated,
            KnowledgeTier::Consolidated => KnowledgeTier::Working,
            KnowledgeTier::Working      => KnowledgeTier::Transient,
            KnowledgeTier::Transient    => KnowledgeTier::Transient,
        }
    }
}
```

### Promotion Thresholds (reference)

```
                     3 confs              10 confs             25 confs
(new) --> TRANSIENT ---------> WORKING ----------> CONSOLIDATED ---------> PERSISTENT
           (0.1x)              (0.5x)                (1.0x)                 (5.0x)
```

---

## 3. KnowledgeSource

File: `source.rs`

```rust
use ethereum_types::H256;

/// Where a piece of knowledge came from. Used for trust weighting.
#[derive(Clone, Debug, PartialEq)]
pub enum KnowledgeSource {
    /// Derived from the agent's own observations or reasoning.
    SelfDerived,
    /// Received from the shared on-chain substrate. Includes the
    /// publisher's agent ID.
    SharedSubstrate { publisher: H256 },
    /// Confirmed by multiple external agents.
    MultiAgentConfirmed { count: u32 },
    /// Received from a single external agent.
    SingleAgent { agent_id: H256 },
    /// Anonymous on-chain submission (lowest trust).
    Anonymous,
}
```

---

## 4. KnowledgeEntry

File: `entry.rs`

```rust
use ethereum_types::H256;
use crate::algebra::HdcVector;
use crate::affect::PadState;
use super::{KnowledgeKind, KnowledgeTier, KnowledgeSource};

/// A single knowledge entry in the local store.
#[derive(Clone, Debug)]
pub struct KnowledgeEntry {
    /// Unique identifier. Computed as `H256::from_slice(&keccak256(vector))` at
    /// creation time.
    pub key: H256,
    /// The 10,240-bit HDC vector encoding this knowledge.
    pub vector: HdcVector,
    /// Which of the six knowledge kinds.
    pub kind: KnowledgeKind,
    /// Current retention tier.
    pub tier: KnowledgeTier,
    /// Human-readable content (the text that was encoded into the vector).
    pub content: String,
    /// Origin of this knowledge.
    pub source: KnowledgeSource,
    /// Number of independent confirmations received.
    pub confirmations: u32,
    /// Tick when this entry was last accessed (queried or confirmed).
    pub last_accessed: u64,
    /// Tick when decay was last applied to this entry.
    pub last_decay_tick: u64,
    /// Current balance in [0.0, 1.0]. Decays exponentially over time.
    /// When this drops below GC_THRESHOLD (0.01), the entry is eligible
    /// for garbage collection.
    pub balance: f64,
    /// PAD emotional state captured at creation time.
    pub emotional_tag: PadState,
    /// True if anti-knowledge resonance was detected against this entry.
    pub contradicted: bool,
}
```

---

## 5. Decay / Demurrage

File: `decay.rs`

This is the core economics of the knowledge store. Every entry's balance decays
exponentially over time. The decay rate is determined by the entry's kind (base
half-life) and tier (multiplier).

### Formula

```
effective_half_life = kind.base_half_life_hours() * tier.multiplier()
lambda_eff          = ln(2) / effective_half_life
balance(t)          = balance(t0) * exp(-lambda_eff * (t - t0))
```

where `t` and `t0` are in **hours**.

### Implementation

```rust
/// Garbage collection threshold. Entries below this balance are removed
/// (or demoted, for Persistent entries).
pub const GC_THRESHOLD: f64 = 0.01;

/// Compute the current balance of an entry after demurrage.
///
/// # Arguments
/// * `balance_at_t0` -- balance at the time of last decay application
/// * `kind` -- knowledge kind (determines base half-life)
/// * `tier` -- retention tier (determines multiplier)
/// * `elapsed_hours` -- hours elapsed since last decay was applied
///
/// # Returns
/// The new balance, clamped to [0.0, 1.0].
pub fn compute_decayed_balance(
    balance_at_t0: f64,
    kind: &KnowledgeKind,
    tier: &KnowledgeTier,
    elapsed_hours: f64,
) -> f64 {
    let effective_hl = kind.base_half_life_hours() * tier.multiplier();
    let lambda = (2.0_f64).ln() / effective_hl;
    let decayed = balance_at_t0 * (-lambda * elapsed_hours).exp();
    decayed.clamp(0.0, 1.0)
}
```

### Worked Example: Insight at Transient Tier

Given:
- kind = Insight, base half-life = 72 h
- tier = Transient, multiplier = 0.1
- effective half-life = 72 * 0.1 = 7.2 h
- lambda_eff = ln(2) / 7.2 = 0.6931 / 7.2 = 0.09627 per hour
- Initial balance = 1.0

After 7.2 hours (1 half-life):
```
balance = 1.0 * exp(-0.09627 * 7.2) = 1.0 * exp(-0.6931) = 0.500
```

After 24 hours:
```
balance = 1.0 * exp(-0.09627 * 24) = 1.0 * exp(-2.3105) = 0.099
```

After 48 hours:
```
balance = 1.0 * exp(-0.09627 * 48) = 1.0 * exp(-4.621) = 0.0098 < 0.01 => GC candidate
```

So a Transient Insight with no reinforcement is garbage-collected in about 48 hours.

### Worked Example: Heuristic at Persistent Tier

Given:
- kind = Heuristic, base half-life = 168 h
- tier = Persistent, multiplier = 5.0
- effective half-life = 168 * 5.0 = 840 h (35 days)
- lambda_eff = ln(2) / 840 = 0.000825 per hour

After 30 days (720 hours):
```
balance = 1.0 * exp(-0.000825 * 720) = exp(-0.594) = 0.552
```

After 231 days (5544 hours):
```
balance = 1.0 * exp(-0.000825 * 5544) = exp(-4.574) = 0.0103
```

A Persistent Heuristic with no reinforcement hits GC threshold around 231 days.
But Persistent entries are never deleted -- they are demoted to Consolidated instead.

### Converting Ticks to Hours

The decay formula uses hours. Ticks are block-based. Use this conversion:

```rust
/// Convert elapsed ticks to hours.
/// `tick_duration_ms` is the block time in milliseconds (e.g., 400 for 400 ms blocks).
pub fn ticks_to_hours(elapsed_ticks: u64, tick_duration_ms: u64) -> f64 {
    (elapsed_ticks as f64 * tick_duration_ms as f64) / 3_600_000.0
}
```

---

## 6. Anti-Knowledge

File: `anti.rs`

Anti-knowledge encodes "this is false/harmful" in a structurally distinct
subspace. It is NOT a metadata flag. It uses algebraic binding with a fixed
subspace vector so that anti-knowledge vectors are quasi-orthogonal to the
knowledge they negate.

### ANTI_SUBSPACE

```rust
use std::sync::LazyLock;
use rand_chacha::ChaCha20Rng;
use rand::{Rng, SeedableRng};
use crate::algebra::HdcVector;

/// Consensus-critical seed. Changing this invalidates all existing
/// anti-knowledge entries. Must never change after genesis.
const ANTI_SUBSPACE_SEED: u64 = 0xAE71_5B8C_0000_0001;

/// A fixed random vector that defines the anti-knowledge subspace.
/// All agents derive the identical vector from the same seed.
///
/// MUST be a LazyLock, not a const -- HdcVector::random() uses ChaCha20Rng
/// which cannot run at compile time.
pub static ANTI_SUBSPACE: LazyLock<HdcVector> = LazyLock::new(|| {
    let mut rng = ChaCha20Rng::seed_from_u64(ANTI_SUBSPACE_SEED);
    let mut v = [0u64; 160];
    for w in &mut v {
        *w = rng.gen();
    }
    HdcVector(v)
});
```

### encode_anti / is_anti

```rust
/// Encode anti-knowledge: bind the knowledge vector with ANTI_SUBSPACE.
///
/// Properties:
///   - anti(X) is quasi-orthogonal to X (similarity ~ 0.5)
///   - anti(anti(X)) = X (self-inverse, because bind is XOR)
///   - anti(X) will NOT be accidentally retrieved by a search for X
pub fn encode_anti(knowledge_vector: &HdcVector) -> HdcVector {
    knowledge_vector.bind(&ANTI_SUBSPACE)
}

/// Resonance threshold for anti-knowledge detection.
/// Similarity > 0.90 = strong contradiction (reject).
/// At D=10,240, similarity of 0.90 is ~81 standard deviations above
/// chance (0.5), so false positives are effectively impossible.
const ANTI_RESONANCE_THRESHOLD: f64 = 0.90;

/// Check whether `candidate` is anti-knowledge for `knowledge_vector`.
///
/// Unbinds the candidate from ANTI_SUBSPACE, then checks similarity
/// to the target knowledge vector.
pub fn is_anti(candidate: &HdcVector, knowledge_vector: &HdcVector) -> bool {
    let unbound = candidate.bind(&ANTI_SUBSPACE);
    let dist = unbound.hamming_distance(knowledge_vector);
    let similarity = 1.0 - (dist as f64 / 10240.0);
    similarity > ANTI_RESONANCE_THRESHOLD
}
```

### Self-Inverse Proof

```
anti(X) = bind(X, ANTI_SUBSPACE)                           [definition]
anti(anti(X)) = bind(bind(X, ANTI_SUBSPACE), ANTI_SUBSPACE) [apply again]
              = X XOR ANTI_SUBSPACE XOR ANTI_SUBSPACE        [bind = XOR]
              = X XOR 0                                      [A XOR A = 0]
              = X                                            [X XOR 0 = X]
```

### Anti-Knowledge Search Pipeline

This is the full pipeline for `search_with_anti_check`. It over-fetches 2x
results, checks each for anti-knowledge contradictions, and filters/flags
based on a three-tier severity scale.

```rust
/// A search result annotated with anti-knowledge metadata.
#[derive(Clone, Debug)]
pub struct AntiCheckedResult {
    pub key: H256,
    /// Normalized similarity to the query: 1.0 - (hamming / 10240.0).
    pub similarity: f64,
    /// True if moderate anti-knowledge resonance (0.7-0.9) was detected.
    pub contradicted: bool,
    /// Multiplicative confidence modifier.
    /// 1.0 = no anti-knowledge. 0.5 = moderate contradiction.
    pub confidence_modifier: f64,
    /// Human-readable warnings.
    pub warnings: Vec<String>,
}
```

Pseudocode for the pipeline:

```
fn search_with_anti_check(index, query, top_k) -> Vec<AntiCheckedResult>:
    // Step 1: over-fetch 2x candidates
    raw_results = index.search(query, top_k * 2)   // returns Vec<(H256, u32)>

    filtered = []
    for (key, hamming_dist) in raw_results:
        sim = 1.0 - (hamming_dist as f64 / 10240.0)

        // Step 2: get the entry's vector
        vector = index.get(&key)

        // Step 3: compute the anti-knowledge probe
        //   If this entry has anti-knowledge in the index, its anti-vector
        //   bind(vector, ANTI_SUBSPACE) will be close to some stored vector.
        anti_probe = vector.bind(&ANTI_SUBSPACE)
        anti_matches = index.search(&anti_probe, 1)  // find closest anti-match

        result = AntiCheckedResult {
            key, similarity: sim,
            contradicted: false,
            confidence_modifier: 1.0,
            warnings: [],
        }

        if anti_matches is not empty:
            (_, anti_dist) = anti_matches[0]
            anti_sim = 1.0 - (anti_dist as f64 / 10240.0)

            if anti_sim > 0.9:
                // STRONG contradiction: REJECT entirely, do not include
                continue

            else if anti_sim > 0.7:
                // MODERATE contradiction: include but halve confidence
                result.contradicted = true
                result.confidence_modifier = 0.5
                result.warnings.push("Partially contradicted by anti-knowledge")

            else if anti_sim > 0.5:
                // WEAK signal: include with warning flag
                result.warnings.push("Weak anti-knowledge signal detected")

            // Below 0.5: chance level, no concern

        filtered.push(result)
        if filtered.len() >= top_k:
            break

    return filtered
```

**Why 2x over-fetch:** Strong contradictions (> 0.9) are removed entirely.
If you fetch exactly `top_k`, rejections leave you with fewer than `top_k`
results. Over-fetching 2x ensures enough candidates survive filtering.

---

## 7. Four-Factor Scoring

File: `scoring.rs`

When searching the knowledge store, raw Hamming similarity is not enough.
Results are ranked by a composite score combining four factors.

### Formula

```
score = 0.35 * relevance + 0.20 * recency + 0.25 * importance + 0.20 * emotional_resonance
```

### Factor Definitions

| Factor | Weight | Computation | Range |
|--------|--------|-------------|-------|
| **Relevance** | 0.35 | `1.0 - (hamming_distance / 10240.0)` | [0.0, 1.0] |
| **Recency** | 0.20 | `exp(-(current_tick - entry.last_accessed) / RECENCY_DECAY)` | [0.0, 1.0] |
| **Importance** | 0.25 | `confidence * tier.weight() * ln(confirmations + 1)`, normalized | [0.0, 1.0] |
| **Emotional resonance** | 0.20 | PAD distance between entry's emotional tag and current mood | [0.0, 1.0] |

### ScoredEntry

```rust
/// A knowledge entry with its computed composite score.
#[derive(Clone, Debug)]
pub struct ScoredEntry {
    pub key: H256,
    pub entry: KnowledgeEntry,
    pub score: f64,
    pub hamming_distance: u32,
}
```

### RetrievalContext

```rust
/// Context passed into search/scoring. Captures the agent's current state.
pub struct RetrievalContext {
    /// Current tick number (block height or logical clock).
    pub current_tick: u64,
    /// Agent's current PAD emotional state.
    pub mood: PadState,
    /// Tick duration in milliseconds (for tick-to-hours conversion).
    pub tick_duration_ms: u64,
}
```

### Constants

```rust
/// Controls the half-life of the recency exponential decay.
/// At RECENCY_DECAY ticks ago, recency score = 1/e = 0.368.
/// At 10 ticks ago, recency score = exp(-10/100) = 0.905.
pub const RECENCY_DECAY: f64 = 100.0;

/// Maximum expected importance value, used for normalization.
/// Corresponds to confidence=1.0, tier_weight=1.0, ln(1025)=6.93.
const IMPORTANCE_NORMALIZER: f64 = 7.0;
```

### compute_score

```rust
pub fn compute_score(
    entry: &KnowledgeEntry,
    hamming: u32,
    ctx: &RetrievalContext,
) -> f64 {
    // Factor 1: Relevance
    let relevance = 1.0 - (hamming as f64 / 10240.0);

    // Factor 2: Recency
    let age = ctx.current_tick.saturating_sub(entry.last_accessed) as f64;
    let recency = (-age / RECENCY_DECAY).exp();

    // Factor 3: Importance (normalized to [0, 1])
    //   confidence is in [0.0, 1.0]
    //   tier.weight() is in [0.2, 1.0]
    //   ln(confirmations + 1) is 0.0 at 0 confs, ~6.93 at 1024 confs
    let raw_importance = entry.balance
        * entry.tier.weight()
        * (entry.confirmations as f64 + 1.0).ln();
    let importance = (raw_importance / IMPORTANCE_NORMALIZER).min(1.0);

    // Factor 4: Emotional resonance
    //   PAD distance: 1.0 when identical, 0.0 when maximally distant.
    //   When no emotional tag is present, default to 0.5 (neutral).
    let emotional = pad_similarity(&entry.emotional_tag, &ctx.mood);

    0.35 * relevance + 0.20 * recency + 0.25 * importance + 0.20 * emotional
}

/// Compute PAD similarity between two emotional states.
/// Returns a value in [0.0, 1.0].
///
/// PAD coordinates are each in [-1.0, 1.0]. Max Euclidean distance
/// between two PAD states = sqrt(2^2 + 2^2 + 2^2) = sqrt(12) = 3.464.
fn pad_similarity(a: &PadState, b: &PadState) -> f64 {
    let dp = a.pleasure - b.pleasure;
    let da = a.arousal - b.arousal;
    let dd = a.dominance - b.dominance;
    let distance = (dp * dp + da * da + dd * dd).sqrt();
    let max_distance = 12.0_f64.sqrt(); // 3.464
    1.0 - (distance / max_distance)
}
```

---

## 8. Trust Pipeline

File: `trust.rs`

When the agent retrieves knowledge from the shared on-chain substrate, raw
similarity is not enough. The trust pipeline applies five sequential
multiplicative discounts before knowledge enters the agent's context window.

This is entirely off-chain. Each agent runs it independently.

### Five Stages

```
trust = 1.0

Stage 1: trust *= source_reputation         // [0.1, 1.0], floor 0.1
Stage 2: trust *= freshness_decay           // (0.0, 1.0]
Stage 3: trust *= confirmation_boost        // [0.5, 1.0]
Stage 4: trust *= stake_weight              // [0.5, 1.0]
Stage 5: trust *= relevance_gate            // [0.0, 1.0]

final_trust = trust.clamp(0.0, 1.0)
```

### Implementation

```rust
use ethereum_types::H256;

/// Cold-start reputation floor for new/unknown agents.
pub const COLD_START_REPUTATION: f64 = 0.1;

/// Minimum trust below which knowledge is not admitted to context.
pub const MIN_TRUST_THRESHOLD: f64 = 0.05;

/// Compute the effective trust score for a shared substrate entry.
///
/// The five stages are multiplicative: a zero in any stage zeros the
/// final trust. By design, stages 1-4 have nonzero floors. Stage 5
/// (relevance) CAN produce 0.0, which correctly excludes irrelevant
/// knowledge.
pub fn compute_trust(
    entry: &KnowledgeEntry,
    source_reputation: f64,     // from reputation registry, [0.0, 1.0]
    current_tick: u64,
    tick_duration_ms: u64,
    query_vector: &HdcVector,
) -> f64 {
    let mut trust = 1.0;

    // ── Stage 1: Author Reputation ──
    // Floor of COLD_START_REPUTATION (0.1) ensures new agents are
    // discoverable but heavily penalized.
    let rep = source_reputation.max(COLD_START_REPUTATION);
    trust *= rep;

    // ── Stage 2: Freshness Decay ──
    // Exponential decay based on age and the entry's effective half-life.
    let age_ticks = current_tick.saturating_sub(entry.last_accessed);
    let age_hours = ticks_to_hours(age_ticks, tick_duration_ms);
    let effective_hl = entry.kind.base_half_life_hours() * entry.tier.multiplier();
    let freshness = (-0.693 * age_hours / effective_hl).exp(); // 0.693 = ln(2)
    trust *= freshness;

    // ── Stage 3: Confirmation Boost ──
    // Diminishing returns: first few confirmations matter most.
    // Floor of 0.5 ensures unconfirmed entries are not zeroed out.
    // Reaches 1.0 at 10 confirmations.
    let conf_boost = (0.5 + 0.05 * entry.confirmations as f64).min(1.0);
    trust *= conf_boost;

    // ── Stage 4: Stake Weighting ──
    // Higher stake = more skin in the game. Normalized against MIN_STAKE.
    // Floor of 0.5. For entries without stake info, default to 0.5.
    let stake_weight = 0.5_f64; // placeholder -- wire to actual stake data
    trust *= stake_weight.max(0.5).min(1.0);

    // ── Stage 5: Relevance Gating ──
    // HDC similarity between the entry's vector and the query.
    // This CAN produce 0.0 for maximally irrelevant knowledge.
    let dist = entry.vector.hamming_distance(query_vector);
    let similarity = 1.0 - (dist as f64 / 10240.0);
    // Rescale from [0.5, 1.0] (HDC range for non-random vectors) to [0.0, 1.0].
    let relevance = ((similarity - 0.5) * 2.0).max(0.0);
    trust *= relevance;

    trust.clamp(0.0, 1.0)
}
```

### Why Multiplicative?

A multiplicative pipeline means a catastrophic failure in any single dimension
(zero reputation, zero relevance) zeros the entire trust. This is intentional:
knowledge from a completely untrusted source should not enter the context
window regardless of how relevant it appears, and perfectly reputable but
totally irrelevant knowledge is equally useless.

---

## 9. KnowledgeStore

File: `store.rs`

This is the central struct that ties everything together.

### Struct

```rust
use std::collections::HashMap;
use ethereum_types::H256;
use crate::algebra::HdcVector;
use crate::search::LocalIndex;
use super::*;

pub struct KnowledgeStore {
    /// All knowledge entries, keyed by H256.
    entries: HashMap<H256, KnowledgeEntry>,
    /// Vector index for similarity search.
    index: LocalIndex,
    /// Block/tick duration in milliseconds, for tick-to-hours conversion.
    tick_duration_ms: u64,
}
```

### Constructor

```rust
impl KnowledgeStore {
    pub fn new(tick_duration_ms: u64) -> Self {
        Self {
            entries: HashMap::new(),
            index: LocalIndex::new(),
            tick_duration_ms,
        }
    }
}
```

### insert

```rust
/// Insert a new knowledge entry.
///
/// - Adds the entry to the HashMap
/// - Indexes the vector in LocalIndex
/// - Does NOT check for duplicates -- the caller must check via
///   `index.search()` with DUPLICATE_THRESHOLD (512 Hamming distance)
///   before calling insert.
pub fn insert(&mut self, entry: KnowledgeEntry) {
    let key = entry.key;
    self.index.insert(key, entry.vector.clone());
    self.entries.insert(key, entry);
}
```

### search

```rust
/// Search for the top-k entries most similar to `query`, ranked by the
/// 4-factor composite score.
///
/// Returns results sorted by score descending.
pub fn search(
    &self,
    query: &HdcVector,
    k: usize,
    ctx: &RetrievalContext,
) -> Vec<ScoredEntry> {
    // Fetch 2x candidates from vector index to have room after filtering.
    let candidates = self.index.search(query, k * 2);

    let mut scored: Vec<ScoredEntry> = candidates
        .iter()
        .filter_map(|(key, hamming)| {
            let entry = self.entries.get(key)?;
            let score = compute_score(entry, *hamming, ctx);
            Some(ScoredEntry {
                key: *key,
                entry: entry.clone(),
                score,
                hamming_distance: *hamming,
            })
        })
        .collect();

    // Sort by score descending.
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}
```

### search_with_anti_check

```rust
/// Search with anti-knowledge filtering.
///
/// This is more expensive (2x searches per candidate) but structurally safe.
/// See the anti-knowledge pipeline pseudocode in section 6 for the full logic.
pub fn search_with_anti_check(
    &self,
    query: &HdcVector,
    k: usize,
    ctx: &RetrievalContext,
) -> Vec<ScoredEntry> {
    let candidates = self.index.search(query, k * 2);
    let mut results: Vec<ScoredEntry> = Vec::new();

    for (key, hamming) in &candidates {
        let entry = match self.entries.get(key) {
            Some(e) => e,
            None => continue,
        };

        // Anti-knowledge probe: bind the entry's vector with ANTI_SUBSPACE
        // and search for matching anti-knowledge in the index.
        let anti_probe = entry.vector.bind(&ANTI_SUBSPACE);
        let anti_matches = self.index.search(&anti_probe, 1);

        let mut contradicted = false;
        let mut confidence_modifier = 1.0_f64;

        if let Some((_, anti_dist)) = anti_matches.first() {
            let anti_sim = 1.0 - (*anti_dist as f64 / 10240.0);

            if anti_sim > 0.9 {
                // STRONG contradiction -- reject entirely
                continue;
            } else if anti_sim > 0.7 {
                // MODERATE contradiction -- halve confidence
                contradicted = true;
                confidence_modifier = 0.5;
            }
            // < 0.7: no action (weak signal or chance level)
        }

        let mut scored_entry = entry.clone();
        scored_entry.contradicted = contradicted;

        let raw_score = compute_score(entry, *hamming, ctx);
        let adjusted_score = raw_score * confidence_modifier;

        results.push(ScoredEntry {
            key: *key,
            entry: scored_entry,
            score: adjusted_score,
            hamming_distance: *hamming,
        });

        if results.len() >= k {
            break;
        }
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results
}
```

### tick

The `tick` method is called once per block. It applies decay, runs GC, and
handles tier promotions and demotions.

```rust
/// Per-block maintenance cycle. Call this once per tick.
///
/// Operations (in order):
///   1. Decay balances for all entries.
///   2. Promote entries that have enough confirmations.
///   3. Demote entries that have been inactive too long.
///   4. Garbage-collect entries below GC_THRESHOLD.
///
/// Returns the number of entries garbage-collected.
pub fn tick(&mut self, current_tick: u64) -> usize {
    let mut gc_keys: Vec<H256> = Vec::new();
    let mut promotions: Vec<(H256, KnowledgeTier)> = Vec::new();
    let mut demotions: Vec<(H256, KnowledgeTier)> = Vec::new();

    for (key, entry) in self.entries.iter_mut() {
        // ── Step 1: Decay ──
        let elapsed_ticks = current_tick.saturating_sub(entry.last_decay_tick);
        if elapsed_ticks > 0 {
            let elapsed_hours = ticks_to_hours(elapsed_ticks, self.tick_duration_ms);
            entry.balance = compute_decayed_balance(
                entry.balance,
                &entry.kind,
                &entry.tier,
                elapsed_hours,
            );
            entry.last_decay_tick = current_tick;
        }

        // ── Step 2: Check promotion ──
        // Promotion takes precedence over demotion (see race condition note below).
        if let Some(new_tier) = entry.tier.try_promote(entry.confirmations) {
            promotions.push((*key, new_tier));
            continue; // skip demotion check for this entry
        }

        // ── Step 3: Check demotion (inactivity-based) ──
        let effective_hl = entry.kind.base_half_life_hours() * entry.tier.multiplier();
        let inactivity_ticks = current_tick.saturating_sub(entry.last_accessed);
        let inactivity_hours = ticks_to_hours(inactivity_ticks, self.tick_duration_ms);

        let demotion_threshold_hours = match entry.tier {
            KnowledgeTier::Working      => effective_hl * 5.0,
            KnowledgeTier::Consolidated => effective_hl * 10.0,
            _ => f64::MAX, // Transient and Persistent are not demoted by inactivity
        };

        if inactivity_hours > demotion_threshold_hours {
            demotions.push((*key, entry.tier.demote()));
        }

        // ── Step 4: Check GC eligibility ──
        if entry.balance < GC_THRESHOLD {
            if entry.tier == KnowledgeTier::Persistent {
                // Persistent entries are NEVER deleted. Demote instead.
                demotions.push((*key, KnowledgeTier::Consolidated));
            } else {
                gc_keys.push(*key);
            }
        }
    }

    // Apply promotions.
    for (key, new_tier) in &promotions {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.tier = *new_tier;
        }
    }

    // Apply demotions.
    for (key, new_tier) in &demotions {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.tier = *new_tier;
        }
    }

    // Garbage-collect.
    let gc_count = gc_keys.len();
    for key in &gc_keys {
        self.entries.remove(key);
        self.index.remove(key);
    }

    gc_count
}
```

### Reinforcement Events

```rust
/// Reinforce an entry: snapshot current decayed balance, apply boost,
/// reset the decay clock, and increment confirmations.
pub fn reinforce(&mut self, key: &H256, current_tick: u64, boost: f64) {
    if let Some(entry) = self.entries.get_mut(key) {
        // Snapshot current decayed balance before boosting.
        let elapsed_ticks = current_tick.saturating_sub(entry.last_decay_tick);
        let elapsed_hours = ticks_to_hours(elapsed_ticks, self.tick_duration_ms);
        entry.balance = compute_decayed_balance(
            entry.balance,
            &entry.kind,
            &entry.tier,
            elapsed_hours,
        );

        // Apply boost (capped at 1.0).
        entry.balance = (entry.balance + boost).min(1.0);

        // Reset clocks.
        entry.last_accessed = current_tick;
        entry.last_decay_tick = current_tick;
        entry.confirmations += 1;
    }
}

/// Record a query hit: multiply balance by 1.1 (capped at 1.0) and
/// update last_accessed.
pub fn record_query_hit(&mut self, key: &H256, current_tick: u64) {
    if let Some(entry) = self.entries.get_mut(key) {
        entry.balance = (entry.balance * 1.1).min(1.0);
        entry.last_accessed = current_tick;
    }
}

/// Record a contradiction: halve the entry's balance and set its
/// contradicted flag.
pub fn record_contradiction(&mut self, key: &H256) {
    if let Some(entry) = self.entries.get_mut(key) {
        entry.balance *= 0.5;
        entry.contradicted = true;
    }
}
```

### Reinforcement Effects (reference table)

| Event | Effect on Balance |
|-------|-------------------|
| Confirmation (another agent validates) | Reset to 1.0, increment confirmation counter |
| Query hit (retrieved and used) | Multiply by 1.1 (capped at 1.0) |
| Tier promotion | Apply new tier multiplier (effective half-life increases) |
| Contradiction detected | Multiply by 0.5 |
| Source discredited | Multiply by 0.3 |

---

## 10. Tier Promotion and Demotion Rules

### Promotion (confirmation-based)

| From | To | Threshold |
|------|----|-----------|
| Transient | Working | 3 confirmations |
| Working | Consolidated | 10 confirmations |
| Consolidated | Persistent | 25 confirmations |

Promotion is **immediate** -- as soon as `confirmations >= threshold`, the entry
is promoted during the next `tick()`. The confirmation counter is monotonically
increasing (never reset).

### Demotion (inactivity-based)

| From | Trigger | To |
|------|---------|----|
| Working | 0 queries in 5x effective half-life | Transient |
| Consolidated | 0 queries in 10x effective half-life | Working |
| Persistent | balance < 0.01 | Consolidated (never deleted) |

Transient entries are not demoted (they are GC'd when balance < 0.01).

### Race Condition: Promotion vs. Demotion at the Same Tick

If an entry receives its Nth confirmation at the exact same tick that its
inactivity timer exceeds the demotion threshold:

> **Promotion takes precedence over demotion.**

Rationale: confirmations are explicit agent actions (active validation).
Inactivity is a passive clock. Active validation overrides passive decay.

In the `tick()` implementation above, this is enforced by the `continue`
statement after detecting a promotion -- the demotion check is skipped entirely
for that entry on that tick.

---

## 11. Anti-Patterns

These are mistakes that are easy to make. Do not make them.

### 1. DO NOT use `f64` for on-chain decay

The `exp()` function is a transcendental floating-point operation. Its results
are **non-deterministic across platforms** (different rounding on x86 vs ARM,
different compiler versions, etc.). Using `f64::exp()` in any consensus path
(precompile, contract execution, validator logic) will cause consensus splits.

**All code in this module is off-chain.** If you ever need decay on-chain, use
the `fixed_point_decay()` function from doc 09, which approximates `exp(-x)` as
`(1 - x/N)^N` using integer arithmetic only.

### 2. DO NOT forget to clamp trust to [0.0, 1.0]

Stage 4 of the trust pipeline (stake weighting) can theoretically produce
values > 1.0. The final `trust.clamp(0.0, 1.0)` is mandatory. Without it,
a high-stake entry could bypass the trust floor of other behavioral states.

### 3. DO NOT use `const` for ANTI_SUBSPACE

`HdcVector::random()` uses `ChaCha20Rng` which cannot run at compile time.
`ANTI_SUBSPACE` must be a `LazyLock<HdcVector>`, not a `const`. Attempting to
use `const` will fail to compile. Attempting to pre-compute the bytes and embed
them as a `const` array is fragile and defeats the purpose of the deterministic
seed.

### 4. DO NOT skip the anti-knowledge over-fetch

`search_with_anti_check` fetches `top_k * 2` candidates because strong
contradictions (> 0.9 anti-similarity) are rejected entirely. If you fetch
exactly `top_k`, rejected entries leave you with fewer than `top_k` results.
Always over-fetch.

### 5. DO NOT apply demotion when promotion fires

If an entry crosses both a promotion threshold and a demotion threshold in the
same tick, apply only the promotion. See the race condition section above.

### 6. DO NOT delete Persistent entries

Persistent knowledge is **never garbage-collected**. When a Persistent entry's
balance drops below GC_THRESHOLD, it is **demoted to Consolidated**, not
deleted. The only way Persistent knowledge leaves the store is if it is demoted
all the way to Transient and then GC'd (which requires sustained inactivity
across multiple demotion cycles).

### 7. DO NOT use `entry.confidence` for importance -- use `entry.balance`

The `balance` field reflects the entry's current decayed value (0.0 to 1.0).
This is what the importance factor in the 4-factor scoring should use. The
`confidence` field (if present on your struct) is a static initial confidence
that does not decay. Use `balance` for the time-varying signal.

---

## 12. Checklist

- [x] Create `crates/hdc/core/src/knowledge/mod.rs` with re-exports
- [x] Implement `KnowledgeKind` in `kind.rs` with `base_half_life_hours()` -- Insight(72), Heuristic(168), CausalLink(240), Warning(48), StrategyFragment(96), AntiKnowledge(336)
- [x] Implement `KnowledgeTier` in `tier.rs` with `multiplier()`, `weight()`, `try_promote()`, `demote()` -- Transient(0.1x), Working(0.5x), Consolidated(1.0x), Persistent(5.0x)
- [x] Implement `KnowledgeSource` in `source.rs` -- SelfDerived, SharedSubstrate, MultiAgentConfirmed, SingleAgent, Anonymous
- [x] Implement `KnowledgeEntry` in `entry.rs` -- includes `emotional_tag: PadState` and `contradicted: bool`
- [x] Implement `compute_decayed_balance()` and `ticks_to_hours()` in `decay.rs` -- GC_THRESHOLD = 0.01
- [x] Implement `ANTI_SUBSPACE` via `LazyLock` in `anti.rs` -- seed `0xAE71_5B8C_0000_0001`
- [x] Implement `encode_anti()` and `is_anti()` in `anti.rs`
- [x] Implement `AntiCheckedResult` and `AntiClassification` in `anti.rs` -- 3-tier classification (Strong > 0.90, Moderate > 0.70, Weak > 0.50)
- [x] Implement `classify_anti_signal()` in `anti.rs`
- [x] Implement `ScoredEntry`, `RetrievalContext`, `compute_score()` in `scoring.rs` -- weights: relevance=0.35, recency=0.20, importance=0.25, emotional=0.20
- [x] `pad_similarity()` implemented in `scoring.rs` (PAD Euclidean distance, delegates to `cognitive::affect::pad_similarity`)
- [x] Implement `compute_trust()` with all 5 stages in `trust.rs` -- COLD_START_REPUTATION=0.1, MIN_TRUST_THRESHOLD=0.05
- [x] Implement `KnowledgeStore` struct in `store.rs` -- backed by `LocalIndex` (not Vec)
- [x] Implement `KnowledgeStore::new()`
- [x] Implement `KnowledgeStore::insert()`
- [x] Implement `KnowledgeStore::search()` with 4-factor scoring
- [x] Implement `KnowledgeStore::search_with_anti_check()` -- uses `classify_anti_signal` for 3-tier classification
- [x] Implement `KnowledgeStore::tick()` with decay, promotion, demotion, and GC -- promotion beats demotion
- [x] Implement `KnowledgeStore::reinforce()`, `record_query_hit()`, `record_contradiction()`
- [x] Add `pub mod knowledge;` to `crates/hdc/core/src/lib.rs`
- [x] All tests pass

### Remaining Work

- [ ] Add `#[derive(Serialize, Deserialize)]` to `KnowledgeEntry`, `KnowledgeKind`, `KnowledgeTier` for persistence
- [ ] Implement actual persistence in `KnowledgeStore::open()` (currently returns empty store)
- [ ] Document thread-safety expectations on `KnowledgeStore` struct
- [ ] Add `record_source_discredited()` method for cascading trust invalidation
- [ ] Add missing edge-case tests: `test_pad_similarity_identical`, `test_pad_similarity_opposite`

---

## 13. Test Plan

### Unit Tests

Place in each file as `#[cfg(test)] mod tests { ... }`.

**kind.rs:**
- `test_half_life_values` -- assert each kind returns the correct base half-life.

**tier.rs:**
- `test_multiplier_values` -- assert each tier returns the correct multiplier.
- `test_weight_values` -- assert each tier returns the correct weight.
- `test_try_promote` -- assert promotion at exact thresholds (3, 10, 25) and no
  promotion below threshold.
- `test_demote` -- assert demotion chain: Persistent -> Consolidated -> Working ->
  Transient -> Transient.
- `test_tier_ordering` -- assert `Transient < Working < Consolidated < Persistent`.

**decay.rs:**
- `test_decay_one_half_life` -- start at balance 1.0, decay for exactly one
  effective half-life, assert balance is approximately 0.5 (within 1e-6).
- `test_decay_gc_threshold` -- verify a Transient Insight reaches GC_THRESHOLD
  (0.01) in approximately 48 hours.
- `test_decay_zero_elapsed` -- zero elapsed hours returns original balance.
- `test_decay_clamp` -- balance never exceeds 1.0 or drops below 0.0.
- `test_ticks_to_hours` -- 9000 ticks at 400 ms/tick = 1.0 hours.

**anti.rs:**
- `test_anti_subspace_deterministic` -- access ANTI_SUBSPACE twice, assert
  identical vectors.
- `test_encode_anti_self_inverse` -- `encode_anti(encode_anti(v)) == v` for a
  random vector.
- `test_anti_orthogonal` -- similarity between `v` and `encode_anti(v)` is
  approximately 0.5 (within 0.02).
- `test_is_anti_true_positive` -- `is_anti(encode_anti(v), v)` returns true.
- `test_is_anti_true_negative` -- `is_anti(random_vector, v)` returns false.

**scoring.rs:**
- `test_score_weights_sum_to_one` -- 0.35 + 0.20 + 0.25 + 0.20 = 1.0.
- `test_score_perfect_entry` -- an entry with max relevance, recency, importance,
  and emotional congruence scores close to 1.0.
- `test_score_relevance_dominates` -- when other factors are equal, closer
  Hamming distance wins.
- `test_pad_similarity_identical` -- `pad_similarity(p, p)` returns 1.0.
- `test_pad_similarity_opposite` -- `pad_similarity((-1,-1,-1), (1,1,1))`
  returns 0.0.

**trust.rs:**
- `test_cold_start_floor` -- source_reputation = 0.0 is floored to
  COLD_START_REPUTATION (0.1).
- `test_trust_zero_relevance` -- maximally irrelevant knowledge produces trust 0.0.
- `test_trust_clamp` -- final trust is always in [0.0, 1.0].

### Integration Tests

Place in `store.rs` tests or a separate `tests/` directory.

- `test_insert_and_search` -- insert 10 entries, search for one, verify it
  appears in results with correct score.
- `test_tick_gc` -- insert a Transient Insight, advance tick far enough for
  balance to drop below 0.01, call `tick()`, verify entry is removed.
- `test_tick_promotion` -- insert an entry at Transient, call `reinforce()`
  3 times, call `tick()`, verify promotion to Working.
- `test_tick_demotion` -- insert a Working entry, advance tick past the 5x
  half-life inactivity window without any queries, call `tick()`, verify
  demotion to Transient.
- `test_persistent_never_deleted` -- insert a Persistent entry, advance tick
  until balance < 0.01, call `tick()`, verify entry is demoted to Consolidated
  (not removed).
- `test_promotion_beats_demotion` -- contrive a scenario where an entry's
  confirmations hit the promotion threshold at the same tick its inactivity
  exceeds the demotion threshold. Verify it is promoted, not demoted.
- `test_search_with_anti_check` -- insert a knowledge entry and its
  anti-knowledge (`encode_anti(vector)`). Search for the original. Verify the
  original is either rejected (if anti-similarity > 0.9) or flagged (if 0.7-0.9).
- `test_anti_check_does_not_reject_unrelated` -- insert a knowledge entry and an
  unrelated anti-knowledge entry. Search for the original. Verify it is returned
  without contradiction flags.
- `test_reinforce_resets_balance` -- insert an entry, advance tick to decay
  balance to 0.3, call `reinforce()` with boost 0.7, verify balance is 1.0.
- `test_record_contradiction_halves_balance` -- insert an entry with balance 0.8,
  call `record_contradiction()`, verify balance is 0.4.

---

## Audit Findings

Audit performed: 2026-05-08. Compared spec (this document, sections 0-13) against
implementation files in `crates/hdc/core/src/knowledge/`.

### F01: `LocalIndex` not used -- brute-force Vec instead

**Spec (section 9, struct):**
```rust
index: LocalIndex,   // crate::search::LocalIndex with HNSW auto-switch
```

**Implementation (`store.rs`, line 15):**
```rust
vectors: Vec<([u8; 32], HdcVector)>,
```

The spec requires `KnowledgeStore` to use the existing `LocalIndex` (defined in
`crates/hdc/core/src/search/local.rs`), which auto-switches from brute-force to
HNSW at 10,000 entries. The implementation instead uses a raw `Vec` of key-vector
pairs with a hand-rolled `index_search` method (`store.rs`, lines 64-72) that
does a linear scan every time.

**Impact:** O(n) search at all sizes. The `LocalIndex` was specifically built for
this use case and provides O(log n) approximate search via HNSW above 10,000
entries. The hand-rolled `Vec`-based search also duplicates data (vectors are
stored in both `entries` HashMap and `vectors` Vec) without the `LocalIndex`'s
deduplication or memory-efficient structure.

**Severity:** High -- performance and architectural regression.

### F02: Key type is `[u8; 32]` instead of `H256`

**Spec (section 4, entry.rs):**
```rust
pub key: H256,
```

**Implementation (`entry.rs`, line 10):**
```rust
pub key: [u8; 32],
```

All spec references use `ethereum_types::H256`. The implementation uses raw
`[u8; 32]` arrays everywhere: `KnowledgeEntry.key`, `KnowledgeStore.entries`
HashMap, `ScoredEntry.key`, and all method signatures. The `H256` type alias
`type H256 = [u8; 32]` already exists in `crates/hdc/core/src/search/mod.rs`
(line 10), so the types are structurally compatible, but the implementation does
not import or use it.

**Impact:** Medium -- breaks interop with `LocalIndex` (which accepts `H256`
keys), loses semantic clarity, and prevents using `H256` methods. Also means the
entry uses `crate::vector_id()` instead of
`H256::from_slice(&keccak256(vector))` as specified.

### F03: `KnowledgeEntry` missing `emotional_tag: PadState` field

**Spec (section 4, entry.rs):**
```rust
pub emotional_tag: PadState,
```

**Implementation (`entry.rs`):** Field is absent. `PadState` is never imported.

The spec requires each entry to carry a `PadState` (Pleasure-Arousal-Dominance)
emotional tag captured at creation time. `PadState` exists in
`crates/hdc/core/src/cognitive/affect.rs` (line 16). The implementation omits
this field entirely.

**Impact:** High -- cascading failure. Without `emotional_tag`, the 4th scoring
factor (emotional resonance) cannot be computed, which changes the scoring
formula (see F04).

### F04: 4-factor scoring formula has wrong weights and missing factor

**Spec (section 7, compute_score):**
```
score = 0.35 * relevance + 0.20 * recency + 0.25 * importance + 0.20 * emotional_resonance
```

**Implementation (`scoring.rs`, line 58):**
```rust
0.35 * relevance + 0.25 * recency + 0.25 * importance + 0.15 * trust
```

Two deviations:
1. **Emotional resonance (0.20) replaced by "trust" (0.15):** The spec's 4th
   factor is PAD emotional resonance computed via `pad_similarity()`. The
   implementation substitutes `entry.balance` as a "trust proxy" (line 56),
   which is just the decay balance -- a completely different signal. The
   `pad_similarity()` function is missing from this file entirely (it only
   exists in `crates/hdc/core/src/cognitive/affect.rs`).
2. **Recency weight changed from 0.20 to 0.25:** The sum still equals 1.0
   (0.35 + 0.25 + 0.25 + 0.15 = 1.0) but the distribution differs from spec.

**Impact:** High -- scoring formula is fundamentally different from spec. The
`RetrievalContext` struct is also missing the `mood: PadState` field
(`scoring.rs`, line 14-18 vs spec section 7).

### F05: `AntiCheckedResult` struct not implemented

**Spec (section 6):**
```rust
pub struct AntiCheckedResult {
    pub key: H256,
    pub similarity: f64,
    pub contradicted: bool,
    pub confidence_modifier: f64,
    pub warnings: Vec<String>,
}
```

**Implementation:** Does not exist anywhere in the codebase (confirmed via grep).
The `search_with_anti_check` method in `store.rs` (lines 104-159) returns
`Vec<ScoredEntry>` instead of `Vec<AntiCheckedResult>`. While `ScoredEntry`
carries some of the same data, it lacks:
- `similarity` (normalized, separate from raw `hamming_distance`)
- `confidence_modifier` (consumed internally but not exposed)
- `warnings: Vec<String>` (not tracked at all)

**Impact:** Medium -- API surface is less informative than specified. Callers
cannot inspect why confidence was modified or see warning messages.

### F06: Weak anti-knowledge signal (0.5-0.7) not handled

**Spec (section 6, anti-check pipeline pseudocode):**
```
else if anti_sim > 0.5:
    // WEAK signal: include with warning flag
    result.warnings.push("Weak anti-knowledge signal detected")
```

**Implementation (`store.rs`, lines 126-137):** Only handles > 0.9 (reject) and
> 0.7 (halve confidence). The 0.5-0.7 range (weak signal) is silently ignored.

**Impact:** Low-medium -- spec explicitly defines a three-tier severity scale.
The implementation only implements two tiers.

### F07: `source_discredited` reinforcement event not implemented

**Spec (section 9, Reinforcement Effects table):**
```
| Source discredited | Multiply by 0.3 |
```

**Implementation:** No `record_source_discredited` or equivalent method exists in
`store.rs`. Confirmed via grep across entire `crates/hdc/` tree.

**Impact:** Medium -- missing API for a specified reinforcement event.

### F08: `KnowledgeStore::open()` is a no-op stub

**Implementation (`store.rs`, lines 33-35):**
```rust
pub fn open(_path: &std::path::Path, tick_duration_ms: u64) -> Result<Self, std::io::Error> {
    Ok(Self::new(tick_duration_ms))
}
```

This method ignores the path argument and returns an empty store. The spec does
not define an `open()` method at all. This is a vestigial stub that implies
persistence support but delivers none.

**Impact:** Low -- the method exists but is documented as a stub. However, it
could mislead callers who expect persistence.

### F09: `max_entries` field unused

**Implementation (`store.rs`, lines 19, 28):**
```rust
max_entries: usize,
// ...
max_entries: 10_000,
```

The `max_entries` field is set in the constructor but never read or enforced.
`insert()` does not check capacity. The spec does not mention a capacity limit.

**Impact:** Low -- dead field. Either enforce it or remove it.

### F10: Vector data duplication -- entries + vectors Vec

**Implementation (`store.rs`, lines 13-16):**
```rust
entries: HashMap<[u8; 32], KnowledgeEntry>,  // has .vector
vectors: Vec<([u8; 32], HdcVector)>,          // duplicates .vector
```

Each `HdcVector` is 160 * 8 = 1,280 bytes. Every entry stores it twice: once in
the `HashMap` value (`KnowledgeEntry.vector`) and once in the `vectors` Vec.
At 10,000 entries this wastes ~12.5 MB.

The spec avoids this by using `LocalIndex`, which stores vectors in its own
structure and the `HashMap` only holds metadata entries.

**Impact:** Medium -- doubles memory for vectors. Direct consequence of F01.

### F11: GC does not remove from `vectors` Vec efficiently

**Implementation (`store.rs`, line 239):**
```rust
self.vectors.retain(|(k, _)| k != key);
```

Each GC'd key triggers a full linear scan of the `vectors` Vec via `retain()`.
If N entries are GC'd in one tick, this is O(N * M) where M is total vector
count. The same O(n) `retain` pattern is used in `remove()` (line 52).

The `LocalIndex.delete()` method provides O(1) removal by key.

**Impact:** Medium -- O(n^2) worst-case GC performance per tick.

### F12: Thread safety -- entirely absent

The `KnowledgeStore` uses `&mut self` for all mutation and `&self` for reads. It
provides no interior mutability or synchronization primitives. The spec states
"all operations run inside a single agent's local process" which makes `&mut self`
acceptable, but there is no `Send`/`Sync` consideration documented.

If the store is ever accessed from multiple async tasks (e.g., search while
ticking), an `Arc<RwLock<KnowledgeStore>>` wrapper would be needed. The current
struct is inherently single-threaded, which is correct for the spec but should be
documented.

**Impact:** Low (acceptable per spec) -- but worth noting for future async usage.

### F13: `KnowledgeSource` uses `[u8; 32]` not `H256`

**Spec (`source.rs`):**
```rust
SharedSubstrate { publisher: H256 },
SingleAgent { agent_id: H256 },
```

**Implementation (`source.rs`, lines 9, 12):**
```rust
SharedSubstrate { publisher: [u8; 32] },
SingleAgent { agent_id: [u8; 32] },
```

Same `[u8; 32]` vs `H256` issue as F02 but in the source enum.

**Impact:** Low -- structurally identical since `H256 = [u8; 32]` but loses
semantic type.

### F14: Missing unit tests from spec test plan

The following tests from the spec (section 13) are not implemented:
- `test_pad_similarity_identical` (scoring.rs) -- because `pad_similarity` does
  not exist
- `test_pad_similarity_opposite` (scoring.rs) -- same reason
- No test for the `source_discredited` reinforcement event (because the method
  does not exist)

All other specified tests are present and structurally match the spec.

---

## Implementation Status

| Spec Item | File | Status | Notes |
|-----------|------|--------|-------|
| `KnowledgeKind` enum | `kind.rs` | COMPLETE | Exact match to spec |
| `base_half_life_hours()` | `kind.rs` | COMPLETE | All 6 values correct |
| `KnowledgeTier` enum | `tier.rs` | COMPLETE | Exact match |
| `multiplier()` | `tier.rs` | COMPLETE | All 4 values correct |
| `weight()` | `tier.rs` | COMPLETE | All 4 values correct |
| `try_promote()` | `tier.rs` | COMPLETE | Thresholds 3/10/25 correct |
| `demote()` | `tier.rs` | COMPLETE | Exact match |
| `KnowledgeSource` enum | `source.rs` | PARTIAL | Uses `[u8; 32]` not `H256` (F13) |
| `KnowledgeEntry` struct | `entry.rs` | PARTIAL | Missing `emotional_tag: PadState` (F03) |
| `KnowledgeEntry::new()` | `entry.rs` | PARTIAL | No `emotional_tag` param; uses `vector_id()` not `H256` keccak |
| `compute_decayed_balance()` | `decay.rs` | COMPLETE | Exact match to spec formula |
| `ticks_to_hours()` | `decay.rs` | COMPLETE | Exact match |
| `GC_THRESHOLD` | `decay.rs` | COMPLETE | 0.01, correct |
| `ANTI_SUBSPACE` | `anti.rs` | PARTIAL | Seed correct but uses `HdcVector::random()` not explicit ChaCha20Rng construction (see below) |
| `encode_anti()` | `anti.rs` | COMPLETE | Uses free fn `bind()` instead of method syntax, functionally identical |
| `is_anti()` | `anti.rs` | COMPLETE | Uses `D` constant instead of literal `10240.0`, correct |
| `ANTI_RESONANCE_THRESHOLD` | `anti.rs` | COMPLETE | 0.90 |
| `AntiCheckedResult` struct | -- | MISSING | Not implemented (F05) |
| `ScoredEntry` struct | `scoring.rs` | PARTIAL | Uses `[u8; 32]` not `H256` |
| `RetrievalContext` struct | `scoring.rs` | PARTIAL | Missing `mood: PadState` field (F04) |
| `compute_score()` | `scoring.rs` | DEVIATED | Wrong weights, missing emotional resonance factor (F04) |
| `pad_similarity()` | -- | MISSING | Not in scoring.rs (exists in affect.rs as different impl) |
| `compute_trust()` | `trust.rs` | COMPLETE | All 5 stages match spec |
| `COLD_START_REPUTATION` | `trust.rs` | COMPLETE | 0.1 |
| `MIN_TRUST_THRESHOLD` | `trust.rs` | COMPLETE | 0.05 |
| `KnowledgeStore` struct | `store.rs` | DEVIATED | Uses `Vec` not `LocalIndex` (F01); extra `max_entries` field (F09) |
| `KnowledgeStore::new()` | `store.rs` | DEVIATED | Different field initialization due to F01 |
| `KnowledgeStore::insert()` | `store.rs` | PARTIAL | Pushes to Vec instead of `LocalIndex` |
| `KnowledgeStore::search()` | `store.rs` | PARTIAL | Uses hand-rolled brute-force instead of `LocalIndex` |
| `search_with_anti_check()` | `store.rs` | PARTIAL | Missing weak signal tier (F06), missing `AntiCheckedResult` (F05) |
| `KnowledgeStore::tick()` | `store.rs` | PARTIAL | Logic correct but uses `Vec::retain` for GC (F11) |
| `reinforce()` | `store.rs` | COMPLETE | Exact match |
| `record_query_hit()` | `store.rs` | COMPLETE | Exact match |
| `record_contradiction()` | `store.rs` | COMPLETE | Exact match |
| `record_source_discredited()` | -- | MISSING | Not implemented (F07) |
| `open()` stub | `store.rs` | EXTRA | Not in spec, no-op stub (F08) |
| Unit tests (kind.rs) | `kind.rs` | COMPLETE | |
| Unit tests (tier.rs) | `tier.rs` | COMPLETE | |
| Unit tests (decay.rs) | `decay.rs` | COMPLETE | |
| Unit tests (anti.rs) | `anti.rs` | COMPLETE | |
| Unit tests (scoring.rs) | `scoring.rs` | PARTIAL | Missing pad_similarity tests |
| Unit tests (trust.rs) | `trust.rs` | COMPLETE | |
| Integration tests | `store.rs` + `tests/knowledge.rs` | MOSTLY COMPLETE | All core scenarios covered |
| `pub mod knowledge` in lib.rs | `lib.rs` | COMPLETE | |

---

## Anti-Patterns & Duct Tape

### AP01: Hand-rolled brute-force search (duct tape)

**File:** `crates/hdc/core/src/knowledge/store.rs`, lines 64-72

```rust
fn index_search(&self, query: &HdcVector, k: usize) -> Vec<([u8; 32], u32)> {
    let mut scored: Vec<([u8; 32], u32)> = self.vectors
        .iter()
        .map(|(key, vec)| (*key, hamming_distance(query, vec)))
        .collect();
    scored.sort_by_key(|(_, d)| *d);
    scored.truncate(k);
    scored
}
```

This reimplements the exact functionality of `BruteForceIndex::search()` from
`crates/hdc/core/src/search/brute.rs`, but without any of `LocalIndex`'s
benefits (auto-switch to HNSW, `SearchIndex` trait compliance, `H256` key
typing). Classic duct-tape: avoid importing the real dependency by inlining a
worse version.

### AP02: Dual data storage (architectural anti-pattern)

**File:** `crates/hdc/core/src/knowledge/store.rs`, lines 13-16

Storing `HdcVector` in both `entries` HashMap and `vectors` Vec creates a
consistency invariant that must be manually maintained. The `remove()` method
(line 50-53) and `tick()` GC (line 237-239) both use `Vec::retain()` to keep
them in sync, but there is no encapsulation enforcing this. A bug in any mutation
path that forgets to update `vectors` would create ghost entries or orphaned
vectors.

### AP03: Scoring factor substitution (deviation, not duct tape)

**File:** `crates/hdc/core/src/knowledge/scoring.rs`, lines 55-56

```rust
// Factor 4: Trust (use balance as proxy for source trust)
let trust = entry.balance;
```

Using `balance` as a proxy for emotional resonance is a design substitution, not
a temporary workaround. The comment says "trust" but the spec says "emotional
resonance." This is neither -- it reuses the decay balance as a signal that
conflates temporal decay with source trustworthiness. The real trust pipeline
exists in `trust.rs` and is unrelated.

### AP04: Stub persistence method (dead code)

**File:** `crates/hdc/core/src/knowledge/store.rs`, lines 33-35

```rust
pub fn open(_path: &std::path::Path, tick_duration_ms: u64) -> Result<Self, std::io::Error> {
    Ok(Self::new(tick_duration_ms))
}
```

A method that takes a path argument and ignores it, returning an empty store, is
misleading API surface. Either implement persistence or remove the method and
let callers use `new()`.

### AP05: Unused capacity field (dead code)

**File:** `crates/hdc/core/src/knowledge/store.rs`, line 19

```rust
max_entries: usize,
```

Set to 10,000 but never checked on `insert()`. Dead field with no enforcement.

### AP06: `ANTI_SUBSPACE` generation differs from spec

**File:** `crates/hdc/core/src/knowledge/anti.rs`, lines 13-15

```rust
pub static ANTI_SUBSPACE: LazyLock<HdcVector> = LazyLock::new(|| {
    HdcVector::random(ANTI_SUBSPACE_SEED)
});
```

The spec explicitly constructs a `ChaCha20Rng` from the seed and fills the
vector word-by-word. The implementation delegates to `HdcVector::random()` which
may or may not use the same RNG construction internally. If `HdcVector::random()`
changes its RNG strategy, this silently breaks consensus on the anti-knowledge
subspace. The spec's explicit construction is more robust.

---

## Recommended Changes Checklist

### Critical (must fix -- breaks spec compliance)

- [ ] **Replace `Vec<([u8; 32], HdcVector)>` with `LocalIndex`** in
  `KnowledgeStore` (`store.rs`). Remove the `vectors` field and `index_search()`
  method. Use `self.index.search()` and `self.index.insert()` instead.
  Delete the hand-rolled brute-force.
  *Files:* `crates/hdc/core/src/knowledge/store.rs` lines 11-20, 38-72

- [ ] **Add `emotional_tag: PadState` to `KnowledgeEntry`** (`entry.rs`).
  Import `PadState` from `crate::cognitive::affect`. Add it to the `new()`
  constructor with a `PadState::neutral()` default.
  *File:* `crates/hdc/core/src/knowledge/entry.rs`

- [ ] **Fix scoring formula in `compute_score()`** to match spec weights
  (0.35/0.20/0.25/0.20). Replace the "trust" factor with `pad_similarity()`
  emotional resonance. Add `mood: PadState` to `RetrievalContext`.
  *File:* `crates/hdc/core/src/knowledge/scoring.rs` lines 14-18, 37-58

- [ ] **Implement `pad_similarity()` in `scoring.rs`** (PAD Euclidean distance
  normalized to [0.0, 1.0]). Or re-export from `crate::cognitive::affect` if
  it already exists there.
  *File:* `crates/hdc/core/src/knowledge/scoring.rs`

### High (should fix -- missing spec features)

- [ ] **Implement `AntiCheckedResult` struct** in `anti.rs` with fields: `key`,
  `similarity`, `contradicted`, `confidence_modifier`, `warnings`.
  *File:* `crates/hdc/core/src/knowledge/anti.rs`

- [ ] **Add weak anti-knowledge signal handling (0.5-0.7 range)** in
  `search_with_anti_check()`. Push warning string when anti_sim is in [0.5, 0.7).
  *File:* `crates/hdc/core/src/knowledge/store.rs` lines 126-137

- [ ] **Implement `record_source_discredited()`** method on `KnowledgeStore`:
  multiply balance by 0.3 for entries from a given source.
  *File:* `crates/hdc/core/src/knowledge/store.rs`

- [ ] **Switch key types from `[u8; 32]` to `H256`** across all knowledge files.
  Import `H256` from `crate::search` (where `type H256 = [u8; 32]` is defined).
  *Files:* `entry.rs`, `store.rs`, `scoring.rs`, `source.rs`

### Medium (should fix -- code quality)

- [ ] **Make `ANTI_SUBSPACE` generation explicit** -- use `ChaCha20Rng::seed_from_u64`
  and fill word-by-word as specified, rather than delegating to
  `HdcVector::random()`.
  *File:* `crates/hdc/core/src/knowledge/anti.rs` lines 13-15

- [ ] **Remove `max_entries` field** or enforce it in `insert()` with capacity
  check and error/eviction.
  *File:* `crates/hdc/core/src/knowledge/store.rs` lines 19, 28

- [ ] **Remove `open()` stub** or implement actual persistence. A no-op method
  with a path parameter is misleading.
  *File:* `crates/hdc/core/src/knowledge/store.rs` lines 33-35

- [ ] **Add missing unit tests**: `test_pad_similarity_identical`,
  `test_pad_similarity_opposite`.
  *File:* `crates/hdc/core/src/knowledge/scoring.rs`

### Low (nice to have)

- [ ] **Document thread-safety expectations** on `KnowledgeStore` struct.
  Add a doc comment noting it is `!Sync` by design (single-agent local process)
  and that `Arc<RwLock<_>>` is needed for async access.
  *File:* `crates/hdc/core/src/knowledge/store.rs` line 11

- [ ] **Add `#[derive(Serialize, Deserialize)]`** to `KnowledgeEntry`,
  `KnowledgeKind`, `KnowledgeTier`, `KnowledgeSource` for future persistence.
  *Files:* `entry.rs`, `kind.rs`, `tier.rs`, `source.rs`

---

## Second-Pass Remediation Detail

Second pass performed: 2026-05-08. This section narrows the first-pass findings
into concrete implementation steps against the current code shape in
`crates/hdc/core/src/knowledge/` and `crates/hdc/core/src/search/`.

### P0: Integrate `LocalIndex` as the only vector search path

**Files/functions:**
- `crates/hdc/core/src/knowledge/store.rs`
  - `KnowledgeStore` struct
  - `KnowledgeStore::new`
  - `KnowledgeStore::insert`
  - `KnowledgeStore::remove`
  - `KnowledgeStore::search`
  - `KnowledgeStore::search_with_anti_check`
  - `KnowledgeStore::tick`
- `crates/hdc/core/src/search/local.rs`
  - `LocalIndex::new`
  - `SearchIndex for LocalIndex::{insert, delete, search, len}`

**Concrete fix:**
1. Import the real search API:
   ```rust
   use crate::search::{H256, HdcIndexError, LocalIndex, SearchIndex};
   ```
2. Replace the current raw vector field:
   ```rust
   vectors: Vec<([u8; 32], HdcVector)>,
   ```
   with:
   ```rust
   index: LocalIndex,
   ```
3. Initialize `index: LocalIndex::new()` in `KnowledgeStore::new`.
4. Delete `KnowledgeStore::index_search`; all candidate retrieval must go
   through `self.index.search(query, requested_k)`.
5. Use the actual `SearchIndex` trait contract, not the original doc shortcut:
   `search()` returns `Result<Vec<(H256, u32)>, HdcIndexError>`, and deletion is
   `delete(&H256)`, not `remove(&H256)`.
6. Make `KnowledgeStore::insert` return `Result<(), HdcIndexError>` or a
   knowledge-specific error wrapper. Insert into `LocalIndex` first, then insert
   into `entries` only after the index succeeds. This prevents a duplicate-key
   index error from leaving `entries` and `index` inconsistent.
7. In `KnowledgeStore::search` and `search_with_anti_check`, treat
   `HdcIndexError::EmptyIndex` as an empty result. Propagate or wrap other index
   errors if the public API is made fallible.
8. In `KnowledgeStore::remove` and GC inside `KnowledgeStore::tick`, call
   `self.index.delete(key)` exactly once for every removed entry.

### P0: Remove ad-hoc dual storage and ghost-vector risks

**Files/functions:**
- `crates/hdc/core/src/knowledge/store.rs`
  - `KnowledgeStore` fields
  - `KnowledgeStore::remove`
  - `KnowledgeStore::tick`

**Concrete fix:**
- Remove `vectors` from `KnowledgeStore`; do not keep a second raw
  `Vec<(_, HdcVector)>` beside `LocalIndex`.
- Remove every `self.vectors.push(...)` and `self.vectors.retain(...)` call.
- After the change, the only mutation paths that touch indexed vectors should be
  `self.index.insert(...)` and `self.index.delete(...)`.
- Keep `KnowledgeEntry.vector` only if `search_with_anti_check` continues to
  need the candidate vector for `encode_anti`/`bind`. If the goal is true
  single-owner vector storage, split `KnowledgeEntry` into metadata plus vector
  storage and add a real `SearchIndex::get(&H256) -> Option<&HdcVector>` API to
  the search module before removing `KnowledgeEntry.vector`.
- Add an invariant test that removes or GC's an entry and then verifies the same
  key is absent from both `entries` and subsequent `LocalIndex` search results.

### P0: Restore PAD emotional scoring weights

**Files/functions:**
- `crates/hdc/core/src/knowledge/scoring.rs`
  - `RetrievalContext`
  - `compute_score`
  - tests in `mod tests`
- `crates/hdc/core/src/knowledge/entry.rs`
  - `KnowledgeEntry`
  - `KnowledgeEntry::new`
- `crates/hdc/core/src/cognitive/affect.rs`
  - `PadState`
  - `pad_similarity`

**Concrete fix:**
1. Add the current mood to retrieval context:
   ```rust
   pub struct RetrievalContext {
       pub current_tick: u64,
       pub tick_duration_ms: u64,
       pub mood: PadState,
   }
   ```
2. Add an emotional tag to each stored entry:
   ```rust
   pub emotional_tag: PadState,
   ```
   Import it from `crate::cognitive::affect::PadState` or the re-export
   `crate::cognitive::PadState`.
3. Update `KnowledgeEntry::new` to either accept `emotional_tag: PadState` or
   provide a second constructor such as `new_with_emotion(...)`. If backward
   compatibility matters, keep `new(...)` and set `PadState::neutral()`.
4. Replace the current scoring formula:
   ```rust
   0.35 * relevance + 0.25 * recency + 0.25 * importance + 0.15 * trust
   ```
   with:
   ```rust
   let emotional = pad_similarity(&entry.emotional_tag, &ctx.mood);
   0.35 * relevance + 0.20 * recency + 0.25 * importance + 0.20 * emotional
   ```
5. Do not use `entry.balance` as a "trust" proxy in `compute_score`; balance
   already participates in the `importance` term.
6. Decide explicitly whether `scoring.rs` should re-use
   `crate::cognitive::affect::pad_similarity` (currently cosine-based, neutral
   returns 0.5) or implement the spec's Euclidean PAD-distance version locally.
   Do not silently mix both definitions.

### P1: Define the trust pipeline boundary

**Files/functions:**
- `crates/hdc/core/src/knowledge/trust.rs`
  - `compute_trust`
  - `MIN_TRUST_THRESHOLD`
- `crates/hdc/core/src/knowledge/scoring.rs`
  - `compute_score`
- `crates/hdc/core/src/knowledge/store.rs`
  - `KnowledgeStore::insert`
  - future shared-substrate ingestion method

**Concrete fix:**
- Keep `compute_score` as local retrieval ranking only: relevance, recency,
  importance, emotional resonance.
- Keep `compute_trust` as an admission/context-boundary check for shared
  substrate knowledge, where caller-supplied reputation, stake, freshness, and
  query relevance are available.
- Apply `MIN_TRUST_THRESHOLD` before inserting shared entries into
  `KnowledgeStore` or before merging external candidates into a query context.
  Do not hide this inside `compute_score`, because local retrieval does not have
  source reputation or stake inputs.
- Add a source-specific API boundary, for example:
  `KnowledgeStore::insert_shared(entry, source_reputation, current_tick, query)`
  or a separate substrate ingestion module that calls `trust::compute_trust`
  before `KnowledgeStore::insert`.
- Add a regression test proving that changing `KnowledgeSource` alone does not
  change `compute_score`; trust changes must happen in the trust pipeline.

### P1: Replace the persistence stub with a real contract

**Files/functions:**
- `crates/hdc/core/src/knowledge/store.rs`
  - `KnowledgeStore::open`
  - proposed `KnowledgeStore::flush` or `KnowledgeStore::snapshot`
- `crates/hdc/core/src/knowledge/{entry,kind,tier,source}.rs`
  - serde derives if file-backed persistence is kept

**Concrete fix:**
- Do not leave `open(path, tick_duration_ms)` returning `Ok(Self::new(...))`.
  Pick one of these two contracts:
  1. Remove `open` entirely until persistence is implemented.
  2. Implement file-backed persistence and rebuild `LocalIndex` from persisted
     entries on open.
- If implementing persistence, persist `entries`, `tick_duration_ms`, and schema
  version. Do not persist `LocalIndex` internals; rebuild it by iterating over
  entries and calling `index.insert(entry.key, entry.vector.clone())`.
- Add `Serialize`/`Deserialize` derives to `KnowledgeEntry`, `KnowledgeKind`,
  `KnowledgeTier`, `KnowledgeSource`, and `PadState` if these types are stored.
- Make failed index rebuild during `open` a hard error; otherwise the store can
  open with entries that are not searchable.
- Add tests for round-trip persistence: insert entries, persist, reopen, assert
  `len`, `get`, and `search` behavior match.

### P2: Resolve `max_entries`

**Files/functions:**
- `crates/hdc/core/src/knowledge/store.rs`
  - `KnowledgeStore` struct
  - `KnowledgeStore::new`
  - `KnowledgeStore::insert`

**Concrete fix:**
- Prefer removing `max_entries`. The current value `10_000` is especially
  confusing because `LocalIndex` switches to HNSW after 10,000 entries; it is
  not a capacity limit.
- If product requirements need a cap, make it explicit:
  - add `KnowledgeStore::with_max_entries(tick_duration_ms, max_entries)`;
  - check `self.entries.len() >= self.max_entries` before index insertion;
  - return `KnowledgeStoreError::CapacityExceeded { capacity }`;
  - add tests for exact-boundary insertion and no partial index mutation.
- Do not keep a private field that is initialized but never read.

### P2: Test coverage to add after remediation

**Files/functions:**
- `crates/hdc/core/src/knowledge/store.rs` tests
- `crates/hdc/core/src/knowledge/scoring.rs` tests
- `crates/hdc/core/src/knowledge/trust.rs` tests
- optional integration tests under `crates/hdc/core/tests/`

**Required tests:**
- `store_uses_local_index_for_insert_search`: insert entries through
  `KnowledgeStore::insert`, search through `KnowledgeStore::search`, and assert
  results come back through the `SearchIndex` path. This should fail if
  `index_search` or `vectors` returns.
- `store_delete_removes_from_index`: remove an entry, then search for its exact
  vector and assert the deleted key is absent.
- `store_gc_deletes_from_index`: force GC in `KnowledgeStore::tick` and assert
  the GC'd key is absent from both `entries` and search results.
- `insert_duplicate_does_not_mutate_entries`: duplicate `LocalIndex::insert`
  failure must not leave an extra or replaced `entries` value.
- `score_weights_match_spec`: assert `0.35 + 0.20 + 0.25 + 0.20 == 1.0` and
  guard against the old `0.25` recency / `0.15` trust weights.
- `score_uses_emotional_resonance`: same entry/query with different
  `RetrievalContext::mood` changes the score through PAD similarity.
- `score_does_not_use_trust_proxy`: changing `entry.source` without changing
  PAD state, balance, confirmations, or distance does not change `compute_score`.
- `shared_ingress_applies_min_trust`: shared substrate entries below
  `MIN_TRUST_THRESHOLD` are rejected before insertion or context merge.
- `open_round_trip_rebuilds_index`: persisted entries are searchable after
  `KnowledgeStore::open`.
- `max_entries_boundary`: only needed if `max_entries` is retained and enforced.

### Prioritized Checklist

- [ ] **P0:** In `store.rs`, replace `vectors` with `index: LocalIndex`, import
  `SearchIndex`, delete `index_search`, and route all candidate lookup through
  `LocalIndex::search`.
- [ ] **P0:** Make `KnowledgeStore::insert` fallible or otherwise handle
  `HdcIndexError` without mutating `entries` after index failure.
- [ ] **P0:** In `store.rs`, update `remove` and GC inside `tick` to call
  `LocalIndex::delete`; remove all `Vec::retain` vector-maintenance code.
- [ ] **P0:** In `entry.rs`, add `emotional_tag: PadState` and update
  constructors/tests.
- [ ] **P0:** In `scoring.rs`, add `RetrievalContext::mood`, use PAD similarity,
  and restore weights to `0.35/0.20/0.25/0.20`.
- [ ] **P1:** Keep `trust.rs::compute_trust` outside `compute_score`; enforce
  `MIN_TRUST_THRESHOLD` at shared-substrate ingestion or context merge.
- [ ] **P1:** Remove the `KnowledgeStore::open` stub or implement persistence
  with index rebuild from persisted entries.
- [ ] **P2:** Remove `max_entries` or make it an explicit, tested capacity
  contract.
- [ ] **P2:** Add regression tests listed above before considering the HDC
  knowledge store remediation complete.
