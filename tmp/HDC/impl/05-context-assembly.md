# 05 -- Context Assembly Pipeline

> **STATUS: FULLY IMPLEMENTED (with significant spec deviations)**
>
> The context assembly pipeline is implemented as a single file at
> `crates/hdc/core/src/context.rs` (855 lines). All four phases (gather, rank,
> compress, assemble) are present and the full pipeline runs end-to-end.
> However, the implementation deviates from this spec in structure (free functions
> vs. `ContextAssembler` struct), scoring (freshness replaces emotional resonance),
> and features (no contrarian retrieval, no 9-layer prompt, no summarization
> fallback, no stateful VCG payment persistence). The VCG knapsack solver is
> working with O(n x capacity) DP. See Audit Findings at the bottom for full
> deviation inventory.
>
> **Last audit:** 2026-05-08

Implementation plan for the dynamic context assembly pipeline within `kora-hdc`.

**Crate:** `kora-hdc` (at `crates/hdc/core/`)
**Module path:** `crates/hdc/core/src/context/`
**Design spec:** `/Users/will/dev/nunchi/daeji/tmp/HDC/05-context-assembly.md`
**Depends on:** `02-kora-hdc-core` (algebra), `03-vector-search` (search), `04-knowledge-store` (knowledge types)

---

## 0. Orientation

The context assembly pipeline takes a query, searches the agent's knowledge
stores, scores/ranks/compresses results, and produces a 9-layer prompt that
fits within a token budget. It runs once per cognitive tick, entirely off-chain.

The pipeline has four sequential phases and one allocation mechanism:

```
  Query
    |
    v
 [1. Gather]  -- HDC search across local + shared + contrarian sources
    |
    v
 [2. Rank]    -- 4-factor scoring (relevance, recency, importance, emotion)
    |
    v
 [3. Compress] -- VCG auction for token allocation + summarization fallback
    |
    v
 [4. Assemble] -- 9-layer prompt construction
    |
    v
  ContextResult
```

Because this is entirely off-chain, **f64 arithmetic is acceptable**. The
context assembler never touches consensus-critical state. Different agents
may produce slightly different results due to f64 non-determinism; this is
fine.

---

## 1. File Layout

Create the following files under `crates/hdc/core/src/context/`:

```
context/
  mod.rs          -- pub mod declarations, re-exports, ContextAssembler struct
  types.rs        -- all types: ContextRequest, ScoredEntry, ContextResult, etc.
  gather.rs       -- Phase 1: HDC similarity search
  rank.rs         -- Phase 2: four-factor scoring
  compress.rs     -- Phase 3: VCG auction + token budgeting
  assemble.rs     -- Phase 4: 9-layer prompt builder
  vcg.rs          -- VCG knapsack solver (used by compress)
  tests.rs        -- unit tests
```

Add `pub mod context;` to `crates/hdc/core/src/lib.rs`.

---

## 2. Types (`types.rs`)

These are the data structures that flow through the pipeline. Every struct
listed here must be defined in `types.rs`.

```rust
use ethereum_types::H256;

// --- Enums ---

/// Where a knowledge entry was retrieved from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnowledgeSource {
    Local,
    Shared,
    Contrarian,
}

/// Context mode determines token budget sizing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMode {
    /// 4K total. For single-step, low-ambiguity queries.
    Surgical,
    /// 12K total. For moderate-complexity tasks.
    Focused,
    /// 24K total. For multi-step reasoning with ambiguity.
    Full,
}

/// Identifies which layer of the 9-layer prompt a fragment belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptLayerKind {
    Identity,              // Layer 1
    Capabilities,          // Layer 2
    WorldState,            // Layer 3
    TaskContext,           // Layer 4
    Knowledge,             // Layer 5
    AntiKnowledgeWarnings, // Layer 5b
    ContrarianPerspectives,// Layer 5c
    EmotionalContext,      // Layer 6 (labeled "Emotional State" in spec Layer 8)
    ConversationHistory,   // Layer 7
    Constraints,           // Layer 6 in spec
    OutputFormat,          // Layer 9
}

// --- Structs ---

/// Input to the context assembly pipeline.
pub struct ContextRequest {
    /// HDC query vector encoding the current task/question.
    pub query: HdcVector,
    /// Maximum tokens the assembled prompt may consume.
    pub token_budget: usize,
    /// Agent's current PAD mood state (from ALMA affect system).
    pub mood: PadState,
    /// Current simulation tick (for recency calculations).
    pub current_tick: u64,
    /// Operating mode (determines budget allocation).
    pub mode: ContextMode,
    /// Optional task context (for Layer 4).
    pub task: Option<TaskContext>,
}

/// A single scored knowledge entry, produced by gather+rank.
pub struct ScoredEntry {
    /// Key identifying the knowledge entry in the store.
    pub key: H256,
    /// Composite four-factor score (0.0 to ~1.0).
    pub score: f64,
    /// The text content of the entry.
    pub content: String,
    /// Where this entry was retrieved from.
    pub source: KnowledgeSource,
    /// Estimated token count (via `estimate_tokens`).
    pub tokens: usize,
}

/// One layer of the assembled prompt.
pub struct PromptLayer {
    pub kind: PromptLayerKind,
    pub content: String,
    pub tokens: usize,
}

/// Output of the full context assembly pipeline.
pub struct ContextResult {
    /// The assembled prompt layers, in order.
    pub layers: Vec<PromptLayer>,
    /// Keys of the knowledge entries that made it into the prompt.
    pub entries_used: Vec<H256>,
    /// How many tokens remain unused from the budget.
    pub budget_remaining: usize,
}

/// Token budget breakdown by category.
pub struct TokenBudget {
    pub system_allocation: usize,      // Layers 1-2, 8
    pub knowledge_allocation: usize,   // Layer 5 (HDC output)
    pub task_allocation: usize,        // Layers 3-4, 6-7
    pub response_allocation: usize,    // Reserved for LLM output
}

/// Intermediate: a candidate from the Gather phase, before scoring.
pub struct Candidate {
    /// The knowledge entry retrieved.
    pub entry: KnowledgeEntry,
    /// Normalized Hamming similarity to query (0.0-1.0).
    pub similarity: f64,
    /// Retrieval source.
    pub source: KnowledgeSource,
    /// Trust score: Local=1.0, Shared=compute_trust(), Contrarian=0.8.
    pub trust: f64,
}

/// A Candidate that has been scored by the Rank phase.
pub struct RankedCandidate {
    pub candidate: Candidate,
    /// Composite four-factor score.
    pub score: f64,
}

/// Describes the current task for Layer 4 content.
pub struct TaskContext {
    pub task_description: String,
    pub deadline_ticks: Option<u64>,
    pub current_tick: u64,
    pub mode: ContextMode,
}

/// Per-entry VCG payment state (persists across ticks).
pub struct VcgEntryState {
    /// Accumulated VCG payment from prior rounds. Decays each tick.
    pub accumulated_payment: f64,
    /// Number of consecutive rounds this entry won a context slot.
    pub consecutive_wins: u32,
}

/// Winner from VCG allocation.
pub struct VcgWinner {
    /// Index into the input candidate array.
    pub candidate_index: usize,
    /// VCG externality payment.
    pub payment: f64,
}
```

### TokenBudget construction

```rust
impl TokenBudget {
    pub fn from_mode(mode: &ContextMode) -> Self {
        match mode {
            ContextMode::Surgical => Self {
                system_allocation: 500,
                knowledge_allocation: 1_000,
                task_allocation: 1_500,
                response_allocation: 1_000,
            },
            ContextMode::Focused => Self {
                system_allocation: 1_000,
                knowledge_allocation: 4_000,
                task_allocation: 4_000,
                response_allocation: 3_000,
            },
            ContextMode::Full => Self {
                system_allocation: 2_000,
                knowledge_allocation: 8_000,
                task_allocation: 8_000,
                response_allocation: 6_000,
            },
        }
    }
}
```

### Token estimation

```rust
/// Approximate token count for a string.
/// Uses cl100k_base heuristic: ~3.5 chars/token for mixed code/prose.
/// Conservative (overestimates) to avoid exceeding budget.
pub fn estimate_tokens(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }
    ((content.len() as f64 / 3.5).ceil() as usize).max(1)
}
```

---

## 3. Constants

Define these in `mod.rs` or a dedicated `constants.rs`. Use exactly these
values unless the design spec is updated.

```rust
/// Maximum local candidates retrieved per gather call.
pub const MAX_LOCAL_CANDIDATES: usize = 70;

/// Maximum shared (on-chain) candidates retrieved per gather call.
pub const MAX_SHARED_CANDIDATES: usize = 30;

/// Score threshold below which entries are dropped rather than summarized.
pub const SUMMARIZE_THRESHOLD: f64 = 0.6;

/// Exponential decay constant for recency scoring.
/// Age of 100 ticks -> recency score ~0.37 (1/e).
pub const RECENCY_DECAY: f64 = 100.0;

/// Hamming distance below which two vectors are considered duplicates.
/// Corresponds to similarity > 0.95 for D=10240.
pub const DUPLICATE_THRESHOLD: u32 = 512;

/// VCG payment decay per tick (payments lose 20% each tick).
pub const PAYMENT_DECAY_PER_TICK: f64 = 0.8;

/// Payments below this value are zeroed to avoid f64 dust.
pub const PAYMENT_EPSILON: f64 = 1e-6;

/// Maximum expected raw importance value (for normalization).
/// confidence=1.0 * tier_weight=1.0 * ln(1025) ~= 6.93, rounded up.
pub const IMPORTANCE_NORMALIZER: f64 = 7.0;

/// DP granularity for the VCG knapsack solver: 10 tokens per unit.
pub const VCG_TOKEN_UNIT: usize = 10;

/// Contrarian trust score (less than 1.0 for aligned local knowledge).
pub const CONTRARIAN_TRUST: f64 = 0.8;
```

---

## 4. Phase 1: Gather (`gather.rs`)

The Gather phase performs HDC similarity search across three sources, then
deduplicates.

### Signature

```rust
impl ContextAssembler {
    /// Search all knowledge sources and return deduplicated candidates.
    ///
    /// Sources searched:
    /// 1. Local knowledge index (private to this agent)
    /// 2. Shared substrate (on-chain, trust-verified)
    /// 3. Contrarian (local index searched with anti-query)
    ///
    /// Contrarian candidates fill 1/7 of MAX_LOCAL_CANDIDATES (~15% of slots).
    /// This is a hard reservation -- always runs regardless of scores.
    pub fn gather(&self, query: &HdcVector, context: &TaskContext) -> Vec<Candidate> {
        // ...
    }
}
```

### Implementation

```rust
pub fn gather(&self, query: &HdcVector, context: &TaskContext) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    // --- 1. Local knowledge search ---
    // search() returns Vec<(H256, u32)> where u32 is Hamming distance.
    let local_results = self.local_index.search(query, MAX_LOCAL_CANDIDATES);
    for (key, dist) in local_results {
        if let Some(entry) = self.local_store.get(&key) {
            candidates.push(Candidate {
                entry,
                similarity: 1.0 - dist as f64 / 10240.0,
                source: KnowledgeSource::Local,
                trust: 1.0,
            });
        }
    }

    // --- 2. Shared substrate search (on-chain) ---
    let shared_results = self.chain_substrate.search(query, MAX_SHARED_CANDIDATES);
    for (key, dist) in shared_results {
        if let Some(entry) = self.chain_substrate.get(&key) {
            let trust = self.compute_trust(&entry, context);
            candidates.push(Candidate {
                entry,
                similarity: 1.0 - dist as f64 / 10240.0,
                source: KnowledgeSource::Shared,
                trust,
            });
        }
    }

    // --- 3. Contrarian retrieval (mandatory, ~15% of slots) ---
    // Bind query with ANTI_SUBSPACE to rotate into the contrarian region.
    // ANTI_SUBSPACE is generated deterministically from a consensus seed:
    //   ChaCha20Rng::seed_from_u64(ANTI_SUBSPACE_SEED) -> 10240-bit vector
    let anti_query = query.bind(&ANTI_SUBSPACE);
    let contrarian_slots = MAX_LOCAL_CANDIDATES / 7; // ~10 slots
    let contrarian_results = self.local_index.search(&anti_query, contrarian_slots);
    for (key, dist) in contrarian_results {
        if let Some(entry) = self.local_store.get(&key) {
            candidates.push(Candidate {
                entry,
                similarity: 1.0 - dist as f64 / 10240.0,
                source: KnowledgeSource::Contrarian,
                trust: CONTRARIAN_TRUST,
            });
        }
    }

    // --- 4. Deduplicate ---
    // Remove entries whose vectors are within DUPLICATE_THRESHOLD Hamming
    // distance of a higher-similarity entry already in the list.
    dedup_candidates(&mut candidates);

    candidates
}

/// Remove near-duplicate candidates. Keep the one with higher similarity.
/// Two candidates are duplicates if their entry vectors have Hamming
/// distance <= DUPLICATE_THRESHOLD (512, i.e., similarity > 0.95).
fn dedup_candidates(candidates: &mut Vec<Candidate>) {
    // Sort descending by similarity so we keep the better copy.
    candidates.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));

    let mut keep = vec![true; candidates.len()];
    for i in 0..candidates.len() {
        if !keep[i] {
            continue;
        }
        for j in (i + 1)..candidates.len() {
            if !keep[j] {
                continue;
            }
            let dist = candidates[i].entry.vector.hamming(&candidates[j].entry.vector);
            if dist <= DUPLICATE_THRESHOLD {
                keep[j] = false; // j has lower similarity, drop it
            }
        }
    }

    let mut idx = 0;
    candidates.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}
```

### Key points

- `search()` returns `Vec<(H256, u32)>` where `u32` is raw Hamming distance.
  Convert to similarity: `1.0 - dist as f64 / 10240.0`.
- The contrarian search uses `query.bind(&ANTI_SUBSPACE)` (XOR binding). This
  rotates the query vector into the region where anti-knowledge entries were
  stored. The `ANTI_SUBSPACE` vector is defined in the algebra module (doc 02).
- Deduplication is O(n^2) over candidates. With n <= 110 (70+30+10), this is
  at most ~6,000 Hamming distance computations, each taking <0.5 us. Total:
  ~3 ms worst case.
- Contrarian slots (1/7 of MAX_LOCAL_CANDIDATES) are a **hard reservation**.
  The contrarian search always runs. Do not gate it behind a score threshold.

---

## 5. Phase 2: Rank (`rank.rs`)

The Rank phase computes a composite score for each candidate using four
weighted factors.

### Scoring weights

```
  relevance         = 0.35   (highest -- task-relevance is paramount)
  importance        = 0.25
  recency           = 0.20
  emotional_resonance = 0.20
                     ------
                      1.00
```

### Signature

```rust
/// Compute four-factor composite scores for all candidates.
/// Returns candidates sorted by score descending, with ties broken by
/// total_cmp on the key (deterministic ordering).
pub fn rank(
    candidates: &[Candidate],
    current_tick: u64,
    current_mood: &PadState,
) -> Vec<RankedCandidate> {
    // ...
}
```

### Implementation

```rust
pub fn rank(
    candidates: &[Candidate],
    current_tick: u64,
    current_mood: &PadState,
) -> Vec<RankedCandidate> {
    const W_RELEVANCE: f64  = 0.35;
    const W_IMPORTANCE: f64 = 0.25;
    const W_RECENCY: f64    = 0.20;
    const W_EMOTIONAL: f64  = 0.20;

    let mut ranked: Vec<RankedCandidate> = candidates
        .iter()
        .map(|c| {
            let score = compute_score(c, current_tick, current_mood);
            RankedCandidate {
                candidate: c.clone(),
                score,
            }
        })
        .collect();

    // Sort descending by score. Use total_cmp for deterministic NaN handling.
    // Break ties by key to ensure stable ordering across runs.
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.candidate.entry.key.cmp(&b.candidate.entry.key))
    });

    ranked
}

fn compute_score(
    candidate: &Candidate,
    current_tick: u64,
    current_mood: &PadState,
) -> f64 {
    const W_RELEVANCE: f64  = 0.35;
    const W_IMPORTANCE: f64 = 0.25;
    const W_RECENCY: f64    = 0.20;
    const W_EMOTIONAL: f64  = 0.20;

    // Factor 1: Recency (exponential decay, Ebbinghaus forgetting curve)
    let age = current_tick.saturating_sub(candidate.entry.last_reinforced) as f64;
    let recency = (-age / RECENCY_DECAY).exp();

    // Factor 2: Importance = confidence * tier_weight * ln(confirmations + 1)
    // Normalize to [0, 1] by dividing by IMPORTANCE_NORMALIZER (~7.0).
    let raw_importance = candidate.entry.confidence
        * candidate.entry.tier.weight()
        * (candidate.entry.confirmation_count as f64 + 1.0).ln();
    let importance = (raw_importance / IMPORTANCE_NORMALIZER).min(1.0);

    // Factor 3: Relevance = HDC similarity (already in [0, 1])
    let relevance = candidate.similarity;

    // Factor 4: Emotional congruence (PAD similarity)
    // Entries without emotional tags get a neutral score of 0.5.
    let emotional = match &candidate.entry.emotional_tag {
        Some(tag) => pad_similarity(tag, current_mood),
        None => 0.5,
    };

    W_RECENCY * recency
        + W_IMPORTANCE * importance
        + W_RELEVANCE * relevance
        + W_EMOTIONAL * emotional
}
```

### `KnowledgeTier::weight()` mapping

This must be implemented on the `KnowledgeTier` enum (defined in doc 04's
knowledge store). If it does not already exist, add it:

```rust
impl KnowledgeTier {
    pub fn weight(&self) -> f64 {
        match self {
            KnowledgeTier::Transient    => 0.2,
            KnowledgeTier::Working      => 0.4,
            KnowledgeTier::Consolidated => 0.7,
            KnowledgeTier::Persistent   => 1.0,
        }
    }
}
```

### `pad_similarity()` function

Computes proximity in PAD space, normalized to [0, 1]:

```rust
/// PAD similarity: 1.0 - (euclidean_distance / MAX_PAD_DISTANCE).
/// MAX_PAD_DISTANCE = sqrt(3) * 2.0 ~= 3.464 for PAD in [-1, 1]^3.
fn pad_similarity(a: &PadState, b: &PadState) -> f64 {
    let dp = a.pleasure - b.pleasure;
    let da = a.arousal - b.arousal;
    let dd = a.dominance - b.dominance;
    let dist = (dp * dp + da * da + dd * dd).sqrt();
    const MAX_PAD_DISTANCE: f64 = 3.464_101_615_137_754; // sqrt(12)
    1.0 - (dist / MAX_PAD_DISTANCE)
}
```

### Key points

- Use `total_cmp` (not `partial_cmp`) for the sort. `total_cmp` handles NaN
  deterministically (NaN sorts after all values). This matters because f64
  from bad data could theoretically produce NaN.
- Tie-break by key (`H256` comparison) so the output is deterministic across
  runs with identical inputs.
- `last_reinforced` is a tick counter on `KnowledgeEntry`. Use
  `saturating_sub` to avoid underflow if tick tracking is not yet initialized.

---

## 6. Phase 3: Compress (`compress.rs`)

The Compress phase selects which ranked candidates fit within the token
budget. It uses a VCG auction for optimal allocation, then applies
summarization as a fallback for high-value entries that do not fit.

### Signature

```rust
impl ContextAssembler {
    /// Select entries for the knowledge layer, respecting the token budget.
    ///
    /// Steps:
    /// 1. Compute effective scores (raw score - VCG penalty).
    /// 2. Run VCG knapsack allocation.
    /// 3. Enforce contrarian floor: at least one contrarian entry must appear.
    /// 4. For remaining budget, attempt summarization of high-value rejects.
    /// 5. Update VCG state for next tick.
    ///
    /// Returns the entries to include and the remaining token budget.
    pub fn compress(
        &mut self,
        ranked: &[RankedCandidate],
        budget: &TokenBudget,
    ) -> (Vec<ContextEntry>, usize) {
        // ...
    }
}
```

### Implementation sketch

```rust
pub fn compress(
    &mut self,
    ranked: &[RankedCandidate],
    budget: &TokenBudget,
) -> (Vec<ContextEntry>, usize) {
    let knowledge_budget = budget.knowledge_allocation;

    // 1. Build ScoredEntry list with effective scores (applying VCG penalties).
    let entries: Vec<ScoredEntry> = ranked
        .iter()
        .map(|rc| {
            let tokens = estimate_tokens(&rc.candidate.entry.content);
            let effective = self.effective_score(&rc.candidate.entry.key, rc.score);
            ScoredEntry {
                key: rc.candidate.entry.key,
                score: effective,
                content: rc.candidate.entry.content.clone(),
                source: rc.candidate.source.clone(),
                tokens,
            }
        })
        .collect();

    // 2. Run VCG knapsack allocation.
    let vcg_capacity = knowledge_budget / VCG_TOKEN_UNIT;
    let winners = vcg_allocate(&entries, vcg_capacity);

    // 3. Enforce contrarian floor.
    //    At least one contrarian entry must appear in the final context.
    //    If VCG excluded all contrarian candidates, displace the lowest-scoring
    //    aligned winner to make room for the highest-scoring contrarian.
    let mut winner_indices: Vec<usize> = winners.iter().map(|w| w.candidate_index).collect();
    let has_contrarian = winner_indices.iter().any(|&i| entries[i].source == KnowledgeSource::Contrarian);

    if !has_contrarian {
        // Find the best contrarian candidate (may not be a winner).
        let best_contrarian = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.source == KnowledgeSource::Contrarian)
            .max_by(|(_, a), (_, b)| a.score.total_cmp(&b.score));

        if let Some((contrarian_idx, _)) = best_contrarian {
            // Displace the lowest-scoring aligned (non-contrarian) winner.
            if let Some(displace_pos) = winner_indices
                .iter()
                .enumerate()
                .filter(|(_, &i)| entries[i].source != KnowledgeSource::Contrarian)
                .min_by(|(_, &a), (_, &b)| entries[a].score.total_cmp(&entries[b].score))
                .map(|(pos, _)| pos)
            {
                winner_indices[displace_pos] = contrarian_idx;
            }
        }
    }

    // 4. Build result, tracking token usage.
    let mut result = Vec::new();
    let mut tokens_used = 0;

    // Sort winners by score descending for prompt ordering.
    winner_indices.sort_by(|&a, &b| entries[b].score.total_cmp(&entries[a].score));

    for &idx in &winner_indices {
        let entry = &entries[idx];
        if tokens_used + entry.tokens <= knowledge_budget {
            result.push(ContextEntry::Full(entry.key, entry.content.clone(), entry.source.clone()));
            tokens_used += entry.tokens;
        } else if entry.score > SUMMARIZE_THRESHOLD {
            // Try summarization for high-value entries that don't fit.
            let remaining = knowledge_budget - tokens_used;
            let summary = self.summarize(&entry.content, remaining);
            let summary_tokens = estimate_tokens(&summary);
            if summary_tokens > 0 && tokens_used + summary_tokens <= knowledge_budget {
                result.push(ContextEntry::Summarized(entry.key, summary, entry.source.clone()));
                tokens_used += summary_tokens;
            }
        }
        // Low-scoring entries that don't fit are silently dropped.
    }

    // 5. Update VCG state for next tick.
    let payment_pairs: Vec<(H256, f64)> = winners
        .iter()
        .filter(|w| winner_indices.contains(&w.candidate_index))
        .map(|w| (entries[w.candidate_index].key, w.payment))
        .collect();
    self.update_vcg_state(&payment_pairs);

    let budget_remaining = knowledge_budget.saturating_sub(tokens_used);
    (result, budget_remaining)
}
```

### `ContextEntry` enum

```rust
/// An entry included in the final context, either in full or summarized.
pub enum ContextEntry {
    /// Full content included.
    Full(H256, String, KnowledgeSource),
    /// Content was summarized to fit budget.
    Summarized(H256, String, KnowledgeSource),
}
```

---

## 7. VCG Auction (`vcg.rs`)

The VCG auction solves a 0/1 knapsack problem to maximize total value of
selected entries within the token budget, then computes externality payments
for each winner.

### `knapsack_01` -- the DP solver

```rust
/// Standard 0/1 knapsack via dynamic programming.
///
/// Arguments:
/// - `items`: each item has a `score` (value) and `tokens` (weight)
/// - `capacity`: budget in token-units (budget_tokens / VCG_TOKEN_UNIT)
///
/// Returns: (total_value, indices_of_selected_items)
///
/// Complexity: O(n * capacity). For n=100, capacity=800 -> 80K cells.
pub fn knapsack_01(items: &[ScoredEntry], capacity: usize) -> (f64, Vec<usize>) {
    let n = items.len();
    // dp[w] = best value achievable with capacity w.
    // Using 1D rolling array (space = O(capacity)).
    let mut dp = vec![0.0_f64; capacity + 1];
    // Track choices for backtracking.
    let mut choice = vec![vec![false; capacity + 1]; n];

    for i in 0..n {
        // Weight in token-units (ceiling division).
        let w_i = (items[i].tokens + VCG_TOKEN_UNIT - 1) / VCG_TOKEN_UNIT;
        if w_i > capacity {
            continue; // Item too large for any configuration.
        }
        // Iterate in reverse to avoid using item i twice (0/1 constraint).
        for w in (w_i..=capacity).rev() {
            let value_with = dp[w - w_i] + items[i].score;
            if value_with > dp[w] {
                dp[w] = value_with;
                choice[i][w] = true;
            }
        }
    }

    // Backtrack to find selected items.
    let mut selected = Vec::new();
    let mut w = capacity;
    for i in (0..n).rev() {
        if choice[i][w] {
            selected.push(i);
            let w_i = (items[i].tokens + VCG_TOKEN_UNIT - 1) / VCG_TOKEN_UNIT;
            w -= w_i;
        }
    }

    (dp[capacity], selected)
}
```

### `vcg_allocate` -- the full VCG mechanism

```rust
/// Run VCG allocation over scored entries.
///
/// 1. Solve 0/1 knapsack with all candidates to find optimal allocation.
/// 2. For each winner, re-solve the knapsack excluding that winner.
/// 3. Compute VCG payment: how much value others lost because of this winner.
///
/// VCG payment formula:
///   payment_i = value_without_i - (total_value_with_all - value_of_i)
///
/// In words: "the value of the best allocation without me, minus the value
/// that everyone else gets in the allocation that includes me."
///
/// Complexity: O((1 + n_winners) * n * capacity).
/// With n=100, capacity=800, n_winners~25 -> ~26 knapsack solves -> ~2ms.
pub fn vcg_allocate(entries: &[ScoredEntry], capacity: usize) -> Vec<VcgWinner> {
    if entries.is_empty() || capacity == 0 {
        return Vec::new();
    }

    // Step 1: Optimal allocation with all candidates.
    let (optimal_value, winners) = knapsack_01(entries, capacity);

    // Step 2: For each winner, compute VCG payment.
    let mut results = Vec::new();
    for &winner_idx in &winners {
        // Build candidate list excluding this winner.
        let others: Vec<ScoredEntry> = entries
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != winner_idx)
            .map(|(_, e)| e.clone())
            .collect();

        let (value_without, _) = knapsack_01(&others, capacity);

        // VCG payment = value others would get without me
        //             - value others actually get with me.
        let my_value = entries[winner_idx].score;
        let others_value_with_me = optimal_value - my_value;
        let payment = (value_without - others_value_with_me).max(0.0);

        results.push(VcgWinner {
            candidate_index: winner_idx,
            payment,
        });
    }

    results
}
```

### Key points

- The DP table uses O(n * W) memory with the choice matrix. For n=100 and
  W=800, this is 80K booleans = ~80KB. Acceptable.
- If memory becomes a concern, switch to the 1D DP array and reconstruct
  selected items via a separate bitmap. But for current candidate counts this
  optimization is unnecessary.
- `VCG_TOKEN_UNIT = 10` means the DP operates in units of 10 tokens. An
  8,000-token budget becomes a capacity of 800. This keeps the DP table
  small without losing meaningful resolution.
- The `payment.max(0.0)` clamp is a safety measure. VCG payments are
  theoretically non-negative, but floating-point arithmetic can produce tiny
  negative values.

---

## 8. VCG Payment State

The `ContextAssembler` struct maintains per-entry VCG state that persists
across cognitive ticks. This state drives the priority decay that prevents
knowledge monopolization.

### Effective score computation

```rust
impl ContextAssembler {
    /// Compute the effective score for VCG bidding.
    /// Raw four-factor score is reduced by accumulated prior payments,
    /// amplified by consecutive wins.
    pub fn effective_score(&self, entry_key: &H256, raw_score: f64) -> f64 {
        let default = VcgEntryState {
            accumulated_payment: 0.0,
            consecutive_wins: 0,
        };
        let state = self.vcg_state.get(entry_key).unwrap_or(&default);

        // Consecutive-win amplification.
        // More consecutive wins -> LARGER penalty (discourages monopolization).
        //
        // The multiplier is (1.0 / PAYMENT_DECAY_PER_TICK)^consecutive_wins:
        //   0 wins -> (1/0.8)^0 = 1.0   (no amplification)
        //   1 win  -> (1/0.8)^1 = 1.25
        //   2 wins -> (1/0.8)^2 = 1.5625
        //   5 wins -> (1/0.8)^5 = 3.0518
        //  10 wins -> (1/0.8)^10 = 9.3132
        let amplifier = (1.0 / PAYMENT_DECAY_PER_TICK).powi(state.consecutive_wins as i32);

        let penalty = state.accumulated_payment * amplifier;
        (raw_score - penalty).max(0.0)
    }
}
```

### CRITICAL BUG FIX: The consecutive-win amplification formula

The design spec originally had an inverted formula. Here is the correct
version and why the other is wrong:

**CORRECT (use this):**
```rust
let amplifier = (1.0 / PAYMENT_DECAY_PER_TICK).powi(consecutive_wins as i32);
// With PAYMENT_DECAY_PER_TICK = 0.8:
//   (1.0 / 0.8) = 1.25
//   1.25^consecutive_wins -> grows > 1.0 -> penalty INCREASES
```

**WRONG (do NOT use):**
```rust
let amplifier = PAYMENT_DECAY_PER_TICK.powi(consecutive_wins as i32);
// With PAYMENT_DECAY_PER_TICK = 0.8:
//   0.8^consecutive_wins -> shrinks toward 0.0 -> penalty DECREASES
//   This rewards monopolization instead of penalizing it!
```

The intuition: `PAYMENT_DECAY_PER_TICK = 0.8` means "payments decay by 20%
per tick." For the amplifier, we want the INVERSE behavior: more consecutive
wins should AMPLIFY the penalty, not decay it. So we use the reciprocal
`1.0 / 0.8 = 1.25` as the base.

If you write `0.8.powi(wins)`, an entry winning 5 rounds in a row gets a
penalty multiplied by `0.8^5 = 0.328` -- its penalty is reduced to 33% of
the base, making it EASIER to keep winning. This is the opposite of what the
mechanism is designed to do.

### VCG state update (end of each tick)

```rust
impl ContextAssembler {
    /// After each context assembly round, update VCG state.
    ///
    /// 1. Decay all existing payments by PAYMENT_DECAY_PER_TICK.
    /// 2. Zero out payments below PAYMENT_EPSILON.
    /// 3. Reset consecutive_wins for non-winners.
    /// 4. Add new payments and increment consecutive_wins for winners.
    pub fn update_vcg_state(&mut self, winners: &[(H256, f64)]) {
        // Decay all existing payments and reset win streaks.
        for state in self.vcg_state.values_mut() {
            state.accumulated_payment *= PAYMENT_DECAY_PER_TICK;
            if state.accumulated_payment < PAYMENT_EPSILON {
                state.accumulated_payment = 0.0;
            }
            // Reset consecutive wins -- only winners get them back below.
            state.consecutive_wins = 0;
        }

        // Apply new payments to winners.
        let default = VcgEntryState {
            accumulated_payment: 0.0,
            consecutive_wins: 0,
        };
        for (key, payment) in winners {
            let state = self.vcg_state.entry(*key).or_insert_with(|| default.clone());
            state.accumulated_payment += payment;
            state.consecutive_wins += 1;
        }
    }
}
```

### Payment lifecycle example

```
Tick 0: Entry X wins. Payment = 0.4. State: {accumulated: 0.4, wins: 1}
Tick 1: Entry X wins again. Decay first: 0.4 * 0.8 = 0.32.
        New payment = 0.3. State: {accumulated: 0.62, wins: 2}
        Effective penalty = 0.62 * (1/0.8)^2 = 0.62 * 1.5625 = 0.969
        -> Entry X's effective score is nearly zero. It will likely lose.
Tick 2: Entry X does not win. Decay: 0.62 * 0.8 = 0.496. Wins reset to 0.
        State: {accumulated: 0.496, wins: 0}
        Effective penalty = 0.496 * (1/0.8)^0 = 0.496 * 1.0 = 0.496
        -> Still penalized but recovering.
Tick 5: After 3 more ticks without winning:
        0.496 * 0.8^3 = 0.254. Wins = 0. Penalty = 0.254.
        -> Largely recovered. Can compete again.
```

---

## 9. Phase 4: Assemble (`assemble.rs`)

The Assemble phase constructs the final prompt from 9 ordered layers.

### Layer ordering

The order is deliberate and informed by the "Lost in the Middle" finding
(Liu et al. 2024): LLMs attend most to content at the start and end of the
context window. Critical framing (identity, capabilities) goes first.
Constraints and output format go last. Knowledge occupies the middle --
but within the Knowledge layer, entries are ordered by score descending.

```
Layer 1: System Instruction (Identity)
Layer 2: Persona / Capabilities
Layer 3: Task Context
Layer 4: Retrieved Knowledge (full entries, score descending)
Layer 5: Anti-Knowledge Warnings (if any contrarian entries flagged concerns)
Layer 6: Contrarian Perspectives (explicitly labeled as dissenting)
Layer 7: Emotional Context (current PAD state + behavioral mode)
Layer 8: Conversation History (recent actions, compressed)
Layer 9: User Query / Output Format
```

### Signature

```rust
/// Assemble the 9-layer prompt from compressed context entries.
pub fn assemble(
    &self,
    request: &ContextRequest,
    entries: &[ContextEntry],
    budget: &TokenBudget,
) -> ContextResult {
    // ...
}
```

### Implementation sketch

```rust
pub fn assemble(
    &self,
    request: &ContextRequest,
    entries: &[ContextEntry],
    budget: &TokenBudget,
) -> ContextResult {
    let mut layers = Vec::new();
    let mut entries_used = Vec::new();
    let mut total_tokens = 0;

    // Layer 1: Identity
    let identity = self.build_identity_layer();
    total_tokens += identity.tokens;
    layers.push(identity);

    // Layer 2: Capabilities
    let capabilities = self.build_capabilities_layer();
    total_tokens += capabilities.tokens;
    layers.push(capabilities);

    // Layer 3: Task Context
    if let Some(task) = &request.task {
        let task_layer = self.build_task_layer(task);
        total_tokens += task_layer.tokens;
        layers.push(task_layer);
    }

    // Layer 4: Retrieved Knowledge (aligned entries, score descending)
    // Layer 5: Anti-Knowledge Warnings
    // Layer 6: Contrarian Perspectives
    let (knowledge_layers, used_keys) = self.build_knowledge_layers(entries);
    for layer in &knowledge_layers {
        total_tokens += layer.tokens;
    }
    layers.extend(knowledge_layers);
    entries_used.extend(used_keys);

    // Layer 7: Emotional Context
    let emotional = self.build_emotional_layer(&request.mood);
    total_tokens += emotional.tokens;
    layers.push(emotional);

    // Layer 8: Conversation History
    let history = self.build_history_layer(budget.task_allocation);
    total_tokens += history.tokens;
    layers.push(history);

    // Layer 9: Output Format
    let output_format = self.build_output_format_layer();
    total_tokens += output_format.tokens;
    layers.push(output_format);

    let total_budget = budget.system_allocation
        + budget.knowledge_allocation
        + budget.task_allocation;
    let budget_remaining = total_budget.saturating_sub(total_tokens);

    ContextResult {
        layers,
        entries_used,
        budget_remaining,
    }
}
```

### Building knowledge layers from entries

Contrarian entries must be explicitly marked so the LLM can weigh them
appropriately:

```rust
fn build_knowledge_layers(
    &self,
    entries: &[ContextEntry],
) -> (Vec<PromptLayer>, Vec<H256>) {
    let mut knowledge_lines = Vec::new();
    let mut contrarian_lines = Vec::new();
    let mut used_keys = Vec::new();

    for entry in entries {
        let (key, content, source) = match entry {
            ContextEntry::Full(k, c, s) => (k, c.clone(), s),
            ContextEntry::Summarized(k, c, s) => (k, format!("[SUMMARIZED] {}", c), s),
        };
        used_keys.push(*key);

        match source {
            KnowledgeSource::Contrarian => {
                contrarian_lines.push(format!("[CONTRARIAN] {}", content));
            }
            _ => {
                knowledge_lines.push(content);
            }
        }
    }

    let mut layers = Vec::new();

    // Layer 4: Retrieved Knowledge
    if !knowledge_lines.is_empty() {
        let text = knowledge_lines.join("\n\n");
        layers.push(PromptLayer {
            kind: PromptLayerKind::Knowledge,
            tokens: estimate_tokens(&text),
            content: text,
        });
    }

    // Layer 6: Contrarian Perspectives
    if !contrarian_lines.is_empty() {
        let text = contrarian_lines.join("\n\n");
        layers.push(PromptLayer {
            kind: PromptLayerKind::ContrarianPerspectives,
            tokens: estimate_tokens(&text),
            content: text,
        });
    }

    (layers, used_keys)
}
```

---

## 10. The `ContextAssembler` Struct (`mod.rs`)

This is the top-level struct that owns the pipeline state and exposes
`assemble_context()` as the single entry point.

```rust
use std::collections::HashMap;
use ethereum_types::H256;

pub struct ContextAssembler {
    /// Local knowledge index (HDC vector search).
    local_index: Box<dyn VectorIndex>,
    /// Local knowledge store (content retrieval by key).
    local_store: Box<dyn KnowledgeStore>,
    /// Shared substrate (on-chain knowledge access).
    chain_substrate: Box<dyn SharedSubstrate>,
    /// Agent identity configuration (for Layers 1-2).
    agent_config: AgentConfig,
    /// Per-entry VCG payment state, persisted across ticks.
    vcg_state: HashMap<H256, VcgEntryState>,
    /// Conversation history buffer.
    history: Vec<String>,
    /// Summarization client (for compress fallback).
    summarization_client: Box<dyn SummarizationClient>,
}

impl ContextAssembler {
    /// Run the full context assembly pipeline.
    ///
    /// This is the single entry point. Call once per cognitive tick.
    pub fn assemble_context(&mut self, request: &ContextRequest) -> ContextResult {
        let budget = TokenBudget::from_mode(&request.mode);
        let task = request.task.as_ref().unwrap_or(&TaskContext::default());

        // Phase 1: Gather
        let candidates = self.gather(&request.query, task);

        // Phase 2: Rank
        let ranked = rank(&candidates, request.current_tick, &request.mood);

        // Phase 3: Compress (VCG + token budgeting)
        let (entries, _budget_remaining) = self.compress(&ranked, &budget);

        // Phase 4: Assemble (9-layer prompt)
        self.assemble(request, &entries, &budget)
    }
}
```

---

## 11. Anti-Patterns

Things to get wrong and how to avoid them.

### 1. Using `0.8^wins` for consecutive-win amplification

**Wrong:**
```rust
let amplifier = PAYMENT_DECAY_PER_TICK.powi(consecutive_wins as i32);
```
This makes the penalty SMALLER with more wins (0.8^5 = 0.33), rewarding
monopolization. Use `(1.0 / PAYMENT_DECAY_PER_TICK).powi(...)` instead.

### 2. Skipping contrarian retrieval

Do not gate contrarian search behind a flag, a score threshold, or a
"is there budget left?" check. The contrarian search is mandatory. It
always runs. The slots are a hard reservation (1/7 of MAX_LOCAL_CANDIDATES).
The at-least-one-contrarian-in-the-final-prompt rule is a hard floor.

### 3. Forgetting deduplication

Without dedup, the same knowledge entry can appear from both the local index
and the shared substrate (if it was published on-chain). It can also appear
from both the regular search and the contrarian search (if the entry's
vector happens to be near both query regions). Always run `dedup_candidates`
after gathering from all sources.

### 4. Using `partial_cmp` instead of `total_cmp`

`f64::partial_cmp` returns `None` for NaN, which causes `sort_by` to produce
undefined ordering. Always use `total_cmp` for f64 sorting. NaN will sort
deterministically (as greater than all values) rather than silently corrupting
the sort.

### 5. Not clamping effective score to >= 0.0

Without the `.max(0.0)`, accumulated VCG penalties can drive effective scores
negative, which breaks the knapsack solver's assumptions (negative values
mean "I should be paid to include this entry"). Always clamp:
```rust
(raw_score - penalty).max(0.0)
```

### 6. Treating PAYMENT_DECAY_PER_TICK = 0.8 as "multiply by 0.8 to amplify"

`0.8` is the decay factor -- it makes things smaller. The amplifier needs
the reciprocal (`1.0 / 0.8 = 1.25`) to make things larger. This is the
same mistake as anti-pattern #1 but expressed differently.

### 7. Stuffing the context window

More tokens is not better. Context rot is real and empirically validated
across all frontier models (see Chroma 2025, Du et al. 2025 in the design
spec). The Surgical (4K) mode with precisely selected, high-confidence
knowledge will outperform Full (24K) mode packed with marginally relevant
material. Default to smaller modes. Escalate only when task complexity
requires it.

---

## 12. Checklist

Use this to track progress. Each item maps to a concrete code artifact.

### Types and constants
- [x] Define `KnowledgeSource` enum -- exists at `knowledge/source.rs` (5 variants vs spec's 3; no `Contrarian`)
- [ ] Define `ContextMode` enum (Surgical/Focused/Full) -- **MISSING**
- [ ] Define `PromptLayerKind` enum -- **MISSING**
- [ ] Define `ContextRequest` struct -- **MISSING** (free function params used instead)
- [x] Define `ScoredEntry` struct -- exists at `knowledge/scoring.rs` (divergent fields)
- [ ] Define `PromptLayer` struct -- **MISSING**
- [x] Define `ContextResult` struct -- `AssembledContext` exists (no layers, has `entries` + `payments`)
- [ ] Define `TokenBudget` struct with `from_mode()` -- **MISSING** (flat `usize` budget)
- [x] Define `Candidate` struct -- `ContextCandidate` exists (missing `source`, `emotional_tag`)
- [ ] Define `RankedCandidate` struct -- **MISSING** (ranking done in-place)
- [ ] Define `TaskContext` struct -- **MISSING**
- [ ] Define `VcgEntryState` struct -- **MISSING** (no payment persistence)
- [x] Define `VcgWinner` struct -- `VcgResult` exists (different shape)
- [ ] Define `ContextEntry` enum (Full/Summarized) -- **MISSING**
- [x] Implement `estimate_tokens()` -- exists but takes `usize` not `&str` (F10)
- [x] Define all constants -- core constants inline in `context.rs`; missing `MAX_SHARED_CANDIDATES`, `SUMMARIZE_THRESHOLD`, `CONTRARIAN_TRUST`

**Token budget constants (from spec):**
- Surgical mode: system=500, knowledge=1,000, task=1,500, response=1,000 (total 4K)
- Focused mode: system=1,000, knowledge=4,000, task=4,000, response=3,000 (total 12K)
- Full mode: system=2,000, knowledge=8,000, task=8,000, response=6,000 (total 24K)

**Key numeric constants (implemented):**
- `DUPLICATE_THRESHOLD = 512` (Hamming distance for near-duplicate detection, ~95% similarity)
- `RECENCY_DECAY = 100.0` (age of 100 ticks -> recency ~0.37)
- `VCG_TOKEN_UNIT = 10` (DP granularity: 10 tokens per unit)
- `PAYMENT_DECAY_PER_TICK = 0.8` (20% decay per tick)
- `PAYMENT_EPSILON = 1e-6` (dust threshold)
- `MAX_LOCAL_CANDIDATES = 70`

### Phase 1: Gather
- [x] Implement `gather()` -- exists as free function (delegates to closure, not method)
- [x] Implement local index search -- delegated to caller via `query_fn` closure
- [ ] Implement shared substrate search with trust computation -- **MISSING**
- [ ] Implement contrarian search (`query.bind(&ANTI_SUBSPACE)`, 1/7 of slots) -- **MISSING**
- [x] Implement `dedup_candidates()` using `DUPLICATE_THRESHOLD` -- exists in `compress()` phase
- [ ] Verify contrarian slots are a hard reservation -- **N/A** (no contrarian retrieval)

### Phase 2: Rank
- [x] Implement `compute_score()` -- 3 of 4 factors correct; emotional resonance replaced by freshness (F02)
- [x] Implement `rank()` returning sorted candidates -- correct structure, `total_cmp` tiebreaker
- [x] Implement `pad_similarity()` -- exists in `cognitive/affect.rs` but **NOT USED** from context pipeline
- [x] Implement `KnowledgeTier::weight()` -- correct values in `knowledge/tier.rs`
- [x] Verify sort uses `total_cmp` with key tiebreaker -- CORRECT (line 160-164)
- [x] Verify weights sum to 1.0 -- CORRECT (0.35 + 0.25 + 0.20 + 0.20 = 1.0, but 4th factor is semantically wrong)

### Phase 3: Compress
- [x] Implement `compress()` -- exists but only does dedup, not VCG+budgeting (AP02)
- [ ] Implement `effective_score()` with VCG penalty -- **MISSING**
- [ ] Verify amplifier uses `(1.0 / PAYMENT_DECAY_PER_TICK).powi(...)` -- **N/A** (no payment state)
- [ ] Implement contrarian floor enforcement -- **MISSING**
- [ ] Implement summarization fallback for entries above `SUMMARIZE_THRESHOLD` -- **MISSING**
- [ ] Implement `update_vcg_state()` with decay + win tracking -- **MISSING** (AP04: payments computed but thrown away)

### VCG Auction
- [x] Implement `knapsack_01()` -- CORRECT DP with reverse iteration (returns `Vec<usize>` not `(f64, Vec<usize>)`)
- [x] Implement `vcg_allocate()` -- CORRECT VCG externality computation (returns `VcgResult` not `Vec<VcgWinner>`)
- [x] Verify DP iterates in reverse for 0/1 constraint -- CORRECT (line 235)
- [x] Verify payment clamped to `>= 0.0` -- CORRECT (line 298)
- [x] Verify backtracking reconstructs correct item set -- CORRECT (lines 245-255)

### Phase 4: Assemble
- [ ] Implement `assemble()` producing 9-layer prompt -- **MISSING** (does VCG selection only)
- [ ] Implement `build_knowledge_layers()` separating aligned vs. contrarian -- **MISSING**
- [ ] Verify contrarian entries are prefixed with `[CONTRARIAN]` -- **MISSING**
- [ ] Verify layer ordering matches spec -- **MISSING** (no layers exist)
- [ ] Implement `build_emotional_layer()` with PAD state -- **MISSING**

### Integration
- [ ] Define `ContextAssembler` struct -- **MISSING** (free functions only)
- [x] Implement `assemble_context()` as the single entry point -- free function at line 369
- [x] Add `pub mod context;` to `crates/hdc/core/src/lib.rs` -- line 32
- [ ] Wire trait dependencies: `VectorIndex`, `KnowledgeStore`, `SharedSubstrate` -- **MISSING**
- [ ] Verify `vcg_state` is `HashMap<H256, VcgEntryState>` on the assembler -- **MISSING** (no stateful struct)

---

## 13. Test Plan

### Unit tests (`tests.rs`)

**Scoring tests:**
```rust
#[test]
fn test_compute_score_weights_sum_to_one() {
    // Create a candidate where all factors = 1.0.
    // Score should be exactly 1.0 (0.35 + 0.25 + 0.20 + 0.20).
    let score = compute_score(&perfect_candidate, 0, &neutral_mood);
    assert!((score - 1.0).abs() < 1e-10);
}

#[test]
fn test_recency_decay() {
    // Entry accessed 0 ticks ago -> recency ~= 1.0
    // Entry accessed 100 ticks ago -> recency ~= 0.368 (1/e)
    // Entry accessed 1000 ticks ago -> recency ~= 0.0
    let recent = compute_recency(0);
    let medium = compute_recency(100);
    let old = compute_recency(1000);
    assert!(recent > 0.99);
    assert!((medium - (-1.0_f64).exp()).abs() < 0.01);
    assert!(old < 0.001);
}

#[test]
fn test_emotional_neutral_fallback() {
    // Entry with no emotional tag gets score 0.5.
    let candidate = make_candidate_without_emotional_tag();
    let score = compute_score(&candidate, 0, &any_mood);
    // The emotional component should contribute exactly 0.20 * 0.5 = 0.10.
}
```

**VCG knapsack tests:**
```rust
#[test]
fn test_knapsack_trivial() {
    // Single item that fits -> selected.
    let items = vec![scored_entry(score: 1.0, tokens: 100)];
    let (value, selected) = knapsack_01(&items, 100 / VCG_TOKEN_UNIT);
    assert_eq!(selected, vec![0]);
    assert!((value - 1.0).abs() < 1e-10);
}

#[test]
fn test_knapsack_prefers_higher_value() {
    // Two items, only one fits. Higher value wins.
    let items = vec![
        scored_entry(score: 0.5, tokens: 100),
        scored_entry(score: 0.9, tokens: 100),
    ];
    let capacity = 100 / VCG_TOKEN_UNIT;
    let (_, selected) = knapsack_01(&items, capacity);
    assert_eq!(selected, vec![1]);
}

#[test]
fn test_knapsack_value_density() {
    // One large item (score=1.0, tokens=200) vs two small items
    // (score=0.6 each, tokens=100 each). Budget = 200 tokens.
    // Two small items have higher total value (1.2 > 1.0).
    let items = vec![
        scored_entry(score: 1.0, tokens: 200),
        scored_entry(score: 0.6, tokens: 100),
        scored_entry(score: 0.6, tokens: 100),
    ];
    let capacity = 200 / VCG_TOKEN_UNIT;
    let (value, selected) = knapsack_01(&items, capacity);
    assert!((value - 1.2).abs() < 1e-10);
    assert!(selected.contains(&1));
    assert!(selected.contains(&2));
    assert!(!selected.contains(&0));
}

#[test]
fn test_vcg_payment_is_externality() {
    // Three items, budget fits two. Items: A(0.9, 100), B(0.8, 100), C(0.7, 100).
    // Budget = 200 tokens. Optimal: A + B = 1.7.
    // Without A: B + C = 1.5. Others' value with A = 0.8. Payment_A = 1.5 - 0.8 = 0.7.
    // Without B: A + C = 1.6. Others' value with B = 0.9. Payment_B = 1.6 - 0.9 = 0.7.
    let items = vec![
        scored_entry(score: 0.9, tokens: 100),
        scored_entry(score: 0.8, tokens: 100),
        scored_entry(score: 0.7, tokens: 100),
    ];
    let winners = vcg_allocate(&items, 200 / VCG_TOKEN_UNIT);
    let payment_a = winners.iter().find(|w| w.candidate_index == 0).unwrap().payment;
    let payment_b = winners.iter().find(|w| w.candidate_index == 1).unwrap().payment;
    assert!((payment_a - 0.7).abs() < 1e-10);
    assert!((payment_b - 0.7).abs() < 1e-10);
}

#[test]
fn test_vcg_zero_payment_when_no_displacement() {
    // One item, budget can fit it easily, no competitors displaced.
    // Payment should be 0.0.
    let items = vec![scored_entry(score: 0.5, tokens: 50)];
    let winners = vcg_allocate(&items, 500 / VCG_TOKEN_UNIT);
    assert_eq!(winners.len(), 1);
    assert!((winners[0].payment - 0.0).abs() < 1e-10);
}
```

**Consecutive-win amplification tests:**
```rust
#[test]
fn test_amplifier_increases_with_wins() {
    // 0 wins -> amplifier = 1.0
    // 1 win  -> amplifier = 1.25
    // 5 wins -> amplifier = ~3.05
    let amp_0 = (1.0_f64 / PAYMENT_DECAY_PER_TICK).powi(0);
    let amp_1 = (1.0_f64 / PAYMENT_DECAY_PER_TICK).powi(1);
    let amp_5 = (1.0_f64 / PAYMENT_DECAY_PER_TICK).powi(5);
    assert!((amp_0 - 1.0).abs() < 1e-10);
    assert!((amp_1 - 1.25).abs() < 1e-10);
    assert!((amp_5 - 3.0517578125).abs() < 1e-6);
    // Verify it's monotonically increasing.
    assert!(amp_1 > amp_0);
    assert!(amp_5 > amp_1);
}

#[test]
fn test_wrong_formula_decreases_with_wins() {
    // This test documents the bug so it is not reintroduced.
    let wrong_amp_5 = PAYMENT_DECAY_PER_TICK.powi(5);
    assert!(wrong_amp_5 < 1.0); // 0.8^5 = 0.328, penalty shrinks!
    // The correct formula should always be >= 1.0 for wins >= 0.
}

#[test]
fn test_effective_score_never_negative() {
    let mut assembler = make_assembler();
    // Give an entry a massive accumulated payment.
    assembler.vcg_state.insert(key, VcgEntryState {
        accumulated_payment: 100.0,
        consecutive_wins: 10,
    });
    let effective = assembler.effective_score(&key, 0.5);
    assert!(effective >= 0.0); // Clamped, not negative.
}
```

**Contrarian floor tests:**
```rust
#[test]
fn test_contrarian_floor_enforcement() {
    // Set up candidates where all contrarian entries score below aligned ones.
    // VCG would exclude all contrarian entries. Verify that after compress(),
    // at least one contrarian entry appears in the result.
    let ranked = make_ranked_with_contrarian_at_bottom();
    let (entries, _) = assembler.compress(&ranked, &budget);
    let has_contrarian = entries.iter().any(|e| match e {
        ContextEntry::Full(_, _, s) | ContextEntry::Summarized(_, _, s) =>
            *s == KnowledgeSource::Contrarian,
    });
    assert!(has_contrarian, "At least one contrarian entry must be present");
}
```

**Deduplication tests:**
```rust
#[test]
fn test_dedup_removes_near_duplicates() {
    // Two candidates with Hamming distance = 256 (< DUPLICATE_THRESHOLD = 512).
    // The one with lower similarity should be removed.
    let mut candidates = vec![
        make_candidate(similarity: 0.90),
        make_candidate_near_duplicate(similarity: 0.85), // dist=256 from first
    ];
    dedup_candidates(&mut candidates);
    assert_eq!(candidates.len(), 1);
    assert!((candidates[0].similarity - 0.90).abs() < 1e-10);
}

#[test]
fn test_dedup_keeps_distant_entries() {
    // Two candidates with Hamming distance = 1024 (> DUPLICATE_THRESHOLD).
    // Both should survive dedup.
    let mut candidates = vec![
        make_candidate(similarity: 0.90),
        make_candidate_distant(similarity: 0.85), // dist=1024 from first
    ];
    dedup_candidates(&mut candidates);
    assert_eq!(candidates.len(), 2);
}
```

**Token budget tests:**
```rust
#[test]
fn test_token_budget_surgical() {
    let b = TokenBudget::from_mode(&ContextMode::Surgical);
    assert_eq!(b.system_allocation, 500);
    assert_eq!(b.knowledge_allocation, 1_000);
    assert_eq!(b.task_allocation, 1_500);
    assert_eq!(b.response_allocation, 1_000);
}

#[test]
fn test_estimate_tokens_empty() {
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn test_estimate_tokens_short() {
    // "hello" = 5 chars. 5 / 3.5 = 1.43, ceil = 2.
    assert_eq!(estimate_tokens("hello"), 2);
}
```

### Integration tests

- **Round-trip test:** Create a `ContextAssembler` with a mock local index
  containing 100 entries, a mock shared substrate with 30 entries, run
  `assemble_context()`, and verify:
  - `ContextResult` has 9 or more layers (some may be empty).
  - `entries_used` is non-empty.
  - `budget_remaining >= 0`.
  - At least one contrarian entry appears.

- **Multi-tick monopolization test:** Run `assemble_context()` for 10 ticks
  with the same query. Verify that the set of `entries_used` changes over
  time (VCG payments rotate entries).

- **Mode escalation test:** Create a Surgical-mode request where the top
  entry has 2,000 tokens (exceeds the 1,000 knowledge budget). Verify
  that either a summary is produced or a budget warning appears.

---

## Audit Findings

Audit date: 2026-05-08. Files reviewed:

- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/context.rs` (the entire context module -- single file, not the directory layout prescribed by spec)
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/bundle.rs`
- `/Users/will/dev/nunchi/daeji/crates/kora-hdc/src/bundle.rs` (duplicate of the above)
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/lib.rs`
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/` (entry.rs, tier.rs, source.rs, scoring.rs, store.rs, mod.rs)
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/cognitive/affect.rs`
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/constants.rs`

### F01: File layout does not match spec

The spec prescribes a `context/` directory module with seven files (`mod.rs`, `types.rs`, `gather.rs`, `rank.rs`, `compress.rs`, `assemble.rs`, `vcg.rs`, `tests.rs`). The implementation collapses everything into a single 855-line file at `crates/hdc/core/src/context.rs`. This means:

- No `types.rs` -- types are defined inline.
- No separate `gather.rs`, `rank.rs`, `compress.rs`, `assemble.rs`, `vcg.rs` files.
- No `ContextAssembler` struct. The pipeline is exposed as free functions (`gather`, `rank`, `compress`, `assemble`, `assemble_context`), not methods on a stateful struct.

### F02: Scoring factor mismatch -- emotional resonance replaced by "freshness"

**Spec** (section 5): Four scoring factors are `relevance` (0.35), `importance` (0.25), `recency` (0.20), `emotional_resonance` (0.20). The emotional factor uses `pad_similarity()` to measure congruence between the entry's emotional tag and the agent's current PAD mood.

**Implementation** (`context.rs` lines 30-35): The fourth factor is called `W_FRESHNESS` (0.20) and is computed as `c.trust * recency` (line 153). This is a trust-weighted recency score, not emotional congruence at all. The `ContextCandidate` struct has no `emotional_tag` or `PadState` field. The `rank()` function has no `current_mood` parameter.

This is a significant semantic deviation. The emotional resonance factor is entirely absent from the pipeline. The implementation does not use the `PadState` or `pad_similarity()` function defined in `crates/hdc/core/src/cognitive/affect.rs`, despite it being available in the crate.

### F03: No `ContextAssembler` struct -- no stateful VCG payment tracking

The spec's central design element is the `ContextAssembler` struct (section 10) that holds:
- `local_index`, `local_store`, `chain_substrate` (knowledge sources)
- `vcg_state: HashMap<H256, VcgEntryState>` (per-entry VCG payment state)
- `history`, `summarization_client`, `agent_config`

The implementation has no such struct. The `assemble_context()` function at line 369 is a stateless free function that takes a closure `query_fn` instead of holding references to concrete stores. VCG payments (`vcg_allocate` at line 268) are computed but there is no persistence across ticks -- the `VcgResult` is returned and never stored. The anti-monopolization mechanism is structurally broken: without `update_vcg_state()` and `effective_score()`, an entry can dominate context indefinitely.

### F04: No contrarian retrieval

The spec mandates three knowledge sources with hard reservations:
1. Local index (MAX_LOCAL_CANDIDATES = 70)
2. Shared substrate (MAX_SHARED_CANDIDATES = 30)
3. Contrarian search via `query.bind(&ANTI_SUBSPACE)` (1/7 of slots = ~10 entries, hard reservation)

The implementation's `gather()` function (line 109) delegates to a single `query_fn` closure. There is no multi-source search, no contrarian retrieval, no anti-query binding. The `ANTI_SUBSPACE` vector exists in `knowledge/anti.rs` but is never referenced from `context.rs`.

### F05: No contrarian floor enforcement

The spec's compress phase (section 6) requires that at least one contrarian entry appears in the final context. If VCG excludes all contrarian candidates, the lowest-scoring aligned winner must be displaced. The implementation has no `KnowledgeSource` awareness anywhere in the context pipeline. `ContextCandidate` has no `source` field.

### F06: No `ContextEntry` enum (Full vs. Summarized)

The spec defines a `ContextEntry` enum with `Full(H256, String, KnowledgeSource)` and `Summarized(H256, String, KnowledgeSource)` variants. The implementation produces `AssembledContext` containing raw `ContextCandidate` structs. There is no summarization fallback, no `SUMMARIZE_THRESHOLD`, and no `summarize()` method.

### F07: No 9-layer prompt assembly

The spec's Phase 4 (section 9) produces a 9-layer structured prompt (`ContextResult` with `Vec<PromptLayer>`). The implementation's `assemble()` function (line 316) merely selects candidates that fit the token budget via VCG. There are no prompt layers, no `PromptLayerKind` enum, no `PromptLayer` struct, no `build_identity_layer()`, `build_capabilities_layer()`, `build_task_layer()`, `build_emotional_layer()`, `build_history_layer()`, or `build_output_format_layer()` methods.

### F08: No `ContextMode` / `TokenBudget` system

The spec defines three context modes (Surgical/4K, Focused/12K, Full/24K) with a `TokenBudget` struct that allocates tokens across system, knowledge, task, and response categories. The implementation takes a flat `token_budget: usize` parameter. There is no mode-based budget allocation.

### F09: No `ContextRequest` input type

The spec defines `ContextRequest` with fields for `query`, `token_budget`, `mood` (PadState), `current_tick`, `mode` (ContextMode), and optional `task` (TaskContext). The implementation uses raw parameters: `query: &HdcVector`, `token_budget: usize`, `current_tick: u64`, `max_candidates: usize`.

### F10: `estimate_tokens` signature mismatch

**Spec**: `pub fn estimate_tokens(content: &str) -> usize` (takes a string reference).
**Implementation** (`context.rs` line 93): `pub fn estimate_tokens(content_len: usize) -> usize` (takes a raw length). The implementation cannot be called with a `&str` directly. This is a minor API deviation but it means the function does not validate content, just counts bytes.

### F11: Duplicate `BundleAccumulator` across two crates

`crates/hdc/core/src/bundle.rs` and `crates/kora-hdc/src/bundle.rs` are byte-for-byte identical (127 lines each). This is a code duplication issue. One crate should depend on the other, or the shared code should be factored into a common crate.

### F12: `KnowledgeSource` enum mismatch

**Spec** (section 2): `KnowledgeSource { Local, Shared, Contrarian }` -- three simple variants for context assembly use.
**Implementation** (`knowledge/source.rs`): `KnowledgeSource { SelfDerived, SharedSubstrate { publisher: [u8; 32] }, MultiAgentConfirmed { count: u32 }, SingleAgent { agent_id: [u8; 32] }, Anonymous }` -- five variants with payload fields. There is no `Contrarian` variant, and the `ContextCandidate` struct does not reference `KnowledgeSource` at all.

### F13: Scoring weight discrepancy in `knowledge/scoring.rs`

The knowledge store's `scoring.rs` uses weights `relevance=0.35, recency=0.25, importance=0.25, trust=0.15` (sum=1.0). The context pipeline's `context.rs` uses `relevance=0.35, importance=0.25, recency=0.20, freshness=0.20` (sum=1.0). The spec mandates `relevance=0.35, importance=0.25, recency=0.20, emotional=0.20`. There are now three different scoring formulas for the same concept.

### F14: `knapsack_01` return type mismatch

**Spec**: Returns `(f64, Vec<usize>)` -- total value and indices.
**Implementation** (`context.rs` line 218): Returns `Vec<usize>` only. The total value is not returned. The `vcg_allocate` function recomputes it separately (`optimal_value` at line 278). This works correctly but does not match the spec signature.

### F15: Missing VCG payment state types

The spec defines `VcgEntryState { accumulated_payment: f64, consecutive_wins: u32 }` and `VcgWinner { candidate_index: usize, payment: f64 }`. The implementation defines `VcgResult { selected: Vec<usize>, payments: Vec<f64> }` instead of `Vec<VcgWinner>`. There is no `VcgEntryState` at all since no stateful payment tracking exists.

### F16: `KnowledgeEntry` field name differences

The spec references `entry.confidence`, `entry.last_reinforced`, `entry.confirmation_count`, and `entry.emotional_tag`. The actual `KnowledgeEntry` (in `knowledge/entry.rs`) has `balance` (not `confidence`), `last_accessed` (not `last_reinforced`), `confirmations` (not `confirmation_count`), and no `emotional_tag` field. The `ContextCandidate` struct in `context.rs` uses `confidence`, `last_reinforced`, and `confirmation_count` (spec names), but these are disconnected from the actual `KnowledgeEntry` type.

---

## Implementation Status

### Types and constants

| Item | Spec location | Status | Notes |
|------|--------------|--------|-------|
| `KnowledgeSource` enum | `types.rs` | WRONG | Exists in `knowledge/source.rs` but has 5 variants instead of 3; no `Contrarian` variant |
| `ContextMode` enum | `types.rs` | MISSING | Not implemented anywhere |
| `PromptLayerKind` enum | `types.rs` | MISSING | Not implemented anywhere |
| `ContextRequest` struct | `types.rs` | MISSING | Free function params used instead |
| `ScoredEntry` struct | `types.rs` | DIVERGENT | Exists in `knowledge/scoring.rs` with different fields (`entry: KnowledgeEntry`, `hamming_distance: u32` vs. spec's `content: String`, `source: KnowledgeSource`, `tokens: usize`) |
| `PromptLayer` struct | `types.rs` | MISSING | Not implemented anywhere |
| `ContextResult` struct | `types.rs` | MISSING | `AssembledContext` exists but has `entries: Vec<ContextCandidate>` instead of `layers: Vec<PromptLayer>` |
| `TokenBudget` struct | `types.rs` | MISSING | Not implemented; flat `usize` budget used |
| `Candidate` struct | `types.rs` | PARTIAL | `ContextCandidate` exists but lacks `source`, `emotional_tag`; uses different field names |
| `RankedCandidate` struct | `types.rs` | MISSING | Ranking is done in-place on `ContextCandidate` |
| `TaskContext` struct | `types.rs` | MISSING | Not implemented |
| `VcgEntryState` struct | `types.rs` | MISSING | No stateful payment tracking |
| `VcgWinner` struct | `types.rs` | PARTIAL | `VcgResult` exists with different shape |
| `ContextEntry` enum | `types.rs` | MISSING | No Full/Summarized distinction |
| `estimate_tokens()` | `types.rs` | PARTIAL | Exists but takes `usize` not `&str` |
| Constants | `mod.rs` | PARTIAL | Core constants exist inline in `context.rs` but `MAX_LOCAL_CANDIDATES`, `MAX_SHARED_CANDIDATES`, `SUMMARIZE_THRESHOLD`, `CONTRARIAN_TRUST` are missing |

### Phase 1: Gather

| Item | Status | Notes |
|------|--------|-------|
| `gather()` on `ContextAssembler` | PARTIAL | Free function, not method. Delegates to a closure instead of searching concrete stores. |
| Local index search | DELEGATED | Pushed to caller via `query_fn` closure |
| Shared substrate search | MISSING | No multi-source search |
| Contrarian search | MISSING | No anti-query binding, no `ANTI_SUBSPACE` usage |
| `dedup_candidates()` | MOVED | Dedup happens in `compress()` phase, not `gather()`. Functionally present but in wrong phase. |
| Contrarian hard reservation | MISSING | No contrarian retrieval at all |

### Phase 2: Rank

| Item | Status | Notes |
|------|--------|-------|
| `compute_score()` | PARTIAL | Three of four factors correct. Emotional resonance replaced by trust*recency. |
| `rank()` | IMPLEMENTED | Correct structure: sorts descending by score with `total_cmp` tiebreaker. |
| `pad_similarity()` | NOT USED | Exists in `cognitive/affect.rs` but not called from context pipeline. |
| `KnowledgeTier::weight()` | IMPLEMENTED | Correct values in `knowledge/tier.rs`. |
| Sort uses `total_cmp` | CORRECT | Line 160-164 |
| Weights sum to 1.0 | CORRECT | 0.35 + 0.25 + 0.20 + 0.20 = 1.0 (but fourth factor is wrong semantically) |

### Phase 3: Compress

| Item | Status | Notes |
|------|--------|-------|
| `compress()` on `ContextAssembler` | PARTIAL | Free function that does dedup only (near-duplicate removal). No VCG penalty, no token budgeting. |
| `effective_score()` | MISSING | No VCG penalty computation |
| Amplifier formula | N/A | No amplifier since no payment state exists |
| Contrarian floor | MISSING | No source-aware displacement logic |
| Summarization fallback | MISSING | No `SUMMARIZE_THRESHOLD`, no `summarize()` |
| `update_vcg_state()` | MISSING | No payment state persistence |

### VCG Auction

| Item | Status | Notes |
|------|--------|-------|
| `knapsack_01()` | IMPLEMENTED | Correct DP with reverse iteration. Returns `Vec<usize>` (no total value). |
| `vcg_allocate()` | IMPLEMENTED | Correct VCG externality computation. Returns `VcgResult` not `Vec<VcgWinner>`. |
| DP reverse iteration | CORRECT | Line 235 |
| Payment clamped >= 0.0 | CORRECT | Line 298 |
| Backtracking | CORRECT | Lines 245-255 |

### Phase 4: Assemble

| Item | Status | Notes |
|------|--------|-------|
| 9-layer prompt | MISSING | `assemble()` does VCG selection only; no prompt construction |
| `build_knowledge_layers()` | MISSING | No layer separation |
| Contrarian `[CONTRARIAN]` prefix | MISSING | No source-aware formatting |
| Layer ordering | MISSING | No layers exist |
| `build_emotional_layer()` | MISSING | No PAD state integration |

### Integration

| Item | Status | Notes |
|------|--------|-------|
| `ContextAssembler` struct | MISSING | Free functions only |
| `assemble_context()` entry point | IMPLEMENTED | Free function at line 369, correct pipeline sequence |
| `pub mod context;` in lib.rs | IMPLEMENTED | Line 32, with re-exports at line 42 |
| Trait dependencies | MISSING | No `VectorIndex`, `SharedSubstrate`, `SummarizationClient` traits |
| `vcg_state` on assembler | MISSING | No stateful struct |

---

## Anti-Patterns & Duct Tape

### AP01: Closure-based gather is a structural workaround

**File:** `crates/hdc/core/src/context.rs`, line 109-124

```rust
pub fn gather<F>(query: &HdcVector, max_candidates: usize, query_fn: F) -> Vec<ContextCandidate>
where
    F: FnOnce(&HdcVector, usize) -> Vec<ContextCandidate>,
```

The `query_fn` closure pushes all knowledge source integration to the caller. This avoids defining traits for `VectorIndex`, `KnowledgeStore`, and `SharedSubstrate`, but it means the gather phase cannot implement multi-source search (local + shared + contrarian) internally. The caller would need to manually merge three search results, compute trust, generate anti-queries, and tag sources -- defeating the purpose of the pipeline abstraction. This is duct tape to avoid the trait-based architecture.

### AP02: `compress()` is misnamed -- it only deduplicates

**File:** `crates/hdc/core/src/context.rs`, lines 175-205

The function named `compress()` performs near-duplicate removal via Hamming distance threshold. The spec's "Compress" phase is a VCG auction with token budgeting, contrarian floor enforcement, and summarization fallback. The actual VCG auction logic is in `assemble()` instead. The naming creates confusion about where each spec phase maps to in the code.

### AP03: `ContextCandidate` is an ad-hoc flattened struct

**File:** `crates/hdc/core/src/context.rs`, lines 40-66

`ContextCandidate` has raw scoring inputs (`confidence`, `tier_weight`, `confirmation_count`, `trust`) as flat `f64`/`u32` fields rather than referencing a `KnowledgeEntry` + `KnowledgeSource`. The field names do not match the actual `KnowledgeEntry` type (`confidence` vs `balance`, `last_reinforced` vs `last_accessed`, `confirmation_count` vs `confirmations`). This creates a translation layer problem: whoever constructs a `ContextCandidate` must manually map fields, with no compile-time enforcement that the mapping is correct.

### AP04: VCG payments are computed but thrown away

**File:** `crates/hdc/core/src/context.rs`, lines 316-355

The `assemble()` function calls `vcg_allocate()` and stores payments in `AssembledContext.payments`, but there is no mechanism to feed these payments back into the next tick's scoring. The `assemble_context()` free function (line 369) returns `AssembledContext` and terminates -- there is no state mutation. The VCG anti-monopolization mechanism is architecturally dead code.

### AP05: `compress()` takes `duplicate_threshold` but defaults to the constant

**File:** `crates/hdc/core/src/context.rs`, lines 176-179

```rust
let threshold = if duplicate_threshold == 0 {
    DUPLICATE_THRESHOLD
} else {
    duplicate_threshold
};
```

Using 0 as a sentinel for "use default" is a code smell. The function is only ever called with `DUPLICATE_THRESHOLD` from `assemble_context()` (line 386), making the parameter redundant. If customization is needed, an `Option<u32>` would be cleaner.

### AP06: `assemble_context()` hard-wires `DUPLICATE_THRESHOLD` twice

**File:** `crates/hdc/core/src/context.rs`, line 386

The full pipeline function passes `DUPLICATE_THRESHOLD` to `compress()`, which then checks if it is 0 and potentially replaces it with... `DUPLICATE_THRESHOLD` again. The value flows through two redundant layers.

### AP07: No error handling in the pipeline

All functions return bare values with no `Result` type. If `query_fn` panics, or if token costs overflow, or if the knapsack solver runs into pathological input (e.g., capacity larger than available memory for the DP table), the pipeline will panic or produce silently incorrect results. The spec does not prescribe error handling explicitly, but a production pipeline should handle degenerate inputs gracefully.

### AP08: `knapsack_01` allocates O(n * capacity) choice matrix

**File:** `crates/hdc/core/src/context.rs`, line 228

```rust
let mut choice = vec![vec![false; capacity + 1]; n];
```

With VCG running (1 + n_winners) knapsack solves, each allocating a fresh choice matrix, memory usage becomes O((1 + k) * n * capacity). For pathological inputs (many items, large budget), this could be problematic. The spec notes this is "acceptable" for current sizes but the implementation has no guard against oversized inputs.

---

## Recommended Changes Checklist

### Critical (spec conformance)

- [ ] **C01**: Create `ContextAssembler` struct with `vcg_state: HashMap<[u8; 32], VcgEntryState>` and implement `effective_score()`, `update_vcg_state()` as methods. Without this, VCG anti-monopolization is dead. (`context.rs` -- restructure into struct)
- [ ] **C02**: Add emotional resonance scoring factor. Replace `W_FRESHNESS = trust * recency` with `W_EMOTIONAL = pad_similarity(entry_emotional_tag, current_mood)` per spec. Import `pad_similarity` from `cognitive/affect.rs`. Add `current_mood: &PadState` parameter to `rank()`. (`context.rs` line 153)
- [ ] **C03**: Implement multi-source gather with three search phases: local, shared substrate, and contrarian (anti-query via `bind(query, &ANTI_SUBSPACE)`). Replace `query_fn` closure with trait-based store access. (`context.rs` lines 109-124)
- [ ] **C04**: Add `KnowledgeSource::Contrarian` variant or add a `source` field to `ContextCandidate`. Implement contrarian floor enforcement in the compress/assemble phase. (`context.rs`, `knowledge/source.rs`)
- [ ] **C05**: Implement 9-layer prompt assembly. Define `PromptLayerKind`, `PromptLayer`, `ContextResult` types. Build layers in the correct order (Identity -> Capabilities -> Task -> Knowledge -> Anti-Knowledge -> Contrarian -> Emotional -> History -> Output Format). (`context.rs` -- new `assemble` logic)
- [ ] **C06**: Add `ContextMode` and `TokenBudget` with `from_mode()`. Replace flat `token_budget: usize` with mode-based budget allocation. (`context.rs`)
- [ ] **C07**: Implement `ContextRequest` input type encapsulating `query`, `token_budget`, `mood`, `current_tick`, `mode`, and optional `task`. (`context.rs`)
- [ ] **C08**: Implement summarization fallback in compress: entries scoring above `SUMMARIZE_THRESHOLD` (0.6) that do not fit in budget should be summarized rather than dropped. (`context.rs`)

### Important (correctness / maintenance)

- [ ] **I01**: Fix `compress()` naming and phase separation. Current `compress()` does dedup (spec: gather phase). Current `assemble()` does VCG selection (spec: compress phase). Rename to match spec phases. (`context.rs` lines 175, 316)
- [ ] **I02**: Align `ContextCandidate` field names with `KnowledgeEntry`: `confidence` -> `balance`, `last_reinforced` -> `last_accessed`, `confirmation_count` -> `confirmations`. Or make `ContextCandidate` hold a `KnowledgeEntry` reference. (`context.rs` lines 40-66)
- [ ] **I03**: Fix `estimate_tokens()` signature to take `&str` instead of `usize`. The spec and all callsites in the spec reference `estimate_tokens(&content)`. (`context.rs` line 93)
- [ ] **I04**: Remove duplicate `BundleAccumulator` in `crates/kora-hdc/src/bundle.rs`. Either have `kora-hdc` depend on `hdc-core`, or extract into a shared crate. (`kora-hdc/src/bundle.rs`)
- [ ] **I05**: Align scoring weights between `knowledge/scoring.rs` (0.35/0.25/0.25/0.15) and `context.rs` (0.35/0.25/0.20/0.20). There should be one canonical formula, ideally the spec's (0.35/0.25/0.20/0.20 with emotional resonance). (`context.rs` lines 32-35, `knowledge/scoring.rs` line 58)

### Module structure

- [ ] **M01**: Split `context.rs` (855 lines) into the spec's directory layout: `context/mod.rs`, `context/types.rs`, `context/gather.rs`, `context/rank.rs`, `context/compress.rs`, `context/assemble.rs`, `context/vcg.rs`, `context/tests.rs`. (`crates/hdc/core/src/context.rs` -> `crates/hdc/core/src/context/`)
- [ ] **M02**: Add `VcgEntryState` and `VcgWinner` types. Change `VcgResult` to `Vec<VcgWinner>` for spec conformance and `knapsack_01` to return `(f64, Vec<usize>)`. (`context.rs` lines 69-75, 218, 268)
- [ ] **M03**: Define missing constants: `MAX_LOCAL_CANDIDATES` (70), `MAX_SHARED_CANDIDATES` (30), `SUMMARIZE_THRESHOLD` (0.6), `CONTRARIAN_TRUST` (0.8). Move inline constants to a dedicated section or `constants.rs`. (`context.rs` lines 16-28)

### Testing gaps

- [ ] **T01**: Add tests for VCG payment persistence across ticks (requires `ContextAssembler` struct first). The spec's "multi-tick monopolization test" cannot be written with the current stateless design.
- [ ] **T02**: Add tests for contrarian floor enforcement. No tests currently verify that at least one contrarian entry survives.
- [ ] **T03**: Add tests for emotional resonance scoring (requires `PadState` integration first).
- [ ] **T04**: Add tests for `ContextMode` / `TokenBudget` allocation (requires types first).
- [ ] **T05**: Add tests for summarization fallback behavior (requires `SUMMARIZE_THRESHOLD` and summarization logic first).

---

## Second-Pass Remediation Detail

This section converts the first-pass audit into a concrete target architecture
for replacing the current single-file, stateless context pipeline. The target
must be implemented in `crates/hdc/core/src/context/` and must keep the current
off-chain boundary: context assembly may use `f64`, dynamic allocation, and
source adapters, but it must not write consensus-critical state.

### Target module architecture

Replace `crates/hdc/core/src/context.rs` with a directory module:

```text
crates/hdc/core/src/context/
  mod.rs          -- ContextAssembler, public re-exports, top-level orchestration
  types.rs        -- request/result/candidate/budget/error/state types
  source.rs       -- source adapter traits and adapter output normalization
  gather.rs       -- local/shared/contrarian retrieval and deduplication
  rank.rs         -- four-factor scoring with PAD emotional resonance
  compress.rs     -- VCG allocation, token budgeting, contrarian floor, summaries
  assemble.rs     -- 9-layer prompt builder
  vcg.rs          -- bounded knapsack and VCG externality pricing
  tests.rs        -- context module unit tests
```

`lib.rs` should continue to expose `pub mod context;`, but root re-exports
should change from the current free-function API to the assembler API:

```rust
pub use context::{
    ContextAssembler, ContextMode, ContextRequest, ContextResult,
    ContextError, SourceAdapter,
};
```

The current free functions can be preserved temporarily as compatibility
wrappers only if they delegate to a `ContextAssembler` with explicit adapters.
They must not remain the primary architecture because they cannot retain VCG
state across ticks.

### `ContextAssembler` target shape

`ContextAssembler` should own pipeline configuration and state, while borrowing
knowledge sources through adapter traits. Keep VCG state in the assembler, not
inside `KnowledgeEntry`, because VCG payments are context-selection history,
not intrinsic knowledge facts.

```rust
pub struct ContextAssembler<S> {
    sources: S,
    config: ContextConfig,
    vcg_state: HashMap<[u8; 32], VcgEntryState>,
    history: ConversationHistory,
    summarizer: Option<Box<dyn SummarizationClient>>,
}

impl<S> ContextAssembler<S>
where
    S: ContextSources,
{
    pub fn assemble_context(
        &mut self,
        request: ContextRequest,
    ) -> Result<ContextResult, ContextError> {
        request.validate()?;
        let budget = TokenBudget::from_mode(request.mode, request.max_prompt_tokens)?;
        let candidates = self.gather(&request, &budget)?;
        let ranked = rank::rank(&candidates, request.current_tick, &request.mood)?;
        let compressed = self.compress(&ranked, &budget)?;
        assemble::assemble(&request, &budget, compressed, &self.history)
    }
}
```

`ContextConfig` should hold all tunables currently scattered as constants:

```rust
pub struct ContextConfig {
    pub max_local_candidates: usize,     // default 70
    pub max_shared_candidates: usize,    // default 30
    pub contrarian_divisor: usize,       // default 7
    pub duplicate_threshold: u32,        // default DUPLICATE_THRESHOLD
    pub summarize_threshold: f64,        // default 0.6
    pub vcg_token_unit: usize,           // default 10
    pub max_vcg_capacity_units: usize,   // default 2_400 for a 24K prompt
    pub contrarian_trust: f64,           // default 0.8
}
```

Do not reuse `knowledge::KnowledgeSource` as the retrieval-source enum. The
existing enum records provenance (`SelfDerived`, `SharedSubstrate`, etc.).
Context assembly also needs to know which retrieval channel produced the
candidate. Add a separate type:

```rust
pub enum ContextSourceKind {
    Local,
    Shared,
    Contrarian,
}
```

`Candidate` should hold a real `KnowledgeEntry` plus context-specific metadata:

```rust
pub struct Candidate {
    pub key: [u8; 32],
    pub entry: KnowledgeEntry,
    pub source_kind: ContextSourceKind,
    pub hamming_distance: u32,
    pub similarity: f64,
    pub trust: f64,
    pub emotional_tag: Option<PadState>,
}
```

Map current `KnowledgeEntry` fields directly:

- `balance` is the confidence/strength input.
- `last_accessed` is the recency tick input.
- `confirmations` is the confirmation-count input.
- `tier.weight()` is already implemented and should be reused.
- `contradicted` drives anti-knowledge warning placement in assembly.

If emotional tags are not yet persisted on `KnowledgeEntry`, use
`Candidate::emotional_tag = None` and let ranking fall back to the existing
`pad_similarity` neutral behavior (`0.5`). Do not invent fake PAD values from
content.

### Source adapters

`gather.rs` must not accept an opaque closure that hides source semantics.
Define source adapter traits that expose exactly what context assembly needs:

```rust
pub trait LocalKnowledgeAdapter {
    fn search_local(
        &self,
        query: &HdcVector,
        limit: usize,
    ) -> Result<Vec<AdapterHit>, ContextError>;

    fn get_local(&self, key: &[u8; 32]) -> Result<Option<KnowledgeEntry>, ContextError>;
}

pub trait SharedKnowledgeAdapter {
    fn search_shared(
        &self,
        query: &HdcVector,
        limit: usize,
    ) -> Result<Vec<AdapterHit>, ContextError>;

    fn get_shared(&self, key: &[u8; 32]) -> Result<Option<SharedKnowledgeEntry>, ContextError>;
}

pub trait ContextSources: LocalKnowledgeAdapter + SharedKnowledgeAdapter {}

pub struct AdapterHit {
    pub key: [u8; 32],
    pub hamming_distance: u32,
}
```

The local adapter can wrap the current `KnowledgeStore` and its brute-force
index path. The shared adapter should wrap on-chain/shared-substrate access and
return enough metadata for `knowledge::trust::compute_trust()`: author
reputation, tick duration, current tick, and the query vector. Shared entries
below `MIN_TRUST_THRESHOLD` should be dropped during gather.

Adapter failures are non-fatal by source:

- Local source failure: return `Err(ContextError::LocalSource(...))`; without
  local memory the context is unreliable enough to fail the request.
- Shared source failure: record a warning on `ContextResult` and continue with
  local + contrarian candidates.
- Contrarian retrieval failure: record a warning and continue, but
  `ContextResult::contrarian_status` must say `Unavailable`, not `Satisfied`.

### Gather module

`gather.rs` should be responsible for retrieval, source tagging, trust, and
deduplication. Token allocation is not gather's job.

Required flow:

1. Search local source with `max_local_candidates`.
2. Search shared source with `max_shared_candidates`.
3. Search contrarian source with `bind(query, &ANTI_SUBSPACE)` and
   `max(1, max_local_candidates / contrarian_divisor)`.
4. Convert every hit into a `Candidate` with a `ContextSourceKind`.
5. Compute `similarity = 1.0 - hamming_distance as f64 / D as f64`.
6. Drop shared candidates with `trust < MIN_TRUST_THRESHOLD`.
7. Deduplicate by entry key first, then by vector Hamming distance
   `<= duplicate_threshold`.
8. Keep the candidate with the higher gather priority:
   `Contrarian` beats `Shared` beats `Local` only when the keys match;
   otherwise keep the higher `similarity * trust` candidate.

Contrarian retrieval must use the current anti-knowledge implementation:

```rust
let anti_query = bind(&request.query, &ANTI_SUBSPACE);
let contrarian_limit = (config.max_local_candidates / config.contrarian_divisor).max(1);
let hits = sources.search_local(&anti_query, contrarian_limit)?;
```

A contrarian candidate is not the same as an entry whose provenance is
`KnowledgeSource::SharedSubstrate`; the contrarian flag is about retrieval
channel. Preserve both:

```rust
Candidate {
    source_kind: ContextSourceKind::Contrarian,
    entry, // entry.source remains SelfDerived/SharedSubstrate/etc.
    trust: config.contrarian_trust,
    ..
}
```

### Rank module

`rank.rs` should use one canonical formula for context scoring:

```text
score =
  0.35 * relevance
+ 0.25 * importance
+ 0.20 * recency
+ 0.20 * emotional_resonance
```

Concrete inputs:

- `relevance = candidate.similarity.clamp(0.0, 1.0)`
- `importance = (entry.balance * entry.tier.weight() * ln(entry.confirmations + 1)) / 7.0`
- `recency = exp(-age_ticks / 100.0)`, with `age_ticks =
  current_tick.saturating_sub(entry.last_accessed)`
- `emotional_resonance = pad_similarity(candidate.emotional_tag, current_mood)`,
  or `0.5` when no tag exists

Trust should not replace emotional resonance. Use trust as a multiplier or gate
after the four-factor score:

```rust
let raw = relevance_w + importance_w + recency_w + emotional_w;
let score = (raw * candidate.trust.clamp(0.0, 1.0)).clamp(0.0, 1.0);
```

Sort with `total_cmp` and stable key tie-breakers:

```rust
ranked.sort_by(|a, b| {
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.candidate.key.cmp(&b.candidate.key))
        .then_with(|| source_order(a.candidate.source_kind).cmp(&source_order(b.candidate.source_kind)))
});
```

Reject or sanitize non-finite scores before sorting:

```rust
if !score.is_finite() {
    return Err(ContextError::NonFiniteScore { key: candidate.key });
}
```

### Compress module

`compress.rs` should own token allocation, VCG scoring, contrarian floor
enforcement, summarization fallback, and VCG payment retention.

Required output:

```rust
pub struct CompressedContext {
    pub entries: Vec<ContextEntry>,
    pub payments: Vec<VcgWinner>,
    pub token_usage: TokenUsage,
    pub contrarian_status: ContrarianStatus,
    pub warnings: Vec<ContextWarning>,
}

pub enum ContextEntry {
    Full {
        key: [u8; 32],
        content: String,
        source_kind: ContextSourceKind,
        score: f64,
        tokens: usize,
    },
    Summarized {
        key: [u8; 32],
        content: String,
        source_kind: ContextSourceKind,
        original_tokens: usize,
        summary_tokens: usize,
        score: f64,
    },
}
```

Pipeline:

1. Estimate tokens from content with `estimate_tokens(&entry.content)`.
2. Drop candidates with zero content or zero effective score.
3. Convert raw rank score to effective VCG score with `effective_score()`.
4. Reject a single item that exceeds `knowledge_allocation` unless it can be
   summarized under budget and has `score >= summarize_threshold`.
5. Run `vcg_allocate()` with capacity
   `knowledge_allocation / vcg_token_unit`.
6. Enforce the contrarian floor.
7. Fill remaining space with summaries for high-scoring rejects.
8. Update VCG payment state only after the final selected set is known.

The contrarian floor must be explicit:

```text
If at least one contrarian candidate exists after gather:
  final context must contain at least one contrarian entry unless every
  contrarian candidate is too large to include or summarize.
```

Displacement rule:

- Pick highest effective-score contrarian candidate not already selected.
- If it fits in remaining budget, add it.
- Otherwise displace the lowest effective-score non-contrarian winner whose
  removal creates enough room.
- If no full entry can fit, try summarization.
- If no contrarian can fit even after summarization, set
  `ContrarianStatus::UnavailableDueToBudget` and add a warning.

### VCG payment retention

VCG payments must survive across cognitive ticks. The current implementation
returns payments but throws away their effect. The target state is:

```rust
#[derive(Clone, Debug)]
pub struct VcgEntryState {
    pub accumulated_payment: f64,
    pub consecutive_wins: u32,
    pub last_seen_tick: u64,
}

#[derive(Clone, Debug)]
pub struct VcgWinner {
    pub candidate_index: usize,
    pub key: [u8; 32],
    pub payment: f64,
    pub effective_score: f64,
}
```

Effective score:

```rust
pub fn effective_score(&self, key: &[u8; 32], raw_score: f64) -> f64 {
    let Some(state) = self.vcg_state.get(key) else {
        return raw_score.clamp(0.0, 1.0);
    };

    let amplifier = (1.0 / PAYMENT_DECAY_PER_TICK)
        .powi(state.consecutive_wins.min(64) as i32);
    let penalty = state.accumulated_payment * amplifier;
    (raw_score - penalty).max(0.0)
}
```

Update order matters:

1. Decay every retained payment by `PAYMENT_DECAY_PER_TICK` once per tick.
2. Drop states below `PAYMENT_EPSILON` if `consecutive_wins == 0`.
3. Reset `consecutive_wins` to `0` for non-winners.
4. Add current VCG payments for final winners only.
5. Increment `consecutive_wins` for winners.
6. Set `last_seen_tick = current_tick`.

This retention map must be serializable by the caller if the agent wants
cross-process memory, but the context module should not require persistence.
Expose:

```rust
pub fn vcg_state(&self) -> &HashMap<[u8; 32], VcgEntryState>;
pub fn replace_vcg_state(&mut self, state: HashMap<[u8; 32], VcgEntryState>);
```

### Token budgets

The target should support mode defaults and a caller-specified hard cap.
`ContextRequest::max_prompt_tokens` is the absolute cap. `ContextMode` selects
the default distribution and scales down proportionally when the cap is smaller
than the mode default.

```rust
pub enum ContextMode {
    Surgical, // default 4_000
    Focused,  // default 12_000
    Full,     // default 24_000
}

pub struct TokenBudget {
    pub total_prompt: usize,
    pub system: usize,
    pub task: usize,
    pub knowledge: usize,
    pub contrarian_reserved: usize,
    pub history: usize,
    pub output_format: usize,
    pub response_reserved: usize,
}
```

Default distribution:

| Mode | Total | System | Task | Knowledge | Contrarian reserve | History | Output format | Response reserved |
|------|-------|--------|------|-----------|--------------------|---------|---------------|-------------------|
| Surgical | 4,000 | 500 | 900 | 900 | 100 | 500 | 100 | 1,000 |
| Focused | 12,000 | 1,000 | 2,500 | 3,500 | 500 | 1,000 | 500 | 3,000 |
| Full | 24,000 | 2,000 | 5,000 | 7,000 | 1,000 | 2,000 | 1,000 | 6,000 |

`response_reserved` is never available for context entries. The assembled
prompt must satisfy:

```text
system + task + knowledge + contrarian + history + output_format <= total_prompt - response_reserved
```

When fixed layers exceed their allocation, degrade in this order:

1. Compress history.
2. Summarize knowledge entries.
3. Drop lowest-score aligned knowledge entries.
4. Drop contrarian only if it cannot fit as a summary.
5. Return `Err(ContextError::BudgetExhausted)` if identity/capability/output
   layers alone exceed the available prompt budget.

### Assemble module

`assemble.rs` should build the final prompt layers and should never run VCG.
Its job starts after `compress.rs` has decided which entries are allowed.

Required layers:

1. `Identity`
2. `Capabilities`
3. `WorldState`
4. `TaskContext`
5. `Knowledge`
6. `AntiKnowledgeWarnings`
7. `ContrarianPerspectives`
8. `EmotionalContext`
9. `ConversationHistory`
10. `Constraints`
11. `OutputFormat`

The first-pass text called this a 9-layer prompt, but the existing
`PromptLayerKind` list already distinguishes anti-knowledge, contrarian,
constraints, and output format. The remediation target should keep these as
separate typed layers while allowing empty optional layers to be omitted. The
user-facing contract is "ordered typed prompt layers", not a magic count.

Rules:

- Put identity and capabilities first.
- Put constraints and output format last.
- Put contrarian entries in `ContrarianPerspectives`, not mixed silently into
  aligned knowledge.
- Prefix summarized entries with `[SUMMARIZED]`.
- Prefix contrarian entries with `[CONTRARIAN]`.
- Prefix contradicted aligned entries in anti-warning layer with
  `[ANTI-KNOWLEDGE WARNING]`.
- Recompute tokens for every rendered layer and assert the sum does not exceed
  the prompt budget available to context.

### Error handling

All public context assembly entry points should return
`Result<ContextResult, ContextError>`.

Target error type:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("context request has zero token budget")]
    ZeroTokenBudget,
    #[error("context request max candidates is zero")]
    ZeroCandidateLimit,
    #[error("local source failed: {0}")]
    LocalSource(String),
    #[error("shared source failed: {0}")]
    SharedSource(String),
    #[error("summarizer failed for entry {key:?}: {reason}")]
    Summarizer { key: [u8; 32], reason: String },
    #[error("non-finite score for entry {key:?}")]
    NonFiniteScore { key: [u8; 32] },
    #[error("token budget exhausted before required layers could fit")]
    BudgetExhausted,
    #[error("VCG capacity too large: {capacity_units} units")]
    VcgCapacityTooLarge { capacity_units: usize },
}
```

Use warnings on `ContextResult` for recoverable degradation:

```rust
pub enum ContextWarning {
    SharedSourceUnavailable,
    ContrarianUnavailable,
    ContrarianDroppedDueToBudget,
    SummarySkippedNoClient { key: [u8; 32] },
    HistoryTruncated,
}
```

Guards that must exist:

- `max_prompt_tokens > response_reserved`
- `vcg_capacity_units <= config.max_vcg_capacity_units`
- token addition uses `checked_add` or saturating accounting plus an explicit
  final budget check
- all f64 scoring inputs are clamped and finite
- empty local store returns an empty result with warnings only if the request
  permits no-knowledge context; otherwise fail with `LocalSource`

### Tests required for the remediation

Add focused tests before broad integration tests. The minimum set:

**Source adapter and gather tests**

- Local gather converts Hamming distance to similarity using `D`.
- Shared gather calls trust computation and drops entries below
  `MIN_TRUST_THRESHOLD`.
- Contrarian gather binds `query` with `ANTI_SUBSPACE` and uses local search.
- Contrarian candidates are tagged `ContextSourceKind::Contrarian`.
- Dedup by key keeps contrarian channel metadata when the same key appears in
  local and contrarian results.
- Dedup by vector distance keeps the candidate with higher `similarity * trust`.
- Shared-source failure produces a warning and still returns local candidates.

**Ranking tests**

- Perfect factors score to `1.0` before trust multiplication.
- Missing emotional tag contributes `0.5` emotional resonance.
- Positive PAD match scores above opposite PAD match using
  `cognitive::affect::pad_similarity`.
- Non-finite trust or score is rejected.
- Sorting uses score descending, then key, then source order.

**VCG and compression tests**

- `knapsack_01` returns total value and selected indices.
- VCG payment equals externality in the three-item displacement case.
- Effective score decreases after repeated wins.
- The reciprocal amplifier `(1.0 / PAYMENT_DECAY_PER_TICK).powi(wins)` is
  monotonic increasing.
- `update_vcg_state()` decays non-winners, resets streaks, increments winners,
  and drops epsilon dust.
- Oversized VCG capacity returns `ContextError::VcgCapacityTooLarge`.
- Contrarian floor displaces the lowest aligned winner.
- Contrarian floor tries summary before declaring budget failure.
- Summarization is used only for entries with score `>= SUMMARIZE_THRESHOLD`.

**Token budget and assembly tests**

- Surgical, Focused, and Full budgets match the table above.
- A smaller caller hard cap scales allocations without giving away
  `response_reserved`.
- Required layers fail with `BudgetExhausted` when they cannot fit.
- Knowledge, anti-warning, contrarian, emotional, history, constraints, and
  output-format layers render in the required order.
- Rendered layer token sum never exceeds
  `total_prompt - response_reserved`.
- Contrarian entries are visibly labeled `[CONTRARIAN]`.
- Contradicted entries produce an `AntiKnowledgeWarnings` layer.

**Integration tests**

- One-tick assembly with mock local/shared adapters returns a typed
  `ContextResult`, non-empty `entries_used`, and valid token accounting.
- Multi-tick same-query assembly rotates winners due to retained VCG payments.
- Shared adapter outage still assembles local context with a warning.
- No contrarian candidates yields `ContrarianStatus::NoCandidates`, not a false
  floor failure.
- No summarizer client skips summary fallback with a warning instead of panic.

---

## Verification

### Source files

| File | Lines | Role |
|------|-------|------|
| `crates/hdc/core/src/context.rs` | ~855 | Entire context pipeline (single file) |
| `crates/hdc/core/src/knowledge/scoring.rs` | ~90 | Alternate scoring formula |
| `crates/hdc/core/src/knowledge/tier.rs` | ~40 | `KnowledgeTier::weight()` |
| `crates/hdc/core/src/knowledge/source.rs` | ~35 | `KnowledgeSource` (5-variant) |
| `crates/hdc/core/src/cognitive/affect.rs` | ~250 | `pad_similarity()` (exists, unused by context) |
| `crates/hdc/core/src/constants.rs` | ~30 | Shared constants |

### Running tests

```bash
# All context assembly tests (27 tests)
cargo test -p kora-hdc-core context -- --nocapture

# Specific test groups
cargo test -p kora-hdc-core gather_           # 3 tests
cargo test -p kora-hdc-core rank_             # 3 tests
cargo test -p kora-hdc-core compress_         # 3 tests
cargo test -p kora-hdc-core knapsack_01       # 5 tests
cargo test -p kora-hdc-core vcg_              # 5 tests
cargo test -p kora-hdc-core assemble_         # 3 tests
cargo test -p kora-hdc-core full_pipeline     # 2 tests
cargo test -p kora-hdc-core estimate_tokens   # 1 test
```

### Actual test names (from `crates/hdc/core/src/context.rs`)

```
gather_returns_relevant_candidates
gather_caps_at_max_candidates
gather_returns_empty_for_empty_store
rank_sorts_correctly
rank_recency_decays_with_age
rank_scores_are_positive
compress_removes_duplicates
compress_keeps_dissimilar_entries
compress_empty_is_noop
knapsack_01_basic_correctness
knapsack_01_single_item_fits
knapsack_01_single_item_too_heavy
knapsack_01_empty_items
knapsack_01_zero_capacity
knapsack_01_all_items_fit
vcg_payments_are_non_negative
vcg_truthful_pricing
vcg_empty_input
vcg_zero_capacity
vcg_single_item_no_competition
assemble_respects_token_budget
assemble_empty_candidates
assemble_zero_budget
full_pipeline_end_to_end
full_pipeline_with_duplicates
estimate_tokens_correctness
```
