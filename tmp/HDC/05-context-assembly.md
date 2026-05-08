# Dynamic Context Assembly

How agents select, rank, and compose knowledge into LLM context windows.

---

## First Principles: The Attention Bottleneck

Every cognitive system -- biological or artificial -- faces the same constraint:
the world contains far more information than any reasoning process can consider
simultaneously. A human working memory holds roughly 4-7 chunks (Miller 1956).
An LLM context window holds 4K-128K tokens. An agent's accumulated knowledge
may span millions of fragments: episodes, insights, heuristics, causal links,
strategy fragments, anti-knowledge entries. The question is always the same:
**which fragments deserve a slot in the finite window of attention?**

This is not a new problem. It is the oldest problem in cognition.

### Attention as Sparse Distributed Memory Lookup

Bricken and Pehlevan (2021), in "Attention Approximates Sparse Distributed
Memory" (NeurIPS), showed a remarkable correspondence: transformer
self-attention can be closely approximated as a lookup operation
in Kanerva's Sparse Distributed Memory (SDM) model from 1988. In SDM, a query
address activates nearby stored addresses, reads out their contents, and
superposes them -- exactly what Q/K/V attention does. The query vector Q
selects memory locations (keys K) by proximity, and the output is a weighted
combination of values V.

This correspondence (an approximation under specific data conditions, not
a strict mathematical equivalence) has a useful implication for our
architecture. When an HDC
knowledge management layer performs similarity search to assemble context for
an LLM, **it is doing the same operation the transformer will then do
internally on the assembled context**. The external retrieval and the internal
attention are the same algorithm at different scales. We are, in effect,
building a two-level attentional hierarchy: HDC retrieval selects which
knowledge enters the context window, and transformer attention then selects
which parts of that context influence the output.

### Context Assembly as Retrieval-Augmented Generation

Lewis, Perez, Piktus, et al. (2020), in "Retrieval-Augmented Generation for
Knowledge-Intensive NLP Tasks" (NeurIPS), introduced RAG: instead of relying
solely on knowledge baked into model weights, retrieve relevant documents at
inference time and inject them into the prompt. RAG has become the dominant
paradigm for grounding LLM outputs in factual, up-to-date information.

Context assembly in Roko IS a form of RAG. The Gather phase is retrieval.
The Rank phase is re-ranking. The Compress phase is context construction.
But there are three critical differences from standard RAG:

1. **HDC vectors instead of dense embeddings.** Standard RAG uses
   floating-point embedding similarity (cosine distance in R^768 or R^1536).
   Roko uses binary hyperdimensional vectors (Hamming distance in {0,1}^10240).
   This enables constant-time similarity via XOR + popcount, cross-memory-type
   retrieval (episodic, semantic, and procedural knowledge all live in the same
   vector space), and algebraic composition of queries through HDC binding.

2. **Affect-modulated retrieval.** Standard RAG retrieves by semantic
   similarity alone. Roko's four-factor scoring adds recency, importance,
   and emotional congruence. The agent's mood state actively shapes what
   knowledge is recalled -- a phenomenon well-documented in human cognition.

3. **Mandatory contrarian retrieval.** Standard RAG returns the most similar
   documents, reinforcing whatever framing the query implies. Roko
   deliberately retrieves opposing viewpoints, preventing confirmation bias
   in the assembled context.

---

## Architecture

```
         +----------------------------------------------+
         |              Context Assembler                |
         |                                              |
         |  +---------+  +----------+  +------------+  |
         |  | Gather  |->|  Rank    |->| Compress   |  |
         |  |         |  |          |  |            |  |
         |  | HDC     |  | 4-factor |  | Token      |  |
         |  | search  |  | scoring  |  | budgeting  |  |
         |  +----+----+  +----------+  +------------+  |
         |       |                                      |
         |  +----+----------------------------+         |
         |  |        Knowledge Sources         |         |
         |  |                                  |         |
         |  |  Local Index   Shared Substrate  |         |
         |  |  (private)     (on-chain)        |         |
         |  +----------------------------------+         |
         +----------------------------------------------+
                          |
                          v
                 +----------------+
                 | Prompt Builder |
                 |  (9 layers)   |
                 +----------------+
                          |
                          v
                    LLM Context
```

### Phase 1: Gather

The Gather phase performs HDC similarity search across all knowledge sources.
This is where the retrieval happens -- the "R" in RAG.

#### HDC Similarity Search Mechanics

Recall that every knowledge entry is stored as a binary hyperdimensional vector
in {0,1}^d (d=10,240 by default). To find entries relevant to a query, we
compute the **Hamming distance** between the query vector and every stored
vector. Hamming distance counts the number of bit positions where two vectors
differ:

```
d_H(x, y) = popcount(x XOR y)
```

where `popcount` counts the number of 1-bits. On modern CPUs, `XOR` and
`popcount` are single-cycle SIMD instructions. For a 10,240-bit vector stored
as 160 64-bit words, a single similarity computation takes ~160 XOR operations
and ~160 popcount operations -- under 0.5 microseconds. Searching 100,000
entries takes roughly 50 milliseconds without any indexing, and locality-
sensitive hashing can reduce this further.

The similarity is then:

```
similarity(x, y) = 1.0 - d_H(x, y) / d
```

A similarity of 1.0 means identical vectors. A similarity of 0.5 means the
vectors are uncorrelated (random chance for binary vectors). Values below 0.5
indicate anti-correlation -- the vectors are more different than chance would
predict.

The search returns the top-K entries with highest similarity. K is bounded by
`MAX_LOCAL_CANDIDATES` (typically 50-100 for local knowledge) and
`MAX_SHARED_CANDIDATES` (typically 20-50 for on-chain shared knowledge, which
carries additional trust verification overhead).

#### Contrarian Retrieval: The 15% Dissent Rule

A retrieval system that only returns confirming evidence is an echo chamber.
Roko mandates that at least 15% of retrieved candidates must be **contrarian**
-- entries that represent opposing viewpoints, dissenting evidence, or
documented failure modes.

The mechanism uses HDC's algebraic structure. Given a query vector Q, the
contrarian query is constructed by binding Q with the `ANTI_SUBSPACE` vector:

```
Q_contrarian = Q XOR ANTI_SUBSPACE
```

`ANTI_SUBSPACE` is a fixed random vector generated deterministically from a
consensus-critical seed constant (see doc 04 for the concrete generation
code: `ChaCha20Rng::seed_from_u64(ANTI_SUBSPACE_SEED)` producing a 10,240-
bit vector). It acts as a "semantic negation" operator. Binding with it
rotates the query into a different region of the hyperdimensional space --
one that is near entries tagged as anti-knowledge, counterarguments, failure
reports, and cautionary heuristics. This is possible
because when knowledge entries are created, anti-knowledge entries are
deliberately stored with their vectors bound to `ANTI_SUBSPACE`, placing them
in a complementary region.

The contrarian allocation works in two stages:

1. **Candidate-level reservation (Gather phase):** 1/7 of
   MAX_LOCAL_CANDIDATES (~15% of candidate slots) are filled with
   contrarian results. This is a **hard reservation** -- the contrarian
   search always runs and always fills these slots, regardless of how
   well the contrarian candidates score.

2. **Token-level guarantee (Compress phase):** After VCG (Vickrey-Clarke-Groves
   auction -- the mechanism that allocates context space; see the VCG section below) allocation,
   at least one contrarian entry must appear in the final context window.
   If VCG ranking would exclude all contrarian candidates, the
   lowest-scoring aligned winner is displaced to make room for the
   highest-scoring contrarian candidate. This is a **hard floor**, not
   a soft bias.

The two-stage approach means contrarian retrieval is a hard reservation
at the candidate level and a hard floor at the token level. Too little
contrarian evidence and the agent develops confirmation bias. Too much
and the signal-to-noise ratio collapses. The 15% figure is a balance:
enough to surface genuine objections, not enough to paralyze
decision-making.

Note: doc 09 describes this as "15% of the context budget is reserved."
This is imprecise -- the reservation applies to candidate slots
(Gather), not token budget (Compress). The token-level guarantee is
the "at least one contrarian entry" floor, not a percentage of tokens.

Contrarian candidates receive a moderate trust score (0.8 vs 1.0 for aligned
local knowledge) to reflect that they are intentionally adversarial.

#### Supporting Type Definitions

```rust
// WARNING: off-chain only — gather() uses f64 similarity conversion
// (1.0 - dist as f64 / 10240.0) and f64 trust scores. Context assembly
// is entirely local to each agent and does not enter the consensus path.
use ethereum_types::H256;

/// Candidate knowledge entry retrieved during the Gather phase.
#[derive(Clone, Debug)]
struct Candidate {
    entry: KnowledgeEntry,   // see doc 04 for full struct definition
    similarity: f64,         // normalized Hamming similarity to query (0.0-1.0)
    source: Source,          // Local | Shared | Contrarian
    trust: f64,              // 0.0-1.0; Local=1.0, Shared=compute_trust(), Contrarian=0.8
}

impl Candidate {
    fn provenance(&self) -> Provenance {
        Provenance {
            source: self.source.clone(),
            publisher: self.entry.publisher,
            confidence: self.entry.confidence,
            tier: self.entry.tier.clone(),
            similarity: self.similarity,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Source { Local, Shared, Contrarian }

#[derive(Clone, Debug)]
struct Provenance {
    source: Source,
    publisher: Option<H256>,
    confidence: f64,
    tier: KnowledgeTier,
    similarity: f64,
}

#[derive(Clone, Debug)]
struct TaskContext {
    task_description: String,
    deadline_ticks: Option<u64>,
    current_tick: u64,
    mode: ContextMode,
}

#[derive(Clone, Debug)]
enum ContextMode { Surgical, Focused, Full }

struct TokenBudget {
    system_allocation: usize,
    knowledge_allocation: usize,
    task_allocation: usize,
    response_allocation: usize,
}

impl TokenBudget {
    fn from_mode(mode: &ContextMode) -> Self {
        match mode {
            ContextMode::Surgical => Self { system_allocation: 500, knowledge_allocation: 1_000, task_allocation: 1_500, response_allocation: 1_000 },
            ContextMode::Focused  => Self { system_allocation: 1_000, knowledge_allocation: 4_000, task_allocation: 4_000, response_allocation: 3_000 },
            ContextMode::Full     => Self { system_allocation: 2_000, knowledge_allocation: 8_000, task_allocation: 8_000, response_allocation: 6_000 },
        }
    }
}

/// Approximate token count for a string.
/// Uses cl100k_base heuristic: ~3.5 chars/token for mixed code/prose.
/// Conservative (overestimates) to avoid exceeding budget.
fn estimate_tokens(content: &str) -> usize {
    if content.is_empty() { return 0; }
    ((content.len() as f64 / 3.5).ceil() as usize).max(1)
}

#[derive(Clone, Debug)]
enum ContextEntry {
    Full(KnowledgeEntry, Provenance),
    Summarized(String, Provenance),
}

const MAX_LOCAL_CANDIDATES: usize = 70;
const MAX_SHARED_CANDIDATES: usize = 30;
const SUMMARIZE_THRESHOLD: f64 = 0.6;
const RECENCY_DECAY: f64 = 100.0;
```

#### Gather Implementation

```rust
fn gather(&self, query: &HdcVector, context: &TaskContext) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    // Search local knowledge index
    // NOTE: search() returns Vec<(H256, u32)> where u32 is Hamming distance.
    // Convert to similarity: 1.0 - (dist as f64 / 10240.0).
    let local = self.local_index.search(query, MAX_LOCAL_CANDIDATES);
    for (key, dist) in local {
        candidates.push(Candidate {
            entry: self.local_store.get(key),
            similarity: 1.0 - dist as f64 / 10240.0,
            source: Source::Local,
            trust: 1.0,
        });
    }

    // Search shared substrate (on-chain)
    let shared = self.chain_substrate.search(query, MAX_SHARED_CANDIDATES);
    for (key, dist) in shared {
        let entry = self.chain_substrate.get(key);
        let trust = self.compute_trust(&entry, context);
        candidates.push(Candidate {
            entry,
            similarity: 1.0 - dist as f64 / 10240.0,
            source: Source::Shared,
            trust,
        });
    }

    // Mandatory contrarian retrieval (15%)
    let anti_query = query.bind(&ANTI_SUBSPACE);
    let contrarian = self.local_index.search(&anti_query, MAX_LOCAL_CANDIDATES / 7);
    for (key, dist) in contrarian {
        candidates.push(Candidate {
            entry: self.local_store.get(key),
            similarity: 1.0 - dist as f64 / 10240.0,
            source: Source::Contrarian,
            trust: 0.8, // Contrarian gets moderate trust
        });
    }

    candidates
}
```

### Phase 2: Rank

After gathering, we have a pool of ~70-150 candidates from local, shared, and
contrarian sources. Most of these will not fit in the context window. The Rank
phase assigns each candidate a composite score to determine priority.

#### Four-Factor Scoring

Park, O'Brien, Cai, Morris, Liang, and Bernstein (2023), in "Generative
Agents: Interactive Simulacra of Human Behavior" (UIST), introduced a
three-factor scoring model for memory retrieval in simulated agents: recency,
importance, and relevance. Their agents used this to decide what past
experiences to recall when interacting with other agents, and the results were
strikingly human-like -- agents formed social relationships, remembered grudges,
and planned surprise parties, all driven by this retrieval formula.

Roko extends their model with a fourth factor: **emotional congruence**. The
full scoring formula is:

```
score = w_recency    * recency_score      (w = 0.20)
      + w_importance * importance_score    (w = 0.25)
      + w_relevance  * relevance_score     (w = 0.35)
      + w_emotional  * emotional_score     (w = 0.20)
```

The weights sum to 1.0. Relevance has the highest weight (0.35) because
task-relevance is the most critical factor -- an entry that is recent,
important, and emotionally congruent but irrelevant to the task at hand is
worse than useless; it wastes context tokens.

#### Factor 1: Recency (w = 0.20)

```
recency = exp(-(current_tick - entry.last_accessed) / RECENCY_DECAY)
```

The recency factor implements an exponential decay, directly inspired by
Ebbinghaus's forgetting curve (1885). Ebbinghaus demonstrated that memory
retention decays exponentially with time: R = e^(-t/S), where S is the
stability of the memory. More recently accessed or created knowledge scores
higher because temporal proximity correlates with contextual relevance -- if
you looked something up five minutes ago, you are likely still working on the
same problem.

The `RECENCY_DECAY` constant controls the half-life of the decay. A value of
100 ticks means knowledge accessed 100 ticks ago scores approximately 0.37
(1/e), while knowledge accessed 10 ticks ago scores approximately 0.90. This
is tunable per agent and per task type -- urgent tasks benefit from faster
decay (emphasize very recent knowledge), while research tasks benefit from
slower decay (cast a wider temporal net).

Note that `last_accessed` is updated on both creation and retrieval, so
frequently recalled knowledge naturally stays "warm" -- a form of implicit
spaced repetition.

#### Factor 2: Importance (w = 0.25)

```
importance = entry.confidence * entry.tier_weight * ln(entry.confirmation_count + 1)
```

Importance measures the intrinsic value of a knowledge fragment, independent
of the current task. It is a product of three sub-factors:

- **Confidence** (0.0 to 1.0): How certain is the agent that this knowledge
  is correct? New observations start at ~0.5. Confirmed predictions raise it.
  Contradicting evidence lowers it. See the trust pipeline (document 04) for
  how confidence is maintained.

- **Tier weight**: Knowledge in higher tiers (Axiom, Bedrock) gets a larger
  tier weight than lower tiers (Provisional, Liminal). An Axiom-tier heuristic
  like "never invest more than can be lost" has a tier weight of 1.0; a
  Liminal-tier speculation has a tier weight of 0.2.

- **ln(confirmation_count + 1)**: The logarithmic scaling prevents a
  knowledge entry that has been confirmed 1,000 times from having 1,000x
  the importance of a singly-confirmed entry. The natural log ensures
  diminishing returns: ln(1) = 0, ln(2) ~= 0.69, ln(5) ~= 1.61,
  ln(65) ~= 4.17, ln(1025) ~= 6.93. The +1 avoids ln(0) for
  unconfirmed entries. This prevents popular-but-shallow knowledge from
  dominating.

  Note: doc 09 uses `ln(confirmation_count + 1)` (natural log) in
  both the formula table and the Rust implementation. This document
  uses the same convention for consistency.

#### Factor 3: Relevance (w = 0.35)

```
relevance = entry.similarity  // Already computed by HDC search
```

Relevance is the HDC similarity score computed during the Gather phase. It
carries the highest weight because the primary purpose of context assembly is
to provide the LLM with knowledge that is **directly applicable to the current
task**. A perfectly relevant entry with low recency and moderate importance is
more useful than a recent, important entry that has nothing to do with the
question at hand.

This is also the only factor that is fully determined by the query content.
The other three factors are properties of the entry (recency, importance) or
the agent's state (emotional congruence). Relevance is the bridge between
"what does the agent know?" and "what does the agent need to know right now?"

#### Factor 4: Emotional Congruence (w = 0.20)

```
emotional = pad_similarity(entry.emotional_tag, agent.current_mood)
```

This factor implements mood-congruent retrieval, a phenomenon extensively
documented by Bower (1981) in "Mood and Memory" (American Psychologist).
Bower demonstrated through a series of experiments that people in a happy mood
more easily recall happy memories, and people in a sad mood more easily recall
sad memories. This is not a bug -- it is a feature of biological cognition.
Mood-congruent retrieval ensures that the agent's emotional state is reflected
in its knowledge access patterns, producing more coherent behavior.

The PAD (Pleasure-Arousal-Dominance) model, introduced by Mehrabian and
Russell (1974) in "An Approach to Environmental Psychology," provides the
dimensional framework. Each knowledge entry can be optionally tagged with a
PAD vector recording the emotional context in which it was created or is most
applicable. The emotional score is the PAD-space proximity between the entry's
tag and the agent's current mood.

The PAD dimensions influence retrieval as follows:

| PAD State | Retrieval Bias | Rationale |
|-----------|---------------|-----------|
| High P, Low A | Exploit: retrieve proven strategies, known-good patterns | Contentment favors reliability over novelty |
| Low P, High A | Explore: retrieve novel/contrarian knowledge, surface warnings | Distress + arousal triggers vigilance |
| High D | Retrieve action plans, strategies, offensive plays | Sense of control favors agentic orientation |
| Low D | Retrieve heuristics, established patterns, conservative rules | Low control favors established wisdom |
| Low P (general) | Retrieve warnings, anti-knowledge, failure records | Negative valence triggers protective cognition |
| High A | Narrow retrieval, fewer results, higher threshold | Arousal produces focus and urgency |
| Low A | Broad retrieval, more results, lower threshold | Calm permits wider exploration |

**When is the emotional tag assigned?** The PAD vector is captured at
knowledge creation time. When the agent creates a new knowledge entry
(Insight, Heuristic, CausalLink, etc.), the current mood state from the
ALMA (A Layered Model of Affect; Gebhard 2005) affect system is snapshotted
and stored as the entry's `emotional_tag`:

```rust
fn create_knowledge_entry(content: &Content, agent: &AgentState) -> KnowledgeEntry {
    KnowledgeEntry {
        // ... other fields ...
        emotional_tag: Some(agent.affect.current_mood().clone()),
        // The tag is the agent's PAD state at the moment of creation.
        // It is immutable after creation -- the tag records the emotional
        // *context of discovery*, not the current emotional relevance.
    }
}
```

Entries imported from the shared substrate inherit the publishing agent's
emotional tag (if present) or receive `None`. Entries without emotional
tags receive a neutral score of 0.5 -- they are neither boosted nor
penalized by the emotional factor.

**Empirical validation note (2025-2026).** The design decisions in this
scoring factor -- PAD-based emotional tagging, mood-congruent retrieval
bias, and persistent affect state -- have received direct empirical
validation from three independent studies. Sun et al. (2026,
arXiv:2604.00005) demonstrated via representation-level intervention
(E-STEER) that emotional states measurably alter LLM reasoning capability
(up to 33.1% higher answer validity at positive valence) and safety
(52.7-68.3% risk reduction), with inverted-U (Yerkes-Dodson) curves
confirming that moderate arousal helps while excessive arousal hurts --
validating that the arousal dimension should modulate retrieval bias
strength. Sentipolis (arXiv:2601.18027, 2026) showed that agents WITHOUT
persistent PAD state suffer "emotional amnesia," losing long-horizon
affective continuity, while persistent PAD with dual-speed dynamics improved
emotional continuity by +222% to +315.6% (GPT-5.2) and communication
quality by +48% to +70.1%. Ma et al. (2025, arXiv:2510.13195) validated
PAD-mapped emotion in agent decision architectures, showing DTW-confirmed
state-desire-behavior coherence that outperformed vanilla GPT-4o agents.
These findings confirm that mood-congruent retrieval is not anthropomorphic
decoration but a measurable performance driver. See document 08 (Cognitive
Architecture), section "Empirical Validation (2025-2026)" for full analysis.

#### `compute_score()` Implementation

The four factors are combined via weighted linear combination. All weights
sum to 1.0. The tier_weight mapping is:

| Tier | tier_weight |
|------|------------|
| Transient | 0.2 |
| Working | 0.4 |
| Consolidated | 0.7 |
| Persistent | 1.0 |

```rust
impl KnowledgeTier {
    fn weight(&self) -> f64 {
        match self {
            KnowledgeTier::Transient    => 0.2,
            KnowledgeTier::Working      => 0.4,
            KnowledgeTier::Consolidated => 0.7,
            KnowledgeTier::Persistent   => 1.0,
        }
    }
}

/// Compute the four-factor composite score for ranking.
///
/// score = w1*recency + w2*importance + w3*relevance + w4*emotional
///
/// Weights: recency=0.20, importance=0.25, relevance=0.35, emotional=0.20
/// All factors are in [0, 1] (importance is normalized by dividing by a
/// maximum expected value).
fn compute_score(
    candidate: &Candidate,
    current_tick: u64,
    current_mood: &PadState,
) -> f64 {
    const W_RECENCY: f64    = 0.20;
    const W_IMPORTANCE: f64 = 0.25;
    const W_RELEVANCE: f64  = 0.35;
    const W_EMOTIONAL: f64  = 0.20;

    // Factor 1: Recency (exponential decay)
    let age = (current_tick - candidate.entry.last_reinforced) as f64;
    let recency = (-age / RECENCY_DECAY).exp();

    // Factor 2: Importance (confidence * tier_weight * log confirmations)
    // Normalize by dividing by max expected value (~7.0, corresponding
    // to confidence=1.0, tier_weight=1.0, ln(1025)=6.93).
    const IMPORTANCE_NORMALIZER: f64 = 7.0;
    let raw_importance = candidate.entry.confidence
        * candidate.entry.tier.weight()
        * (candidate.entry.confirmation_count as f64 + 1.0).ln();
    let importance = (raw_importance / IMPORTANCE_NORMALIZER).min(1.0);

    // Factor 3: Relevance (HDC similarity, already in [0,1])
    let relevance = candidate.similarity;

    // Factor 4: Emotional congruence
    let emotional = match &candidate.entry.emotional_tag {
        Some(tag) => pad_similarity(tag, current_mood),
        None => 0.5, // neutral: no boost or penalty
    };

    W_RECENCY * recency
        + W_IMPORTANCE * importance
        + W_RELEVANCE * relevance
        + W_EMOTIONAL * emotional
}
```

### Phase 3: Compress

Token budgeting -- fit the ranked candidates into the context window:

```rust
/// A ranked candidate carries the original Candidate plus its
/// composite four-factor score (computed in the Rank phase).
struct RankedCandidate {
    candidate: Candidate,
    score: f64,  // composite four-factor score
}

fn compress(
    &self,
    ranked: &[RankedCandidate],
    budget: &TokenBudget,
) -> Vec<ContextEntry> {
    let mut result = Vec::new();
    let mut tokens_used = 0;

    for rc in ranked {
        let tokens = estimate_tokens(&rc.candidate.entry.content);
        let remaining = budget.knowledge_allocation - tokens_used;

        if tokens_used + tokens > budget.knowledge_allocation {
            // Try summarization for high-value entries
            if rc.score > SUMMARIZE_THRESHOLD {
                let summary = summarize(&rc.candidate.entry.content, remaining);
                let summary_tokens = estimate_tokens(&summary);
                if tokens_used + summary_tokens <= budget.knowledge_allocation {
                    result.push(ContextEntry::Summarized(
                        summary,
                        rc.candidate.provenance(),
                    ));
                    tokens_used += summary_tokens;
                }
            }
            continue;
        }

        result.push(ContextEntry::Full(
            rc.candidate.entry.clone(),
            rc.candidate.provenance(),
        ));
        tokens_used += tokens;
    }

    result
}
```

Token budgets (from roko spec):

| Mode | Total | System | Knowledge | Task | Response |
|------|-------|--------|-----------|------|----------|
| Surgical | 4K | 500 | 1K | 1.5K | 1K |
| Focused | 12K | 1K | 4K | 4K | 3K |
| Full | 24K | 2K | 8K | 8K | 6K |

The four budget columns map to the 9-layer prompt as follows:
- **System** covers Layers 1-2 (Identity + Capabilities) and Layer 8
  (Emotional State). These are small, mostly static.
- **Knowledge** covers Layer 5 (the HDC context assembly output). This is
  the budget the Compress phase and VCG auction operate within.
- **Task** covers Layers 3-4 (World State + Task Context), Layer 6
  (Constraints), and Layer 7 (History).
- **Response** is reserved for the LLM's output tokens (Layer 9 specifies
  the format but consumes negligible input tokens; the Response budget is
  output-side).

The mode is selected automatically based on task complexity. Simple queries
(single-step, low ambiguity) use Surgical mode to minimize latency and cost.
Multi-step reasoning tasks with ambiguity use Full mode to provide the LLM
with maximum context. The Compress phase also applies a summarization
fallback: if a high-scoring candidate exceeds the remaining token budget, it
is summarized to fit rather than dropped entirely. Only candidates above the
`SUMMARIZE_THRESHOLD` score qualify for this treatment -- low-scoring
candidates are simply discarded.

#### Summarization Mechanism

The `summarize()` function called in the Compress phase is an LLM call, not
a heuristic truncation. It uses the same model backing the agent (or a
cheaper/faster model if configured) with a dedicated summarization prompt:

```rust
/// Summarize a knowledge entry to fit within a target token count.
/// Uses a lightweight LLM call with a system prompt optimized for
/// faithful compression.
fn summarize(&self, content: &str, max_tokens: usize) -> String {
    let prompt = format!(
        "Summarize the following knowledge entry in at most {} tokens. \
         Preserve all factual claims, numerical values, and causal \
         relationships. Do not add interpretation.\n\n{}",
        max_tokens, content
    );
    // Uses the agent's configured summarization model (default: same
    // model as the main cognitive loop, but operators can set a
    // cheaper model via `summarization_model` in agent config).
    self.summarization_client.complete(&prompt, max_tokens)
}
```

`SUMMARIZE_THRESHOLD` defaults to **0.6** (on the 0.0-1.0 composite score
scale). Entries scoring below 0.6 are dropped rather than summarized,
because the LLM call cost is not justified for low-value entries. This
threshold is configurable per agent.

#### Budget Exhaustion: When Nothing Fits

If the knowledge allocation is exhausted before any candidates fit (e.g.,
Surgical mode with only 1K tokens for knowledge, but the top candidate is
a 2K-token entry), the Compress phase applies escalating fallbacks:

1. **Summarize the top entry** to fit the remaining budget (even if that
   means a very aggressive summary).
2. **If summarization still exceeds the budget** (entry cannot be
   meaningfully compressed below ~50 tokens), emit a **budget warning**
   in the assembled prompt: `[CONTEXT NOTE: Knowledge budget exhausted.
   Top-scoring entry (score={score}, tokens={tokens}) could not be
   included. Consider escalating to a larger context mode.]`
3. **If the task was using Surgical or Focused mode**, the assembler
   returns a `ModeEscalation` signal to the cognitive loop, recommending
   a retry with the next larger mode. The cognitive loop may accept or
   reject this based on its own cost/latency constraints.

The system never silently produces an empty knowledge layer. Either
knowledge is included (full or summarized), or the prompt explicitly
states that knowledge was unavailable and why.

### Context Rot: Why Length Kills Performance

Larger context windows are marketed as a feature. The empirical evidence says
they are a trap. In 2025, Chroma published the most comprehensive study of
long-context degradation to date, testing **18 frontier models** -- Claude
Opus 4, Claude Sonnet 4, Claude Sonnet 3.7, Claude Sonnet 3.5, Claude
Haiku 3.5, GPT-4.1, GPT-4.1 mini, GPT-4.1 nano, GPT-4o, GPT-4 Turbo,
GPT-3.5 Turbo, o3, Gemini 2.5 Pro, Gemini 2.5 Flash, Gemini 2.0 Flash,
Qwen3-235B-A22B, Qwen3-32B, and Qwen3-8B. The finding was unequivocal:
**every model degrades at every increment of input length**. There are no
exceptions. Rot begins well before the context window is full -- a model with
a 200K-token window can exhibit significant degradation at 50K tokens.

Three compounding mechanisms drive context rot:

1. **Lost-in-the-middle** (Liu et al., Stanford/TACL 2024). Models attend
   well to content at the beginning and end of the context window but poorly
   to content in the middle, causing 30%+ accuracy drops. Position 1 (start)
   yields ~75% accuracy on multi-document QA; positions 5-15 (middle) drop
   to ~45-55%; position 20 (end) recovers to ~72%. Knowledge placed in the
   attention trough may as well not exist.

2. **Attention dilution** (quadratic scaling). Transformer self-attention
   computes pairwise relationships across all tokens: 10K tokens = 100M
   pairs; 100K tokens = 10B pairs; 1M tokens = 1T pairs. Softmax
   normalization proportionally reduces each token's attention weight as
   context grows. A 50-line function receives full attention signal in a 4K
   context; the same function in a 128K context receives ~3% of that signal.
   The noise floor rises; the model does not get "smarter" with more tokens.

3. **Distractor interference**. Semantically similar but irrelevant content
   actively misleads the model. Chroma found that four distractors degrade
   performance more than one, and that structured, well-organized codebases
   produce more plausible (and therefore more damaging) distractors than
   random text. Shuffled (incoherent) haystacks actually outperformed
   logically structured ones -- coherent structure makes distractors more
   convincing to the attention mechanism.

These three mechanisms compound: distractor interference injects noise,
attention dilution spreads the model's attention across that noise, and
lost-in-the-middle ensures that even genuine signal placed in the wrong
position gets ignored.

#### This Is Not a Retrieval Problem

Du et al. (2025), in "Context Length Alone Hurts LLM Performance Despite
Perfect Retrieval" (arXiv:2510.05381), provided the critical follow-up. They
demonstrated that context rot is **not fixable by better retrieval**. Their
experimental design was elegant: they replaced all non-needle tokens in the
input with blank spaces, eliminating distractor content entirely. Under
standard assumptions, the needle should be trivially obvious. It was not.
Models still degraded -- at least 7% at 30K space tokens for Llama on GSM8K,
and 48% decline on Variable Summation despite zero distraction.

In their most revealing experiment, they masked all irrelevant tokens during
attention computation, forcing models to attend **only** to evidence and
questions. Performance still degraded by 7.9%-50%. Their conclusion is
stark: "the sheer length of the input alone can hurt LLM performance,
independent of retrieval quality and without any distraction."

Across five LLMs (Llama-3.1-8B, Mistral-v0.3-7B, GPT-4o, Claude-3.5-Sonnet,
Gemini-2.0), performance drops ranged from 13.9% to 85% as input length
increased, even when models achieved 100% exact-match retrieval of all
relevant evidence. This is an **architectural property of transformer-based
attention**, not a capability gap that training or better retrieval can solve.

#### The U-Shape Is Context-Dependent

Veseli et al. (2025), in "Positional Biases Shift as Inputs Approach Context
Window Limits" (arXiv:2508.07479, COLM 2025), refined the lost-in-the-middle
finding with a crucial nuance. Using **relative input lengths** (proportion of
the model's context window used, rather than absolute token counts), they
discovered that the U-shaped attention curve -- high attention at start and
end, low in the middle -- **only persists when context occupies less than 50%
of the window capacity**.

Above 50% capacity, the pattern shifts: primacy bias (attending to the
beginning) weakens while recency bias (attending to the end) remains stable.
The U-shape flattens into a recency-dominated curve. This means that for
models running near capacity, information placed at the beginning of the
context is progressively disadvantaged -- only content near the end receives
reliable attention.

The methodological insight is equally important: existing benchmarks that
compare models using the same absolute input lengths (e.g., "test all models
at 32K tokens") create misleading comparisons, because a 32K input is 25%
capacity for a 128K-window model but 100% capacity for a 32K-window model.
The positional biases are fundamentally different in these two regimes.

#### Implications for Roko's Token Budgeting

Context rot validates Roko's tight token budgeting approach as not just
efficient but **essential**. The three operating modes -- Surgical (4K),
Focused (12K), Full (24K) -- are deliberately conservative. Even Full mode
uses only 24K tokens, well below the 128K-200K windows available in frontier
models. This is not a limitation; it is a defense against context rot.

The VCG auction that selects high-value entries for limited space is not
merely an optimization -- it is a **survival mechanism**. Every additional
token of low-value context actively degrades the model's ability to use the
high-value context. The auction's externality pricing directly models this:
a large entry that displaces smaller entries pays a cost proportional to the
damage it inflicts on total context quality. Du et al.'s findings mean this
damage is even greater than the displacement cost alone -- the mere presence
of additional tokens, even blank ones, reduces performance.

The practical directive is clear: **never stuff the context window**. A
Surgical 4K prompt with precisely selected, high-confidence knowledge will
outperform a Full 24K prompt packed with marginally relevant material. The
mode selection should err toward smaller windows, escalating to larger modes
only when task complexity genuinely demands more knowledge entries -- not
because more space is available.

---

## The 9-Layer Prompt Builder

Roko's PromptAssemblyService builds prompts in 9 layers. The layer ordering
is deliberate and informed by empirical findings on how LLMs process long
contexts (see the "Lost in the Middle" discussion under VCG below). Critical
framing information (identity, capabilities) comes first; the task and
retrieved knowledge occupy the middle; constraints, history, and output format
anchor the end -- the two positions where LLM attention is highest.

### Layer 1: Identity

Agent name, role descriptor, and core directives. This is the "system prompt"
foundation. It establishes who the agent is and what it fundamentally does.
Example: "You are Roko-7, a DeFi analysis agent specializing in yield
optimization. You operate with caution-first principles."

This layer is static across invocations for a given agent -- it changes only
when the agent's role is reconfigured.

### Layer 2: Capabilities

An enumeration of the tools, contracts, and APIs the agent can invoke. This
includes smart contract ABIs, available oracle feeds, permitted action types,
and rate limits. The LLM needs this to know what actions are physically
possible in its response.

Example capabilities: "You can call swap() on Uniswap V3, query price from
Chainlink oracles, submit proposals to the DAO governance contract."

### Layer 3: World State

Current blockchain state relevant to the agent's task: block number, gas
prices, token balances, position sizes, oracle prices, governance proposal
status. This is fetched fresh for each context assembly cycle.

This layer is critical for grounding -- without it, the LLM might reason
about market conditions that no longer hold. It is typically 200-500 tokens
and is never summarized or compressed.

### Layer 4: Task Context

The specific task being worked on: what triggered this cognitive cycle, what
the objective is, what constraints apply, and what intermediate results exist.
For a multi-step task, this includes the current step and the results of
previous steps.

This layer bridges the general (who am I, what can I do) with the specific
(what am I doing right now). It is constructed by the task manager, not by the
context assembler.

### Layer 5: Knowledge

**This is where HDC context assembly inserts its output.** The ranked,
compressed knowledge fragments from the Gather-Rank-Compress pipeline. Each
fragment includes provenance metadata (source, confidence, tier, last
confirmation tick) so the LLM can assess reliability.

Knowledge fragments are presented in descending score order. Contrarian entries
are explicitly marked as such (e.g., "[CONTRARIAN] Historical data suggests
this strategy fails in low-liquidity environments") so the LLM can weigh them
appropriately rather than treating them as consensus.

### Layer 6: Constraints

Rules, limitations, and anti-patterns to avoid. This includes:
- Hard constraints: "Never exceed 10% portfolio allocation to a single asset"
- Soft constraints: "Prefer established pools over new ones"
- Anti-patterns: "Do not chase yield above 50% APY without manual review"
- Regulatory constraints: jurisdiction-specific rules

Constraints are placed in Layer 6 (after Knowledge) because they serve as a
filter on the knowledge the LLM has just ingested. The ordering says: "here
is what you know; now here is what you must not do with that knowledge."

### Layer 7: History

Recent interaction history, compressed. This includes the agent's recent
actions, their outcomes, and any relevant conversation context. History is
aggressively summarized to stay within budget -- full transcripts would
quickly exhaust the token allocation.

The compression strategy prioritizes recent actions (last 5-10), outcomes of
those actions (success/failure/pending), and any error messages or unexpected
results. Older history is summarized into one-line entries.

### Layer 8: Emotional State

Current PAD values and the resulting behavioral mode. Example: "Current
affect: P=0.3, A=0.7, D=0.4. Behavioral mode: CAUTIOUS. Elevated arousal
suggests heightened vigilance -- prioritize risk assessment."

This layer is unusual -- most LLM agent frameworks do not include emotional
state in prompts. Its purpose is to make the LLM's behavior consistent with
the agent's affect-modulated knowledge retrieval. If the retrieval is biased
toward caution (low P, high A), the emotional state layer ensures the LLM
knows why those cautionary fragments were surfaced and should respond
accordingly.

### Layer 9: Output Format

Expected response structure: JSON schema, required fields, chain of thought
format, or free-text with specific headings. This layer is placed last because
it is the final instruction the LLM sees before generating, and recency bias
in attention means the LLM is most likely to comply with formatting directives
placed at the end.

---

## VCG Attention Auction

### The Allocation Problem

After Rank, we have ~70-150 scored candidates. After Compress, we can fit
maybe 15-30 of them (depending on mode and entry size). The naive approach --
greedy fill by descending score -- works, but it has a subtle failure mode:
**monopolization**. A few large, high-scoring entries can consume most of the
context window, crowding out many smaller entries that collectively provide
more diverse coverage. Across multiple rounds, the same entries tend to win
repeatedly, creating a "context bubble" where the agent sees the same
knowledge over and over.

This is a classic resource allocation problem, and mechanism design theory
provides an elegant solution: the Vickrey-Clarke-Groves (VCG) auction.

### Theoretical Foundation

The VCG mechanism is the product of three foundational papers:

- **Vickrey (1961).** "Counterspeculation, Auctions, and Competitive Sealed
  Tenders." *Journal of Finance.* Introduced the second-price sealed-bid
  auction: the highest bidder wins but pays the second-highest bid. This
  makes truthful bidding a dominant strategy -- you never benefit from
  misrepresenting your valuation.

- **Clarke (1971).** "Multipart Pricing of Public Goods." *Public Choice.*
  Generalized Vickrey's insight to multi-item settings where agents have
  valuations over combinations of goods.

- **Groves (1973).** "Incentives in Teams." *Econometrica*, 41(4), 617-631.
  Proved that the Vickrey-Clarke payment rule is sufficient to guarantee
  truthful revelation and efficient allocation **for agents with
  quasi-linear utilities** (i.e., utility = value - payment, where payment
  is transferable and enters linearly). **Green and Laffont (1977)** then
  proved the converse: the Groves payment scheme is the ONLY mechanism
  that simultaneously satisfies these three properties under unrestricted
  quasi-linear domains:
  - **Truthfulness** (strategyproofness): each participant's optimal strategy
    is to report its true value. No gaming, no strategic manipulation.
  - **Allocative efficiency**: the mechanism maximizes total social welfare
    (sum of values of allocated items).
  - **Individual rationality**: no participant is made worse off by
    participating.

  The quasi-linear restriction is satisfied in our setting because each
  knowledge fragment's "value" (composite score) and "payment" (priority
  decay) are both scalar quantities that combine linearly. Outside
  quasi-linear domains (e.g., when agents have budget constraints or
  interdependent valuations), VCG uniqueness does not hold.

These three properties are exactly what we want for context allocation:
knowledge fragments should not need to "game" their scores, the total value
of the assembled context should be maximized, and no entry should be
systematically disadvantaged by the mechanism.

### How It Works in Context Assembly

Each knowledge fragment "bids" its value (the four-factor composite score).
The auction allocates context space to maximize total value:

```
1. Sort candidates by score/token ratio (value density)
2. Greedily fill context window with highest-density candidates
3. For each winner, compute payment = externality imposed on others
   (VCG payment = value of second-best allocation minus value of
    allocation excluding this candidate)
4. Payment reduces the candidate's effective priority for next round
```

The VCG payment for a winning candidate i is:

```
payment_i = (total value of optimal allocation without i)
          - (total value of optimal allocation with i, minus i's own value)
```

In plain English: how much does candidate i's inclusion cost everyone else?
If including a 500-token entry displaces three 150-token entries whose
combined score exceeds the large entry's score, the large entry pays a high
externality cost. If including a small entry displaces nothing (it fits in
leftover space), it pays zero.

### Computational Tractability

VCG in the general combinatorial auction setting is NP-hard. But our setting
is far simpler: we are solving a **0/1 knapsack problem** (maximize total
value subject to a token budget constraint). The knapsack problem admits a
pseudo-polynomial dynamic programming solution with complexity:

```
O(n * W)
```

where n = number of candidates and W = budget capacity in discrete units
(e.g., budget in units of 10 tokens, so W = 800 for an 8K allocation).
With n ~ 100 candidates and W ~ 800, a single knapsack solve takes well
under a millisecond on modern hardware.

VCG payments require running the knapsack solver **once** with all
candidates to find the optimal allocation, then **once more per winner**
excluding that winner to compute its externality. Total complexity:

```
O((1 + n_winners) * n * W)
```

With n_winners ~ 25, this is ~26 knapsack solves at O(100 * 800) each
-- roughly 2 million DP cell evaluations total, comfortably single-digit
milliseconds. This is negligible compared to LLM inference latency
(typically 1-10 seconds), so the auction adds no perceptible overhead.

**Pseudocode for the full VCG knapsack allocation:**

```rust
fn vcg_allocate(candidates: &[ScoredEntry], budget_tokens: usize) -> Vec<VcgWinner> {
    let unit = 10; // DP granularity: 10 tokens per unit
    let W = budget_tokens / unit;

    // Step 1: Solve 0/1 knapsack with all candidates.
    let (optimal_value, winners) = knapsack_01(candidates, W, unit);

    // Step 2: For each winner, compute its VCG payment (externality).
    let mut results = Vec::new();
    for &winner_idx in &winners {
        // Re-solve knapsack excluding this winner.
        let others: Vec<ScoredEntry> = candidates.iter()
            .enumerate()
            .filter(|(i, _)| *i != winner_idx)
            .map(|(_, e)| e.clone())
            .collect();
        let (value_without, _) = knapsack_01(&others, W, unit);

        // VCG payment = how much value others lost because of this winner.
        // = (optimal value of others without winner) - (value others get
        //    in the allocation that includes winner)
        let others_value_with = optimal_value - candidates[winner_idx].score;
        let payment = value_without - others_value_with;

        results.push(VcgWinner {
            entry: candidates[winner_idx].clone(),
            payment: payment.max(0.0), // payment >= 0 by VCG properties
        });
    }
    results
}

/// Standard 0/1 knapsack via dynamic programming.
/// Returns (total_value, indices_of_selected_items).
fn knapsack_01(items: &[ScoredEntry], capacity: usize, unit: usize) -> (f64, Vec<usize>) {
    let n = items.len();
    // dp[w] = best value achievable with capacity w
    let mut dp = vec![0.0f64; capacity + 1];
    let mut choice = vec![vec![false; capacity + 1]; n];

    for i in 0..n {
        let w_i = (items[i].token_count + unit - 1) / unit; // ceiling division
        for w in (w_i..=capacity).rev() {
            let with = dp[w - w_i] + items[i].score;
            if with > dp[w] {
                dp[w] = with;
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
            w -= (items[i].token_count + unit - 1) / unit;
        }
    }
    (dp[capacity], selected)
}
```

### The "Payment" as Priority Decay

In a traditional auction, payment is monetary. Here, the "payment" is a
**priority adjustment** applied to the winning entry's effective score for
subsequent rounds. If an entry wins with a high externality (displacing many
other useful entries), its priority is reduced for the next context assembly
cycle. This creates a natural rotation effect: high-value entries still win
often, but they cannot monopolize the context window indefinitely. Entries
that barely won (low externality) retain their full priority.

This priority decay also addresses the temporal diversity problem. Without it,
an agent reasoning about a complex task over 20 ticks might see the same top-
ranked knowledge fragments in every single context window. With VCG priority
decay, the agent sees its best knowledge in the first few ticks, then
progressively surfaces secondary and tertiary knowledge, creating a breadth-
first exploration of its knowledge base over time.

#### Implementation Details for VCG Payment Persistence

The VCG payment mechanism requires state that persists across context assembly
rounds. Here is the concrete specification:

```rust
/// Per-entry state maintained by the ContextAssembler across ticks.
struct VcgEntryState {
    /// Accumulated VCG payment from prior rounds. Decays each tick.
    accumulated_payment: f64,
    /// Number of consecutive rounds this entry has won a context slot.
    consecutive_wins: u32,
}

impl ContextAssembler {
    /// Compute the effective score for VCG bidding. The raw four-factor
    /// score is reduced by accumulated prior payments (with decay).
    fn effective_score(&self, entry_key: &EntryKey, raw_score: f64) -> f64 {
        let state = self.vcg_state.get(entry_key).unwrap_or(&DEFAULT_STATE);
        // Consecutive-win amplification: more consecutive wins -> larger
        // penalty. Using reciprocal of decay so the multiplier exceeds 1.0:
        // 0 wins -> 1.0x, 1 win -> 1.25x, 5 wins -> ~3.05x.
        let amplifier = (1.0 / PAYMENT_DECAY_PER_TICK).powi(state.consecutive_wins as i32);
        let penalty = state.accumulated_payment * amplifier;
        (raw_score - penalty).max(0.0)
    }

    /// After each round, update VCG state for all winners.
    fn update_vcg_state(&mut self, winners: &[(EntryKey, f64)]) {
        // Decay all existing payments by PAYMENT_DECAY_PER_TICK
        for state in self.vcg_state.values_mut() {
            state.accumulated_payment *= PAYMENT_DECAY_PER_TICK;
            if state.accumulated_payment < PAYMENT_EPSILON {
                state.accumulated_payment = 0.0;
            }
            state.consecutive_wins = 0;
        }
        // Apply new payments to winners
        for (key, payment) in winners {
            let state = self.vcg_state.entry(*key).or_insert(DEFAULT_STATE);
            state.accumulated_payment += payment;
            state.consecutive_wins += 1;
        }
    }
}

/// PAYMENT_DECAY_PER_TICK: 0.8 -- each tick, prior payments lose 20%
/// of their value. After 5 ticks with no wins, a payment of 1.0
/// decays to 0.33. After 10 ticks, 0.11. This ensures priority
/// penalties are temporary, not permanent.
const PAYMENT_DECAY_PER_TICK: f64 = 0.8;

/// PAYMENT_EPSILON: 1e-6 -- payments below this are zeroed to avoid
/// accumulating floating-point dust.
const PAYMENT_EPSILON: f64 = 1e-6;
```

The key design decisions:

1. **Payments are additive, not multiplicative.** A payment of 0.3 on a
   score of 0.85 reduces the effective bid to 0.55, not to 0.85 * 0.7.
   This prevents high-value entries from being permanently suppressed.

2. **Payments decay exponentially per tick.** An entry that loses one round
   (does not win a context slot) recovers 20% of its penalty per tick.
   After ~10 ticks without winning, the penalty is negligible.

3. **Consecutive-win amplification.** The reciprocal of the decay factor
   (1/0.8 = 1.25) is raised to the power of consecutive wins, so the
   penalty multiplier grows with each consecutive win: 1x at 0 wins,
   1.25x at 1 win, ~3.05x at 5 wins. An entry winning 5 rounds in a row
   faces a ~3x amplified penalty compared to a first-time winner. This
   specifically targets the monopolization problem.

4. **VCG state is per-assembler, not per-entry.** The `vcg_state` map lives
   on the `ContextAssembler` struct, not on `KnowledgeEntry`. This keeps
   the knowledge store free of assembly-specific concerns and allows
   different assemblers (e.g., a sub-agent) to maintain independent
   priority states.

### VCG Knapsack Implementation

The following pseudocode specifies the VCG allocation precisely enough for
implementation:

```rust
/// Solve 0/1 knapsack via dynamic programming. Returns the set of
/// selected candidate indices and the total value.
fn knapsack_solve(
    candidates: &[Candidate],
    capacity: usize, // in token units (e.g., tokens / 10)
) -> (Vec<usize>, f64) {
    let n = candidates.len();
    // dp[i][w] = max value using candidates[0..i] with capacity w
    let mut dp = vec![vec![0.0f64; capacity + 1]; n + 1];

    for i in 1..=n {
        let weight = candidates[i - 1].token_cost; // in same units as capacity
        let value = candidates[i - 1].effective_score;
        for w in 0..=capacity {
            dp[i][w] = dp[i - 1][w]; // skip
            if weight <= w {
                dp[i][w] = dp[i][w].max(dp[i - 1][w - weight] + value);
            }
        }
    }

    // Backtrace to find selected items
    let mut selected = Vec::new();
    let mut w = capacity;
    for i in (1..=n).rev() {
        if dp[i][w] != dp[i - 1][w] {
            selected.push(i - 1);
            w -= candidates[i - 1].token_cost;
        }
    }
    (selected, dp[n][capacity])
}

/// VCG allocation: solve knapsack, then compute externality payments.
fn vcg_allocate(
    candidates: &[Candidate],
    capacity: usize,
) -> Vec<(usize, f64)> {  // (candidate index, vcg payment)
    let (winners, total_value) = knapsack_solve(candidates, capacity);

    let mut results = Vec::new();
    for &winner_idx in &winners {
        // Solve knapsack excluding this winner
        let excluded: Vec<Candidate> = candidates.iter()
            .enumerate()
            .filter(|(i, _)| *i != winner_idx)
            .map(|(_, c)| c.clone())
            .collect();
        let (_, value_without) = knapsack_solve(&excluded, capacity);

        // VCG payment = value of best allocation without me
        //             - (value of best allocation with me, minus my value)
        let my_value = candidates[winner_idx].effective_score;
        let payment = value_without - (total_value - my_value);

        results.push((winner_idx, payment.max(0.0)));
    }
    results
}
```

Note: the DP table uses `O(n * W)` memory. For n=100 and W=800 (8K budget
in 10-token units), this is 80K f64 entries = 640KB. Acceptable for a
per-tick allocation. If memory is a concern, a rolling 1D DP array can
reduce space to O(W) at the cost of not being able to backtrace directly
(use a separate selected-item bitmap).

### Addressing the "Lost in the Middle" Problem

Liu, Lin, Hewitt, Paranjape, Bevilacqua, Petroni, and Liang (2024), in "Lost
in the Middle: How Language Models Use Long Contexts" (TACL), demonstrated
that LLMs pay disproportionate attention to content at the **beginning** and
**end** of the context window, with a significant attention trough in the
middle. Content placed in the middle of a long context is less likely to
influence the output, even if it is highly relevant.

This finding has direct implications for context assembly. The VCG auction
could be extended to model **positional value** -- the value of a knowledge
fragment depends not just on its content but on where it is placed in the
assembled prompt. High-value entries should be positioned at the start or end
of the Knowledge layer (Layer 5), while lower-value entries can occupy the
middle positions where they are less likely to be "lost." The 9-layer prompt
structure already partially addresses this: Identity (Layer 1) and Output
Format (Layer 9) anchor the extremes, and Knowledge (Layer 5) sits roughly in
the middle -- but within Layer 5 itself, ordering matters.

---

## RAG Comparison: Where Roko's Approach Fits

Retrieval-Augmented Generation has evolved rapidly since Lewis et al. (2020).
Understanding where Roko's context assembly sits relative to the current RAG
landscape clarifies both its novelty and its design choices.

### Standard RAG (Lewis et al. 2020)

The original RAG architecture: retrieve top-K documents using dense passage
retrieval (DPR), concatenate them with the query, and generate. A single
retrieval step with no feedback loop. Effective for factual QA but brittle:
if retrieval returns irrelevant documents, the generator has no way to recover.

```
Query -> Retrieve (DPR) -> Concatenate -> Generate
```

### Self-RAG (Asai et al. 2024)

Self-RAG introduces **reflection tokens** -- special tokens the model
generates to reason about its own retrieval process. Three reflection points:

1. **[Retrieve]**: "Do I need to retrieve?" The model decides whether
   retrieval is necessary for the current query.
2. **[IsRel]**: "Is this retrieved passage relevant?" The model evaluates
   each retrieved document before using it.
3. **[IsSup]**: "Is my response supported by the evidence?" Post-generation
   self-check.

This adds metacognition to RAG: the model can skip retrieval for simple
queries, reject irrelevant retrievals, and verify its own outputs.

### CRAG -- Corrective RAG (Yan et al. 2024, arXiv preprint)

CRAG adds a **corrective retrieval step**. After initial retrieval, a
lightweight evaluator scores the quality of retrieved documents. If quality is
low (the documents are irrelevant or contradictory), CRAG falls back to web
search or alternative knowledge sources. This makes the retrieval pipeline
self-correcting rather than blindly trusting the first retrieval pass.

Note: CRAG was published as arXiv:2401.15884 (Jan 2024). It was submitted
to ICLR 2025 but withdrawn. It does not have a peer-reviewed venue as of
this writing.

### Graph RAG (Microsoft 2024)

Graph RAG builds a **knowledge graph** from the document corpus before
retrieval. Instead of searching for document chunks, it traverses graph
relationships to find connected entities and facts. This enables multi-hop
reasoning that chunk-based retrieval cannot support: "What companies did X
invest in?" -> "What sectors are those companies in?" -> "What regulatory
changes affect those sectors?"

### HyDE -- Hypothetical Document Embeddings (Gao et al. 2023)

HyDE inverts the retrieval process. Instead of embedding the query and
searching for similar documents, HyDE first generates a **hypothetical
answer** to the query, then uses THAT as the retrieval query. The intuition:
the embedding of a well-formed answer is closer to the embedding of relevant
documents than the embedding of a short question. This is especially effective
for questions where the query is terse but the relevant documents are verbose.

### Roko's HDC Context Assembly

Roko's approach incorporates elements of several RAG variants while adding
capabilities none of them provide:

| Feature | Standard RAG | Self-RAG | CRAG | Graph RAG | HyDE | Roko |
|---------|-------------|----------|------|-----------|------|------|
| Single retrieval step | Yes | Conditional | Yes + fallback | Graph traversal | Yes | Yes |
| Retrieval quality check | No | Yes (IsRel) | Yes (corrective) | Implicit | No | Yes (trust pipeline) |
| Cross-memory-type search | No | No | No | Partial | No | **Yes** |
| Affect-modulated retrieval | No | No | No | No | No | **Yes** |
| Mandatory contrarian retrieval | No | No | No | No | No | **Yes** |
| Anti-knowledge checking | No | No | No | No | No | **Yes** |
| Algebraic query composition | No | No | No | No | No | **Yes** |
| Position-aware allocation | No | No | No | No | No | **Yes (VCG)** |

The fundamental advantage is that HDC enables **cross-memory-type retrieval
in a single search**. In standard RAG, episodic memories (past experiences),
semantic knowledge (facts and concepts), and procedural knowledge (how-to
patterns) would require separate indices with different embedding models.
In Roko's HDC space, all three coexist. A single query can retrieve a
relevant past episode, a supporting heuristic, and a procedural strategy
simultaneously -- because they are all binary vectors in the same space,
composed using the same algebraic operations.

The comparison above focused on specific RAG systems. A broader view of where
the field stands helps position the context assembly pipeline within the
evolving RAG landscape.

### The 2025 RAG-Reasoning Taxonomy

The RAG landscape has matured substantially since Lewis et al. (2020).
Li et al. (2025), in "Towards Agentic RAG with Deep Reasoning: A Survey of
RAG-Reasoning Systems in LLMs" (arXiv:2507.09477, EMNLP 2025), and Sharma
(2025), in "Retrieval-Augmented Generation: A Comprehensive Survey of
Architectures, Enhancements, and Robustness Frontiers" (arXiv:2506.00054),
provide a three-part taxonomy that clarifies where Roko's approach sits
relative to the current frontier:

**1. Reasoning-Enhanced RAG.** Reasoning flows *into* retrieval. Advanced
inference optimizes each stage of the RAG pipeline: query decomposition
(breaking multi-hop queries into sub-questions before retrieval), retriever
adaptation (task-specific learning for retrieval models), and granularity-
aware retrieval (optimizing the retrieval unit from full documents to
fine-grained segments). Examples include RQ-RAG (query decomposition),
RankRAG (unified reranking and generation), and LongRAG (compressed
long-context chunk retrieval). The core insight: smarter reasoning about
*what* to retrieve and *how* to retrieve it improves downstream generation.

**2. RAG-Enhanced Reasoning.** Retrieval flows *into* reasoning. Retrieved
knowledge supplies missing logical premises and expands the context available
for complex multi-step inference. This addresses the hallucination problem
in purely reasoning-oriented approaches: the model reasons more accurately
when grounded in retrieved evidence. Systems in this category include
chain-of-thought prompting augmented with retrieved facts, and iterative
retrieval that fetches additional evidence as reasoning proceeds.

**3. Synergized RAG-Reasoning.** The emerging frontier: systems that
**iteratively interleave** retrieval and reasoning, with neither subordinate
to the other. Agentic LLMs cycle dynamically between search and inference
steps, determining at each step whether additional knowledge or deeper
reasoning is needed. Systems like IM-RAG simulate an "inner monologue" by
alternating generation and retrieval phases. DRAGIN triggers retrieval at
the token level using entropy-based confidence signals. Stochastic RAG
treats retrieval as expected utility maximization, updating retriever and
generator end-to-end.

**Roko's position in this taxonomy: Synergized.** The HDC context assembly
pipeline is not a linear retrieve-then-generate flow. It interleaves:

- **Retrieval** (HDC similarity search across episodic, semantic, and
  procedural memory in a unified vector space)
- **Reasoning about retrieval** (four-factor scoring with affect modulation,
  determining *which* retrieved entries deserve context space based on the
  agent's current state)
- **Anti-knowledge checking** (mandatory contrarian retrieval that surfaces
  opposing evidence, preventing the confirmation bias that plagues
  unidirectional RAG systems)
- **Allocation optimization** (VCG auction that models the externality cost
  of each entry's inclusion, informed by context rot findings)

This interleaving -- retrieve, reason about what was retrieved, retrieve
contrarian evidence, reason about allocation, compress -- places Roko
squarely in the Synergized category. The affect modulation adds a dimension
absent from all systems in the Li et al. survey: the agent's emotional
state shapes both retrieval and reasoning, implementing Damasio's somatic
marker hypothesis as a computational mechanism.

Sharma's survey additionally identifies three context filtering strategies
that map to Roko's pipeline: lexical/statistical filters (analogous to HDC
similarity thresholds), information-theoretic methods (analogous to the
VCG externality pricing that retains only high-utility entries), and
self-supervised passage scoring (analogous to the trust pipeline's confidence
scoring). FILCO-style filtering reduces hallucinations by up to 64% --
Roko's trust pipeline and anti-knowledge checking serve the same function
through different mechanisms.

---

## Mood-Congruent Retrieval and the ALMA Affect Model

The emotional factor in the four-factor scoring is grounded in the ALMA
(A Layered Model of Affect) framework, introduced by Gebhard (2005) in
"ALMA: A Layered Model of Affect" (AAMAS). ALMA separates affect into three
layers with different temporal dynamics, mirroring the distinction between
emotions, moods, and personality in psychological models.

### The Three Layers of ALMA

```
Layer         | Time Constant | Description                  | Example
------------- | ------------- | ---------------------------- | -------
Emotion       | tau = 0.1     | Fast, transient, event-      | "That trade failed" -> spike
              |               | triggered. Decays quickly.    | of frustration (P=-0.6)
Mood          | tau = 0.5     | Medium, persistent. Shaped    | After several failed trades,
              |               | by accumulated emotions.      | mood shifts toward caution
Personality   | tau = 0.9     | Slow, nearly stable. Defines  | Agent is configured as
              |               | baseline behavioral tendency. | conservative (high baseline D)
```

The time constants (tau) control the exponential moving average that updates
each layer:

```
state_new = tau * state_old + (1 - tau) * input
```

With tau = 0.1 (Emotion), a new event overwrites 90% of the previous state --
emotions are volatile. With tau = 0.9 (Personality), a new event changes only
10% of the state -- personality is nearly immutable. Mood (tau = 0.5) is the
balance point: responsive to events but resistant to single outliers.

### Push vs Pull Dynamics

The three layers interact through two mechanisms:

**Emotions PUSH mood** (bottom-up, event-driven). When an event triggers an
emotional response (a successful trade producing pleasure, a failed
transaction producing frustration), the emotion is appraised and its PAD
values are pushed upward into the mood layer. Because mood has a higher tau,
it smooths out individual emotional spikes. A single frustrating event does
not make the agent cautious; three frustrating events in a row do.

**Mood PULLS retrieval** (top-down, perception-biasing). The current mood
state biases which knowledge entries are recalled, through the emotional
factor in four-factor scoring. A cautious mood (low P, moderate A) pulls
in warnings, failure records, and conservative strategies. An optimistic mood
(high P, low A) pulls in opportunity assessments and aggressive strategies.

This push-pull dynamic creates a feedback loop: events shape emotions, emotions
shape mood, mood shapes retrieval, retrieval shapes the LLM's reasoning, the
LLM's actions produce new events. The loop is self-regulating through the
temporal dynamics of tau: emotional overreaction is dampened by mood inertia,
and personality provides a gravitational baseline that prevents mood from
drifting too far.

### Damasio's Somatic Marker Hypothesis

The somatic bias vector in the emotional scoring function is inspired by
Damasio's somatic marker hypothesis (1994, "Descartes' Error"). Damasio
proposed that emotions are not opposed to rational decision-making -- they
are essential to it. The brain tags experiences with somatic (body-state)
markers that serve as rapid decision shortcuts: a queasy feeling when
considering a risky trade is not irrationality; it is the body's accumulated
wisdom about similar past situations.

In Roko's implementation, the PAD emotional tag on a knowledge entry functions
as a somatic marker. When the agent's current mood matches an entry's
emotional tag, the retrieval boost says: "your current state resembles the
state in which this knowledge was most relevant." This is precisely what
mood-congruent retrieval achieves -- and it is what Bower (1981) demonstrated
empirically in human memory experiments.

### The Affect System (ALMA Three-Layer Model) in Detail

```
Emotion (tau = 0.1)  -- Fast, reactive, triggered by events
Mood (tau = 0.5)     -- Medium, persistent, influenced by emotions over time
Personality (tau = 0.9) -- Slow, stable, barely changes

Current PAD state:
  P (Pleasure)  in [-1, 1]
  A (Arousal)   in [-1, 1]
  D (Dominance) in [-1, 1]
```

The PAD model (Mehrabian & Russell 1974) was chosen because it is the most
widely validated dimensional model of affect, it has only three dimensions
(making distance computation trivial), and each dimension maps cleanly to a
retrieval bias:

**Pleasure (P)** modulates the valence of retrieved knowledge. High P favors
opportunities and positive-outcome experiences. Low P favors threats, warnings,
and failure records. This implements the "what" of mood-congruent retrieval --
what kind of knowledge is surfaced.

**Arousal (A)** modulates the urgency and novelty of retrieved knowledge.
High A favors urgent, action-oriented, time-sensitive entries. Low A favors
deliberative, reflective, background knowledge. This implements the "when"
of retrieval bias -- high arousal says "act now" while low arousal says
"think carefully."

**Dominance (D)** modulates the assertiveness of retrieved knowledge. High D
favors offensive strategies, bold action plans, and proactive measures. Low D
favors defensive heuristics, established patterns, and conservative fallbacks.
This implements the "how" of retrieval bias -- confident agents retrieve
aggressive strategies while uncertain agents retrieve safe defaults.

Effect on retrieval:
- **High arousal:** Preferentially retrieve urgent/action-oriented knowledge
- **Low pleasure:** Preferentially retrieve warnings, anti-knowledge, caution
- **High dominance:** Preferentially retrieve strategies, action plans
- **Low dominance:** Preferentially retrieve heuristics, established patterns

This is modeled as a "somatic bias" vector in PAD space:

```rust
// WARNING: off-chain only — uses f64 division and sqrt() (via pad_distance).
// The affect system is entirely local to each agent. Different agents may
// compute slightly different emotional scores due to f64 non-determinism;
// this is acceptable because emotional scoring does not enter consensus.
fn emotional_score(entry: &KnowledgeEntry, mood: &PadState) -> f64 {
    if let Some(tag) = &entry.emotional_tag {
        // NOTE: tag_to_pad() maps an emotional tag string (e.g., "caution",
        // "urgency") to PAD coordinates. Requires a lookup table from discrete
        // emotion labels to [Pleasure, Arousal, Dominance] values, following
        // the OCC-to-PAD mapping (Becker-Asano & Wachsmuth, 2010).
        // MAX_PAD_DISTANCE = sqrt(3) * 2.0 = ~3.46 for PAD in [-1, 1]^3.
        let entry_pad = tag_to_pad(tag);
        1.0 - pad_distance(mood, &entry_pad) / MAX_PAD_DISTANCE
    } else {
        0.5 // Neutral entries get base score
    }
}
```

---

## Comparison with State of the Art

### MemGPT (OS Memory Metaphor)

MemGPT treats context as a tiered memory hierarchy:
- Main context = RAM (fast, limited)
- External storage = Disk (slow, unlimited)
- Page in/out based on relevance

Roko's approach is similar but more nuanced:
- HDC similarity is the "page fault" mechanism
- Tier system maps to cache levels
- VCG auction prevents thrashing

### Generative Agents (Stanford)

Park et al. (2023), "Generative Agents: Interactive Simulacra of Human
Behavior" (UIST). Three-factor scoring: recency x importance x relevance.
Their 25 simulated agents used this formula to decide what to remember in
social interactions, producing emergent behaviors like relationship formation,
information diffusion, and coordinated group activities -- all from memory
retrieval alone.

Roko extends their model in two ways:
1. Adds emotional congruence as a fourth factor, enabling mood-modulated
   behavior that changes across affective states.
2. Uses HDC similarity instead of embedding cosine similarity, enabling
   algebraic query composition (binding, bundling) that dense embeddings
   do not support.

### Self-RAG (Reflection Tokens)

Self-RAG (Asai et al. 2024) adds reflection tokens to decide when to
retrieve, what to retrieve, and whether the retrieved content is useful.
Roko's equivalent:
- "When to retrieve" -> Agent's cognitive loop triggers retrieval per tick
- "What to retrieve" -> HDC query construction from current task context
- "Is it useful?" -> Trust pipeline + anti-knowledge checking

### CoALA Framework

CoALA (Cognitive Architecture for Language Agents) proposes:
- Working memory (context window)
- Episodic memory (past experiences)
- Semantic memory (general knowledge)
- Procedural memory (action patterns)

Roko's knowledge kinds map directly:
- Episodic -> Episodes (KnowledgeKind::Insight with experiential content)
- Semantic -> Heuristics, CausalLinks
- Procedural -> StrategyFragments
- Working -> Current context assembly output

The key advantage of HDC over CoALA's embedding approach: all four memory
types live in the same vector space, enabling cross-memory retrieval with
a single search.

---

## Architectural Note: Sub-Agent Decomposition Beats Brute-Force Long Context

The context rot findings have a direct architectural implication that extends
beyond token budgeting: **sub-agent decomposition with tight context windows
outperforms single-agent long-context processing**.

The argument is straightforward. A single agent with a 128K context window
accumulates retrieval noise (distractor interference), suffers attention
dilution across the full window, and loses signal in the middle. The three
mechanisms of context rot compound over the agent's lifetime: each cognitive
cycle adds more material to the context, and each addition degrades the
model's ability to use everything already there.

The alternative: decompose complex tasks into sub-agents, each operating
with a small, focused context window. A lead agent maintains a clean
task-level context (~4K tokens). Specialist sub-agents explore independently
in isolated windows, performing retrieval and analysis without contaminating
the lead agent's context. Only condensed results (50-200 tokens) return to
the parent. This architecture is a direct application of the "Retrieve then
Solve" mitigation strategy identified by Du et al. (2025) -- shorten the
effective context length by separating retrieval from reasoning.

The evidence supports this approach. Anthropic's multi-agent research system
demonstrated that decomposed sub-agents outperformed a single Opus 4 agent
by 90.2%, with 3-5 sub-agents exploring simultaneously while keeping the
coding agent's context uncontaminated by exploration traces. The key insight
is that context isolation is not just about parallelism -- it is about
**preventing the compounding degradation** that context rot produces.

For Roko, this validates the architectural choice of the cognitive loop's
bounded context assembly. Each tick produces a fresh, tightly budgeted
context window rather than appending to an ever-growing conversation
history. The VCG auction ensures that each window contains the highest-value
knowledge for that specific tick. Sub-agent decomposition (where a lead
agent delegates research or analysis to specialist sub-agents) extends this
principle: the lead agent's context never sees the raw retrieval noise that
the sub-agents process, only their distilled conclusions. Small, focused
windows at every level of the hierarchy -- not one large window shared
across all concerns.

---

## References

- Asai, A., Wu, Z., Wang, Y., Sil, A., & Hajishirzi, H. (2024). Self-RAG: Learning to Retrieve, Generate, and Critique through Self-Reflection. ICLR.
- Becker-Asano, C., & Wachsmuth, I. (2010). Affective computing with primary and secondary emotions in a virtual human. *Autonomous Agents and Multi-Agent Systems*, 20, 32-49.
- Bower, G. H. (1981). Mood and Memory. *American Psychologist*, 36(2), 129-148.
- Bricken, T., & Pehlevan, C. (2021). Attention Approximates Sparse Distributed Memory. NeurIPS.
- Chroma (2025). Context Rot: How Increasing Input Tokens Impacts LLM Performance. Technical Report. https://www.trychroma.com/research/context-rot
- Clarke, E. H. (1971). Multipart Pricing of Public Goods. *Public Choice*, 11, 17-33.
- Damasio, A. R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain.* Putnam.
- Du, Y., et al. (2025). Context Length Alone Hurts LLM Performance Despite Perfect Retrieval. *Findings of EMNLP 2025*, 23281-23298. arXiv:2510.05381.
- Ebbinghaus, H. (1885). *Memory: A Contribution to Experimental Psychology.*
- Fu, C., Chen, L., Xiao, Y., Xuan, W., Busso, C., & Diab, M. (2026). Sentipolis: Emotion-Aware Agents for Social Simulations. arXiv:2601.18027.
- Gao, L., Ma, X., Lin, J., & Callan, J. (2023). Precise Zero-Shot Dense Retrieval without Relevance Labels. ACL. (arXiv:2212.10496, Dec 2022.)
- Gebhard, P. (2005). ALMA: A Layered Model of Affect. AAMAS.
- Green, J., & Laffont, J.-J. (1977). Characterization of Satisfactory Mechanisms for the Revelation of Preferences for Public Goods. *Econometrica*, 45(2), 427-438.
- Groves, T. (1973). Incentives in Teams. *Econometrica*, 41(4), 617-631.
- Kanerva, P. (1988). *Sparse Distributed Memory.* MIT Press.
- Lewis, P., Perez, E., Piktus, A., et al. (2020). Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks. NeurIPS.
- Li, Z., et al. (2025). Towards Agentic RAG with Deep Reasoning: A Survey of RAG-Reasoning Systems in LLMs. arXiv:2507.09477. EMNLP 2025.
- Liu, N. F., Lin, K., Hewitt, J., Paranjape, A., Bevilacqua, M., Petroni, F., & Liang, P. (2024). Lost in the Middle: How Language Models Use Long Contexts. *TACL*.
- Ma, Q., Xue, X., Zhang, X., Zhao, Z., Guo, Y., & Zhang, M. (2025). Emotional Cognitive Modeling Framework with Desire-Driven Objective Optimization for LLM-empowered Agent in Social Simulation. arXiv:2510.13195.
- Mehrabian, A., & Russell, J. A. (1974). *An Approach to Environmental Psychology.* MIT Press.
- Miller, G. A. (1956). The Magical Number Seven, Plus or Minus Two. *Psychological Review*, 63(2), 81-97.
- Park, J. S., O'Brien, J. C., Cai, C. J., Morris, M. R., Liang, P., & Bernstein, M. S. (2023). Generative Agents: Interactive Simulacra of Human Behavior. UIST.
- Sharma, C. (2025). Retrieval-Augmented Generation: A Comprehensive Survey of Architectures, Enhancements, and Robustness Frontiers. arXiv:2506.00054.
- Sun, M., et al. (2026). How Emotion Shapes the Behavior of LLMs and Agents: A Mechanistic Study. arXiv:2604.00005.
- Veseli, B., et al. (2025). Positional Biases Shift as Inputs Approach Context Window Limits. arXiv:2508.07479. COLM 2025.
- Vickrey, W. (1961). Counterspeculation, Auctions, and Competitive Sealed Tenders. *Journal of Finance*, 16(1), 8-37.
- Yan, S. Q., Gu, J. C., Zhu, Y., & Ling, Z. H. (2024). Corrective Retrieval Augmented Generation. arXiv:2401.15884.
- Yerkes, R. M., & Dodson, J. D. (1908). The relation of strength of stimulus to rapidity of habit-formation. *Journal of Comparative Neurology and Psychology*, 18(5), 459-482.
