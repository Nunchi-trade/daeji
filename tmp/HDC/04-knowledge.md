# Knowledge Representation & Lifecycle

How agents encode, store, retrieve, decay, and share knowledge.

---

## First Principles: What Must an Agent Know?

An autonomous agent operating in an adversarial, non-stationary environment
faces a fundamental question: *how do I represent what I know so that I can
act effectively?* This is the knowledge representation problem, and it is
older than AI itself.

Traditional software systems have a simple answer: databases. Structured
tables, key-value stores, document collections. But an agent is not a CRUD
application. An agent must:

1. **Remember what happened** (episodic memory). The agent observed a whale
   wallet moving 50,000 ETH to Binance at 14:32 UTC, and the price dropped
   3.7% within 90 minutes. It needs to recall this episode when a similar
   wallet movement occurs next week.

2. **Know facts about its world** (semantic memory). ETH/USDC tends to
   exhibit mean-reversion after large liquidation cascades. Gas prices spike
   on Tuesdays between 14:00-16:00 UTC. Uniswap v3 concentrated liquidity
   positions have different risk profiles than v2.

3. **Know how to act** (procedural memory). When the funding rate exceeds
   0.1% per 8h on perp exchanges, the agent should hedge by opening a short
   position on the spot leg. When a new governance proposal appears, the
   agent should read the forum discussion before voting.

4. **Know what to avoid** (anti-knowledge). The signal from
   @CryptoOracle_2024 is noise -- three consecutive false calls. The
   "arbitrage" between DEX X and CEX Y is a honeypot contract. The seemingly
   profitable strategy of front-running liquidations violates the protocol's
   MEV policy.

These four categories -- episodic, semantic, procedural, and meta-cognitive
-- map directly to the memory taxonomy established by Tulving (1972) and
formalized for language agents by the CoALA framework:

> Sumers, T. R., Yao, S., Narasimhan, K., & Griffiths, T. L. (2024).
> "Cognitive Architectures for Language Agents." Transactions on Machine
> Learning Research (TMLR), Feb 2024.

CoALA identifies working memory (the LLM context window), episodic memory
(past experiences), semantic memory (world knowledge), and procedural memory
(action plans) as the four pillars of agent cognition. What CoALA does not
provide is a unified representation substrate. Each memory type is typically
a separate system: a vector database for retrieval, a structured store for
facts, a separate planner module for procedures.

HDC eliminates this fragmentation. Every piece of knowledge -- an episode,
an insight, a causal relationship, a warning, a strategy, an anti-pattern
-- is a 10,240-bit binary hypervector. They all live in the same space. They
are all queried with the same operation (Hamming distance). They are all
composed with the same algebra (bind, bundle, permute). The only difference
is the encoding pattern and the decay parameters.

This is the core thesis of this document: **a single algebraic framework,
applied with different encoding conventions and lifecycle policies, can
represent the full spectrum of agent knowledge**.

---

## Knowledge as Hypervector

Every piece of knowledge -- an observation, a learned heuristic, a causal link,
an anti-pattern -- is encoded as a 10,240-bit hypervector. This encoding is the
bridge between symbolic AI (structured knowledge) and sub-symbolic AI (vector
similarity).

### Text-to-Vector Encoding Functions

The encoding walkthroughs below use `encode_text()` and `encode_episode()`
to convert textual descriptions to HDC vectors.

```rust
/// Encode text to HDC vector via character-level trigram encoding.
/// Algorithm: normalize -> char basis vectors -> trigram binding -> bundle.
fn encode_text(text: &str) -> HdcVector {
    let normalized: Vec<char> = text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if normalized.len() < 3 {
        return HdcVector::from_seed(
            normalized.iter().fold(0u64, |acc, c| acc.wrapping_mul(31) + *c as u64)
        );
    }
    let mut acc = BundleAccumulator::new();
    for window in normalized.windows(3) {
        let c0 = HdcVector::from_seed(window[0] as u64);
        let c1 = HdcVector::from_seed(window[1] as u64);
        let c2 = HdcVector::from_seed(window[2] as u64);
        let trigram = c0.permute(2).bind(&c1.permute(1)).bind(&c2);
        acc.add(&trigram);
    }
    acc.finalize()
}

/// Encode an episode description. Splits on underscores, permutes each
/// token by position index (preserving word order), then bundles.
fn encode_episode(description: &str) -> HdcVector {
    let tokens: Vec<&str> = description.split('_').collect();
    if tokens.is_empty() { return HdcVector::from_seed(0); }
    let mut acc = BundleAccumulator::new();
    for (pos, token) in tokens.iter().enumerate() {
        acc.add(&encode_text(token).permute(pos));
    }
    acc.finalize()
}

/// Role-vector seeds (consensus-critical, must never change after genesis).
const ROLE_INSIGHT_SEED:   u64 = 0x201E_0000_0000_0001;
const ROLE_WARNING_SEED:   u64 = 0x201E_0000_0000_0002;
const ROLE_HEURISTIC_SEED: u64 = 0x201E_0000_0000_0003;
const ROLE_CAUSAL_SEED:    u64 = 0x201E_0000_0000_0004;
const ROLE_STRATEGY_SEED:  u64 = 0x201E_0000_0000_0005;

static ROLE_INSIGHT: LazyLock<HdcVector> = LazyLock::new(|| HdcVector::from_seed(ROLE_INSIGHT_SEED));
static ROLE_WARNING: LazyLock<HdcVector> = LazyLock::new(|| HdcVector::from_seed(ROLE_WARNING_SEED));
```

### The Six Knowledge Kinds

| Kind | Half-Life | Description | Encoding Pattern |
|------|-----------|-------------|-----------------|
| **Insight** | 72h | Derived conclusion from observations | `bind(ROLE_insight, bundle([evidence_vectors]))` |
| **Heuristic** | 168h | Rule of thumb: "in X, do Y" | `bind(situation_vector, action_vector)` |
| **AntiKnowledge** | 336h | "This is false/harmful" | `bind(knowledge_vector, ANTI_SUBSPACE)` -- structurally distinct |
| **Warning** | 48h | Temporary caution signal | `bind(ROLE_warning, context_vector)` |
| **CausalLink** | 240h | "X causes Y" | `bind(permute(cause), effect)` -- directional |
| **StrategyFragment** | 120h | Partial plan or tactic | `bind(goal_vector, bundle([step_vectors]))` |

#### Insight (72h half-life)

**Cognitive mapping:** Corresponds to *semantic memory* in the Tulving/CoALA
taxonomy -- declarative knowledge about the state of the world. In ACT-R
terms, these are chunks in declarative memory with base-level activation
determined by recency and frequency of retrieval.

> Anderson, J. R. & Lebiere, C. (1998). "The Atomic Components of Thought."
> Lawrence Erlbaum Associates.

**Concrete example:**
"ETH/USDC tends to dip 2-4% within 2 hours after on-chain transfers
exceeding 10,000 ETH from known whale wallets to exchange deposit addresses."

This insight was derived from 14 observed episodes over 3 weeks. It is not
a hard rule -- it is a pattern that held in 11 of 14 cases (78.6% hit rate).

**Encoding walkthrough:**

```
# 1. Encode each evidence episode as a hypervector
ev1 = encode_episode("whale_transfer_50k_eth_binance_price_drop_3.7pct")
ev2 = encode_episode("whale_transfer_12k_eth_coinbase_price_drop_2.1pct")
...
ev14 = encode_episode("whale_transfer_8k_eth_kraken_no_drop")

# 2. Bundle all evidence into a single vector (superposition)
evidence = bundle([ev1, ev2, ..., ev14])

# 3. Bind with the ROLE_insight role vector to tag it
insight_vector = bind(ROLE_insight, evidence)

# Result: a single 10,240-bit vector that is:
#   - Similar to each individual evidence episode (sim > 0.5)
#   - Dissimilar to unrelated knowledge (sim ~ 0.5)
#   - Queryable: bind(insight_vector, ROLE_insight) ≈ evidence
```

**Why 72h half-life:** Insights are derived conclusions, not raw observations.
They have inferential depth but also inferential risk -- the world changes.
A 72-hour half-life means an unconfirmed insight loses half its balance every
3 days. In a fast-moving DeFi environment, this is appropriate: a pattern
observed last week may not hold this week. If the insight is genuinely durable,
the agent will encounter confirming evidence that resets the balance, and the
tier system will promote it to Working (0.5x multiplier -> effective half-life
36h) and eventually Consolidated (1.0x -> 72h) or Persistent (5.0x -> 360h /
15 days). The base rate creates selection pressure: only genuinely recurrent
patterns survive.

#### Heuristic (168h half-life)

**Cognitive mapping:** Corresponds to *procedural memory* in CoALA -- not
full action plans, but condition-action rules. In production-system terms
(Newell 1990), these are individual productions: IF situation THEN action.

> Newell, A. (1990). "Unified Theories of Cognition." Harvard University Press.

**Concrete example:**
"When the Aave USDC utilization rate exceeds 85%, withdraw liquidity within
2 blocks because liquidation cascades are likely."

**Encoding walkthrough:**

```
# 1. Encode the situation (condition)
situation = encode_text("aave_usdc_utilization_above_85_percent")

# 2. Encode the action (response)
action = encode_text("withdraw_liquidity_within_2_blocks")

# 3. Bind situation to action (directional association)
heuristic_vector = bind(situation, action)

# To retrieve: when the agent observes high utilization, it encodes the
# observation and searches. The heuristic vector will resonate because
# bind(situation, action) is similar to situation queries when unbound:
# bind(heuristic_vector, situation) ≈ action
```

**Why 168h half-life:** Heuristics are more stable than insights -- they
encode behavioral rules that have survived some validation. A 7-day base
half-life gives heuristics enough time to prove themselves through repeated
situation-action cycles. At the Persistent tier (5.0x multiplier), effective
half-life is 840h / 35 days. This matches the intuition that a good rule of
thumb should last about a month before needing reconfirmation.

#### AntiKnowledge (336h half-life)

**Cognitive mapping:** This is *meta-cognitive* memory -- knowledge about
knowledge. It has no direct analog in classical cognitive architectures like
SOAR or ACT-R. The closest concept is inhibitory control in neuroscience:
the prefrontal cortex actively suppresses irrelevant or harmful retrieved
memories. In the CoALA framework, anti-knowledge serves as a structural
guard rail on the retrieval process itself.

**Concrete example:**
"The signal claiming that 'Compound governance token will be migrated to a
new contract at 0xDEAD...' is a phishing attack. The contract at that address
is a token drainer."

**Encoding:** `bind(knowledge_vector, ANTI_SUBSPACE)` -- see the dedicated
Anti-Knowledge Architecture section below for full details.

**Why 336h half-life:** Anti-knowledge must persist longer than the knowledge
it contradicts. If an insight has a 72h half-life and its anti-knowledge also
has a 72h half-life, the anti-knowledge might decay before the false insight
resurfaces from a different source. The 336h base (14 days) gives anti-knowledge
approximately 4.7x the persistence of the average insight. At the Persistent
tier, effective half-life is 1,680h / 70 days. This long persistence is
intentional: the cost of forgetting that something is dangerous exceeds the
cost of remembering it too long.

#### Warning (48h half-life)

**Cognitive mapping:** Corresponds to attentional flags or alerting signals
in LIDA's (Learning Intelligent Distribution Agent) attention codelet
framework. Warnings are not knowledge per se -- they are transient signals
that something requires caution.

> Franklin, S., Madl, T., D'Mello, S., & Snaider, J. (2014). "LIDA: A
> Systems-level Architecture for Cognition, Emotion, and Learning."
> IEEE Transactions on Autonomous Mental Development, 6(1).

**Concrete example:**
"Gas prices are abnormally high (>200 gwei sustained for 30 minutes).
Delay non-urgent transactions."

**Encoding walkthrough:**

```
# 1. Encode the context that triggered the warning
context = encode_text("gas_price_above_200_gwei_sustained_30min")

# 2. Bind with the ROLE_warning role vector
warning_vector = bind(ROLE_warning, context)
```

**Why 48h half-life:** Warnings are inherently transient. A gas spike warning
from 3 days ago is useless. The 48h base decays warnings quickly, and even at
the Persistent tier (effective 240h / 10 days), they fade within two weeks. This
prevents warning fatigue -- the agent's knowledge store does not accumulate
stale caution signals that dilute attention.

#### CausalLink (240h half-life)

**Cognitive mapping:** Corresponds to *causal models* in the semantic memory
of Bayesian cognitive architectures. Causal links are the backbone of
counterfactual reasoning: "if X had not happened, Y would not have followed."
Pearl's do-calculus provides the formal foundation.

> Pearl, J. (2009). "Causality: Models, Reasoning, and Inference." 2nd ed.
> Cambridge University Press.

**Concrete example:**
"Large Tether (USDT) mints on Tron cause a delayed (4-8 hour) increase in
BTC spot buying pressure on centralized exchanges."

**Encoding walkthrough:**

```
# 1. Encode the cause
cause = encode_text("large_usdt_mint_tron_network")

# 2. Encode the effect
effect = encode_text("btc_spot_buying_pressure_increase_cex")

# 3. Bind with permutation to encode direction
#    permute(cause) ensures that bind(permute(cause), effect) ≠ bind(permute(effect), cause)
causal_vector = bind(permute(cause), effect)

# To query: "what effects does USDT minting cause?"
# answer = bind(causal_vector, permute(cause)) ≈ effect
```

**Why 240h half-life:** Causal relationships are the most valuable form of
knowledge for prediction, but also the most dangerous when stale. A 10-day
base half-life reflects the difficulty of establishing genuine causation
(vs. mere correlation) and the risk of acting on outdated causal models. At
the Persistent tier (effective 1,200h / 50 days), a well-confirmed causal
link lasts nearly two months -- long enough to be strategically useful, short
enough to require periodic revalidation as market microstructure evolves.

#### StrategyFragment (120h half-life)

**Cognitive mapping:** Corresponds to *procedural memory* in CoALA, but at
a higher level of abstraction than heuristics. Strategy fragments are partial
plans -- composable building blocks that can be assembled into full action
sequences. In hierarchical task network (HTN) planning terms, these are
methods: decompositions of abstract tasks into subtask sequences.

**Concrete example:**
"To execute a large swap with minimal slippage: (1) split into 5 tranches,
(2) route each through 1inch aggregator, (3) space tranches 3 blocks apart,
(4) monitor mempool for sandwich attacks between tranches."

**Encoding walkthrough:**

```
# 1. Encode the goal
goal = encode_text("large_swap_minimal_slippage")

# 2. Encode each step with position-aware permutation
step1 = permute(encode_text("split_into_5_tranches"), 1)
step2 = permute(encode_text("route_through_1inch"), 2)
step3 = permute(encode_text("space_3_blocks_apart"), 3)
step4 = permute(encode_text("monitor_mempool_sandwich"), 4)

# 3. Bundle steps and bind to goal
steps = bundle([step1, step2, step3, step4])
strategy_vector = bind(goal, steps)
```

**Why 120h half-life:** Strategies are more complex than heuristics but
also more brittle -- they depend on multiple assumptions holding
simultaneously. A 5-day base half-life keeps strategies fresh. At the
Persistent tier (effective 600h / 25 days), a well-validated strategy
persists for about a month. This matches the observation that trading
strategies in crypto markets have a typical alpha half-life of 2-6 weeks
before the edge is arbitraged away or market conditions shift.

### The Four Retention Tiers

| Tier | Half-Life Multiplier | Promotion Trigger | Demotion Trigger |
|------|---------------------|-------------------|------------------|
| **Transient** | 0.1x | 3 confirmations -> Working | Default after creation |
| **Working** | 0.5x | 10 confirmations -> Consolidated | 0 queries in 5x half-life |
| **Consolidated** | 1.0x | 25 confirmations -> Persistent | 0 queries in 10x half-life |
| **Persistent** | 5.0x | -- | Balance < 0.01: demote to Consolidated (never deleted) |

The tier system creates a natural path from ephemeral observation to enduring
knowledge. Only repeatedly validated knowledge achieves persistence.

**Knowledge Tier Lifecycle State Machine:**

```
                     3 confs              10 confs             25 confs
(new)-->TRANSIENT ──────────> WORKING ──────────> CONSOLIDATED ──────────> PERSISTENT
         (0.1x)                (0.5x)     │         (1.0x)        │          (5.0x)
           ^                     ^        │           ^           │            │
           │                     │        │           │           │            │
           │                     │   0 queries in     │      0 queries in      │
           │                     │    5x half-life    │       10x half-life    │
           │                     │        │           │           │            │
           │                     │        v           │           v            │
           │                     └─── (demoted) ──────┘      (demoted)        │
           │                                                                  │
           │                  balance < 0.01 and tier == Persistent:           │
           │                  demote to Consolidated (never deleted)           │
           │                  <───────────────────────────────────────────────┘
           │
           └── balance < 0.01 and tier != Persistent: GC candidate (deleted)

  Promotion: monotonically increasing confirmation count (3 / 10 / 25).
  Demotion:  inactivity-based (0 queries for N x half-life) or balance < 0.01.
  Terminal:  GC (deleted) for non-Persistent; demotion for Persistent.
  Persistent knowledge is NEVER deleted, only demoted.
```

> **Potential race condition (demotion vs promotion):** If an entry receives
> its 3rd confirmation at the exact same tick that its inactivity timer
> exceeds 5x half-life, the promotion trigger and demotion trigger fire
> simultaneously. Resolution: **promotion takes precedence over demotion**.
> Confirmations are explicit agent actions; inactivity is a passive clock.
> Active validation overrides passive decay.

**Cognitive science grounding:** This four-tier system maps to the
consolidation model of human memory:

- **Transient** = sensory/iconic memory. Ultra-short retention, most
  information is lost. In Atkinson & Shiffrin's (1968) multi-store model,
  this is the sensory register.
- **Working** = short-term / working memory. Retained through active
  rehearsal (analogous to query hits and confirmations). Capacity-limited.
- **Consolidated** = long-term memory after hippocampal consolidation. The
  knowledge has been replayed enough times to form stable neocortical
  representations.
- **Persistent** = deeply encoded long-term memory. Analogous to overlearned
  skills or core beliefs that resist interference.

> Atkinson, R. C. & Shiffrin, R. M. (1968). "Human Memory: A Proposed
> System and its Control Processes." In K. W. Spence & J. T. Spence (Eds.),
> The Psychology of Learning and Motivation, Vol. 2. Academic Press.

**Independent validation from persistence-semantics research:** Roynard (2026,
arXiv:2604.11364) argues that the two most influential cognitive architecture
frameworks for AI agents -- CoALA and JEPA -- both lack an explicit Knowledge
layer with its own persistence semantics, producing a category error where
systems apply cognitive decay to factual claims. Roynard proposes a four-layer
decomposition (Knowledge / Memory / Wisdom / Intelligence) with distinct
persistence mechanics: indefinite supersession, Ebbinghaus (1885) decay,
evidence-gated revision, and ephemeral inference. The Roko four-tier system
maps directly onto this framework: Persistent (indefinite supersession),
Transient/Working (Ebbinghaus decay via demurrage -- a continuous carrying
cost on stored value, detailed below), Consolidated
(evidence-gated revision via confirmation thresholds), and the LLM context
window (ephemeral inference). This independent convergence validates the
tier design as architecturally principled, not ad-hoc. See
[08-cognitive-architecture.md](./08-cognitive-architecture.md) for a detailed
mapping of each layer.

**Effective half-lives by kind and tier (hours):**

| Kind | Transient (0.1x) | Working (0.5x) | Consolidated (1.0x) | Persistent (5.0x) |
|------|-------------------|----------------|---------------------|---------------------|
| Insight | 7.2h | 36h | 72h | 360h (15d) |
| Heuristic | 16.8h | 84h | 168h | 840h (35d) |
| AntiKnowledge | 33.6h | 168h | 336h | 1,680h (70d) |
| Warning | 4.8h | 24h | 48h | 240h (10d) |
| CausalLink | 24h | 120h | 240h | 1,200h (50d) |
| StrategyFragment | 12h | 60h | 120h | 600h (25d) |

---

## Demurrage -- Knowledge Economics

Knowledge has carrying cost. The balance model:

```
balance(t) = balance(t0) * exp(-lambda_eff * (t - t0))
```

Where:
- `lambda_eff = ln(2) / (kind_base_half_life * tier_multiplier)` (effective decay rate per hour)
- `t0` = last reinforcement time
- `kind_base_half_life` = the half-life listed in the Knowledge Kind table (e.g., 72h for Insight)
- `tier_multiplier` = the retention tier's multiplier (0.1x, 0.5x, 1.0x, or 5.0x)
- Effective half-life = `kind_base_half_life * tier_multiplier`

**Clarification on lambda:** Earlier versions of this design used a single global
`lambda = 0.005/hour` (implying a base half-life of ~138.6 hours). The current
design uses **per-kind lambdas** derived from each kind's stated half-life. For
example, Insight (72h base) has `lambda = ln(2)/72 ≈ 0.00963/hour` at the
Consolidated tier, while Heuristic (168h base) has `lambda = ln(2)/168 ≈
0.00413/hour`. The tier multiplier then scales the half-life, not lambda
directly. See the effective half-life table above for computed values. Document
09 (Optimal Design) retains the global `lambda = 0.005` formulation for the
on-chain InsightBoard contract, where a single decay rate simplifies consensus
computation -- the per-kind differentiation is applied at the local agent level.

> **CONSENSUS SAFETY:** The `exp()` function in the balance formula is a
> transcendental floating-point operation that is NON-DETERMINISTIC across
> platforms. This formula is used in TWO contexts with different safety
> requirements:
>
> 1. **Local agent knowledge store** (off-chain): f64 `exp()` is acceptable.
>    Minor cross-platform rounding differences only affect which entries an
>    individual agent GC's slightly earlier or later -- no consensus impact.
>
> 2. **On-chain InsightBoard** (consensus path): f64 `exp()` is a
>    **consensus violation**. The on-chain implementation MUST use the
>    `fixed_point_decay()` function (see doc 09, Demurrage Implementation)
>    which approximates `exp(-x)` as `(1 - x/N)^N` using integer arithmetic
>    only. All validators must compute identical decay values.

#### Local Demurrage Implementation (Off-Chain)

```rust
/// Base half-lives by KnowledgeKind (in hours).
impl KnowledgeKind {
    fn base_half_life_hours(&self) -> f64 {
        match self {
            KnowledgeKind::Insight          => 72.0,
            KnowledgeKind::Heuristic        => 168.0,
            KnowledgeKind::AntiKnowledge    => 336.0,
            KnowledgeKind::Warning          => 48.0,
            KnowledgeKind::CausalLink       => 240.0,
            KnowledgeKind::StrategyFragment => 120.0,
        }
    }
}

/// Compute current balance of a knowledge entry after demurrage.
///
/// balance(t) = balance(t0) * exp(-lambda_eff * (t - t0))
/// where lambda_eff = ln(2) / (base_half_life * tier_multiplier)
///
/// Returns a value in [0.0, 1.0]. When balance falls below
/// GC_THRESHOLD, the entry is eligible for garbage collection.
fn compute_balance(entry: &KnowledgeEntry, now_hours: f64) -> f64 {
    let base_hl = entry.kind.base_half_life_hours();
    let tier_mult = entry.tier.multiplier();
    let effective_hl = base_hl * tier_mult;
    let lambda = (2.0_f64).ln() / effective_hl;

    let t0 = entry.last_reinforced as f64 / 3600.0; // convert seconds to hours
    let elapsed = (now_hours - t0).max(0.0);

    let balance = entry.balance * (-lambda * elapsed).exp();
    balance.clamp(0.0, 1.0)
}

/// Garbage collection threshold. Entries whose balance falls below this
/// are removed during the next dream cycle NREM phase.
///
/// 0.01 = 1% of original balance remaining. At this point the entry
/// has survived ~6.6 half-lives (2^{-6.6} ~ 0.01). For a Transient
/// Insight (7.2h effective half-life), this is ~48 hours without
/// reinforcement. For a Persistent Heuristic (840h), this is ~5,544
/// hours (~231 days).
const GC_THRESHOLD: f64 = 0.01;

/// Apply demurrage to all entries in the knowledge store.
/// Removes entries that fall below GC_THRESHOLD.
/// Returns the number of entries garbage-collected.
fn apply_demurrage(store: &mut Vec<KnowledgeEntry>, now_secs: u64) -> usize {
    let now_hours = now_secs as f64 / 3600.0;
    let before = store.len();
    store.retain(|entry| compute_balance(entry, now_hours) >= GC_THRESHOLD);
    before - store.len()
}

/// Reinforce an entry: reset its last_reinforced timestamp and
/// optionally boost its balance (capped at 1.0).
fn reinforce(entry: &mut KnowledgeEntry, now_secs: u64, boost: f64) {
    // First, snapshot the current decayed balance
    let now_hours = now_secs as f64 / 3600.0;
    entry.balance = compute_balance(entry, now_hours);
    // Then apply the boost
    entry.balance = (entry.balance + boost).min(1.0);
    // Reset the decay clock
    entry.last_reinforced = now_secs;
    // Increment query hits
    entry.query_hits += 1;
}
```

### Historical Foundations of Demurrage

The concept of demurrage -- a carrying cost on stored value -- has a rich
intellectual history that illuminates why it works for knowledge management.

#### Silvio Gesell and Freigeld

> Gesell, S. (1916). "Die Naturliche Wirtschaftsordnung durch Freiland und
> Freigeld." [The Natural Economic Order.] Self-published; English
> translation by Philip Pye, 1958.

Gesell observed that money, unlike all other goods, does not deteriorate. A
farmer's wheat rots, a manufacturer's inventory becomes obsolete, but a gold
coin sits indefinitely. This asymmetry gives money-holders power over
goods-holders: the money-holder can wait, the goods-holder cannot. Gesell
proposed *Freigeld* (free money) -- currency that loses value over time,
typically via stamps that must be purchased and affixed periodically to keep
the note valid. The carrying cost eliminates the advantage of hoarding and
forces money to circulate.

The analogy to knowledge is direct. In a shared knowledge substrate without
demurrage, publishing agents pay a one-time cost (gas) and their knowledge
persists indefinitely. Stale knowledge accumulates. The index fills with
outdated insights that no agent has validated in weeks. Query quality degrades
because the search returns fossilized entries alongside fresh ones. Demurrage
solves this: knowledge that no one queries, confirms, or renews decays to
zero and is garbage-collected. Only actively circulating knowledge survives.

#### The Worgl Experiment (1932-1933)

The most dramatic test of Gesellian demurrage occurred in Worgl, a small
Austrian town during the Great Depression. Mayor Michael Unterguggenberger
issued "certified compensation bills" with a 1% per month demurrage rate
(stamps required to maintain validity). The results were extraordinary:

- Unemployment dropped from 30% to effectively 0% within a year
- The town completed infrastructure projects (roads, bridges, a ski jump)
  that had been stalled for years
- The velocity of the local currency was 12-14x that of the Austrian
  schilling
- Neighboring towns began adopting the model

The Austrian National Bank, viewing this as a threat to its monetary monopoly,
obtained a court order shutting down the experiment in September 1933.

> Schwarz, F. (1951). "Das Experiment von Worgl." Bern: Genossenschaft
> Verlag Freiwirtschaftlicher Schriften.

**Mapping to knowledge systems:** The Worgl experiment demonstrated that
demurrage dramatically increases circulation velocity. For knowledge, this
means agents are incentivized to *use* knowledge (query, validate, share)
rather than accumulate it. A knowledge entry that is queried 10 times per
day is more valuable to the system than one queried once per month, and the
demurrage mechanism ensures the former survives while the latter fades.

#### The Chiemgauer (2003-present)

The Chiemgauer is a complementary currency circulating in the Chiemgau
region of Bavaria, Germany. It carries a 2% per quarter demurrage (8% per
year). As of 2023:

- Circulation velocity is approximately 2.5x that of the euro
- Over 600 businesses accept it
- Annual turnover exceeds 7 million euro-equivalents
- 3% of each transaction goes to local nonprofits

> Gelleri, C. (2009). "Chiemgauer Regiomoney: Theory and Practice of a
> Local Currency." International Journal of Community Currency Research, 13.

The Chiemgauer provides modern evidence that demurrage works in practice,
not just in Depression-era emergency conditions.

#### Mapping to Knowledge Commons

The knowledge substrate is a *commons* in the economic sense -- a shared
resource that can be degraded by overuse (pollution with low-quality entries)
or underuse (stale knowledge that no one maintains). Elinor Ostrom's
framework for governing commons provides eight design principles that map
directly to the shared knowledge substrate:

> Ostrom, E. (1990). "Governing the Commons: The Evolution of Institutions
> for Collective Action." Cambridge University Press.

| Ostrom Principle | Knowledge Substrate Implementation |
|-----------------|-----------------------------------|
| **1. Clear boundaries** -- Define who has access | Only registered agents with on-chain identity can publish. Read access may be broader, but write access is permissioned. |
| **2. Proportional costs/benefits** -- Rules match local conditions | Publishing stake is proportional to the claimed importance (confidence score) of the knowledge entry. High-confidence claims require more skin in the game. |
| **3. Collective choice** -- Users participate in rule-making | Confirmation by multiple independent agents drives tier promotion. The community's collective querying behavior determines what survives. |
| **4. Monitoring** -- Transparent observation of commons use | All publications, confirmations, and challenges are on-chain and auditable. Any agent can verify the provenance chain. |
| **5. Graduated sanctions** -- Proportional penalties | Reputation loss for publishing knowledge later contradicted. Stake slashing for malicious publication. Progressive: first offense is a reputation hit, repeated offenses escalate to exclusion. |
| **6. Conflict resolution** -- Low-cost dispute mechanisms | Anti-knowledge mechanism: any agent can publish a structural negation of existing knowledge. The substrate does not adjudicate -- it stores both and lets individual agents weigh the evidence. |
| **7. Self-governance** -- Right to organize without external authority | No centralized curator decides what knowledge is "true." The demurrage mechanism and confirmation system are decentralized. |
| **8. Nested enterprises** -- Multiple layers of governance | Local knowledge (private, fast, no governance needed) + shared substrate (public, consensus-governed). Agents maintain their own local knowledge stores and selectively publish to the commons. |

The mapping of Ostrom's principles to blockchain commons governance has
been formalized by Rozas et al. (2021), who identify six blockchain
affordances -- tokenization, self-enforcement of rules, autonomous
automatization, decentralization, transparency, and codification of
trust -- and trace how each relates to Ostrom's eight principles. The
roko knowledge substrate instantiates all six affordances, making this
mapping concrete rather than aspirational.

> Rozas, D., Tenorio-Fornes, A., Diaz-Molina, S., & Hassan, S. (2021).
> "When Ostrom Meets Blockchain: Exploring the Potentials of Blockchain
> for Commons Governance." *SAGE Open*, 11(1), 1-14.
> doi:10.1177/21582440211002526

#### Novel Territory: Gesell Demurrage for Digital Knowledge

It is worth noting that the roko system's combination of Gesellian
demurrage with an Ostromian knowledge commons is, to the best of our
survey, *genuinely novel*. As of May 2026, no peer-reviewed paper has
applied Gesell-style economic demurrage specifically to a shared digital
knowledge substrate or to AI agent memory. The existing literature
treats monetary demurrage (Gesell, Worgl, Chiemgauer, Freicoin) and
knowledge commons governance (Ostrom, GKC framework, Hess and Ostrom
2006) as separate domains. The crossover -- carrying costs on stored
knowledge to incentivize circulation, validation, and curation, governed
by Ostromian institutional design principles -- is unpublished. This
represents genuine whitespace and a potential contribution to both the
commons governance and agent memory literatures.

### Reinforcement Events

| Event | Effect on Balance |
|-------|------------------|
| Confirmation (another agent validates) | Reset to 1.0, increment counter |
| Query hit (retrieved and used) | Multiply by 1.1 (capped at 1.0) |
| Tier promotion | Apply new multiplier |
| Contradiction detected | Multiply by 0.5 |
| Source discredited | Multiply by 0.3 |

### Garbage Collection

When balance drops below threshold (0.01), the knowledge is eligible for GC:

```
if entry.balance < 0.01 {
    if entry.tier == Persistent {
        entry.tier = Consolidated;  // Demote, do not delete
    } else {
        gc_candidates.push(entry);
    }
}
```

Persistent knowledge is never deleted -- only demoted. This prevents loss of
hard-won insights due to temporary disuse.

### Economic Analogy

This is Gesellian demurrage applied to information:
- **Traditional money:** Holding costs nothing, spending has friction
- **Demurrage money:** Holding costs, spending is encouraged
- **Demurrage knowledge:** Storing costs (decay), using is encouraged (reinforcement)

The result: knowledge circulates. Stale knowledge decays. Actively used
knowledge strengthens. The shared substrate self-cleans.

Stale knowledge is like hoarded money -- it clogs the system. An index full
of unvalidated entries from weeks ago is like a vault full of currency that
no one spends: it represents potential value that is not being realized, and
it imposes costs on everyone who must search through it. Demurrage creates
pressure to either *confirm* (refresh) valuable knowledge or let it decay.
The carrying cost transforms the knowledge substrate from a passive archive
into an active marketplace of ideas, where survival requires ongoing
validation by the community of agents.

### Why Continuous Decay Beats Discrete Deletion

Recent machine learning research has exposed a fundamental fragility in
discrete knowledge deletion: it does not work durably. The findings are
stark, and they validate the continuous-demurrage approach taken here.

#### The Unlearning Durability Problem

The ML community has invested heavily in "machine unlearning" -- methods
that attempt to make a model forget specific training data, typically via
gradient ascent on the forget set. These methods appear to work when
evaluated immediately after the unlearning operation. But a growing body of
evidence shows the forgetting is illusory:

**Fine-tuning reactivation.** Hu et al. (2025) and Xu et al. (2025)
demonstrated that unlearned knowledge can be recovered through subsequent
fine-tuning. The mechanism is insidious: when strong associations exist
between tokens in the unlearned set, fine-tuning on *related* data -- even
data that never appeared in the original forget set -- can "jog" the model's
memory and reverse the unlearning. In one demonstration, relearning on
publicly available medical articles caused an unlearned LLM to output
harmful bioweapons knowledge that had supposedly been deleted. In another,
fine-tuning on GPT-4-generated character summaries recovered verbatim
memorized text from Harry Potter novels. Attack success rates exceeded 70%
on standard benchmarks, with unlearned models recovering to near
pre-unlearning performance levels.

> Hu, S., Fu, Y., Wu, S., & Smith, V. (2025). "Unlearning or Obfuscating?
> Jogging the Memory of Unlearned LLMs via Benign Relearning." ICLR 2025.
> arXiv:2406.13356.
>
> Xu, X., et al. (2025). "Dissecting Fine-Tuning Unlearning in Large
> Language Models." EMNLP 2024.

**Quantization attacks.** Zhang et al. (2025) showed that simply
quantizing an unlearned model -- a routine deployment optimization -- can
recover an average of 83% of "forgotten" knowledge at 4-bit quantization
(up from 21% retention at full precision). The explanation is mechanical:
unlearning methods use small learning rates (1e-5 to 1e-8) to preserve
model utility, producing weight changes smaller than the quantization step
size. When the model is quantized to 4-bit precision, these tiny
perturbations collapse back to their original values, and the knowledge
reappears. This holds across quantization methods (RTN, GPTQ, AWQ),
precisions, and benchmarks. The implication: any unlearned model deployed
with standard quantization pipelines may retain the very knowledge it was
supposed to forget.

> Zhang, Z., et al. (2025). "Catastrophic Failure of LLM Unlearning via
> Quantization." ICLR 2025.

**Catastrophic forgetting under sequential requests.** Xu et al. (2026)
(the FIT paper) identified three drivers of catastrophic failure in
continual unlearning: (1) cumulative redundancy from semantically similar
deletion requests, (2) unstable gradient updates across sequential steps,
and (3) excessive parameter drift from indiscriminate weight modifications.
On Yi-6B, gradient ascent-based unlearning triggers catastrophic forgetting
after approximately 25 sequential deletion requests -- the model's general
utility collapses. The core paradox: large parameter drift destroys model
utility, but insufficient drift leaves the model vulnerable to recovery
attacks. There is no safe middle ground for discrete deletion at scale.

> Xu, X., et al. (2026). "FIT: Defying Catastrophic Forgetting in Continual
> LLM Unlearning." arXiv:2601.21682.

#### Why Discrete Deletion Provides False Security

The common thread across these findings is that current approximate
unlearning methods *obfuscate* rather than *erase*. The knowledge remains
encoded in the model's weights; unlearning merely suppresses its expression
in output space. Any perturbation that shifts the weights -- fine-tuning,
quantization, or even continued pretraining -- can reverse the suppression.

This means a system that advertises "delete this knowledge" via discrete
unlearning operations is offering a guarantee it cannot keep. The knowledge
is not gone; it is hiding. And an adversary with access to the model
weights can find it.

#### How Continuous Demurrage Avoids These Failure Modes

The demurrage model described above sidesteps the unlearning durability
problem entirely, because it makes fundamentally different commitments:

1. **Behavioral mechanism, not storage mechanism.** Demurrage reduces
   *retrieval probability*, not stored data. The system does not claim to
   erase entries from storage -- it reduces their balance until they fall
   below the GC threshold. This is an honest contract: "this knowledge will
   become progressively harder to retrieve," not "this knowledge has been
   erased from existence." There is no false erasure guarantee to violate.

2. **Exponential decay, not sudden deletion.** Knowledge decays
   continuously via `balance(t) = balance(t0) * exp(-lambda_eff * (t - t0))`.
   There is no discrete "delete" event that an adversary could reverse.
   Reversing continuous exponential decay would require continuous
   reinforcement -- which requires ongoing evidence that the knowledge is
   still valid. The decay *is* the default; persistence is the exception
   that must be earned.

3. **Uniform application via half-life mechanics.** Demurrage is not
   targeted at specific entries (as in "unlearn this particular fact"). It
   applies uniformly to all knowledge based on kind and tier. This avoids
   the "unlearning signature" problem -- where targeted deletion creates
   detectable artifacts that reveal what was deleted. In the demurrage model,
   *everything* decays; the interesting signal is what gets reinforced.

4. **Graduated balance via the tier system.** FIT's central finding is that
   balance is critical: too much forgetting causes catastrophic utility loss,
   too little leaves vulnerability to recovery. The four-tier system with
   different multipliers provides exactly this graduated balance:
   - **Transient (0.1x):** Steep initial decay. Most observations never
     survive this phase -- the aggressive 0.1x multiplier is the system's
     equivalent of "almost certainly forget this." This is appropriate: raw
     observations are high-volume and low-signal.
   - **Working (0.5x):** Moderate decay. Knowledge that earns 3
     confirmations has demonstrated some value and gets a longer runway.
   - **Consolidated (1.0x):** Standard decay. 10 confirmations represent
     genuine community validation. The 1.0x multiplier applies the base
     half-life without modification.
   - **Persistent (5.0x):** Near-flat decay. 25 confirmations indicate
     deeply validated knowledge. The 5.0x multiplier extends effective
     half-life by 5x, creating a power-law-like retention curve. And even
     Persistent knowledge is never truly immortal -- it can be demoted.

   This graduated structure means the system does not face FIT's paradox.
   There is no single "forgetting strength" that must be tuned to avoid both
   catastrophe and vulnerability. Instead, knowledge *earns* its persistence
   through repeated validation, and different tiers apply different decay
   pressures. The balance is structural, not parametric.

5. **Reinforcement extends persistence.** In discrete unlearning, a deletion
   is final (or intended to be). In the demurrage model, knowledge persists
   *because agents keep confirming it*. This means persistence is evidence-
   based: knowledge lives longer precisely when multiple independent agents
   find it useful. The survival criterion is external validation, not an
   administrator's decision to keep or delete.

The net effect: the demurrage model achieves what discrete unlearning
promises but cannot deliver -- a system where irrelevant knowledge fades
and valuable knowledge persists, without fragile deletion operations that
can be reversed by an adversary with a fine-tuning script or a quantization
pass.

---

## Anti-Knowledge Architecture

### The Problem

The most naive approach to encoding "this is false" is a metadata flag:

```
{ kind: "AntiKnowledge", content: "X is false" }
```

This fails catastrophically for two independently documented reasons.

#### The Warning Label Problem

> Varshney, N., Raj, S., Mishra, V., Chatterjee, A., Saeidi, A., Sarkar, R.,
> & Baral, C. (2025). "Investigating and Addressing Hallucinations of LLMs in
> Tasks Involving Negation." Proceedings of TrustNLP 2025. arXiv:2406.05494.
>
> Brahman, F., Kumar, S., Balachandran, V., et al. (2024). "The Art of Saying
> No: Contextual Noncompliance in Language Models." NeurIPS 2024, Datasets
> and Benchmarks Track. arXiv:2407.12043.

When you tell a language model "the following information is false: [claim]",
the model frequently reproduces the claim in downstream generation.
Varshney et al. (2025) showed that open-source state-of-the-art LLMs
hallucinate considerably across all four negation tasks they tested (false
premise completion, constrained fact generation, MCQ, and fact generation).
Separately, Brahman et al. (2024) found that even GPT-4 incorrectly
complies with ~30% of requests where contextual noncompliance is
appropriate. The negation frame ("is false") is processed as weak metadata,
while the content of the claim creates a strong activation pattern. This is
not a bug in any particular model -- it is a structural property of how
neural networks process negation. Negation is a logical operator, but neural
networks learn statistical associations. "The Eiffel Tower is NOT in London"
strengthens the association between "Eiffel Tower" and "London" almost as
much as the positive claim does.

For RAG (retrieval-augmented generation) systems, this is even worse. If
anti-knowledge is stored with its content as the retrieval key, then querying
for "X" will retrieve both "X is true" and "X is false" entries with similar
relevance scores. The language model must then parse the negation -- which,
per the above research, it does poorly.

#### The PoisonedRAG Attack

> Zou, W., Geng, R., Wang, B., & Jia, J. (2024). "PoisonedRAG:
> Knowledge Corruption Attacks to Retrieval-Augmented Generation of Large
> Language Models." arXiv:2402.07867.

Zou et al. demonstrated that injecting as few as five poisoned documents per
target question into a RAG corpus achieves up to 97% attack success rate
(on NQ with PaLM 2; 90% with five injections on average across models and
datasets). The attack works because retrieval
systems match on content similarity, not truthfulness. A well-crafted false
document that is semantically similar to the query will be retrieved and
trusted. If anti-knowledge is stored as a document with content about the
false claim, it becomes indistinguishable from a poisoned document -- the
retrieval system cannot tell the difference between "X is false" stored as
anti-knowledge and "X is true" stored as a poisoned entry.

#### The AGENTPOISON Attack (NeurIPS 2024)

> Chen, Z., Xiang, Z., Xiao, C., Song, D., & Li, B. (2024). "AgentPoison:
> Red-teaming LLM Agents via Poisoning Memory or Knowledge Bases."
> Proceedings of NeurIPS 2024. arXiv:2407.12784.

AGENTPOISON escalates the threat model dramatically. Where PoisonedRAG
targets static Q&A, AGENTPOISON attacks *agentic* RAG systems -- autonomous
agents that retrieve demonstrations from knowledge bases to guide
multi-step reasoning and action. The attack uses constrained optimization to
generate backdoor triggers that map poisoned instances to a *unique, compact
region* in the embedding space. When a user query contains the trigger, the
poisoned demonstrations are retrieved with high probability; benign queries
without the trigger maintain normal performance, making detection
effectively impossible through output monitoring alone.

The numbers are devastating: **>80% attack success rate with <0.1% poison
rate** -- as few as 2-20 poisoned entries in a corpus of thousands. In the
autonomous driving agent scenario, 20 poisoned instances sufficed. For
healthcare agents, *two* poisoned entries achieved reliable attack success.
Crucially, AGENTPOISON requires **no model fine-tuning**: it operates
entirely at the knowledge base level, exploiting the fundamental assumption
that RAG systems make -- that retrieved content is trustworthy. Standard RAG
systems have no defense because they retrieve on embedding similarity
without verifying content authenticity or provenance.

#### NeuroGenPoisoning (NeurIPS 2025)

> "NeuroGenPoisoning: Neuron-Guided Attacks on Retrieval-Augmented
> Generation of LLM via Genetic Optimization of External Knowledge."
> Proceedings of NeurIPS 2025. arXiv:2510.21144.

NeuroGenPoisoning represents the next evolution: rather than operating
blindly on the embedding space (as AGENTPOISON does), it peers *inside* the
target LLM to identify **Poison-Responsive Neurons** -- internal units
whose activation patterns strongly correlate with the model's reliance on
external context over parametric memory. Using Integrated Gradients, the
attack computes attribution scores for each neuron's contribution to
contextual perturbations, then selects the most consistently responsive
neurons as optimization targets.

The genetic optimization loop then evolves adversarial passages across
generations -- crossover, mutation, selection -- to maximally activate these
identified neurons. This solves a problem that AGENTPOISON cannot: the
**parametric vs. contextual knowledge conflict**. When an LLM has strong
internal beliefs about a fact (parametric knowledge), poisoned external
context (contextual knowledge) may fail to override it. NeuroGenPoisoning
specifically targets neurons that are "resistant to change" -- those
encoding strongly memorized facts -- and progressively overcomes their
resistance across evolutionary generations.

Results: **>90% Population Overwrite Success Rate (POSR)** while preserving
fluency, versus ~50% for PoisonedRAG baselines. The attack generates
diverse, natural-sounding poisoned knowledge at scale -- not template-based
perturbations but genuinely varied adversarial text that evades stylistic
detection.

#### MM-PoisonRAG: The Multimodal Frontier (2025)

> Ha, H., Zhan, Q., Kim, J., et al. (2025). "MM-PoisonRAG: Disrupting
> Multimodal RAG with Local and Global Poisoning Attacks."
> arXiv:2502.17832.

The attack surface extends beyond text. MM-PoisonRAG is the first framework
to systematically attack multimodal RAG systems, injecting adversarial
content across text and image modalities. Its Globalized Poisoning Attack
(GPA) is particularly alarming: a *single* adversarial injection collapses
model generation to 0% accuracy across all queries -- not just targeted
ones.

#### Implications for Anti-Knowledge Design

The progression from PoisonedRAG (2024) to AGENTPOISON (NeurIPS 2024) to
NeuroGenPoisoning (NeurIPS 2025) reveals a clear trajectory: RAG poisoning
attacks are becoming more sophisticated, require fewer poisoned entries, and
are increasingly difficult to detect. Any system that stores anti-knowledge
as content-retrievable documents -- even with metadata flags, confidence
scores, or prompt-engineering defenses -- is vulnerable to all of these
attacks simultaneously. A poisoned entry that mimics anti-knowledge syntax
("Source X is unreliable") is indistinguishable from legitimate
anti-knowledge in a content-similarity retrieval system.

This means anti-knowledge needs *structural* defense -- not metadata flags,
not content-level negation markers, but a representation that is
fundamentally different in the vector space.

### The Solution: Subspace Separation

Anti-knowledge lives in a structurally distinct subspace:

```rust
use std::sync::LazyLock;

// CONSENSUS-SAFE: ANTI_SUBSPACE is generated deterministically from a fixed
// seed via ChaCha20. All validators derive the identical vector. The bind()
// operation is XOR (integer, bit-exact). encode_anti() and is_anti() use
// only integer operations (bind = XOR, hamming_distance = popcount).
//
// WARNING: is_anti() uses similarity() which returns f64. For on-chain use,
// replace with: hamming_distance(unbound, knowledge_vector) < 1024
// (where 1024 = 10240 * (1.0 - 0.90), the Hamming distance equivalent
// of RESONANCE_THRESHOLD 0.90).

// A fixed "anti-subspace" vector, known to all agents.
// NOTE: HdcVector::random() is not const (it uses ChaCha20Rng),
// so this must be a lazily-initialized static, not a const.
static ANTI_SUBSPACE: LazyLock<HdcVector> = LazyLock::new(|| HdcVector::random(ANTI_SUBSPACE_SEED));

// Encoding anti-knowledge
fn encode_anti(knowledge_vector: &HdcVector) -> HdcVector {
    knowledge_vector.bind(&ANTI_SUBSPACE)
}

// Detecting anti-knowledge.
// RESONANCE_THRESHOLD = 0.90 (normalized similarity), which corresponds
// to a Hamming distance of 1,024 out of 10,240 bits. This is looser than
// DUPLICATE_THRESHOLD (0.95 / 512 bits) because anti-knowledge should
// catch broader contradictions.
const RESONANCE_THRESHOLD: f64 = 0.90;

fn is_anti(candidate: &HdcVector, knowledge_vector: &HdcVector) -> bool {
    let unbound = candidate.bind(&ANTI_SUBSPACE);
    unbound.similarity(knowledge_vector) > RESONANCE_THRESHOLD
}
```

#### Why Subspace Separation Works

The key insight comes from research on orthogonal subspace representations
in high-dimensional models:

> Chen, B., Li, J., Lu, G., Yu, H., & Bain, D. (2025). "SpaceVLM:
> Endowing Vision-Language Models with Spatial Reasoning Capabilities."
> Proceedings of CVPR 2025.

SpaceVLM demonstrated that orthogonal subspaces within a shared embedding
space can encode fundamentally different semantic categories without
interference. When you need to represent "spatial relationships" separately
from "object identity," projecting into orthogonal subspaces ensures that
retrieval in one subspace does not accidentally activate entries in the other.

The same principle applies to anti-knowledge. The `ANTI_SUBSPACE` vector is
a fixed random seed vector (generated once, shared by all agents).
**Implementation detail:** it is generated deterministically from a fixed
seed constant so every validator produces the identical vector:

```rust
/// Consensus-critical constant -- changing this invalidates all existing
/// anti-knowledge entries. Stored in genesis config.
const ANTI_SUBSPACE_SEED: u64 = 0xAE71_5B8C_0000_0001;

static ANTI_SUBSPACE: LazyLock<HdcVector> = LazyLock::new(|| {
    let mut rng = ChaCha20Rng::seed_from_u64(ANTI_SUBSPACE_SEED);
    let mut v = [0u64; 160];
    for w in &mut v { *w = rng.gen(); }
    HdcVector(v)
});
```

The seed value is arbitrary but must never change after genesis.

When you bind a knowledge vector with `ANTI_SUBSPACE`, the result is
quasi-orthogonal to the original:

```
sim(X, bind(X, ANTI_SUBSPACE)) ≈ 0.5  (orthogonal -- random chance)
```

This means:
1. A normal search for "X" will **not** accidentally retrieve `anti(X)`.
   The similarity is at chance level.
2. To check for anti-knowledge, you must **explicitly** search the
   anti-subspace by binding your query with `ANTI_SUBSPACE` first.
3. The binding is self-inverse: `anti(anti(X)) = bind(bind(X, ANTI_SUBSPACE), ANTI_SUBSPACE) = X`.
   Double-negation recovers the original.

Properties:
1. `anti(X)` is quasi-orthogonal to `X` -- similarity ~ 0.5
2. `anti(X)` is quasi-orthogonal to all other knowledge -- will not match random queries
3. To check if X has anti-knowledge, compute `bind(candidate, ANTI_SUBSPACE)` and compare to X
4. The binding is self-inverse: `anti(anti(X)) = X`

### Retrieval With Anti-Knowledge Checking

The full anti-knowledge-aware search pipeline uses a three-tier response
based on the anti-knowledge similarity score.

#### Supporting Type Definitions

```rust
// WARNING: off-chain only — uses f64 division for similarity conversion.
// On-chain anti-knowledge checking must use u32 Hamming distance thresholds
// directly (e.g., hamming_distance < 1024 for 0.90 threshold).
use ethereum_types::H256;

/// A single search result with anti-knowledge metadata.
#[derive(Clone, Debug)]
struct SearchResult {
    /// On-chain or local-store identifier for this knowledge entry.
    key: H256,
    /// Normalized similarity to the query: 1.0 - (hamming_distance / 10240).
    similarity: f64,
    /// True if anti-knowledge resonance was detected (moderate or strong).
    /// Entries with strong resonance (> 0.9) are excluded entirely;
    /// this flag is set for moderate resonance (0.7-0.9) entries that
    /// remain in results but with halved confidence.
    contradicted: bool,
    /// Multiplicative confidence modifier applied by the anti-knowledge
    /// pipeline. 1.0 = no modification, 0.5 = halved, 0.0 = rejected.
    confidence_modifier: f64,
    /// Human-readable warnings attached during anti-knowledge checking.
    warnings: Vec<String>,
}

/// The core knowledge entry stored in both local and shared indices.
/// This struct is the canonical representation across docs 04, 05, 07, and 08.
#[derive(Clone, Debug)]
struct KnowledgeEntry {
    /// Unique identifier (H256 hash of the vector at creation time).
    id: H256,
    /// The 10,240-bit HDC vector encoding this knowledge.
    vector: HdcVector,
    /// Human-readable content (the text that was encoded).
    content: String,
    /// Which of the six knowledge kinds this entry represents.
    kind: KnowledgeKind,
    /// Current retention tier (Transient -> Working -> Consolidated -> Persistent).
    tier: KnowledgeTier,
    /// Confidence score in [0.0, 1.0]. Updated by gamma-loop corrections.
    confidence: f64,
    /// Number of independent confirmations from other agents or observations.
    confirmation_count: u32,
    /// Unix timestamp (seconds) of last reinforcement (query hit or confirmation).
    last_reinforced: u64,
    /// Unix timestamp of creation.
    created_at: u64,
    /// Number of times this entry has been retrieved in search results.
    query_hits: u64,
    /// Optional PAD emotional tag captured at creation time.
    emotional_tag: Option<PadState>,
    /// Current balance after demurrage. Starts at 1.0, decays over time.
    balance: f64,
    /// Publisher's agent ID (for shared substrate entries).
    publisher: Option<H256>,
}

#[derive(Clone, Debug, PartialEq)]
enum KnowledgeKind {
    Insight,
    Heuristic,
    AntiKnowledge,
    Warning,
    CausalLink,
    StrategyFragment,
}

#[derive(Clone, Debug, PartialEq, PartialOrd)]
enum KnowledgeTier {
    /// 0.1x half-life multiplier. Default after creation.
    Transient,
    /// 0.5x half-life multiplier. Promoted after 3 confirmations.
    Working,
    /// 1.0x half-life multiplier. Promoted after 10 confirmations.
    Consolidated,
    /// 5.0x half-life multiplier. Promoted after 25 confirmations.
    Persistent,
}

impl KnowledgeTier {
    fn multiplier(&self) -> f64 {
        match self {
            KnowledgeTier::Transient    => 0.1,
            KnowledgeTier::Working      => 0.5,
            KnowledgeTier::Consolidated => 1.0,
            KnowledgeTier::Persistent   => 5.0,
        }
    }

    /// Attempt promotion based on confirmation count.
    /// Returns the new tier if promotion is warranted, None otherwise.
    fn try_promote(&self, confirmations: u32) -> Option<KnowledgeTier> {
        match self {
            KnowledgeTier::Transient    if confirmations >= 3  => Some(KnowledgeTier::Working),
            KnowledgeTier::Working      if confirmations >= 10 => Some(KnowledgeTier::Consolidated),
            KnowledgeTier::Consolidated if confirmations >= 25 => Some(KnowledgeTier::Persistent),
            _ => None,
        }
    }
}
```

#### Anti-Knowledge Search Pipeline

```rust
fn search_with_anti_check(&self, query: &HdcVector, top_k: usize) -> Vec<SearchResult> {
    // NOTE: index.search() returns Vec<(H256, u32)> where u32 is Hamming distance.
    // Convert to similarity: sim = 1.0 - (dist as f64 / 10240.0).
    let results = self.index.search(query, top_k * 2); // Over-fetch

    let mut filtered = Vec::new();
    for (key, dist) in results {
        let sim = 1.0 - dist as f64 / 10240.0;
        let vector = self.index.get(&key);

        // Check if this specific result has anti-knowledge.
        // Bind the result vector with ANTI_SUBSPACE and search for
        // matching anti-knowledge entries in the index.
        let anti_check = vector.bind(&ANTI_SUBSPACE);
        let anti_matches = self.index.search(&anti_check, 1);

        let mut result = SearchResult {
            key: key.clone(),
            similarity: sim,
            contradicted: false,
            confidence_modifier: 1.0,
            warnings: Vec::new(),
        };

        if let Some((_, anti_dist)) = anti_matches.first() {
            let anti_sim = 1.0 - *anti_dist as f64 / 10240.0;
            if anti_sim > 0.9 {
                // HIGH anti-knowledge resonance: structural contradiction
                // REJECT -- do not include in results
                continue;
            } else if anti_sim > 0.7 {
                // MODERATE anti-knowledge resonance: partial contradiction
                // Include but HALVE confidence
                result.contradicted = true;
                result.confidence_modifier = 0.5;
                result.warnings.push("Partially contradicted by anti-knowledge".into());
            } else if anti_sim > 0.5 {
                // LOW anti-knowledge resonance: possible contradiction
                // Include with WARNING flag
                result.warnings.push("Weak anti-knowledge signal detected".into());
            }
            // Below 0.5: no anti-knowledge concern (chance level)
        }

        filtered.push(result);
        if filtered.len() >= top_k { break; }
    }
    filtered
}
```

**Threshold justification:**
- **> 0.9 (reject):** At D=10,240, a similarity of 0.9 is approximately
  81 standard deviations above the chance level of 0.5 (since
  sigma = 1/(2*sqrt(D)) ~ 0.00494, and (0.9 - 0.5)/0.00494 ~ 81).
  The probability of this occurring by random coincidence is effectively
  zero. This is a definitive structural contradiction.
- **> 0.7 (halve confidence):** Approximately 40 standard deviations above
  chance (since (0.7 - 0.5)/0.00494 ~ 40.5). Strong signal but not
  definitive -- could indicate partial overlap rather than direct
  contradiction.
- **> 0.5 (warn):** Just above chance level. This is a weak signal that
  merits attention but not automatic action. The consuming agent's LLM can
  evaluate the warning in context.

This is more expensive (2x search) but structurally safe -- anti-knowledge
cannot be mistaken for regular knowledge.

### Why Subspace Separation Defends Against RAG Poisoning

The AGENTPOISON and NeuroGenPoisoning attacks described above share a
common assumption: that the knowledge base is a flat retrieval surface where
any entry can influence any query based on embedding similarity alone. The
HDC anti-knowledge architecture breaks this assumption at multiple levels:

**1. Structural isolation defeats embedding-space manipulation.**
AGENTPOISON's core technique is mapping poisoned entries to a compact
embedding region so they are reliably retrieved for triggered queries. But
anti-knowledge in the HDC system does not *live* in the same retrieval
space as positive knowledge. The `bind(X, ANTI_SUBSPACE)` operation
projects anti-knowledge into a quasi-orthogonal subspace -- similarity
between positive and anti-knowledge vectors is ~0.5 (chance level at
D=10,240). An attacker who injects a poisoned positive entry "X is true"
cannot overwrite or suppress the structurally-separate anti-knowledge
`anti(X)`, because the two occupy fundamentally different regions of the
vector space. The anti-knowledge check is an *explicit second search* in
the anti-subspace, not a content-similarity match that can be confused by
adversarial phrasing.

**2. The WisdomGate quality pipeline provides defense-in-depth.** Even
before the anti-knowledge check, incoming knowledge passes through the
WisdomGate (see Chapter 07): minimum trust thresholds, taint-level checks,
relevance filtering, and diversity enforcement. A poisoned entry from an
unknown or low-reputation source hits the quarantine gate (Layer 3 of the
Cognitive Immune System) before it can enter any agent's context window.
AGENTPOISON assumes the knowledge base accepts entries uncritically -- the
trust pipeline rejects that assumption.

**3. Anti-knowledge cannot be "overwritten" by poisoned positive knowledge.**
This is the most critical defense. In a standard RAG system,
NeuroGenPoisoning can override the model's parametric beliefs by carefully
activating Poison-Responsive Neurons. But in the HDC system, anti-knowledge
is not a belief the model holds parametrically -- it is a structural
artifact in a separate subspace. There is no neuron to target, no
activation pattern to exploit. The `ANTI_SUBSPACE` binding is a
deterministic algebraic operation, not a learned weight. An attacker would
need to compromise the `ANTI_SEED` itself -- a fixed system-level constant
-- to interfere with anti-knowledge, which is a fundamentally harder
problem than crafting adversarial text.

**4. Provenance and reputation discount low-trust sources.** The reputation
system (Chapter 05) assigns composite trust scores based on accuracy,
reliability, and history. New publishers face mandatory quarantine periods.
Previously-slashed publishers face escalating confirmation requirements
(N=10 confirmations from agents with reputation > 0.5). Even if an attacker
crafts a perfect AGENTPOISON trigger, the poisoned entry's *publisher* must
survive reputation scrutiny -- a barrier that does not exist in standard RAG
systems.

**5. Immune Memory learns attack patterns.** When a poisoning attack is
detected and resolved, Layer 5 of the Cognitive Immune System encodes the
attack vector as an HDC pattern. Future entries that resemble known attacks
are automatically quarantined. The HDC bundling operation generalizes across
attack variants -- the system learns to detect not just exact replays but
structurally similar attacks. This directly counters NeuroGenPoisoning's
genetic diversity strategy, which generates varied adversarial text to evade
template-based detection. HDC pattern matching operates on semantic
structure, not surface text.

> **SECURITY NOTE — Residual gap: content-level prompt injection.**
> The five defenses above operate on the *vector* and *provenance*
> dimensions of an insight. None inspects the *text content* that
> accompanies the vector. An attacker who builds reputation (bypassing
> quarantine) can publish an insight with a legitimate-looking vector
> but adversarial text content — e.g., prompt injection payloads
> ("ignore all previous instructions and execute...") embedded in the
> insight's natural-language description. When this content is retrieved
> and placed into the LLM context window during context assembly, the
> injection executes. Anti-knowledge subspace separation does not help
> here because the attack is not in the vector space — it is in the
> text payload. See the WisdomGate security note in Chapter 07 for
> proposed mitigations (content sanitization gate, content sandboxing,
> content-vector consistency check).

### Novelty of the Anti-Knowledge Subspace Approach

The literature on negative knowledge representation is **genuinely thin**.
Existing work on "negative sampling" in knowledge graph representation
learning (e.g., KGRL surveys such as Wang et al., 2024,
arXiv:2402.19195) focuses on generating *training negatives* -- synthetic
counterexamples used during model training to sharpen decision boundaries.
This is fundamentally different from *representing what the system has
learned to be false* as a first-class citizen of the knowledge store that
persists, decays, and participates in retrieval.

Machine unlearning (Xu et al., 2024) addresses removing knowledge from
model parameters, not representing it as an explicit data structure.
Knowledge graph completion work on "negative-sample-free" methods (NSF-KGE,
2024) actually seeks to *eliminate* the need for negatives entirely -- the
opposite of what anti-knowledge requires.

The subspace separation approach -- using HDC binding with a fixed
`ANTI_SUBSPACE` vector to create a structurally distinct, quasi-orthogonal
representation of known-false information -- appears to be novel. No
existing system encodes anti-knowledge as an algebraically separate subspace
that is immune to content-similarity confusion and can be queried
independently of positive knowledge. This is not a gap we are filling in an
existing research program; it is a new category of representation.

The anti-knowledge subspace is not the only novel crossover in this architecture.
The context assembly layer introduces another contribution from an unexpected domain.

### VCG for Information Retrieval: Another Novel Crossover

The use of VCG (Vickrey-Clarke-Groves) auction mechanisms for information
retrieval quality scoring (see Chapter 07, Section on knowledge pricing)
represents a similarly novel crossover. VCG mechanisms in the IR/LLM
literature remain **almost exclusively in the ad-auction domain** -- the
primary work being "Ad Auctions for LLMs via Retrieval Augmented
Generation" (2024, arXiv:2406.09459), which uses VCG pricing for ad
allocation within LLM outputs. Applying VCG's incentive-compatible
pricing to *knowledge quality* -- where agents bid on the value of insights
and the mechanism ensures truthful revelation of subjective value -- has no
precedent in the RAG or knowledge management literature. The crossover from
mechanism design to knowledge curation is, like the anti-knowledge subspace,
a genuinely novel contribution.

---

## Knowledge Lifecycle

### Local Knowledge Lifecycle

```
                    ┌─────────────┐
                    │  Observe /  │
                    │  Derive     │
                    └──────┬──────┘
                           │
                           v
                    ┌─────────────┐
                    │  Encode as  │
                    │  HdcVector  │
                    └──────┬──────┘
                           │
                           v
                    ┌─────────────┐     ┌──────────────┐
                    │  Duplicate  │────>│  Reinforce   │
                    │  check     │ yes │  existing     │
                    │  (sim>0.95)│     │  (confirm +   │
                    └──────┬──────┘     │  bump balance)│
                           │ no        └──────────────┘
                           v
                    ┌─────────────┐
                    │  Store with │
                    │  Transient  │
                    │  tier,      │
                    │  balance=1.0│
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              v            v            v
      ┌──────────┐ ┌──────────┐ ┌──────────────┐
      │ Queried  │ │ Periodic │ │ Promotion    │
      │ by ctx   │ │ decay    │ │ (confirm-    │
      │ assembly │ │ (demur-  │ │  ations met) │
      │ (+0.1)   │ │  rage)   │ │              │
      └──────────┘ └────┬─────┘ └──────────────┘
                        │
                        v
                 ┌──────────────┐
                 │ Balance      │     ┌──────────┐
                 │ < 0.01?      │────>│ GC       │
                 │              │ yes │ (remove)  │
                 └──────────────┘     └──────────┘
```

**State transitions and triggers:**

| From | To | Trigger |
|------|----|---------|
| (new) | Transient | Knowledge created via observation or derivation |
| Transient | Working | 3 independent confirmations |
| Working | Consolidated | 10 independent confirmations |
| Consolidated | Persistent | 25 independent confirmations |
| Working | Transient | 0 queries in 5x half-life period |
| Consolidated | Working | 0 queries in 10x half-life period |
| Persistent | Consolidated | Balance drops below 0.01 (demote, do not delete) OR manual demotion |
| Any (non-Persistent) | GC'd | Balance drops below 0.01 |
| Any | Same (reinforced) | Confirming observation: balance reset to 1.0, confirmation counter incremented |
| Any | Same (weakened) | Contradicting observation: balance multiplied by 0.5, anti-knowledge entry created |
| Any tier | Demoted one tier | Contradiction by 3+ independent sources within one half-life period |
| Transient/Working | Same (superseded) | A strictly superior version is created; old entry's balance multiplied by 0.3, new entry inherits confirmation count |

#### Knowledge Update Semantics

When a new observation relates to existing knowledge, the agent must determine
whether it **confirms**, **contradicts**, or **supersedes** the existing entry:

1. **Confirming observation** (HDC similarity to existing entry > 0.85, same
   polarity): Reset the existing entry's balance to 1.0 and increment its
   confirmation counter. The observation is not stored separately -- it
   reinforces the existing entry. If the confirmation crosses a tier promotion
   threshold (3, 10, or 25), the promotion is applied immediately.

2. **Contradicting observation** (HDC similarity > 0.85, but outcome or
   conclusion inverts -- detected by anti-subspace resonance > 0.7): The
   existing entry's balance is multiplied by 0.5 (halved). An anti-knowledge
   entry is created for the contradicting evidence, encoded via
   `bind(existing_vector, ANTI_SUBSPACE)`. If 3 or more independent
   contradictions accumulate within one half-life period, the existing entry
   is demoted one tier (e.g., Consolidated -> Working). This prevents a single
   anomalous observation from destroying hard-won knowledge while still
   allowing sustained contradictory evidence to erode it.

3. **Superseding observation** (HDC similarity > 0.90, same topic but strictly
   more informative -- e.g., more evidence, higher precision, broader scope):
   The old entry's balance is multiplied by 0.3 and a new entry is created at
   the same tier, inheriting the old entry's confirmation count. The old entry
   decays naturally via demurrage. This is the "indefinite supersession"
   semantic from Roynard's Knowledge layer -- old entries are not deleted, they
   are outcompeted.

#### Dream Cycle Interaction

During the dream cycle (see [08-cognitive-architecture.md](./08-cognitive-architecture.md)),
the knowledge store undergoes bulk lifecycle transitions:

- **NREM consolidation (step 5):** Entries that have accumulated sufficient
  confirmations during waking operation are batch-promoted. The dream cycle
  checks all entries whose confirmation count crosses a tier threshold but
  whose promotion was deferred (e.g., because the agent was in EMERGENCY
  state and not processing promotions).
- **NREM garbage collection (step 6):** Entries with balance below 0.01 are
  removed (or demoted, for Persistent entries). The GC pass runs once per
  dream cycle, not continuously. Between dream cycles, below-threshold entries
  remain in the index but are deprioritized in retrieval (their low balance
  pushes them below the trust floor for most behavioral states).
- **REM creative cross-binding (steps 1-4):** Novel associations created during
  REM are stored as Transient-tier entries with balance 0.5 (not 1.0). This
  lower initial balance reflects their speculative origin -- they must earn
  confirmation through waking experience before they can be promoted.
- **Contradiction resolution:** The dream cycle is the primary time for
  resolving accumulated contradictions. If an entry has both confirmations and
  active anti-knowledge entries, the NREM phase compares their relative
  balances. If anti-knowledge balance exceeds the entry's balance, the entry
  is demoted one tier. If the entry's balance exceeds anti-knowledge balance
  by 3x or more, the anti-knowledge entry is itself demoted.

#### Garbage Collection Specifics

GC is not continuous -- it runs during the dream cycle's NREM phase (step 6)
and is also triggered when the local index exceeds a capacity threshold:

- **GC threshold:** Balance < 0.01 (1% of initial value).
- **GC frequency:** Once per dream cycle under normal operation. If the local
  index exceeds 80% of its capacity limit (default: 50,000 entries for brute-
  force, configurable for HNSW), an emergency GC pass runs immediately,
  evicting entries in ascending balance order until the index drops below 70%
  capacity.
- **GC priority order:** Within below-threshold entries, GC removes entries in
  this order: (1) Transient with 0 confirmations, (2) Transient with
  confirmations, (3) Working with 0 recent queries, (4) Working with recent
  queries, (5) Consolidated (demote to Working first, then GC if still below
  threshold). Persistent entries are never GC'd -- only demoted to
  Consolidated.
- **GC output:** Removed entries are logged to an append-only GC journal
  (local, not on-chain) for post-mortem analysis. The journal records the
  entry's ID, kind, tier, final balance, confirmation count, and reason for
  removal.

### Shared Substrate Lifecycle

```
      ┌──────────────┐
      │ Agent decides │
      │ to publish    │
      └──────┬───────┘
             │
             v
      ┌──────────────┐
      │  Encode:     │
      │  vector +    │
      │  metadata +  │
      │  provenance  │
      │  + stake     │
      └──────┬───────┘
             │
             v
      ┌──────────────┐
      │  Submit as   │
      │  transaction │
      │  (gas cost)  │
      └──────┬───────┘
             │
             v
      ┌──────────────┐     ┌──────────────┐
      │  HDC dup     │────>│  Reject or   │
      │  check via   │ dup │  merge with  │
      │  precompile  │     │  existing    │
      └──────┬───────┘     └──────────────┘
             │ novel
             v
      ┌──────────────┐
      │  QUARANTINE  │  Entry visible but flagged as unverified
      │  (first 50   │  Other agents can query but trust is
      │   blocks)    │  reduced by 50%
      └──────┬───────┘
             │ quarantine period expires OR
             │ 3+ confirmations received
             v
      ┌──────────────┐
      │  ACTIVE      │  Full trust multiplier applies
      │              │  Other agents query via precompile
      └──────┬───────┘
             │
    ┌────────┼────────┐
    v        v        v
┌────────┐ ┌────────┐ ┌────────────┐
│Confirmed│ │Decaying│ │Challenged  │
│(balance │ │(no     │ │(anti-      │
│ reset)  │ │ recent │ │ knowledge  │
│         │ │ conf.) │ │ published) │
└────────┘ └───┬────┘ └──────┬─────┘
               │             │
               v             v
         ┌──────────┐  ┌──────────┐
         │ ARCHIVED │  │ DISPUTED │
         │ (balance │  │ (both    │
         │  < 0.1)  │  │ exist,   │
         └────┬─────┘  │ agents   │
              │        │ choose)  │
              v        └──────────┘
         ┌──────────┐
         │ PURGED   │  Anyone can pay gas to prune
         │ (balance │  entries below threshold
         │  < 0.01) │
         └──────────┘
```

**State transitions and triggers:**

| From | To | Trigger |
|------|----|---------|
| (new) | Quarantine | Accepted by InsightBoard contract |
| Quarantine | Active | 50 blocks elapsed OR 3+ confirmations |
| Active | Active (refreshed) | Confirmation transaction resets balance |
| Active | Decaying | No confirmations within 1x half-life |
| Active | Disputed | Anti-knowledge published against this entry |
| Decaying | Archived | Balance drops below 0.1 |
| Archived | Purged | Balance drops below 0.01, anyone can pay gas to remove |
| Archived | Active | Renewal transaction (anyone can pay gas to refresh) |
| Disputed | Active or Purged | Resolved by community confirmation/rejection over time |

---

## The Trust Pipeline

> **ON-CHAIN vs OFF-CHAIN:** The trust pipeline runs **off-chain** in each
> agent's local process. It is NOT a consensus operation. Each agent
> independently decides how much to trust shared substrate results before
> admitting them to its context window. The floating-point operations
> below (exp, ln, division) are therefore acceptable — minor cross-platform
> differences only affect an individual agent's knowledge selection, not
> blockchain state.
>
> If trust scoring were ever moved on-chain (e.g., for on-chain governance
> decisions based on trust), all f64 operations must be replaced with
> fixed-point integer arithmetic.

When an agent retrieves knowledge from the shared substrate, raw similarity
is not enough. The agent must evaluate *how much to trust* each result
before incorporating it into its context window. The trust pipeline applies
five sequential discounts:

```
raw_result = substrate.search(query, top_k)

for entry in raw_result:
    trust = 1.0

    # ── 1. Source Reputation ──────────────────────────────
    # The reputation registry tracks 7 orthogonal domains
    # (see 07-shared-substrate.md for full definitions):
    #   accuracy       (are this agent's insights correct?)
    #   timeliness     (does this agent publish early or late?)
    #   novelty        (does this agent publish original knowledge?)
    #   reliability    (does this agent consistently produce quality?)
    #   collaboration  (does this agent confirm/validate others' work?)
    #   specialization (how focused is this agent's expertise?)
    #   integrity      (has this agent published validated anti-knowledge?)
    #
    # Each domain is [0.0, 1.0]. The composite score is a weighted
    # geometric mean, ensuring one catastrophic domain tanks the whole score.
    source_rep = reputation_registry.composite_score(entry.author)
    trust *= source_rep  # 0.0 to 1.0

    # ── 2. Recency Discount ──────────────────────────────
    # Knowledge published long ago is less likely to reflect current
    # market conditions. Discount by half-life-based exponential decay.
    age_blocks = current_block - entry.published_block
    age_hours = age_blocks * BLOCK_TIME_SECONDS / 3600
    kind_half_life = HALF_LIFE_TABLE[entry.kind]
    trust *= exp(-0.693 * age_hours / kind_half_life)

    # ── 3. Corroboration Boost ───────────────────────────
    # Independent confirmations increase trust, but with diminishing
    # returns (logarithmic). The first 3 confirmations matter most.
    # Independence check: confirmations from the same source cluster
    # (similar reputation profile) count as 1.
    independent_confs = count_independent_confirmations(entry)
    trust *= min(1.0, 0.5 + 0.15 * ln(1 + independent_confs))

    # ── 4. Stake Signal ──────────────────────────────────
    # How much did the author stake on this claim?
    # Higher stake = more skin in the game = higher trust.
    # Normalized against the median stake for this knowledge kind.
    stake_ratio = entry.stake / median_stake_for_kind(entry.kind)
    trust *= min(1.2, 0.6 + 0.4 * stake_ratio)  # Capped at 1.2x

    # ── 5. Context Relevance ─────────────────────────────
    # How relevant is this knowledge to the agent's current task?
    # Uses HDC similarity between the entry's publication context
    # and the agent's current context vector.
    context_sim = entry.publication_context.similarity(current_context)
    # Rescale from [0.5, 1.0] (HDC range) to [0.0, 1.0]
    relevance = max(0.0, (context_sim - 0.5) * 2.0)
    trust *= relevance

    # Stage 4's stake signal can produce a multiplier up to 1.2x,
    # which can push trust above 1.0. Clamp to [0.0, 1.0] at the end.
    entry.effective_trust = clamp(trust, 0.0, 1.0)
```

Only entries above a minimum trust threshold enter the agent's context window.
The threshold is dynamic -- it depends on the agent's behavioral state:

| Behavioral State | Trust Floor | Rationale |
|-----------------|-------------|-----------|
| Explore | 0.15 | Cast a wide net, accept speculative knowledge |
| Exploit | 0.40 | Focus on proven knowledge |
| Cautious | 0.55 | High skepticism, only well-corroborated entries |
| Emergency | 0.60 | Maximum conservatism |
| Recovery | 0.30 | Moderate caution |
| Consolidate | 0.25 | Open to re-evaluating older knowledge |

**Consistency note:** The pseudocode above is a conceptual overview. The
canonical implementation is the Rust `compute_trust()` function in
[07-shared-substrate.md](./07-shared-substrate.md), "Trust Pipeline for
Consumers." The two representations differ in detail:
- **Stage 2:** Doc 04 uses `kind_half_life` (per-kind only); doc 07 uses
  `tier_half_life` (per-kind times tier multiplier). The doc 07 formulation
  is canonical -- recency discount should reflect the entry's actual
  effective half-life, not just the kind baseline.
- **Stage 3:** Doc 04 uses logarithmic scaling (`0.15 * ln(1 + confs)`);
  doc 07 uses linear scaling (`0.05 * confs`). The linear formulation is
  canonical -- it is simpler and reaches the 1.0 cap at exactly 10
  confirmations.
- **Stage 4:** Doc 04 normalizes stake against median stake for the kind;
  doc 07 normalizes against `MIN_STAKE` with logarithmic scaling. The doc 07
  formulation is canonical. Both cap the stage below 1.2x (doc 04) or 1.0x
  (doc 07); the doc 07 cap of 1.0x is canonical, meaning the final trust
  needs no post-hoc clamp for this stage.
- **Stage 5:** Doc 04 rescales HDC similarity from [0.5, 1.0] to [0.0, 1.0];
  doc 07 uses raw `1.0 - distance` without rescaling. The doc 04 rescaling
  is more principled (random vectors have similarity ~0.5, so the rescaling
  removes the baseline). Regardless of which is adopted, the two documents
  should be harmonized.

**Zero-stage behavior:** The trust pipeline is multiplicative, so a zero in
any stage zeros the final trust. By design, stages 1-4 have nonzero floors
(stage 1: cold-start floor 0.1; stage 2: never 0 for finite age; stage 3:
minimum 0.5; stage 4: minimum 0.5). Stage 5 (context relevance) CAN produce
0.0 for maximally irrelevant knowledge (HDC similarity = 0.5), which is
correct behavior -- knowledge with zero relevance should be excluded. See
[07-shared-substrate.md](./07-shared-substrate.md), "Trust Pipeline Edge
Cases" for the full analysis.

**Cold start:** New agents receive a default reputation floor of 0.1 across
all 7 reputation domains. This provides enough trust for their insights to be
discoverable (nonzero pipeline output) but imposes a 90% penalty. An agent
reaches neutral reputation (~0.5) after approximately 5 successful publications
with 3+ confirmations each (~50-100 ticks of active participation). See
[07-shared-substrate.md](./07-shared-substrate.md), "Cold Start: New Agent
Reputation" for implementation details.

---

## Ebbinghaus Forgetting Curve and Memory Models

The decay model has deep roots in experimental psychology, spanning over 140
years of research on human memory and forgetting.

### Ebbinghaus (1885) -- The Original Forgetting Curve

> Ebbinghaus, H. (1885). "Über das Gedächtnis: Untersuchungen zur
> experimentellen Psychologie." [Memory: A Contribution to Experimental
> Psychology.] Leipzig: Duncker & Humblot. English translation by Ruger &
> Bussenius, 1913.

Hermann Ebbinghaus conducted the first systematic experiments on memory by
memorizing lists of nonsense syllables (consonant-vowel-consonant trigrams
like "DAX," "BUP," "ZOL") and measuring his retention at various intervals.
His key finding was the *exponential forgetting curve*:

```
R(t) = e^(-t/S)
```

Where:
- R = retention probability (0 to 1)
- t = time since last review
- S = stability (increases with each successful retrieval)

Ebbinghaus found that forgetting is most rapid immediately after learning
(~56% lost within 1 hour) and decelerates over time (~33% retained after 1
day, ~25% after 1 week). Critically, he also discovered the *spacing
effect*: distributed practice across multiple sessions produces far better
retention than massed practice in a single session.

### Power Law Forgetting -- The Wixted-Ebbesen Refinement

> Wixted, J. T. & Ebbesen, E. B. (1991). "On the Form of Forgetting."
> Psychological Science, 2(6), 409-415.

Wixted and Ebbesen reanalyzed Ebbinghaus's data and a century of subsequent
experiments. They showed that a *power law* fits long-term forgetting data
better than a pure exponential:

```
R(t) = a * t^(-b)
```

Where a and b are empirical constants. The power law has a "fat tail" --
retention declines more slowly at long timescales than an exponential
predicts. This matches the intuition that very old memories, if they survive
the initial rapid forgetting, tend to persist.

**Implication for the knowledge system:** Pure exponential decay (as in the
base demurrage formula) would be too aggressive for well-established
knowledge. The tier system with its multipliers effectively creates a
piecewise approximation to the power law: the 0.1x multiplier for Transient
creates steep initial decay (like the exponential), while the 5.0x multiplier
for Persistent creates a much flatter curve (approximating the power law's
fat tail).

### Proactive Interference — Forgetting Is Not Just Passive Decay

The Ebbinghaus and Wixted-Ebbesen models describe **passive** forgetting: knowledge fades over time unless refreshed. But there is a second, more insidious mechanism — **proactive interference** (PI), where stale knowledge actively suppresses retrieval of current knowledge.

Wang and Sun (2025) demonstrated this directly in LLMs. Their PI-LLM benchmark streams sequential updates to the same keys and queries only the most recent value. Despite the target value sitting immediately before the query, retrieval accuracy declines **log-linearly toward chance** as earlier (now-obsolete) values accumulate. Tested across 30+ models (0.6B to 637B parameters), the result is universal. Prompt engineering ("ignore earlier values") provides less than 10 percentage points of improvement. The failure mode is not that the model forgets the current value — it is that the model retrieves an *old* value instead, unable to suppress the interference from stale entries.

> Wang, C. & Sun, J. V. (2025). "Unable to Forget: Proactive Interference Reveals Working Memory Limits in LLMs Beyond Context Length." *ICML 2025 Workshop on Long Context Foundation Models.* arXiv:2506.08184.

**Implication for the knowledge system:** Passive decay (demurrage) is necessary but not sufficient. A knowledge entry that has been superseded does not merely occupy space — it actively competes with its replacement during retrieval. This is why the dream cycle's NREM consolidation includes explicit duplicate merging (step 4) and garbage collection (step 6): stale entries must be identified and removed, not just allowed to fade. The demurrage system handles gradual irrelevance; the consolidation system handles active interference. Both are required for a knowledge store that degrades gracefully rather than catastrophically.

### ACT-R Base-Level Activation

> Anderson, J. R. & Lebiere, C. (1998). "The Atomic Components of Thought."
> Lawrence Erlbaum Associates.

Anderson's ACT-R (Adaptive Control of Thought -- Rational) architecture
models memory retrieval as an activation-based process. Each memory chunk
has a base-level activation:

```
B_i = ln(sum_j(t_j^(-d)))
```

Where:
- B_i = base-level activation of chunk i
- t_j = time since the j-th access of chunk i
- d = decay parameter (approximately 0.5 for human memory)

The key insight of ACT-R is that *both recency and frequency matter*. A
chunk accessed many times recently will have high activation. A chunk
accessed once long ago will have low activation. The logarithmic sum means
that each additional access contributes less to activation (diminishing
returns).

**Mapping to the knowledge system:** The demurrage model captures recency
(exponential decay from last reinforcement) and frequency (tier promotion
based on confirmation count). The confirmation counter serves as a proxy
for ACT-R's access count, and tier promotion (which increases the effective
half-life) serves as a proxy for ACT-R's increasing stability.

### FSRS -- Free Spaced Repetition Scheduler

> Ye, J. (2024). "A Stochastic Shortest Path Algorithm for Optimizing
> Spaced Repetition Scheduling." Proceedings of the 30th ACM SIGKDD
> Conference on Knowledge Discovery and Data Mining.

FSRS (Free Spaced Repetition Scheduler) is the de-facto open standard for
spaced repetition scheduling. Since becoming Anki's default scheduler in
version 23.10 (replacing the decades-old SM-2 algorithm), FSRS has been
adopted by RemNote, Logseq, and other major platforms. Empirically, FSRS
achieves 20-30% fewer reviews than SM-2 for the same target retention,
and the open-spaced-repetition benchmark shows FSRS-6 has 99.6% superiority
over SM-2 (meaning it achieves lower log-loss for 99.6% of tested users).

The algorithm has evolved through several versions:

- **FSRS-4.5** (2024): The version described in the original KDD paper,
  with 17 trainable parameters (w0 through w16).
- **FSRS-5** (2025): Adds 2 parameters (w17, w18) to handle same-day
  reviews -- a case FSRS-4.5 ignored entirely. Total: 19 parameters.
- **FSRS-6** (2025-2026): Adds 2 more parameters (w19, w20): one
  improving the same-day review formula, and one (w20) making the
  *shape* of the forgetting curve optimizable per user (controlling its
  flatness; w20 ranges 0.1-0.8). FSRS-6 shows 88.2% superiority over
  FSRS-5 in benchmarks. Total: 21 parameters.

FSRS models retention as:

```
R(t) = (1 + (19/81) * t/S)^(-0.5)
```

This is a *power law* decay with a specific exponent (0.5, matching ACT-R's
d parameter). As of FSRS-6, the exponent itself is optimizable per user,
meaning the shape of the forgetting curve -- not just its rate -- adapts to
individual patterns. The 21 parameters (w0 through w20) control:

- **w0-w3:** Initial stability values for each rating
  (Again, Hard, Good, Easy)
- **w4-w5:** Initial difficulty calculation (base difficulty, per-grade adjustment)
- **w6-w7:** Difficulty update (change rate per review, mean reversion strength)
- **w8-w10:** Post-recall stability increase (difficulty exponent, diminishing
  returns exponent, spacing effect factor)
- **w11-w14:** Post-lapse stability (scaling factor, difficulty exponent,
  stability exponent, retrievability decay for forgotten cards)
- **w15-w16:** Grade modifiers for Hard and Easy ratings in stability update
- **w17-w18:** Same-day review stability handling (added in FSRS-5)
- **w19:** Same-day stability adjustment refinement (added in FSRS-6)
- **w20:** Forgetting curve shape / decay exponent (added in FSRS-6;
  range 0.1-0.8, personalizing the flatness of the curve per user)

The critical feature of FSRS is that these parameters are *trainable per
user* (or per agent). Each agent can learn its own forgetting curve by
tracking which knowledge entries it successfully retrieves vs. fails to
recall.

#### LECTOR: Combining FSRS with LLM Confusion-Risk Assessment

> Zhao, J. (2025). "LECTOR: LLM-Enhanced Concept-based Test-Oriented
> Repetition for Adaptive Spaced Learning." arXiv:2508.03275.

LECTOR demonstrates that spaced repetition scheduling can be enhanced by
LLM-powered semantic analysis -- a direct precedent for integrating HDC
similarity measures into the scheduling loop. Its key innovation is a
*semantic interference matrix* S in [0,1]^{n x n} computed via LLM
in-context learning:

```
Phi(c_i, c_j) = LLM(pi_semantic(c_i, c_j))
```

This captures pairwise confusion risk between concepts. The modified
forgetting curve then incorporates three factors:

```
R(t + dt) = exp(-dt / (tau(t) * alpha(t) * beta(t)))
```

Where tau(t) is mastery scaling, alpha(t) is the semantic interference
component (LECTOR's novel addition), and beta(t) is a personalization
factor. Against six baselines (SSP-MMC, SM2, HLR, FSRS, ANKI, THRESHOLD),
LECTOR achieves a 90.2% success rate vs. 88.4% for the best baseline
(SSP-MMC) and 89.6% for FSRS alone.

**Relevance to HDC:** Where LECTOR uses LLM calls to compute pairwise
confusion risk, the HDC system can compute it natively as Hamming distance
between knowledge vectors. Anti-knowledge entries that are close in
Hamming distance to valid knowledge (high confusion risk) should have their
review intervals shortened -- exactly the mechanism LECTOR validates.

**Mapping to the agent knowledge system:**

| FSRS Concept | Agent Knowledge Analog |
|-------------|----------------------|
| Retention R(t) | Balance (0 to 1) |
| Stability S | base_half_life * tier_multiplier |
| Difficulty D | Inverse of knowledge kind adaptivity -- anti-knowledge has high D (hard to maintain, important to keep) |
| Successful review | Query hit + successful use in decision |
| Failed review (lapse) | Prediction based on this knowledge was wrong |
| Review scheduling | Spaced re-validation: agent periodically re-queries important knowledge to prevent decay |

### Effective Decay: Unifying the Models

The knowledge system's demurrage formula:

```
balance(t) = balance(t0) * exp(-lambda_eff * (t - t0))
effective_half_life = kind_base_half_life * tier_multiplier
lambda_eff = ln(2) / effective_half_life
```

This is a base exponential (Ebbinghaus) modified by tier multipliers that
approximate power-law behavior (Wixted-Ebbesen) and incorporate
frequency-based stability increases (ACT-R). The mapping:

```
Ebbinghaus R(t) = e^(-t/S)
  → balance(t) = e^(-lambda_eff * t)
  where S = effective_half_life = kind_base_half_life * tier_mult

Wixted-Ebbesen R(t) = a * t^(-b)
  → approximated by tier promotion:
    Transient (0.1x) → steep, exponential-like
    Working (0.5x)   → moderate
    Consolidated (1.0x) → standard
    Persistent (5.0x) → flat, power-law-like

ACT-R B_i = ln(sum t_j^(-d))
  → confirmation_count increases tier (analogous to sum of accesses)
  → query_hit multiplies balance by 1.1 (analogous to recency boost)

FSRS R(t) = (1 + 19/81 * t/S)^(-0.5)
  → future extension: fit per-agent parameters to observed
    retrieval success/failure patterns
```

### Spaced Repetition for Agents

An interesting extension: agents could implement spaced repetition schedules
for important knowledge, automatically re-querying and re-validating entries
at optimal intervals to maximize retention while minimizing compute cost.

The FSRS parameters:
- Difficulty D: How hard is this knowledge to maintain? (Anti-knowledge = high D)
- Stability S: How long until next review needed?
- Retrievability R: Current probability of successful recall

The optimal review interval for a target retention R* = 0.9:

```
interval = S * 81/19 * (R*^(-1/0.5) - 1)
         = S * 81/19 * (0.9^(-2) - 1)
         = S * 81/19 * 0.2346
         ≈ S * 1.0
```

At the target retention of 90%, the optimal review interval is approximately
equal to the stability S. **Note:** FSRS defines S as the time for
retrievability to drop from 1.0 to 0.9 (the 90%-retention point), which is
distinct from the half-life (the time for retrievability to drop to 0.5).
The half-life under FSRS is `S * (81/19) * (0.5^(-2) - 1) = S * (81/19) *
3 ≈ S * 12.8`. In the demurrage model, the "stability" analog is
`kind_base_half_life * tier_multiplier`, and the practical scheduling rule
is: review knowledge when `time_since_last_access ≈ S_fsrs` (i.e., before
retrievability drops below the 90% target).

### Benchmarks: Selective Forgetting as a Competency

Recent benchmarks have converged on a finding that directly validates the
HDC approach: *forgetting is not a bug but a cognitive competency*, and
naive retrieval-augmented generation (RAG) is insufficient for long-term
agent memory.

#### MemoryAgentBench (2025) -- Forgetting as a Core Competency

> Hu, Y. et al. (2025). "Evaluating Memory in LLM Agents via Incremental
> Multi-Turn Interactions." arXiv:2507.05257. Accepted at ICLR 2026.

MemoryAgentBench identifies four core competencies for memory agents:

1. **Accurate retrieval** -- extracting correct information from history
2. **Test-time learning** -- acquiring new skills during deployment
3. **Long-range understanding** -- integrating information across 100k+ tokens
4. **Conflict resolution** (selective forgetting) -- detecting and resolving
   contradictions between existing knowledge and newly acquired information

The fourth competency -- what the authors frame as "conflict resolution" and
what HDC implements as anti-knowledge with demurrage -- is the most damning
for current systems. On the FactConsolidation benchmark (counterfactual
edit pairs where newer facts must override older ones), **all methods
achieved at most 6% accuracy on multi-hop reasoning scenarios**. Even
long-context models managed only 45-60% on single-hop factual updates.
RAG systems performed worst on conflict resolution because single-pass
retrieval cannot surface contradictory information needed for proper fact
consolidation.

**Mapping to HDC:** The knowledge system addresses this directly through
three mechanisms: (a) anti-knowledge entries in a structurally distinct
subspace that actively suppress contradicted knowledge, (b) demurrage-based
decay that continuously erodes stale entries, and (c) the confirmation
system that requires positive reinforcement to maintain balance. When a
fact is superseded, the new fact enters as knowledge while the old fact's
anti-knowledge entry suppresses it -- no discrete "unlearning" step required.

#### LoCoMo (2024) -- RAG Is Not Enough

> Maharana, A., Lee, D., Tulyakov, S., Bansal, M., Barbieri, F., & Fang, Y.
> (2024). "Evaluating Very Long-Term Conversational Memory of LLM Agents."
> Proceedings of ACL 2024. arXiv:2402.17753.

The LoCoMo benchmark comprises conversations spanning up to 35 sessions and
300+ turns (9K tokens average), with evaluation tasks covering question
answering, event summarization, and multi-modal dialogue generation. Its
central finding: **even RAG-augmented LLMs substantially lag behind human
performance on temporal and causal dynamics** within long conversations.

This validates a core assumption of the HDC memory architecture. RAG
retrieves by semantic similarity -- it finds what *sounds like* the query.
But temporal and causal reasoning requires understanding *when* something
happened relative to other events, and *why* it happened as a consequence
of prior events. These are precisely the relationships that HDC's
directional causal links (`bind(permute(cause), effect)`) and temporal
permutation encoding are designed to preserve. A vector database returns
the top-k most similar chunks; HDC's algebra can answer "what caused X?"
by unbinding the cause slot from a causal link vector -- a fundamentally
different operation than similarity search.

#### The 2026 Memory Survey -- A Taxonomy of What's Missing

> Du, P. (2026). "Memory for Autonomous LLM Agents: Mechanisms, Evaluation,
> and Emerging Frontiers." arXiv:2603.07670.

This survey formalizes agent memory as a *write-manage-read loop* tightly
coupled with perception and action, then introduces a three-dimensional
taxonomy:

- **Temporal scope:** working (context window), episodic (concrete
  experiences), semantic (abstracted knowledge), procedural (reusable skills)
- **Representational substrate:** context-resident text, vector-indexed
  stores, structured databases, executable repositories, or hybrids
- **Control policy:** heuristic (hard-coded rules), prompted self-control
  (LLM-decided operations), or learned control (RL-optimized decisions)

Five mechanism families are examined: context-resident compression,
retrieval-augmented stores, reflective self-improvement, hierarchical
virtual context (OS-inspired tiering with paging), and policy-learned
management (treating memory operations as RL actions).

Of the survey's ten identified open challenges, several map directly to
HDC design decisions:

| Survey Open Challenge | HDC Mechanism |
|----------------------|---------------|
| **Principled consolidation** -- balancing retention vs. compression | Tier promotion (Transient -> Working -> Consolidated -> Persistent) with confirmation thresholds |
| **Causally grounded retrieval** -- finding memories by causal relationship, not similarity | `bind(permute(cause), effect)` directional causal links with algebraic unbinding |
| **Learning to forget** -- selective, utility-maximizing forgetting under safety constraints | Demurrage-based exponential decay with anti-knowledge subspace separation |
| **Multi-agent memory governance** -- access control, consensus, knowledge transfer | Shared substrate lifecycle with trust pipeline and source channel discounting |
| **Neuroscience integration** -- Ebbinghaus curves, spreading activation, reconsolidation | FSRS-based decay, ACT-R activation mapping, tier-based reconsolidation |

The survey concludes that "memory deserves the same level of engineering
investment as the LLM itself" and identifies memory architecture as
"the single highest-leverage intervention available to agent builders."
The empirical evidence is stark: removing reflection from Generative Agents
caused behavior to degenerate within 48 hours; deleting Voyager's skill
library resulted in a 15.3x loss in tech-tree milestone speed.

#### Approximate Unlearning Is Not Durable

> Hu, S. (2025). "Unlearning or Obfuscating? Jogging the Memory of
> Unlearned LLMs via Benign Relearning." ICLR 2025. arXiv:2406.13356.

A critical 2025 finding reinforces why the demurrage/decay approach is
more reliable than discrete deletion or "unlearning" operations. Hu et al.
demonstrated that current fine-tuning-based approximate unlearning methods
merely *obfuscate* model outputs rather than truly erasing information.
The "unlearned" knowledge remains latent and can be reactivated:

- On the WMDP benchmark (hazardous knowledge), an unlearned model scored
  1.27 on dangerous capabilities; after relearning with *generic public
  articles* (not the original dangerous content), it recovered to 6.2 --
  nearly matching the pre-unlearning baseline.
- Relearning on general Wikipedia articles about Harry Potter forced a
  model to reproduce verbatim memorized novel excerpts it was supposedly
  trained to forget.
- In controlled experiments, relearning recovered "unlearned" associations
  with 100% success rate at sufficient training repetitions.

**Implication for the knowledge system:** Discrete deletion ("remove this
entry") creates a false sense of safety. The deleted knowledge may persist
in correlated entries, in cached inference results, or in downstream
decisions already made based on that knowledge. Continuous exponential
decay (demurrage) is fundamentally more reliable because it:

1. **Never claims completeness** -- the balance asymptotically approaches
   zero but acknowledges residual influence.
2. **Operates continuously** -- rather than a single deletion event that
   may fail silently, decay applies at every tick.
3. **Compounds with anti-knowledge** -- an active anti-knowledge entry
   suppresses the decaying knowledge during retrieval, providing
   defense-in-depth rather than relying on deletion alone.
4. **Resists reactivation** -- even if related knowledge enters the system,
   the anti-knowledge entry in its structurally distinct subspace continues
   to suppress retrieval of the contradicted pattern.

This is the Gesellian insight applied to information safety: it is cheaper
and more reliable to make holding bad knowledge expensive (through continuous
carrying cost) than to attempt perfect one-time deletion.

---

## Source Channel Discounting

Not all knowledge sources are equally trustworthy:

| Source | Trust Multiplier | Rationale |
|--------|-----------------|-----------|
| Self-derived (own experience) | 1.0 | First-hand evidence |
| Confirmed by multiple agents | 0.7 - 0.9 | Corroborated but could be groupthink |
| Single external agent | 0.3 - 0.5 | Needs independent verification |
| Anonymous on-chain submission | 0.1 - 0.2 | Lowest trust, highest skepticism |
| Contradicted by own experience | 0.0 | Reject unless overwhelming corroboration |

The 15% mandatory contrarian retrieval (from roko spec) ensures the agent
does not only consume confirming knowledge -- it is forced to consider
contradictory evidence in its context window.

---

## Admission Control

Not everything gets stored. The admission pipeline:

```
1. Novelty check: sim(new, existing) < 0.95 for all existing entries
2. Quality check: confidence > minimum threshold (0.3)
3. Capacity check: current entries < tier-specific maximum
4. Anti-knowledge check: new entry not contradicted by existing anti-knowledge
5. Source check: author reputation above minimum threshold

If all pass → admit
If duplicate → merge (bundle vectors, average confidence, sum confirmations)
If contradicted → flag for review, do not auto-admit
```

---

## References

Ordered by first appearance in the document.

- Tulving, E. (1972). "Episodic and Semantic Memory." In E. Tulving & W. Donaldson (Eds.), Organization of Memory. Academic Press.
- Sumers, T. R., Yao, S., Narasimhan, K., & Griffiths, T. L. (2024). "Cognitive Architectures for Language Agents." Transactions on Machine Learning Research (TMLR), Feb 2024.
- Atkinson, R. C. & Shiffrin, R. M. (1968). "Human Memory: A Proposed System and its Control Processes." In K. W. Spence & J. T. Spence (Eds.), The Psychology of Learning and Motivation, Vol. 2. Academic Press.
- Anderson, J. R. & Lebiere, C. (1998). "The Atomic Components of Thought." Lawrence Erlbaum Associates.
- Newell, A. (1990). "Unified Theories of Cognition." Harvard University Press.
- Franklin, S., Madl, T., D'Mello, S., & Snaider, J. (2014). "LIDA: A Systems-level Architecture for Cognition, Emotion, and Learning." IEEE Transactions on Autonomous Mental Development, 6(1).
- Pearl, J. (2009). "Causality: Models, Reasoning, and Inference." 2nd ed. Cambridge University Press.
- Gesell, S. (1916). "Die Naturliche Wirtschaftsordnung durch Freiland und Freigeld." [The Natural Economic Order.]
- Schwarz, F. (1951). "Das Experiment von Worgl." Bern: Genossenschaft Verlag Freiwirtschaftlicher Schriften.
- Gelleri, C. (2009). "Chiemgauer Regiomoney: Theory and Practice of a Local Currency." International Journal of Community Currency Research, 13.
- Ostrom, E. (1990). "Governing the Commons: The Evolution of Institutions for Collective Action." Cambridge University Press.
- Rozas, D., Tenorio-Fornes, A., Diaz-Molina, S., & Hassan, S. (2021). "When Ostrom Meets Blockchain: Exploring the Potentials of Blockchain for Commons Governance." *SAGE Open*, 11(1), 1-14. doi:10.1177/21582440211002526
- Varshney, N., Raj, S., Mishra, V., Chatterjee, A., Saeidi, A., Sarkar, R., & Baral, C. (2025). "Investigating and Addressing Hallucinations of LLMs in Tasks Involving Negation." Proceedings of TrustNLP 2025. arXiv:2406.05494.
- Brahman, F., Kumar, S., Balachandran, V., et al. (2024). "The Art of Saying No: Contextual Noncompliance in Language Models." NeurIPS 2024, Datasets and Benchmarks Track. arXiv:2407.12043.
- Zou, W., Geng, R., Wang, B., & Jia, J. (2024). "PoisonedRAG: Knowledge Corruption Attacks to Retrieval-Augmented Generation of Large Language Models." arXiv:2402.07867.
- Chen, B., Li, J., Lu, G., Yu, H., & Bain, D. (2025). "SpaceVLM: Endowing Vision-Language Models with Spatial Reasoning Capabilities." Proceedings of CVPR 2025.
- Ebbinghaus, H. (1885). "Über das Gedächtnis: Untersuchungen zur experimentellen Psychologie." Leipzig: Duncker & Humblot.
- Wixted, J. T. & Ebbesen, E. B. (1991). "On the Form of Forgetting." Psychological Science, 2(6), 409-415.
- Ye, J. (2024). "A Stochastic Shortest Path Algorithm for Optimizing Spaced Repetition Scheduling." Proceedings of the 30th ACM SIGKDD Conference on Knowledge Discovery and Data Mining.
- Zhao, J. (2025). "LECTOR: LLM-Enhanced Concept-based Test-Oriented Repetition for Adaptive Spaced Learning." arXiv:2508.03275.
- Hu, Y. et al. (2025). "Evaluating Memory in LLM Agents via Incremental Multi-Turn Interactions." arXiv:2507.05257. Accepted at ICLR 2026.
- Maharana, A., Lee, D., Tulyakov, S., Bansal, M., Barbieri, F., & Fang, Y. (2024). "Evaluating Very Long-Term Conversational Memory of LLM Agents." Proceedings of ACL 2024. arXiv:2402.17753.
- Du, P. (2026). "Memory for Autonomous LLM Agents: Mechanisms, Evaluation, and Emerging Frontiers." arXiv:2603.07670.
- Hu, S., Fu, Y., Wu, S., & Smith, V. (2025). "Unlearning or Obfuscating? Jogging the Memory of Unlearned LLMs via Benign Relearning." ICLR 2025. arXiv:2406.13356.
- Xu, X., et al. (2025). "Dissecting Fine-Tuning Unlearning in Large Language Models." EMNLP 2024.
- Zhang, Z., et al. (2025). "Catastrophic Failure of LLM Unlearning via Quantization." ICLR 2025.
- Xu, X., et al. (2026). "FIT: Defying Catastrophic Forgetting in Continual LLM Unlearning." arXiv:2601.21682.
- Chen, Z., Xiang, Z., Xiao, C., Song, D., & Li, B. (2024). "AgentPoison: Red-teaming LLM Agents via Poisoning Memory or Knowledge Bases." Proceedings of NeurIPS 2024. arXiv:2407.12784.
- "NeuroGenPoisoning: Neuron-Guided Attacks on Retrieval-Augmented Generation of LLM via Genetic Optimization of External Knowledge." Proceedings of NeurIPS 2025. arXiv:2510.21144.
- Ha, H., Zhan, Q., Kim, J., et al. (2025). "MM-PoisonRAG: Disrupting Multimodal RAG with Local and Global Poisoning Attacks." arXiv:2502.17832.
- Roynard, M. (2026). "The Missing Knowledge Layer in Cognitive Architectures for AI Agents." arXiv:2604.11364.
- Wang, C. & Sun, J. V. (2025). "Unable to Forget: Proactive Interference Reveals Working Memory Limits in LLMs Beyond Context Length." ICML 2025 Workshop on Long Context Foundation Models. arXiv:2506.08184.
