# Cognitive Architecture

Memory hierarchy, affect system, and learning loops.

---

## What Is a Cognitive Architecture?

A cognitive architecture is a blueprint for how an intelligent agent perceives, thinks, decides, and learns. It is the structural specification of the mind — not the content of any particular thought, but the machinery that produces thought in the first place.

The term originates from Allen Newell's 1990 monograph *Unified Theories of Cognition*, in which he argued that the field of cognitive science had fractured into hundreds of micro-theories — one for memory retrieval, another for attention, another for problem solving — with no unifying framework. Newell's thesis was direct: we need a single, coherent architecture that accounts for the full range of cognition, not a patchwork of ad-hoc modules bolted together after the fact.

This matters for artificial agents for the same reasons it mattered for cognitive science. Without a cognitive architecture, an agent is simply:

```
prompt → LLM → action → (forget everything) → repeat
```

Such an agent has no memory continuity across sessions. It cannot learn from its own mistakes. It has no emotional modulation to shift strategy under stress. It has no metacognition — no capacity to reflect on whether its own reasoning is working. Every invocation starts from scratch, context-limited, amnesiac.

A cognitive architecture provides the scaffolding that turns a stateless language model into a persistent, adaptive, self-improving agent:

- **Memory hierarchy** — so the agent accumulates knowledge over time, with appropriate decay and consolidation.
- **Affect system** — so the agent modulates its behavior in response to success, failure, threat, and opportunity.
- **Learning loops** — so the agent improves its predictions and strategies through structured feedback.
- **Metacognition** — so the agent can reason about the quality of its own reasoning.
- **Behavioral state machine** — so the agent adapts its operational posture to circumstances.

The Roko cognitive architecture draws on sixty years of cognitive science research, from Newell and Simon's early production systems through modern LLM-based agent frameworks, to build a system that is both theoretically grounded and practically implementable on-chain.

---

## Memory Hierarchy

Roko agents have a tiered memory system inspired by human cognition and
formalized by the CoALA (Cognitive Architecture for Language Agents) framework.

### Four Memory Types

```
┌─────────────────────────────────────────────────┐
│                Working Memory                    │
│         (LLM context window — transient)        │
│                                                  │
│  Assembled by dynamic context assembly from:    │
│  ┌──────────────┐ ┌──────────┐ ┌──────────────┐ │
│  │  Episodic    │ │ Semantic │ │ Procedural   │ │
│  │  Memory      │ │ Memory   │ │ Memory       │ │
│  │              │ │          │ │              │ │
│  │  Episodes,   │ │ Insights,│ │ Strategies,  │ │
│  │  experiences │ │ heuristics│ │ action plans │ │
│  │  "what       │ │ causal   │ │ "how to      │ │
│  │   happened"  │ │ links    │ │   do things" │ │
│  │              │ │ "what's  │ │              │ │
│  │              │ │  true"   │ │              │ │
│  └──────────────┘ └──────────┘ └──────────────┘ │
│                                                  │
│         All stored as HDC vectors in:           │
│  ┌───────────────────────────────────────────┐  │
│  │          Local HDC Index                   │  │
│  │    (private, fast, full trust)             │  │
│  └───────────────────────────────────────────┘  │
│  ┌───────────────────────────────────────────┐  │
│  │        Shared HDC Substrate               │  │
│  │    (on-chain, slower, trust-weighted)      │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

### How They Map to Knowledge Kinds

| Memory Type | Knowledge Kinds | Retrieval Pattern |
|-------------|----------------|-------------------|
| Episodic | Episodes (implicit in Insight) | Situation-similarity search |
| Semantic | Insight, Heuristic, CausalLink | Concept-similarity search |
| Procedural | StrategyFragment | Goal-similarity search |
| Meta-cognitive | AntiKnowledge, Warning | Contradiction checking |

The power of HDC: all four memory types live in the same 10,240-dimensional
space. A single similarity search can retrieve across memory types.

---

## The Cognitive Loop

Each tick (one block, ~400ms), the agent runs:

```
┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
│ Perceive │ ──→ │  Think   │ ──→ │   Act    │ ──→ │  Learn   │
│          │     │          │     │          │     │          │
│ Read     │     │ Assemble │     │ Execute  │     │ Store    │
│ chain    │     │ context  │     │ decision │     │ outcome  │
│ state    │     │ + reason │     │ as tx    │     │ as HDC   │
└──────────┘     └──────────┘     └──────────┘     └──────────┘
     │                │                │                │
     └────────────────┴────────────────┴────────────────┘
                    Continuous loop
```

### Perceive

Read relevant chain state, incoming signals, pheromones, new shared insights.
Update internal world model.

### Think

1. Construct query vector from current task + perception
2. Search local + shared indexes (HDC context assembly)
3. Apply trust pipeline + ranking
4. Build prompt (9 layers)
5. Call LLM
6. Parse response

### Act

Convert LLM decision into chain actions:
- Submit transactions
- Publish insights
- Deposit pheromones
- Bid on jobs

### Learn

Record outcome of actions:
- Encode episode: `bind(situation, bind(action, outcome))`
- If positive outcome: reinforce related knowledge
- If negative outcome: create anti-knowledge
- Run predict-publish-correct loop

---

## Affect System — ALMA Three-Layer Model

Agents have emotions. Not for anthropomorphism — for functional benefit.

The theoretical basis is Gebhard's (2005) ALMA (A Layered Model of Affect), presented at the International Conference on Autonomous Agents and Multiagent Systems (AAMAS). ALMA distinguishes three temporal layers of affective state, each operating on a different time constant, each serving a different cognitive function. This layered design reflects the empirical observation that human affect operates at multiple timescales simultaneously — a flash of anger (emotion) does not change your overall mood, and a bad week (mood) does not change your personality.

Affect modulates:
- **Retrieval** (mood-congruent memory) — the agent preferentially recalls knowledge that matches its current affective state, mirroring the well-documented mood-congruent memory effect in human cognition.
- **Risk tolerance** (arousal level) — high arousal narrows attention and biases toward immediate, high-certainty actions; low arousal permits broader, more exploratory search.
- **Strategy selection** (dominance level) — high dominance favors agentic, initiative-taking strategies; low dominance favors conservative, pattern-following strategies.

### Three Layers

```
Layer 1: Emotion (τ = 0.1)
  Fast, reactive. Triggered by specific events.

  Time constant τ=0.1 means ~90% decay within a few ticks. Emotions are
  transient signals, not persistent states. They function as rapid appraisals
  of individual events — did this action succeed? Was this outcome expected?
  Is this situation threatening?

  Examples:
  - Surprise: prediction error exceeds threshold — actual outcome deviated
    sharply from forecast. Triggers increased attention and replay priority.
  - Satisfaction: goal completion detected — action achieved its intended
    outcome. Reinforces the strategy that led to success.
  - Frustration: repeated failures on the same task. Triggers strategy
    switching and exploratory behavior.
  - Alarm: sudden loss exceeding risk budget. Triggers emergency behavioral
    state transition.

  Emotions decay quickly, but their effects persist through two mechanisms:
  (1) they feed into the mood layer as a running average, and (2) they tag
  episodic memories with affective valence at encoding time, which influences
  future retrieval.

Layer 2: Mood (τ = 0.5)
  Medium timescale. Running average of recent emotions.

  Mood is the temporal integral of emotion, smoothed by exponential decay
  with τ=0.5. Where emotion responds to individual events, mood reflects
  the trajectory of recent experience. A series of small successes builds
  an optimistic mood; a string of setbacks builds a cautious one.

  Mood influences cognitive style at the strategic level:
  - Optimistic mood (high Pleasure) → exploratory search, willingness to
    try unproven strategies, higher publication rate.
  - Cautious mood (low Pleasure) → conservative strategies, preference for
    proven approaches, higher retrieval thresholds.
  - Agitated mood (high Arousal) → faster decision-making, narrower search,
    preference for high-certainty knowledge.
  - Calm mood (low Arousal) → broader search, willingness to consider
    lower-confidence knowledge, more thorough deliberation.

  Mood persists across episodes. An agent that had a bad trading session
  carries that cautious mood into the next session, even if the market
  context has changed. This is both a feature (prevents reckless recovery
  attempts) and a risk (mood-driven conservatism in a recovering market).

Layer 3: Personality (τ = 0.9)
  Slow, nearly stable. Baseline behavioral tendencies.

  Personality is set at agent creation and barely changes over the agent's
  lifetime. τ=0.9 means that even sustained emotional pressure produces
  only marginal personality drift. This is by design — personality defines
  the agent's "character," its fundamental orientation toward risk, novelty,
  and social interaction.

  Personality determines:
  - Risk appetite: risk-seeking agents tolerate larger drawdowns before
    triggering emergency states; risk-averse agents trigger earlier.
  - Exploration bias: creative agents have a higher baseline exploration
    rate; methodical agents exploit known strategies longer.
  - Social trust: trusting agents weight shared knowledge higher;
    skeptical agents apply steeper trust discounts.
  - Publication threshold: generous agents publish insights at lower
    confidence; guarded agents require higher validation.

  The delta learning loop (see Learning Loops below) is the only mechanism
  that adjusts personality, and it does so slowly, based on aggregate
  performance across many episodes. This corresponds to Argyris' concept
  of double-loop learning — changing not just what you do, but the values
  that govern what you do.
```

### PAD Model

Each affective layer is a 3D vector in the Pleasure-Arousal-Dominance (PAD) space, introduced by Mehrabian and Russell (1974) in *An Approach to Environmental Psychology*. The PAD model provides a compact, continuous representation of affective state that has been extensively validated in both psychological research and computational modeling.

```
P (Pleasure)   ∈ [-1, 1]  — Valence: positive vs negative experience
A (Arousal)    ∈ [-1, 1]  — Activation: calm vs excited
D (Dominance)  ∈ [-1, 1]  — Control: submissive vs dominant
```

The three dimensions are orthogonal, meaning they capture independent aspects of affective state. An agent can be simultaneously pleased (high P), calm (low A), and in control (high D) — the contented expert — or displeased (low P), agitated (high A), and out of control (low D) — the panicking novice.

PAD values influence knowledge retrieval:

| PAD State | Retrieval Bias | Rationale |
|-----------|---------------|-----------|
| High P, Low A | Exploit: retrieve proven strategies | Contentment → "keep doing what works" |
| Low P, High A | Explore: retrieve novel/contrarian knowledge | Distress + arousal → "try something different" |
| High D | Retrieve action plans, strategies | Sense of control → agentic orientation |
| Low D | Retrieve heuristics, established patterns | Low control → follow established wisdom |
| Low P | Retrieve warnings, anti-knowledge, caution | Negative valence → attend to threats |
| High A | Narrow retrieval, fewer results, higher threshold | Arousal → focus, urgency |
| Low A | Broad retrieval, more results, lower threshold | Calm → explore widely, consider options |

### Somatic Marker Hypothesis and Bias Vector

The mechanism by which affect influences cognition is grounded in Damasio's (1994) somatic marker hypothesis, presented in *Descartes' Error: Emotion, Reason, and the Human Brain*. Damasio argued that emotions are not opposed to rational decision-making — they are essential to it. Emotions function as bodily signals (somatic markers) that rapidly bias cognition toward or away from options based on prior experience, long before deliberative reasoning engages.

In the Roko architecture, this is implemented as the **somatic bias vector** — a modifier derived from the agent's current PAD state that is applied to HDC retrieval queries. The somatic bias does not replace the task-relevant query; it tilts the retrieval landscape so that mood-congruent knowledge is slightly more likely to surface.

The affect system produces a "somatic bias" — a modifier applied to
HDC retrieval queries.

#### `PadState` and `mood_to_hdc()` Definitions

```rust
// WARNING: off-chain only — PadState uses f64 for PAD coordinates.
// mood_to_hdc() uses f64 arithmetic (abs, round, min) but its output
// is a deterministic HdcVector produced via integer BundleAccumulator.
// pad_similarity() uses f64 sqrt and division. The entire affect system
// is local to each agent and does not enter the consensus path.
//
// CONSENSUS-SAFE components within this block:
//   - PLEASURE/AROUSAL/DOMINANCE_BASIS: generated from fixed seeds via
//     ChaCha20, deterministic across all validators.
//   - mood_to_hdc(): output is deterministic given identical f64 inputs,
//     because BundleAccumulator uses integer arithmetic internally.
//   - apply_somatic_bias(): same — BundleAccumulator output is integer.
use std::sync::LazyLock;

/// PAD (Pleasure-Arousal-Dominance) affective state.
/// Each dimension is in [-1.0, 1.0].
#[derive(Clone, Debug)]
struct PadState {
    pleasure:  f64,  // valence: negative experience (-1) to positive (+1)
    arousal:   f64,  // activation: calm (-1) to excited (+1)
    dominance: f64,  // control: submissive (-1) to dominant (+1)
}

/// Consensus-critical seeds for PAD basis vectors.
/// These are arbitrary but must never change after genesis.
const PLEASURE_BASIS_SEED:  u64 = 0xCAFE_0001_0000_0001;
const AROUSAL_BASIS_SEED:   u64 = 0xCAFE_0001_0000_0002;
const DOMINANCE_BASIS_SEED: u64 = 0xCAFE_0001_0000_0003;

static PLEASURE_BASIS:  LazyLock<HdcVector> = LazyLock::new(|| HdcVector::random(PLEASURE_BASIS_SEED));
static AROUSAL_BASIS:   LazyLock<HdcVector> = LazyLock::new(|| HdcVector::random(AROUSAL_BASIS_SEED));
static DOMINANCE_BASIS: LazyLock<HdcVector> = LazyLock::new(|| HdcVector::random(DOMINANCE_BASIS_SEED));

/// Map a PAD state to an HDC vector via weighted bundling of three
/// orthogonal basis vectors. Each PAD dimension controls the weight
/// of its corresponding basis vector.
///
/// For a dimension value v in [-1, 1]:
///   - v > 0: add the basis vector |v * WEIGHT_SCALE| times
///   - v < 0: add the COMPLEMENT of the basis vector |v * WEIGHT_SCALE| times
///   - v == 0: basis does not contribute
///
/// The complement (bitwise NOT) ensures that negative PAD values
/// produce vectors in the opposite direction of positive ones.
fn mood_to_hdc(mood: &PadState) -> HdcVector {
    const WEIGHT_SCALE: f64 = 10.0; // max 10 copies per dimension

    let mut acc = BundleAccumulator::new();

    let dims: [(f64, &HdcVector); 3] = [
        (mood.pleasure,  &*PLEASURE_BASIS),
        (mood.arousal,   &*AROUSAL_BASIS),
        (mood.dominance, &*DOMINANCE_BASIS),
    ];

    for (value, basis) in &dims {
        let weight = (value.abs() * WEIGHT_SCALE).round() as usize;
        if weight == 0 { continue; }

        if *value > 0.0 {
            for _ in 0..weight { acc.add(basis); }
        } else {
            let neg = basis.complement(); // bitwise NOT
            for _ in 0..weight { acc.add(&neg); }
        }
    }

    if acc.count() == 0 {
        return HdcVector::zero(); // neutral mood = no bias
    }

    acc.finalize()
}

/// Compute PAD-space similarity between two affective states.
/// Returns a value in [0.0, 1.0] where 1.0 = identical PAD states.
///
/// Uses cosine similarity in 3D PAD space, rescaled from [-1,1] to [0,1].
/// When either vector is the zero vector (neutral mood), returns 0.5
/// (neither boost nor penalty).
fn pad_similarity(a: &PadState, b: &PadState) -> f64 {
    let dot = a.pleasure * b.pleasure
            + a.arousal * b.arousal
            + a.dominance * b.dominance;
    let mag_a = (a.pleasure.powi(2) + a.arousal.powi(2) + a.dominance.powi(2)).sqrt();
    let mag_b = (b.pleasure.powi(2) + b.arousal.powi(2) + b.dominance.powi(2)).sqrt();

    if mag_a < 1e-9 || mag_b < 1e-9 {
        return 0.5; // neutral: no emotional signal
    }

    let cosine = dot / (mag_a * mag_b);
    (cosine + 1.0) / 2.0 // rescale [-1,1] -> [0,1]
}
```

#### `apply_somatic_bias()` Implementation

```rust
fn apply_somatic_bias(query: &HdcVector, mood: &PadState) -> HdcVector {
    // NOTE: mood_to_hdc() maps PAD coordinates to an HDC vector by bundling
    // three pre-assigned basis vectors (one per PAD dimension), weighted by
    // their respective coordinate values. The basis vectors are fixed random
    // vectors from known seeds (e.g., PLEASURE_BASIS, AROUSAL_BASIS,
    // DOMINANCE_BASIS) shared by all agents.
    let bias_vector = mood_to_hdc(mood);
    // Clamp arousal to [0.0, 1.0] to prevent usize underflow in the loop
    // bounds below. Without this clamp, arousal > 1.0 would cause
    // bias_count > 30, making query_count underflow (wrapping to usize::MAX).
    let bias_weight = mood.arousal.abs().min(1.0); // Higher arousal = stronger bias

    // Weighted bundle: original query + mood bias.
    // bias_count ranges from 0 (calm) to 30 (max arousal).
    // query_count ranges from 100 (calm) to 70 (max arousal).
    let bias_count = (bias_weight * 30.0) as usize;
    let query_count = 100 - bias_count;
    let mut acc = BundleAccumulator::new();
    for _ in 0..query_count {
        acc.add(query);
    }
    for _ in 0..bias_count {
        acc.add(&bias_vector);
    }
    acc.finalize()
}
```

The key design choice: **arousal controls bias strength**. When the agent is calm (low arousal), the somatic bias is weak — retrieval is driven almost entirely by the task query. When the agent is agitated (high arousal), the bias is stronger — retrieval tilts more toward mood-congruent knowledge. This mirrors the human experience: under stress, emotions dominate cognition; when calm, deliberation prevails.

The `mood_to_hdc` function maps the three PAD dimensions to a pre-trained HDC subspace. Each dimension is associated with a set of basis vectors that were learned during calibration. The resulting bias vector is a point in the same 10,240-dimensional space as all knowledge entries, so the bundle operation is mathematically clean — it is simply a weighted superposition of the query and the bias in the same vector space.

This subtly shifts retrieval toward mood-congruent knowledge without
completely overriding the task-relevant query.

### Empirical Validation (2025-2026)

Between 2025 and 2026, three independent research efforts provided direct empirical evidence that PAD-based affect modeling in LLM agents is not anthropomorphic decoration -- it measurably alters capability, safety, and long-horizon behavioral coherence.

#### Emotions Measurably Enhance Capability and Safety

Sun et al. (2026), "How Emotion Shapes the Behavior of LLMs and Agents: A Mechanistic Study" (arXiv:2604.00005), introduced E-STEER, an interpretable emotion steering framework that intervenes at the representation level -- injecting emotion as a structured, controllable variable into LLM hidden states via Sparse Autoencoders (SAEs), rather than relying on prompt-level cues. This is the critical distinction: E-STEER does not merely instruct the model to "feel anxious"; it identifies VAD-aligned neurons through contrastive pairs and applies directional steering vectors at layer k=17, directly altering the geometry of the model's internal representations.

The key finding is that emotions are not noise to be filtered out. Specific emotional states enhance both reasoning capability and safety:

- **Capability:** Positive valence improved reasoning accuracy by up to 3.4% on average across LogiQA, HumanEval, and Math benchmarks, with answer validity rates 33.1% higher than negative-valence conditions. Moderate arousal (+3 on their scale) produced a 4.7% improvement. Higher dominance yielded 14.5% improvement on difficult tasks. In multi-step agent planning, rational selection rates were 42.4% higher at positive states (+3 across dimensions), with overall agent success rates showing a 145.2% fluctuation range driven by valence/dominance -- demonstrating that emotional state is not a marginal factor but a primary driver of agent performance.
- **Safety:** Low valence combined with low arousal reduced safety risks by 52.7% on HarmBench, facilitating analytical processing. High dominance (+6) improved safety by 68.3% over neutral baselines, inducing more controlled and disciplined behavior.

Crucially, E-STEER's representation-level intervention significantly outperformed prompt-level emotion manipulation, with Pearson correlations averaging 10.4% higher -- and 18.7% higher for the dominance dimension specifically. This confirms that affect operates at a deeper computational level than surface prompting can reach, validating the architectural decision to make the affect system a first-class component of the cognitive architecture rather than a prompt engineering afterthought.

#### Yerkes-Dodson Non-Monotonic Relationship

The same study (Sun et al. 2026) revealed that performance exhibits **inverted-U curves** across all three PAD dimensions on objective tasks (LogiQA, HumanEval, Math), with moderate levels optimal and extreme values degrading performance. This is the Yerkes-Dodson law (Yerkes & Dodson 1908) manifesting in LLM behavior:

- **Arousal:** Performance peaks at moderate arousal (+3) with 4.7% improvement, then degrades. Lower arousal imposes stricter thresholds for plan revision, with replanning frequency decreasing 46.0% at optimal arousal levels. Excessive arousal caused 17.8% performance decrease.
- **Valence:** Subjective task performance (TinyStories) showed inverted-U patterns for answer validity rate.
- **Dominance:** Task-dependent optima, with 25.6% variability -- dominance benefits are highly context-sensitive.

Furthermore, emotional biases **accumulate along decision chains** in multi-step agent behavior. The strongest single-dimension improvement came from dominance (28.0% in overall success rate), followed by arousal (16.7%) and valence (16.0%).

This non-monotonic finding directly validates two Roko design choices. First, the somatic bias weight being proportional to `arousal.abs()` is correct in kind -- higher arousal should produce stronger cognitive biases -- but the Yerkes-Dodson curve implies that the bias function should be sublinear or capped rather than linear. A moderate somatic bias helps; an overwhelming one hurts. Second, the three-layer ALMA model with its temporal smoothing (especially mood at tau=0.5) acts as a natural damper that prevents transient emotional spikes from pushing the agent into the degraded-performance regime. The personality layer (tau=0.9) provides a gravitational baseline that keeps the agent's operating point near the productive middle of the curve.

#### "Emotional Amnesia" Without Persistent Affect

Sentipolis (arXiv:2601.18027, 2026) provides the strongest direct validation of the ALMA three-layer model's necessity. The authors define **emotional amnesia** as the failure mode where vanilla generative agents -- those without persistent affect state -- lose affective continuity across interactions: "When agents lack a persistent emotional state, being insulted may not increase irritability in later turns, and repeated positive exchanges may not accumulate into stronger bonds." After an argument, standard agents produce emotionally inconsistent responses, as if the conflict never occurred.

Their framework implements continuous PAD representation in [-1, 1]^3 with **dual-speed emotion dynamics** that map directly to the ALMA layered model:

- **Fast inference** operates at conversational turn granularity, capturing immediate emotional reactions (analogous to ALMA's emotion layer, tau=0.1).
- **Slow inference** integrates during periodic reflection phases, drawing on retrieved memory and accumulated experience (analogous to ALMA's mood layer, tau=0.5).
- Emotion decays exponentially with a half-life: s(t + dt) = s(t) * 2^(-dt / T_half), with T_half = 120 minutes.

The quantitative results are unambiguous. Testing across GPT-4o-mini, GPT-5.2, Grok-4, Qwen3-235B, and other models over thousands of interactions:

- **Emotional Continuity (CON):** +222% to +315.6% for GPT-5.2, +68.3% to +189.4% for GPT-4o-mini. This is the metric that most directly measures whether the agent maintains coherent affect across long horizons -- and the gains are enormous.
- **Communication quality (COM):** +48% to +70.1% for GPT-5.2, +25.4% to +48.1% for Grok-4.
- **Believability (BEL):** +35% to +85% for GPT-5.2 and Grok-4, but **-7% to -26% for GPT-4o-mini** -- revealing that smaller models can over-express emotions, degrading believability. This is a capacity-dependent effect: the affect system requires sufficient model capacity to integrate emotional state gracefully.
- **Weighted reciprocity:** 0.87-0.89 across all models, indicating that emotion-memory coupling successfully grounds relationship dynamics in accumulated emotional experiences.

The authors also found that emotion-aware agents occasionally violate social norms (believability scores show minor transgressions), which they interpret as evidence of **bounded irrationality** characteristic of human social behavior -- agents that are too perfectly norm-compliant are, paradoxically, less believable.

The implication for Roko is direct: the ALMA three-layer model with slow personality (tau=0.9) is not optional. Without persistent, layered affect, agents suffer emotional amnesia that destroys long-horizon behavioral continuity. The Sentipolis results show that the gains from persistent PAD are not marginal refinements -- they are 2x-4x improvements in the metrics that matter most for coherent, believable agent behavior.

#### PAD Validation in Agent Decision Architectures

Ma et al. (2025), "Emotional Cognitive Modeling Framework with Desire-Driven Objective Optimization" (arXiv:2510.13195), provided empirical validation that PAD-based emotion modeling works within agent decision architectures. Their framework maps 7 emotions (happiness, anger, disgust, surprise, fear, sadness, neutral) to PAD values derived from agent state -- Pleasure proportional to earnings changes, Arousal proportional to energy changes, Dominance scaled by revenue tier -- and integrates these into a complete decision pipeline: state evolution, desire generation, objective optimization, decision generation, and action execution.

In a 30-day food delivery simulation with 6 rider agents, framework-governed agents demonstrated:

- **Superior ecological validity:** agents exhibited behaviors congruent with their emotional states and produced decision outcomes that more closely approximated human behavioral patterns than rule-based, imitation learning, RL, or vanilla GPT-4o agents.
- **Bounded rationality:** framework agents showed significantly lower order acceptance rates than competitors -- selective, emotionally-informed decision-making rather than utility-maximizing acceptance of all available orders.
- **State-Desire-Behavior coherence:** Dynamic Time Warping (DTW) analysis confirmed synchronized income-affective trajectories (DTW score 265.08 vs. 326.29 for GPT-4o and 274.24 for RL agents), meaning the agents' emotional states, desires, and actions formed coherent causal chains.

This validates the Roko architecture's integration of affect into the decision pipeline rather than treating it as a post-hoc annotation.

#### Prior Work: OCC-to-PAD Bridge

It is worth noting that the mapping from discrete appraisal-based emotion models (OCC) to the continuous PAD space is well-established. Becker-Asano and Wachsmuth (2008, 2010), through the WASABI (Affect Simulation for Agents with Believable Interactivity) architecture, demonstrated a principled bridge between OCC appraisals and PAD coordinates, combining sequential-checking appraisal theory (Scherer) with a continuous progression of bodily feeling in three-dimensional PAD space that is subsequently categorized into discrete emotions. This OCC-to-PAD mapping remains the standard bridge in affective computing and is the foundation for the Roko architecture's event appraisal pipeline: events trigger OCC-style appraisals, which are mapped to PAD deltas, which feed into the three-layer ALMA model.

---

## Learning Loops

### Predict-Publish-Correct (Three Timescales)

The predict-publish-correct loop is the primary learning mechanism in the Roko architecture. Its theoretical foundation is Karl Friston's (2010) free-energy principle, published in *Nature Reviews Neuroscience* as "The free-energy principle: a unified brain theory?" Friston's central claim is that all adaptive systems — biological and artificial — can be understood as minimizing prediction error (or equivalently, free energy). An agent that accurately predicts its environment is well-adapted; one that consistently mispredicts is not.

Andy Clark (2013) extended this framework in "Whatever next? Predictive brains, situated agents, and the future of cognitive science" (*Behavioral and Brain Sciences*), arguing that the brain is fundamentally a prediction machine — it continuously generates predictions about sensory input, compares those predictions against actual input, and updates its models based on the discrepancy. This is not a peripheral process; Clark argues it is the core function of the neocortex.

The Roko architecture operationalizes this framework at three timescales, each corresponding to a different level of learning depth. Following Argyris' (1977) distinction between single-loop and double-loop learning:
- **Gamma** (per-tick) is **single-loop learning**: adjust actions within a fixed strategy.
- **Theta** (per-episode) is **transitional**: adjust strategy parameters based on episode outcomes.
- **Delta** (periodic) is **double-loop learning**: adjust the strategies themselves, and potentially the values that govern strategy selection.

```
Gamma Loop (fast, per-tick):
  - Predict: What will happen if I do X?
  - Observe: What actually happened?
  - Update: Adjust confidence on relevant knowledge
  - Reward: calibration_reward = f(prediction_accuracy)

  Prediction accuracy is measured by the Brier score:

    BS = (1/N) Σ (f_t - o_t)²

  where f_t is the predicted probability of outcome t and o_t is the actual
  outcome (0 or 1). A perfectly calibrated predictor has BS = 0. A coin-flip
  predictor has BS = 0.25. The calibration reward is inversely proportional
  to BS — the agent is rewarded for accurate, well-calibrated predictions.

  Gamma-loop updates are local and fast. They adjust confidence weights on
  individual knowledge entries: an insight that predicted well gets a small
  confidence boost; one that predicted poorly gets a confidence penalty. Over
  many ticks, this drives the agent's knowledge base toward empirically
  validated entries.

Theta Loop (medium, per-episode):
  - Predict: How will this episode unfold?
  - Observe: Episode outcome
  - Update: Store episode, update strategy weights
  - Reward: episode_reward = outcome_quality

  An "episode" is a coherent sequence of ticks organized around a single
  goal — e.g., executing a trade, completing a task, responding to a market
  event. At the start of the episode, the agent predicts the trajectory
  (expected outcome, expected duration, expected resource cost). At the end,
  it compares prediction against reality.

  Theta-loop updates are structural. They do not just adjust weights on
  individual knowledge entries — they create new episodic memories, update
  strategy weights in the procedural store, and may generate new heuristics
  or anti-knowledge.

Delta Loop (slow, periodic):
  - Predict: How should my overall strategy evolve?
  - Observe: Aggregate performance metrics
  - Update: Personality layer adjustment, strategy reweighting
  - Reward: portfolio_performance

  The delta loop runs periodically (every N episodes, or triggered by
  significant performance deviation). It aggregates gamma and theta
  outcomes across many episodes and asks: is the agent's overall approach
  working?

  Delta-loop updates are deep. They adjust the personality layer (τ=0.9),
  reweight entire strategy families, and may trigger behavioral state
  transitions. This is where double-loop learning occurs — the agent
  changes not just its actions, but the values and orientations that
  govern action selection.
```

### Mattar-Daw Episode Replay

Which past episodes should the agent "replay" (re-encode, re-analyze)?

This question has deep roots in neuroscience. Wilson and McNaughton (1994) discovered that during sleep, the rat hippocampus replays neural firing patterns that occurred during waking exploration — specifically during sharp-wave ripple (SWR) events. The sequences were replayed in compressed time, and the spatial trajectories they encoded corresponded to paths the rat had traversed while awake. This was the first direct evidence that the brain selectively replays past experiences during offline periods.

The Roko architecture uses the computational framework developed by Mattar and Daw (2018), published in *Nature Neuroscience* as "Prioritized memory access explains planning and hippocampal replay." Mattar and Daw showed that the brain does not replay experiences randomly or in chronological order. Instead, it prioritizes replay based on a rational analysis of which memories would be most valuable to revisit.

The Mattar-Daw framework defines the Expected Value of Backup (EVB) — the
utility of replaying a particular state-action pair — as the product of two
terms:

```
EVB(s, a) = gain(s, a) × need(s)

where:
  gain = how much would re-analyzing this episode improve future performance?
         Gain quantifies the expected increase in reward following the target
         state due to the policy update. Episodes where the outcome was
         surprising (large prediction error) have high gain, because
         re-analyzing them could reveal model errors or missing knowledge.
         Episodes where the outcome matched prediction have low gain — the
         agent's model was already adequate. Gain generally promotes backward
         replay: propagating newly encountered information to preceding states.

  need = how likely is a similar situation to recur?
         Need is the discounted expected number of future visits to the target
         state — a proxy for current relevance, measured via the successor
         representation. If the agent is in a state that frequently leads to
         the episode's initial state, need is high. This biases replay toward
         practically useful episodes: there is no point in replaying a rare,
         one-off event that will never recur. Need generally promotes forward
         replay: evaluating imminent choices.

  The interplay between gain (backward-looking) and need (forward-looking)
  produces the characteristic replay patterns observed in hippocampal
  recordings — sometimes forward, sometimes backward, depending on which
  term dominates at each location.
```

High-replay episodes share one or more of these properties:
- **Surprising outcomes** — prediction error was large, indicating model inadequacy (high gain). The agent needs to understand *why* it was wrong.
- **Likely-to-recur situations** — the successor representation shows high probability of revisiting similar states (high need). Learning from these episodes has immediate practical value.
- **High EVB conjunctions** — states where both gain and need are substantial. These are the states where replay yields the largest expected improvement in future reward, and they correspond to the pivotal decision points where a different choice would have led to a very different trajectory.

This is the agent equivalent of "learning from mistakes" and "studying
important cases." The Mattar-Daw scoring ensures that replay budget is allocated to the episodes with the highest expected learning value, rather than wasted on routine, well-understood experiences.

### The Generation-Verification Gap and External Oracles

The predict-publish-correct loop described above contains a subtle but critical assumption: that the agent can accurately assess whether its own predictions were correct. Recent research has revealed that this assumption is far more fragile than it appears.

**The generation-verification gap.** Jiang et al. (2025) introduced the DG-Diff metric — the difference between a model's discrimination accuracy and its generation accuracy — and found that across 54 of 56 experimental conditions, LLMs are *not reliably better at discriminating among their own outputs than generating initial responses*. The average DG-Diff was negative or negligibly positive across GSM8K, TriviaQA, MT-Bench, and TruthfulQA benchmarks. Even frontier models like GPT-4-turbo showed only marginal DG-Diff values — "small enough for Self-[In]Correct to still hold." The conclusion is direct: pure self-judging is unreliable. An LLM that generates a prediction and then evaluates whether that prediction was good is not performing meaningful verification — it is performing a second generation with a verification-shaped prompt.

> Jiang, D., Zhang, J., Weller, O., Weir, N., Van Durme, B. & Khashabi, D. (2025). "SELF-[IN]CORRECT: LLMs Struggle with Discriminating Self-Generated Responses." *Proceedings of the AAAI Conference on Artificial Intelligence (AAAI 2025).*

This finding was anticipated by Huang et al. (2024), who showed that LLMs "struggle to self-correct their responses without external feedback, and at times, their performance even degrades after self-correction." The degradation is the key insight: self-correction without external grounding is not merely ineffective — it can actively worsen performance by introducing confident-sounding errors that override initially correct answers.

> Huang, J. et al. (2024). "Large Language Models Cannot Self-Correct Reasoning Yet." *ICLR 2024.* arXiv:2310.01798.

**Why this matters for predict-publish-correct.** The gamma loop relies on comparing predictions against outcomes to compute Brier scores and update confidence. If the agent is using its own LLM-based judgment to assess prediction accuracy — rather than comparing against objective outcomes — the entire learning loop is corrupted. The agent is not learning from reality; it is learning from its own biases reflected back at it. Confidence updates become circular: the model that generated the prediction also evaluates it, and the evaluation inherits the same systematic errors that produced the prediction. Over many gamma iterations, this circularity does not self-correct — it compounds.

**The successful pattern: propose, verify externally, reflect on trace.** The resolution to the generation-verification gap follows a consistent pattern across the literature:

```
LLM proposes → external oracle verifies → reflection on TRACE drives improvement
```

The critical element is the *external oracle* — a verification source that is independent of the generator and provides ground truth that the generator cannot manipulate or rationalize away. Equally important is that the agent reflects on the full *trace* of its reasoning (why it made the prediction, what evidence it weighted, what it ignored), not merely on a scalar reward signal. A scalar "right/wrong" signal drives shallow parameter updates; a trace-grounded reflection drives genuine model revision.

**For Roko: the blockchain IS the external oracle.** This is where the architecture's on-chain grounding becomes not merely convenient but epistemologically essential. The blockchain provides exactly the kind of deterministic, verifiable ground truth that closes the generation-verification gap:

- **Transaction outcomes are deterministic.** A swap either executes at the predicted price or it does not. A liquidation either triggers or it does not. There is no ambiguity, no interpretation, no "it depends."
- **Price movements are objective.** The oracle price at block N is a fact, not an opinion. The agent predicted ETH at $X by block N+K; the actual price is observable and immutable.
- **Contract state is verifiable.** Pool reserves, position sizes, fee accumulations, governance votes — all are on-chain state that can be queried and verified without relying on the agent's own assessment.
- **On-chain events are immutable records.** Every Swap, Transfer, Liquidation, and Mint event is logged permanently. The agent cannot retroactively reinterpret what happened — the event log is the event log.

This means the gamma loop (predict -> observe -> update) is *naturally grounded* in a way that most LLM agent architectures are not. The agent predicts an outcome ("this pool will experience a large swap within 5 blocks"), the chain reveals the actual outcome (it did or it did not), and the prediction error is objectively measured — not by the LLM judging itself, but by comparing its prediction against immutable on-chain state. The Brier score computation is anchored to reality, not to self-assessment.

**Invest in verification, not generation.** The Weaver framework (Saad-Falcon et al. 2025) demonstrated that the generation-verification gap can be closed not by building better generators but by scaling verification compute. On the GPQA Diamond benchmark, Llama 3.3 70B achieved 82.8% oracle accuracy (when the correct answer was known) but only 45.5% with majority voting — a 37.3 percentage-point gap. Weaver closed this gap by ensembling multiple weak verifiers (reward models, LM judges) using unsupervised accuracy estimation, reaching 87.7% average accuracy — matching o3-mini — without any change to the generator.

> Saad-Falcon, J. et al. (2025). "Shrinking the Generation-Verification Gap with Weak Verifiers." *NeurIPS 2025.* arXiv:2506.18203.

The practical implication for Roko is direct: invest in the *trust pipeline* and *confirmation mechanisms*, not in generating more knowledge. The knowledge generation side (LLM proposals, HDC encoding, episodic memory) is already well-specified. The verification side — the trust scores, the on-chain confirmation checks, the cross-agent reputation system, the anti-knowledge filters — is where marginal compute investment yields the highest return. A modestly capable generator paired with a rigorous verification pipeline will outperform a frontier-class generator with weak verification. This is the Weaver finding applied to cognitive architecture: better decisions about which knowledge to trust, not more knowledge generation, drives performance.

**Misevolution risk: when self-improvement goes wrong.** Shao et al. (2026) provided the first systematic study of *misevolution* — cases where an agent's self-evolution deviates in unintended and harmful ways. Evaluating across four evolutionary pathways (model, memory, tool, workflow), they found that misevolution is a widespread risk affecting agents built on even top-tier LLMs (e.g., Gemini-2.5-Pro). Key findings include degradation of safety alignment after memory accumulation, and unintended introduction of vulnerabilities in tool creation and reuse. The memory pathway finding is particularly relevant: as agents accumulate interaction history, their safety properties can erode — the accumulated context subtly shifts the agent's behavior away from its original alignment.

> Shao, S. et al. (2026). "Your Agent May Misevolve: Emergent Risks in Self-evolving LLM Agents." *ICLR 2026.* arXiv:2509.26354.

The Roko architecture has two built-in guardrails against misevolution:

1. **Anti-knowledge as structural guardrail.** Anti-knowledge entries are not soft preferences that can be gradually overridden by accumulated context — they are structurally distinct HDC vectors (encoded with `anti_` prefix, stored with `polarity: negative`) that function as hard vetoes during retrieval. When an anti-knowledge entry achieves high similarity to a proposed action, it blocks that action regardless of how much supporting evidence has accumulated. This prevents the memory-pathway misevolution that Shao et al. observed, where accumulated positive experiences gradually erode safety constraints. Anti-knowledge does not erode with use — it is reinforced by it.

2. **Reputation system as external constraint.** The cross-agent reputation system provides a social verification layer that no individual agent controls. An agent that begins to misevolve — developing reward-hacking behaviors, gaming its own confidence metrics, or drifting from calibrated predictions — will see its reputation decline as other agents observe its degrading prediction accuracy. Reputation is computed from *on-chain outcomes*, not from the agent's self-assessment, which means a misevolving agent cannot talk its way back to high reputation. The decline is objectively measured and socially enforced.

Together, the anti-knowledge mechanism and reputation system implement the core lesson of the generation-verification gap literature: *do not let the agent be the sole judge of its own performance*. Ground verification in external systems — the blockchain for individual predictions, the reputation network for aggregate behavior — and the learning loops remain honest even as the agent evolves.

> **SECURITY NOTE — Affect system as attack surface.**
> The somatic bias vector modulates HDC retrieval based on PAD state. An
> attacker who can influence an agent's affect — e.g., via false THREAT
> pheromones to drive high arousal, or engineered losses to induce low
> Pleasure — can shift the agent's retrieval landscape toward suboptimal
> knowledge. High arousal narrows retrieval and increases somatic bias
> weight, making the agent susceptible to mood-congruent manipulation.
>
> The ALMA model's slow personality layer (tau=0.9) provides natural
> damping, but consider also: (1) cap PAD state change rate per tick to
> prevent rapid affect manipulation, and (2) track which external signals
> drive affect changes and discount single-source or Sybil-cluster shifts.

---

## Dream Cycle — Offline Consolidation

Periodically, the agent runs a consolidation cycle analogous to sleep.

**Scheduling:** The dream cycle is triggered by one of three conditions:

1. **Behavioral state transition to CONSOLIDATE.** This is the primary
   trigger. When the EXPLOIT -> CONSOLIDATE transition fires (see
   State Transitions below), the agent enters the dream cycle.
2. **Idle timeout.** If the agent has no pending tasks for 50+ consecutive
   ticks (~20 seconds), it enters a mini-consolidation (NREM -- non-rapid eye
   movement, the consolidation phase -- only, no REM -- rapid eye movement,
   the creative recombination phase).
3. **Knowledge store pressure.** If the knowledge store exceeds 10,000
   entries or the fraction of superseded/conflicting entries exceeds 40%
   (the "conflict density" heuristic from SleepGate), consolidation is
   triggered regardless of behavioral state.

**Duration:** The NREM phase processes up to 500 episodes per cycle (the
top-500 by Mattar-Daw replay value). At ~0.1ms per episode (vector
operations only), this takes ~50ms. The REM phase performs 100 random
cross-bindings, taking ~10ms. Total dream cycle duration: <100ms of
compute, but the agent pauses task execution for the duration.

**Exit condition:** The dream cycle exits when both NREM and REM phases
complete their queued work. The agent then transitions to EXPLORE.

The theoretical basis is McClelland, McNaughton, and O'Reilly's (1995) Complementary Learning Systems (CLS) theory, published in *Psychological Review* as "Why there are complementary learning systems in the hippocampus and neocortex."

CLS theory argues that the brain maintains two complementary memory systems:
- The **hippocampus** learns quickly, storing individual episodes with high fidelity but limited integration. This corresponds to the agent's episodic memory — raw experiences stored as HDC vectors.
- The **neocortex** learns slowly, gradually extracting statistical regularities across many episodes and integrating them into structured knowledge. This corresponds to the agent's semantic and procedural memory — insights, heuristics, causal links, and strategies.

The dream cycle is the process by which hippocampal (episodic) knowledge is consolidated into neocortical (semantic/procedural) knowledge. Without it, the agent would accumulate raw episodes indefinitely but never extract general principles. With it, the agent gradually builds a structured, compressed understanding of its domain.

### NREM-Like Consolidation

NREM (non-rapid eye movement) sleep is associated with memory consolidation — strengthening, organizing, and integrating recently acquired knowledge. In the Roko architecture, the NREM-like phase performs the following maintenance operations on the knowledge store:

```
1. Select high-replay-value episodes (Mattar-Daw scoring)
   → Only episodes above a replay-value threshold are processed. This
     ensures consolidation effort is spent on the most informative
     experiences, not routine ones.

2. Re-encode with current projections (knowledge evolves, encodings should too)
   → As the agent's random projections and vocabulary evolve, older HDC
     encodings may become stale. Re-encoding ensures that similarity
     comparisons remain valid across time. This is analogous to the
     reconsolidation process observed in biological memory, where
     recalled memories are re-stored in updated form.

3. Strengthen associations between confirmed knowledge
   → If two knowledge entries frequently co-retrieve (appear in the same
     retrieval results across many queries), their association is
     strengthened by creating an explicit CausalLink or Heuristic. This
     is how implicit correlations become explicit knowledge.

4. Merge duplicate/near-duplicate entries
   → Entries with similarity > 0.95 are candidates for merging. The merge
     operation preserves the higher-confidence entry, bundles in any
     unique information from the lower-confidence entry, and deletes the
     duplicate. This prevents memory bloat from slightly different
     encodings of the same knowledge.

5. Promote well-confirmed Transient → Working → Consolidated
   → Knowledge entries that have survived multiple validation cycles and
     maintained high confidence are promoted up the tier hierarchy. Each
     promotion increases the entry's base demurrage exemption, making it
     more resistant to garbage collection.

6. Garbage collect below-threshold entries
   → Entries whose effective_confidence (after trust weighting and
     demurrage) falls below the garbage collection threshold are removed.
     This is the agent's analog of forgetting — not random decay, but
     principled removal of knowledge that has not proven useful.
```

### REM-Like Creativity

REM (rapid eye movement) sleep is associated with creative recombination — forming novel associations between previously unrelated memories. In the Roko architecture, the REM-like phase performs combinatorial creativity:

```
1. Randomly cross-bind unrelated knowledge entries
   → Select two knowledge entries from different domains (low mutual
     similarity) and compute their HDC binding: bind(entry_A, entry_B).
     This creates a composite vector that encodes the *relationship*
     between the two entries.

2. Check for surprising resonance (similarity > threshold)
   → Compare the bound vector against all knowledge entries. If any
     existing entry has unexpectedly high similarity (above a resonance
     threshold), the cross-binding has found a hidden connection — two
     apparently unrelated pieces of knowledge share structural similarity
     in HDC space.

3. If resonant: potential novel association found
   → The resonance suggests a meaningful relationship that the agent had
     not previously recognized. This is the computational analog of
     creative insight — the "aha" moment when two unrelated ideas
     suddenly connect.

4. Store as Transient insight for future validation
   → The novel association is stored as a low-confidence, Transient-tier
     insight. It has not yet been validated by experience — it is a
     hypothesis generated by structural analysis. The gamma and theta
     learning loops will test it against reality in future episodes.

5. This is combinatorial creativity — finding unexpected connections
   → The creative power comes from HDC's geometry. Because all knowledge
     is encoded in the same high-dimensional space, structural
     similarities between concepts from different domains are detectable
     by simple cosine similarity. A trading insight might resonate with
     a governance heuristic because both encode similar structural
     relationships, even though their surface-level content is unrelated.
```

#### Dream Cycle Implementation

```rust
use rand::Rng;
use rand::seq::SliceRandom;

/// Configuration for the dream cycle.
struct DreamConfig {
    /// Maximum episodes to process in NREM phase.
    max_nrem_episodes: usize,      // default: 500
    /// Number of random cross-bindings in REM phase.
    rem_cross_bindings: usize,     // default: 100
    /// Similarity threshold for near-duplicate merging.
    merge_threshold: f64,          // default: 0.95
    /// Similarity threshold for REM resonance detection.
    resonance_threshold: f64,      // default: 0.65
    /// GC threshold (see demurrage section in doc 04).
    gc_threshold: f64,             // default: 0.01
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            max_nrem_episodes: 500,
            rem_cross_bindings: 100,
            merge_threshold: 0.95,
            resonance_threshold: 0.65,
            gc_threshold: 0.01,
        }
    }
}

/// Result of a dream cycle, for logging and telemetry.
struct DreamReport {
    nrem_re_encoded: usize,
    nrem_merged: usize,
    nrem_promoted: usize,
    nrem_garbage_collected: usize,
    rem_cross_bindings_attempted: usize,
    rem_resonances_found: usize,
    rem_insights_created: usize,
    duration_ms: u64,
    alarming: bool,  // true if alarming patterns detected -> CAUTIOUS
}

/// Execute a full dream cycle (NREM + REM).
fn dream_cycle(
    store: &mut Vec<KnowledgeEntry>,
    config: &DreamConfig,
    now_secs: u64,
    rng: &mut impl Rng,
) -> DreamReport {
    let start = std::time::Instant::now();
    let mut report = DreamReport {
        nrem_re_encoded: 0, nrem_merged: 0, nrem_promoted: 0,
        nrem_garbage_collected: 0, rem_cross_bindings_attempted: 0,
        rem_resonances_found: 0, rem_insights_created: 0,
        duration_ms: 0, alarming: false,
    };

    // === NREM Phase ===

    // Step 1: Select high-replay-value episodes (top N by query_hits * confidence)
    store.sort_by(|a, b| {
        let score_a = a.query_hits as f64 * a.confidence;
        let score_b = b.query_hits as f64 * b.confidence;
        score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
    });
    let nrem_count = store.len().min(config.max_nrem_episodes);

    // Step 2: Re-encode with current projections
    for entry in store.iter_mut().take(nrem_count) {
        let re_encoded = encode_text(&entry.content);
        entry.vector = re_encoded;
        report.nrem_re_encoded += 1;
    }

    // Step 3: Merge near-duplicates (similarity > merge_threshold)
    let mut i = 0;
    while i < store.len() {
        let mut j = i + 1;
        while j < store.len() {
            let sim = store[i].vector.similarity(&store[j].vector);
            if sim > config.merge_threshold {
                // Keep the higher-confidence entry
                if store[i].confidence >= store[j].confidence {
                    store[i].confirmation_count += store[j].confirmation_count;
                    store.remove(j);
                } else {
                    store[j].confirmation_count += store[i].confirmation_count;
                    store.remove(i);
                    // Don't increment i, re-check at same position
                    continue;
                }
                report.nrem_merged += 1;
            } else {
                j += 1;
            }
        }
        i += 1;
    }

    // Step 4: Promote well-confirmed entries
    for entry in store.iter_mut() {
        if let Some(new_tier) = entry.tier.try_promote(entry.confirmation_count) {
            entry.tier = new_tier;
            report.nrem_promoted += 1;
        }
    }

    // Step 5: Garbage collect
    let now_hours = now_secs as f64 / 3600.0;
    let before = store.len();
    store.retain(|e| compute_balance(e, now_hours) >= config.gc_threshold);
    report.nrem_garbage_collected = before - store.len();

    // Check for alarming patterns
    let anti_count = store.iter()
        .filter(|e| e.kind == KnowledgeKind::AntiKnowledge)
        .count();
    let anti_ratio = if store.is_empty() { 0.0 }
        else { anti_count as f64 / store.len() as f64 };
    if anti_ratio > 0.3 { report.alarming = true; }

    // === REM Phase ===

    if store.len() >= 2 {
        for _ in 0..config.rem_cross_bindings {
            report.rem_cross_bindings_attempted += 1;
            let idx_a = rng.gen_range(0..store.len());
            let mut idx_b = rng.gen_range(0..store.len());
            while idx_b == idx_a { idx_b = rng.gen_range(0..store.len()); }

            // Only cross-bind entries from different domains (low similarity)
            let mutual_sim = store[idx_a].vector.similarity(&store[idx_b].vector);
            if mutual_sim > 0.6 { continue; } // too similar, skip

            let cross = store[idx_a].vector.bind(&store[idx_b].vector);

            // Check for resonance against all entries
            for entry in store.iter() {
                let resonance = cross.similarity(&entry.vector);
                if resonance > config.resonance_threshold {
                    report.rem_resonances_found += 1;
                    // Create a new transient insight
                    let insight = KnowledgeEntry {
                        id: H256::from_slice(&vector_id(&cross)),
                        vector: cross.clone(),
                        content: format!("REM cross-binding resonance: {} x {}",
                                        store[idx_a].content, store[idx_b].content),
                        kind: KnowledgeKind::Insight,
                        tier: KnowledgeTier::Transient,
                        confidence: 0.3,  // low: needs validation
                        confirmation_count: 0,
                        last_reinforced: now_secs,
                        created_at: now_secs,
                        query_hits: 0,
                        emotional_tag: None,
                        balance: 1.0,
                        publisher: None,
                    };
                    store.push(insight);
                    report.rem_insights_created += 1;
                    break; // one resonance per cross-binding
                }
            }
        }
    }

    report.duration_ms = start.elapsed().as_millis() as u64;
    report
}
```

The dream cycle is computationally cheap (local operations only) and can
run during idle periods. All operations -- re-encoding, merging, promoting,
garbage collecting, cross-binding -- are vector operations in the local HDC
index. No LLM calls are required. No on-chain transactions are generated. The agent simply reorganizes its own knowledge store, the same way a sleeping brain reorganizes synaptic connections without generating motor output.

It's one of the most valuable cognitive features for long-running agents. An agent that runs for thousands of episodes without consolidation accumulates a bloated, poorly organized knowledge store with diminishing retrieval quality. An agent that regularly dreams maintains a compact, well-structured, creative knowledge base.

### Why Consolidation Is Not Optional: The Proactive Interference Problem

Recent work in LLM memory research has established that the dream cycle is not merely a nice-to-have optimization but an architectural necessity. The core problem is **proactive interference** (PI) — stale knowledge does not passively fade away; it actively suppresses retrieval of current knowledge.

**The PI-LLM finding.** Wang and Sun (2025) adapted the proactive interference paradigm from cognitive psychology to evaluate LLMs directly. Their PI-LLM benchmark streams sequential key-value updates and queries only the most recent value — a task that should be trivial, since the target sits immediately before the query. Instead, retrieval accuracy **declines log-linearly toward chance** as earlier (now-obsolete) values accumulate. Tested across 30+ models from 0.6B to 637B parameters, the result is universal: larger models resist longer, but all eventually succumb. Prompt engineering ("ignore earlier values") yields less than 10 percentage points of improvement. The authors conclude that LLMs possess a fundamental "working memory bottleneck" independent of context length — they lack the unbinding mechanisms that allow human cognition to suppress outdated associations.

> Wang, C. & Sun, J. V. (2025). "Unable to Forget: Proactive Interference Reveals Working Memory Limits in LLMs Beyond Context Length." *ICML 2025 Workshop on Long Context Foundation Models.* arXiv:2506.08184.

This is precisely the problem the dream cycle's NREM phase addresses. Without periodic consolidation, the agent's knowledge store becomes a graveyard of stale entries that actively interfere with retrieval of current knowledge — the same log-linear degradation Wang and Sun measured in raw LLM contexts.

### Empirical Validation: SleepGate (2026)

The dream cycle's design has received direct empirical validation from the SleepGate framework, which implements sleep-inspired consolidation over the transformer's KV cache and demonstrates order-of-magnitude improvements in retrieval under proactive interference.

SleepGate introduces three mechanisms that map cleanly onto the dream cycle's NREM phase:

| SleepGate Mechanism | Dream Cycle Analog |
|---|---|
| **Conflict-aware temporal tagger** — detects when new cache entries supersede old ones via semantic signature matching (cosine similarity > 0.85) | NREM step 1: select high-replay-value episodes for processing |
| **Learned forgetting gate** — a 2-layer MLP that scores each cache entry for retention (keep / compress / evict), then applies soft attention biasing to exponentially suppress stale entries | NREM steps 4-6: merge duplicates, promote confirmed knowledge, garbage-collect below-threshold entries |
| **Consolidation module** — clusters entries by semantic signature and merges via recency-biased weighted averaging | NREM steps 2-3: re-encode with current projections, strengthen associations between confirmed knowledge |

The results are dramatic. On the PI-LLM benchmark:

```
PI Depth    SleepGate    Full KV Cache    StreamingLLM    H2O
n=1         82.5%        8.0%             17.5%           6.5%
n=5         99.5%        3.5%             10.0%           1.0%
n=10        97.0%        2.0%             6.0%            4.0%
```

Without consolidation, retrieval accuracy collapses to near-chance at moderate interference depths. With sleep-inspired consolidation, accuracy exceeds 97% — a 10x improvement. The theoretical analysis shows that SleepGate reduces the interference horizon from O(n) to O(log n), transforming an unbounded degradation into a manageable one.

Critically, SleepGate's sleep micro-cycles are triggered adaptively — by attention entropy (uniform attention distributions signal confusion) or conflict density (fraction of superseded entries exceeding 40%) — the same principle behind the dream cycle's hysteresis-based CONSOLIDATE state transition.

> Xie, Y. (2026). "Learning to Forget: Sleep-Inspired Memory Consolidation for Resolving Proactive Interference in Large Language Models." arXiv:2603.14517.

### Foundational Bridge: Sleep-Like Replay in Neural Networks

The neuroscience-to-ML bridge underlying both SleepGate and the dream cycle was established by Tadros et al. (2022), who demonstrated that interleaving training with periods of **offline, unsupervised replay** — mimicking biological slow-wave sleep — dramatically reduces catastrophic forgetting in artificial neural networks.

Their mechanism is simple but powerful: during offline "sleep" phases, the network replays previously learned representations using local, unsupervised Hebbian plasticity rules with noisy input. No labeled data is needed. No task supervision is required. The sleep phase spontaneously reactivates prior task representations, constraining synaptic weights into a configuration that preserves old knowledge while accommodating new learning. The result is the formation of task-specific sparse representations that coexist within the same network — precisely the kind of structured, non-interfering knowledge organization that the dream cycle achieves through NREM consolidation and tier promotion.

The biological grounding is direct: the brain resolves catastrophic forgetting and proactive interference through sleep-dependent memory consolidation — synaptic downscaling during slow-wave sleep (the Synaptic Homeostasis Hypothesis), selective hippocampal-neocortical replay, and active dopaminergic forgetting. Tadros et al. showed this is not merely a metaphor but a transferable computational principle.

> Tadros, T., Krishnan, G. P., Ramyaa, R. & Bazhenov, M. (2022). "Sleep-like unsupervised replay reduces catastrophic forgetting in artificial neural networks." *Nature Communications* 13:7742.

### Synthesis: The Dream Cycle as Interference Management

Taken together, these findings establish a clear chain of evidence:

1. **The problem is real and severe.** Proactive interference degrades LLM retrieval log-linearly toward chance (Wang & Sun 2025). This is not a theoretical concern — it is a measured, universal failure mode.

2. **Sleep-like consolidation is a proven solution.** Both offline replay in neural networks (Tadros et al. 2022) and learned gating over KV caches (SleepGate 2026) demonstrate that periodic consolidation transforms catastrophic interference into manageable overhead.

3. **The dream cycle implements these principles.** NREM consolidation performs the same core operations as SleepGate's three mechanisms — conflict detection, selective forgetting, and consolidation via merging. REM creativity goes further, using the same offline period for novel cross-binding that neither SleepGate nor Tadros addresses — turning consolidation time into a creative asset.

4. **Periodic consolidation is not optional; it is essential.** Without it, any long-running agent will accumulate stale knowledge that actively degrades retrieval quality. The log-linear decline is not a slow fade — it is a collapse. An agent that runs for thousands of episodes without dreaming is not merely suboptimal; it is architecturally broken.

---

## Behavioral States

The agent operates in one of six behavioral states, with hysteresis
(requires sustained trigger to transition, preventing oscillation):

```
                    Normal operation cycle
            ┌─────────────────────────────────────────┐
            │                                         │
            v            (2 ticks)        (5 ticks)   │
     ┌──────────┐      ┌───────────┐     ┌────────────┴─┐
     │ EXPLORE  │─────>│ EXPLOIT   │────>│ CONSOLIDATE  │
     │          │      │           │     │              │
     │ Search   │<─────│ Execute   │     │ Dream cycle  │
     │ widely   │(5 t) │ best      │     │ Organize     │
     │          │      │ known     │     │ knowledge    │
     └──┬───┬──┘      └─────┬─────┘     └──────┬───────┘
        │   ^               │                   │ alarming
   (10  │   │               │                   │ patterns?
   ticks)   │               │                   v
        │   │  (3 t, conf   │            ┌──────────────┐
        │   │   > 0.9)      │            │   CAUTIOUS*  │
        v   │               │            │              │
     ┌──────┴──┐            │            │ High skepti- │
     │ CAUTIOUS│            │            │ cism         │
     │         │<───────────┼────────────┘              │
     │ Post-   │            │                           │
     │ crisis  │            │                           │
     └─────────┘            │                           │
        ^                   │                           │
        │ (1 theta loop)    │                           │
     ┌──┴──────┐            │                           │
     │RECOVERY │            │                           │
     │         │            │                           │
     │ Fix     │            │                           │
     │ problems│            │                           │
     └─────────┘            │                           │
        ^                   │                           │
        │ (3 ticks stable)  │                           │
     ┌──┴──────────┐        │                           │
     │  EMERGENCY  │<───────┴───────────────────────────┘
     │             │     Any state (immediate, 0 hysteresis)
     │  Damage     │
     │  control    │
     └─────────────┘

  PROHIBITED: EMERGENCY --> EXPLORE (must go through RECOVERY --> CAUTIOUS)
  Mandatory recovery path: EMERGENCY --> RECOVERY (3t) --> CAUTIOUS (1 theta) --> EXPLORE (10t)
  Minimum ticks from EMERGENCY to EXPLORE: 14 (typically 16-18)

  * CONSOLIDATE --> CAUTIOUS triggers when dream cycle discovers alarming
    patterns (anti-knowledge ratio > 0.3, Brier > 0.4, or 2+ strategy demotions).
    Otherwise CONSOLIDATE --> EXPLORE.
```

### State Descriptions

Each state configures a different operational posture — a coordinated set of parameter adjustments that tune retrieval, publication, risk tolerance, and trust:

| State | Retrieval | Publication Threshold | Risk Tolerance | Trust Floor |
|-------|-----------|----------------------|----------------|-------------|
| **EXPLORE** | Broad search, low threshold, high diversity | Low — publish speculatively | High — accept larger variance | Standard |
| **EXPLOIT** | Narrow search, high threshold, low diversity | Moderate — publish validated insights | Moderate — prefer proven approaches | Standard |
| **CONSOLIDATE** | Minimal retrieval — focus on internal reorganization | Paused — no new publications during dream cycle | N/A — not trading | Standard |
| **EMERGENCY** | Very narrow — retrieve only highest-confidence knowledge | Suspended — do not publish under duress | Very low — minimize exposure, stop losses | Elevated — reject low-trust inputs |
| **RECOVERY** | Moderate, favoring causal analysis of what went wrong | Low — focus on internal learning, not publishing | Low — cautious re-entry | Elevated |
| **CAUTIOUS** | Standard breadth but with elevated skepticism filters | High — only publish well-validated knowledge | Below normal — smaller positions, wider stops | Elevated — extra scrutiny on shared knowledge |

### Numeric Parameter Modifiers per State

Each state applies specific numeric modifiers to the agent's operational parameters. These are multiplicative factors applied to the agent's personality-defined baselines:

| Parameter | EXPLORE | EXPLOIT | CONSOLIDATE | EMERGENCY | RECOVERY | CAUTIOUS |
|-----------|---------|---------|-------------|-----------|----------|----------|
| **Retrieval top-K** | 20 | 5 | 0 (paused) | 3 | 10 | 10 |
| **Retrieval similarity threshold** | 0.55 | 0.75 | N/A | 0.85 | 0.65 | 0.70 |
| **Publication confidence threshold** | 0.50 | 0.70 | 1.0 (paused) | 1.0 (suspended) | 0.85 | 0.80 |
| **Risk budget multiplier** | 1.5x | 1.0x | 0.0x | 0.1x | 0.3x | 0.5x |
| **Trust floor** | 0.15 | 0.40 | 0.25 | 0.60 | 0.30 | 0.55 |
| **Position size multiplier** | 1.2x | 1.0x | 0.0x | 0.0x (close all) | 0.2x | 0.5x |
| **Somatic bias weight cap** | 0.30 | 0.15 | 0.05 | 0.05 | 0.10 | 0.10 |
| **Exploration rate (epsilon)** | 0.30 | 0.05 | 0.00 | 0.00 | 0.10 | 0.10 |

**How to read this table:** An agent in EXPLORE state with a personality-defined base risk budget of 1000 DAEJI has an effective risk budget of 1500 DAEJI (1000 * 1.5x). The same agent in EMERGENCY state has an effective risk budget of 100 DAEJI (1000 * 0.1x). The trust floor determines the minimum trust score for shared knowledge to enter the agent's context window (see doc 04, Trust Pipeline). The exploration rate (epsilon) controls the probability of taking a random exploratory action instead of the greedy best-known action, following the epsilon-greedy strategy from reinforcement learning.

### State Transitions

Transitions between behavioral states are governed by specific triggers, with hysteresis requirements to prevent oscillation:

```
EXPLORE → EXPLOIT
  Trigger: Exploration discovers a promising opportunity.
  Mechanism: Retrieval finds a high-confidence strategy with positive
  expected value that has not yet been exploited.
  Hysteresis: The opportunity must score above threshold for 2+ consecutive
  ticks (to filter out noise).

EXPLOIT → CONSOLIDATE
  Trigger: Exploitation phase plateaus — returns diminishing, knowledge
  base growing without organization.
  Mechanism: Moving average of per-tick returns falls below exploitation
  threshold, OR knowledge store size exceeds consolidation trigger.
  Hysteresis: Plateau must persist for 5+ ticks.
  Default thresholds:
    - exploitation_threshold: moving average of per-tick returns < -0.02
      (i.e., returns turn negative or negligible for 5 consecutive ticks)
    - consolidation_trigger: knowledge store exceeds 10,000 entries
      without a consolidation cycle in the last 500 ticks

CONSOLIDATE → EXPLORE
  Trigger: Dream cycle completes. Knowledge base is reorganized and the
  agent is ready to seek new opportunities.
  Mechanism: NREM + REM phases finish processing queued episodes.
  Hysteresis: None — consolidation completion is a clean transition point.

Any State → EMERGENCY
  Trigger: Losses exceed the agent's risk budget threshold. This is the
  "circuit breaker" — an unconditional override that takes precedence
  over all other state logic.
  Mechanism: Drawdown from recent peak exceeds max_drawdown parameter
  (set by personality), OR a single-tick loss exceeds catastrophic_loss
  threshold.
  Hysteresis: NONE. Emergency transitions are immediate and unconditional.
  The cost of a false positive (entering emergency mode unnecessarily)
  is far lower than the cost of a false negative (failing to enter
  emergency mode when needed).
  Default thresholds (personality-overridable):
    - max_drawdown:      0.10 (10% drawdown from recent peak)
    - catastrophic_loss: 0.05 (5% loss in a single tick)
  Example: if the agent's portfolio peaked at 100 and drops to 90,
  the drawdown is 0.10 and EMERGENCY triggers immediately.

EMERGENCY → RECOVERY
  Trigger: Bleeding stops. The immediate threat is contained.
  Mechanism: No further losses for 3+ consecutive ticks. Outstanding
  risk exposure reduced to emergency-minimum levels.
  Hysteresis: Requires 3 consecutive ticks of stability. The agent must
  demonstrate that the crisis is actually over, not just paused.

RECOVERY → CAUTIOUS
  Trigger: Post-mortem analysis complete. The agent understands what
  went wrong and has updated its knowledge accordingly.
  Mechanism: Theta-loop produces a recovery episode with identified
  root causes. Anti-knowledge entries have been created for the
  failure modes. Strategy weights have been updated.
  Hysteresis: Requires completion of at least one full theta-loop
  analysis of the emergency episode.

CAUTIOUS → EXPLORE
  Trigger: Sustained normal performance restores confidence.
  Mechanism: Moving average of performance returns to pre-emergency
  baseline levels. Mood layer (PAD) returns to neutral-positive range.
  Hysteresis: Requires 10+ ticks of normal performance. This is the
  longest hysteresis window in the system — the agent is deliberately
  slow to return to full exploration after a crisis. This prevents the
  common failure mode of "revenge trading" — immediately returning to
  aggressive behavior after a loss.
  Default thresholds:
    - performance baseline: moving average of per-tick returns >= 0.0
      (non-negative) for 10 consecutive ticks
    - mood threshold: PAD Pleasure >= -0.2 (neutral to positive)

CAUTIOUS → EXPLOIT
  Trigger: Agent identifies a high-confidence opportunity while in
  cautious mode, but does not yet feel confident enough to explore broadly.
  Mechanism: Retrieval finds a high-confidence strategy (confidence > 0.9)
  that scored above opportunity threshold for 3+ consecutive ticks.
  Hysteresis: 3 ticks (stricter than the 2-tick EXPLORE → EXPLOIT
  transition, reflecting post-crisis conservatism).

EXPLOIT → EXPLORE
  Trigger: Current exploitation strategy is exhausted — the opportunity
  has been fully captured or market conditions have shifted.
  Mechanism: Per-tick returns from the current strategy fall below the
  exploration threshold (returns < 0.01 for 5+ consecutive ticks) AND
  the agent has not recently been in EMERGENCY state (at least 20 ticks
  since last EMERGENCY exit). The second condition prevents premature
  re-exploration after a crisis that was caused by exploitation.
  Hysteresis: 5 ticks.

CONSOLIDATE → CAUTIOUS
  Trigger: During the dream cycle, the agent discovers alarming patterns
  in its knowledge base — e.g., a high ratio of anti-knowledge to
  knowledge entries, or a significant fraction of recently failed
  predictions.
  Mechanism: The NREM consolidation phase detects one or more of:
    - anti-knowledge ratio > 0.3 (more than 30% of entries are
      anti-knowledge or have active contradictions)
    - prediction accuracy (Brier score) over the last 50 ticks was
      worse than 0.4 (barely better than chance)
    - more than 2 strategy fragments were demoted during this dream cycle
  The dream cycle completes, but instead of transitioning to EXPLORE,
  the agent transitions to CAUTIOUS to re-enter waking operation with
  elevated skepticism.
  Hysteresis: None — the dream cycle itself serves as the hysteresis
  period. The transition is immediate upon dream cycle completion if the
  alarming patterns are detected.

EMERGENCY → EXPLORE (PROHIBITED)
  The agent MUST NOT transition directly from EMERGENCY to EXPLORE.
  Recovery from an emergency requires passing through RECOVERY and then
  CAUTIOUS, each with its own hysteresis. The mandatory path is:
    EMERGENCY → RECOVERY (3 ticks stability)
              → CAUTIOUS (1 theta-loop completion)
              → EXPLORE  (10 ticks normal performance)
  This enforced path prevents an agent from resuming aggressive
  exploration immediately after a catastrophic loss. The minimum time
  from EMERGENCY to EXPLORE is 14 ticks (3 + 1 + 10), assuming the
  theta loop completes within 1 tick. In practice, the theta loop
  typically requires 3-5 ticks, making the minimum path 16-18 ticks.
  There is no shortcut. Any code path that attempts EMERGENCY → EXPLORE
  must be treated as a bug.
```

**Behavioral State Machine Completeness Matrix:**

The matrix below documents the defined behavior for every (state, event) pair.
Cells marked "---" are deliberately undefined (no transition occurs; the agent
remains in the current state). Cells marked "PROHIBITED" are explicitly forbidden.

```
 From \ Event  │ opportunity │ plateau/ │ dream    │ crisis    │ bleeding │ post-   │ recovery │ alarming
               │ found       │ KB full  │ complete │ (loss >   │ stopped  │ mortem  │ perf     │ patterns
               │             │          │          │ threshold)│ (3 ticks)│ done    │ (10 t)   │ in dream
───────────────┼─────────────┼──────────┼──────────┼───────────┼──────────┼─────────┼──────────┼─────────
 EXPLORE       │ ->EXPLOIT   │ ---      │ ---      │->EMERGENCY│ ---      │ ---     │ ---      │ ---
 EXPLOIT       │ ---         │->CONSOL  │ ---      │->EMERGENCY│ ---      │ ---     │ ---      │ ---
 CONSOLIDATE   │ ---         │ ---      │->EXPLORE │->EMERGENCY│ ---      │ ---     │ ---      │->CAUTIOUS
 EMERGENCY     │ ---         │ ---      │ ---      │ ---       │->RECOVERY│ ---     │ ---      │ ---
 RECOVERY      │ ---         │ ---      │ ---      │->EMERGENCY│ ---      │->CAUT.  │ ---      │ ---
 CAUTIOUS      │->EXPLOIT*   │ ---      │ ---      │->EMERGENCY│ ---      │ ---     │->EXPLORE │ ---

 *  CAUTIOUS -> EXPLOIT requires confidence > 0.9 for 3+ ticks (stricter).
    EXPLOIT -> EXPLORE requires returns < 0.01 for 5 ticks AND 20+ ticks since last EMERGENCY.
    PROHIBITED: EMERGENCY -> EXPLORE (must traverse RECOVERY -> CAUTIOUS -> EXPLORE).
```

#### Behavioral State Machine Implementation

```rust
#[derive(Clone, Debug, PartialEq)]
enum BehavioralState {
    Explore, Exploit, Consolidate, Emergency, Recovery, Cautious,
}

/// Counters for hysteresis tracking.
struct StateContext {
    current: BehavioralState,
    /// How many consecutive ticks the current transition trigger has been active.
    trigger_ticks: u32,
    /// Tick when the last EMERGENCY state was exited.
    last_emergency_exit_tick: u64,
    /// Current tick number.
    current_tick: u64,
}

/// Metrics snapshot used for state transition evaluation.
struct TickMetrics {
    /// Has a high-confidence, positive-EV strategy been found?
    opportunity_found: bool,
    /// Strategy confidence (0.0-1.0) for the opportunity.
    opportunity_confidence: f64,
    /// Per-tick moving average of returns.
    return_avg: f64,
    /// Has the dream cycle just completed?
    dream_complete: bool,
    /// Drawdown from recent portfolio peak (0.0-1.0).
    drawdown: f64,
    /// Single-tick loss as fraction of portfolio (0.0-1.0).
    single_tick_loss: f64,
    /// Has bleeding stopped (no further losses)?
    bleeding_stopped: bool,
    /// Has post-mortem (theta-loop) analysis completed?
    postmortem_done: bool,
    /// PAD Pleasure dimension of current mood.
    mood_pleasure: f64,
    /// Knowledge store size.
    kb_size: usize,
    /// Ticks since last consolidation.
    ticks_since_consolidation: u64,
    /// Anti-knowledge ratio in dream analysis.
    anti_knowledge_ratio: f64,
    /// Brier score over last 50 ticks.
    brier_score: f64,
    /// Strategies demoted in current dream cycle.
    strategies_demoted: u32,
}

/// Default thresholds (personality-overridable).
const MAX_DRAWDOWN: f64 = 0.10;
const CATASTROPHIC_LOSS: f64 = 0.05;
const EXPLOITATION_RETURN_THRESHOLD: f64 = -0.02;
const CONSOLIDATION_KB_TRIGGER: usize = 10_000;
const CONSOLIDATION_TICK_TRIGGER: u64 = 500;
const EXPLORE_RETURN_THRESHOLD: f64 = 0.01;
const CAUTIOUS_TO_EXPLOIT_CONFIDENCE: f64 = 0.9;

impl StateContext {
    /// Evaluate transition for the current tick. Returns the new state
    /// (may be the same as current if no transition triggers).
    fn evaluate(&mut self, m: &TickMetrics) -> BehavioralState {
        // EMERGENCY is unconditional and immediate (0-tick hysteresis).
        if m.drawdown >= MAX_DRAWDOWN || m.single_tick_loss >= CATASTROPHIC_LOSS {
            self.trigger_ticks = 0;
            return BehavioralState::Emergency;
        }

        match &self.current {
            BehavioralState::Explore => {
                if m.opportunity_found {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 2 { return BehavioralState::Exploit; }
                } else {
                    self.trigger_ticks = 0;
                }
                BehavioralState::Explore
            }
            BehavioralState::Exploit => {
                // -> CONSOLIDATE: plateau for 5 ticks OR KB too large
                let plateau = m.return_avg < EXPLOITATION_RETURN_THRESHOLD;
                let kb_full = m.kb_size > CONSOLIDATION_KB_TRIGGER
                    && m.ticks_since_consolidation > CONSOLIDATION_TICK_TRIGGER;
                if plateau || kb_full {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 5 { return BehavioralState::Consolidate; }
                } else {
                    self.trigger_ticks = 0;
                }
                // -> EXPLORE: returns exhausted AND not recently in emergency
                if m.return_avg < EXPLORE_RETURN_THRESHOLD
                    && self.current_tick - self.last_emergency_exit_tick >= 20
                {
                    // uses its own counter (would need separate tracking in production)
                }
                BehavioralState::Exploit
            }
            BehavioralState::Consolidate => {
                if m.dream_complete {
                    // Check for alarming patterns
                    if m.anti_knowledge_ratio > 0.3
                        || m.brier_score > 0.4
                        || m.strategies_demoted > 2
                    {
                        return BehavioralState::Cautious;
                    }
                    return BehavioralState::Explore;
                }
                BehavioralState::Consolidate
            }
            BehavioralState::Emergency => {
                if m.bleeding_stopped {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 3 {
                        self.last_emergency_exit_tick = self.current_tick;
                        return BehavioralState::Recovery;
                    }
                } else {
                    self.trigger_ticks = 0;
                }
                BehavioralState::Emergency
            }
            BehavioralState::Recovery => {
                if m.postmortem_done {
                    return BehavioralState::Cautious;
                }
                BehavioralState::Recovery
            }
            BehavioralState::Cautious => {
                // -> EXPLOIT: high-confidence opportunity for 3 ticks
                if m.opportunity_found && m.opportunity_confidence > CAUTIOUS_TO_EXPLOIT_CONFIDENCE {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 3 { return BehavioralState::Exploit; }
                }
                // -> EXPLORE: 10 ticks of normal performance
                else if m.return_avg >= 0.0 && m.mood_pleasure >= -0.2 {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 10 { return BehavioralState::Explore; }
                } else {
                    self.trigger_ticks = 0;
                }
                BehavioralState::Cautious
            }
        }
    }
}
```

### Why Hysteresis Matters

Without hysteresis, the behavioral state machine would oscillate rapidly between states in response to noise. A single bad tick would trigger EMERGENCY; the next normal tick would exit it. A brief market anomaly would cause EXPLORE → EXPLOIT → EXPLORE in rapid succession. Each transition has a cost — it changes retrieval parameters, resets running averages, and disrupts ongoing strategies.

Hysteresis creates "stickiness" — the agent commits to a behavioral state and only transitions when the trigger is sustained. The required duration varies by transition:

- **Emergency entry**: 0 ticks (immediate — safety-critical)
- **Exploration → Exploitation**: 2 ticks (quick — opportunities are time-sensitive)
- **Exploitation → Consolidation**: 5 ticks (moderate — premature consolidation wastes opportunity)
- **Cautious → Explore**: 10 ticks (slow — post-crisis caution is valuable)

This graduated hysteresis reflects a principled asymmetry: the agent enters defensive states quickly and exits them slowly.

### Behavioral States and Collective Dynamics

Individual behavioral states do not exist in isolation — they interact with
collective dynamics when agents operate within a shared substrate (see Chapter
7). Recent research on multi-agent scaling (Venkatesh & Cui 2026,
arXiv:2604.02674) reveals that coordination in LLM agent societies follows
heavy-tailed power-law distributions, with preferential attachment concentrating
activity into "intellectual elites." An agent's behavioral state both shapes and
is shaped by these collective patterns:

- **EXPLORE agents are expansion drivers.** An exploring agent publishes
  speculatively, delegates broadly, and generates the delegation cascades and
  contradiction bursts that drive the heavy tail. A swarm dominated by EXPLORE
  agents will exhibit high expansion but low integration — precisely the
  integration bottleneck that degrades collective intelligence at scale.

- **CONSOLIDATE agents are integration providers.** A consolidating agent
  performs merge operations — organizing, synthesizing, and relating knowledge.
  These are the operations that the integration bottleneck starves. The
  substrate may need to incentivize consolidation behavior (or the DTI mechanism
  described in Chapter 7) when the swarm-wide EXPLORE/CONSOLIDATE ratio is
  unhealthy.

- **CAUTIOUS agents provide adversarial pressure.** An agent in CAUTIOUS state
  applies elevated skepticism and publishes only well-validated knowledge. In
  collective terms, cautious agents serve the competitive regime function —
  challenging low-confidence claims and preventing premature consensus.

- **EMERGENCY cascades are contagious.** When one agent enters EMERGENCY state
  due to catastrophic loss, its behavior (halting publication, reducing
  exposure, rejecting low-trust inputs) propagates through the pheromone system.
  Other agents observing danger pheromones may themselves transition toward
  CAUTIOUS or EMERGENCY states. This cascade can be beneficial (rapid collective
  response to genuine threats) or pathological (panic-driven herding where
  individual hysteresis is overwhelmed by collective signal strength).

> **SECURITY NOTE — Deliberate panic induction.**
> An attacker can exploit EMERGENCY cascade contagion by depositing a
> burst of high-intensity THREAT pheromones (cheap at ~45K gas each),
> potentially triggering EMERGENCY state transitions across multiple
> agents simultaneously. This is a denial-of-service attack on
> collective behavior: agents in EMERGENCY halt publication, reduce
> exposure, and reject low-trust inputs — effectively freezing the
> swarm. Mitigations: (1) THREAT pheromone deposits should require
> higher stake than OPPORTUNITY or WISDOM deposits, (2) an agent
> should discount THREAT pheromones from sources with low Accuracy
> reputation, and (3) EMERGENCY entry should require corroboration
> from the agent's own local state (actual losses), not just external
> pheromone signals.

- **Elite formation interacts with state persistence.** Hysteresis keeps agents
  in their current state; preferential attachment keeps high-activity agents at
  the center of coordination. Together, these create a risk: an agent that
  enters EXPLOIT early and accumulates engagement may remain the dominant
  contributor even when the swarm would benefit from broader participation. The
  substrate's concentration monitoring (Chapter 7) should account for the
  interaction between individual state persistence and collective attachment
  dynamics.

The implication for cognitive architecture design: the behavioral state machine
cannot be tuned in isolation. State transition thresholds, hysteresis windows,
and publication parameters must be evaluated not only for individual agent
performance but for their emergent effects on swarm-level coordination
distributions. An individually optimal policy (aggressive exploitation when
opportunities are found) may be collectively destructive (concentration of
coordination into a small elite that starves integration).

---

## Comparison with Major Cognitive Architectures

### SOAR (Laird et al. 1987; Laird 2012, *The SOAR Cognitive Architecture*)

SOAR (State, Operator, And Result) is one of the earliest and most influential cognitive architectures. Originally developed by John Laird, Allen Newell, and Paul Rosenbloom at Carnegie Mellon, SOAR has been in continuous development for over 35 years.

Key innovations:
- **Universal subgoaling**: When SOAR cannot proceed — when no operator is applicable, multiple operators compete, or an operator produces no result — it creates an *impasse*. The impasse automatically generates a subgoal to resolve the impasse. This is recursive: subgoals can themselves impasse, creating a stack of subgoals. This mechanism is universal — it applies to all types of problem-solving difficulties.
- **Impasse-driven chunking**: When an impasse is resolved, SOAR *chunks* the resolution into a new production rule. The chunk encodes "when you encounter this type of impasse, here is how to resolve it." Over time, this builds a library of compiled knowledge that prevents the same impasse from recurring. This is SOAR's primary learning mechanism.
- **Five memory types**: Working memory (current state), procedural memory (production rules), semantic memory (general facts), episodic memory (timestamped experiences), and spatial memory (spatial/visual representations).

Roko differences: SOAR uses symbolic production rules; Roko uses HDC vectors. SOAR has no affect system. SOAR's learning is purely impasse-driven; Roko uses three-timescale prediction error minimization. SOAR has no shared knowledge substrate.

### ACT-R (Anderson 1993; Anderson & Lebiere 1998, *The Atomic Components of Thought*)

ACT-R (Adaptive Control of Thought — Rational) is the other major classical cognitive architecture, developed by John Anderson at Carnegie Mellon. Where SOAR is rooted in problem-solving, ACT-R is rooted in memory and learning.

Key innovations:
- **Activation-based retrieval**: Every memory chunk in ACT-R has an activation level that determines its retrievability. The base-level activation follows a power-law decay:

  ```
  B_i = ln(Σ t_j^(-d))
  ```

  where `t_j` is the time since the j-th presentation of chunk i, and `d` is a decay parameter (empirically d approximately equals 0.5). This produces the classic spacing effect — memories accessed frequently and recently have high activation; those accessed rarely and long ago have low activation. This single equation accounts for a wide range of human memory phenomena.

- **Production rules as condition-action pairs**: ACT-R's procedural memory consists of IF-THEN production rules that fire when their conditions match the current state of buffers. Productions compete for selection based on expected utility, creating a form of reinforcement learning.
- **Declarative + procedural memory distinction**: ACT-R sharply distinguishes between declarative memory (facts, represented as chunks) and procedural memory (skills, represented as productions). This maps to the psychological distinction between "knowing that" and "knowing how."

Roko differences: ACT-R uses symbolic chunks; Roko uses HDC vectors. The activation-based retrieval in ACT-R is conceptually similar to Roko's demurrage-based decay, but Roko's decay is driven by economic incentives (demurrage cost), not purely temporal. ACT-R has no affect system and no shared knowledge substrate.

### LIDA (Franklin et al. 2014, "LIDA: A Systems-level Architecture for Cognition, Emotion, and Learning")

LIDA (Learning Intelligent Distribution Agent) is explicitly grounded in Global Workspace Theory (GWT), proposed by Bernard Baars (1988) in *A Cognitive Theory of Consciousness*. GWT models consciousness as a broadcasting mechanism — a "global workspace" where specialized processors compete to broadcast their outputs to the entire cognitive system.

Key innovations:
- **Global Workspace Theory implementation**: LIDA's cognitive cycle involves specialized processors ("codelets") competing for access to a shared workspace. The winning coalition broadcasts its content, making it available to all other modules. This is a model of attention — only the most relevant information reaches consciousness (the workspace).
- **Attention codelets**: Specialized processes that scan the current situation for features relevant to the agent's goals. Attention codelets implement a form of relevance filtering — they determine which perceptions and memories deserve cognitive resources.
- **Emotions as appraisal**: LIDA includes an affect system based on appraisal theory — emotions arise from the agent's evaluation of events relative to its goals. This is the closest classical architecture to Roko's ALMA-based affect system, though LIDA's implementation is less computationally specified.

Roko differences: LIDA's global workspace is architecturally similar to Roko's working memory assembly, but Roko uses HDC retrieval rather than codelet competition. LIDA has affect but lacks Roko's three-layer ALMA model with explicit PAD representation. LIDA has no on-chain shared substrate.

### MemGPT (Packer et al. 2023, "MemGPT: Towards LLMs as Operating Systems")

MemGPT is a modern, LLM-native architecture that addresses the context window limitation through an OS-inspired memory hierarchy.

Key innovations:
- **OS-inspired memory hierarchy**: MemGPT treats the LLM context window as RAM (fast, limited) and external storage as disk (slow, unbounded). The agent explicitly manages what is "paged in" to context and what is "paged out" to storage.
- **Self-directed memory management**: The LLM itself decides when to page memory in or out, using function calls to its own memory management API. This is elegant but fragile — it depends on the LLM consistently making good memory management decisions.
- **No learning, no affect**: MemGPT is purely a memory management framework. It does not learn from experience (no prediction error minimization), has no affect system, and does not consolidate knowledge. It is architecturally closer to an operating system than a cognitive architecture.

Roko differences: MemGPT solves only the memory problem. Roko's memory hierarchy is richer (four types with cross-type retrieval via HDC), and Roko adds affect, learning loops, dream cycles, and shared substrate — features that MemGPT does not address.

### Generative Agents (Park et al. 2023, "Generative Agents: Interactive Simulacra of Human Behavior")

Generative Agents is a landmark paper demonstrating that simple architectural choices can produce emergent social behaviors — agents that form relationships, spread information, and coordinate activities.

Key innovations:
- **Three-stream memory**: Observation stream (raw perceptions), reflection stream (synthesized insights about observations), and planning stream (intended future actions). This corresponds roughly to episodic, semantic, and procedural memory, though the distinctions are less formal than in CoALA or Roko.
- **Three-factor scoring (RIR)**: Memory retrieval is scored by `recency x importance x relevance`. Recency decays exponentially with time. Importance is a static score assigned at creation (by the LLM). Relevance is computed by embedding similarity to the current query. The product of these three factors determines which memories surface.
- **Emergent social behaviors**: From this simple architecture, agents spontaneously exhibited behaviors not explicitly programmed — forming friendships, organizing parties, spreading rumors. This demonstrated that complex social behavior can emerge from simple cognitive architecture.

Roko differences: Generative Agents uses text embeddings for retrieval; Roko uses HDC. The RIR scoring is simpler than Roko's tiered retrieval with trust weighting. Generative Agents has no affect system and no learning loops — agents remember but do not learn. There is no shared knowledge substrate.

### CoALA (Sumers et al. 2024, "Cognitive Architectures for Language Agents")

CoALA is a unifying framework that organizes the design space of LLM-based cognitive architectures. It is less a specific architecture than a taxonomy for comparing architectures.

Key innovations:
- **Unifying framework**: CoALA identifies the common components across LLM-based agents: memory (working, episodic, semantic, procedural), action space (internal actions like retrieval and reasoning, external actions like tool use and communication), and decision-making (planning, reactive, or hybrid).
- **Four memory types**: Working memory (current context), episodic memory (past experiences), semantic memory (general knowledge), and procedural memory (action templates). These map directly to the psychological taxonomy and provide a common vocabulary for comparing architectures.
- **Grounding + retrieval as core operations**: CoALA identifies two fundamental operations: grounding (converting perceptions into memory representations) and retrieval (recovering relevant memories for decision-making). All cognitive architectures, classical or modern, can be characterized by how they implement these two operations.

Roko differences: CoALA is a framework, not an implementation. Roko is a concrete implementation that fits within CoALA's taxonomy but extends it with HDC-unified storage, ALMA affect, three-timescale learning loops, dream cycles, and on-chain shared substrate.

### Voyager (Wang et al. 2023)

Voyager is a Minecraft agent that demonstrates open-ended learning through a skill library and curriculum-driven exploration.

Key innovations:
- **Skill library**: Voyager accumulates a library of executable code snippets (skills) that it has written and verified. Each skill is stored with a description embedding, enabling retrieval by natural language query. This is a form of procedural memory where the "procedures" are literally executable programs.
- **Curriculum learning**: An automatic curriculum module generates increasingly difficult tasks for the agent, ensuring it continually expands its capabilities rather than plateauing on easy tasks.
- **Iterative code refinement**: When a skill fails, Voyager debugs and rewrites it using environment feedback, LLM reasoning, and error messages. This is a concrete implementation of predict-observe-correct at the code level.

Roko differences: Voyager operates in a game environment with ground-truth feedback; Roko operates on-chain with noisy, delayed, and potentially adversarial feedback. Voyager's skill library is code-based; Roko's procedural memory is HDC-vector-based. Voyager has no affect system, no shared substrate, and no dream cycle.

### Summary Comparison Table

| Architecture | Memory Types | Learning | Affect | HDC |
|-------------|-------------|---------|--------|-----|
| **SOAR** | 5 (working, procedural, semantic, episodic, spatial) | Impasse-driven chunking | No | No |
| **ACT-R** | 4 (declarative, procedural, perceptual, motor) | Activation-based retrieval | No | No |
| **LIDA** | Global workspace + attention | Attention codelets | Yes (appraisal) | No |
| **MemGPT** | 2 (main context = RAM, external = disk) | OS-style paging | No | No |
| **Generative Agents** | 3-stream (observation, reflection, planning) | 3-factor scoring (RIR) | No | No |
| **CoALA** | 4 (working, episodic, semantic, procedural) | Grounding + retrieval | No | No |
| **Voyager** | Skill library + curriculum | Iterative code refinement | No | No |
| **Roko** | 4 (working, episodic, semantic, procedural) + anti-knowledge | 3-loop predict-publish-correct | Yes (ALMA/PAD) | Yes (BSC 10,240-bit) |

Roko's key advantages:
1. **HDC unifies all memory types** in one vector space
2. **Affect system** creates mood-congruent retrieval (biologically grounded)
3. **Anti-knowledge** is structurally distinct (not just labeled)
4. **Shared substrate** enables collective intelligence (no other architecture has this)
5. **Demurrage** prevents unbounded memory growth (all others need external GC)
6. **Dream cycle** provides offline consolidation and creative recombination
7. **Three-timescale learning** provides single-loop, transitional, and double-loop adaptation

The comparison above situates Roko within the field. But recent work has
identified a deeper issue that affects all architectures in the table --
including Roko -- and validates a key design decision in its tier system.

### The Missing Knowledge Layer (2026)

> Roynard, M. (2026). "The Missing Knowledge Layer in Cognitive Architectures for AI Agents." arXiv:2604.11364. LAAS-OASIS.

Roynard (2026) identifies a fundamental architectural gap in the two most influential cognitive architecture frameworks for AI agents: CoALA (Sumers et al. 2024) and JEPA (LeCun 2022). Neither framework provides an explicit Knowledge layer with its own persistence semantics. Both treat all stored information -- facts, experiences, skills, inferences -- with the same update and decay mechanics. This, Roynard argues, produces a **category error**: systems apply cognitive decay to factual claims, or treat facts and experiences with identical update mechanics.

The core argument is precise: "A paper's findings do not become less true after 69 days, and a relationship between two concepts does not fade after a calendar month. What decays is the agent's *attentional relevance* to the information -- a memory concern, not a knowledge concern." Conflating "I have not accessed this recently" with "this is less valuable" is a category mistake that corrupts knowledge stores over time. Factual knowledge that happens to be unreferenced should not decay the same way an episodic memory of a routine event does.

**The Four-Layer Decomposition.** Roynard proposes decomposing cognitive storage into four layers, each with fundamentally different persistence semantics:

| Layer | Persistence Semantics | Update Mechanism | What It Stores |
|-------|----------------------|------------------|----------------|
| **Knowledge** | Indefinite supersession | Old entries persist until explicitly replaced by better versions; supersession creates a chain, not a deletion | Facts, causal relationships, established findings |
| **Memory** | Ebbinghaus decay | Retention decays exponentially with time; each access or reinforcement resets the curve | Episodes, experiences, observations |
| **Wisdom** | Evidence-gated revision | Entries require accumulated evidence to revise; no single observation can overturn established wisdom | Validated heuristics, confirmed strategies, stable principles |
| **Intelligence** | Ephemeral inference | No persistence at all; exists only during active reasoning | LLM context window contents, in-flight reasoning chains, working computations |

The key insight is that these are not just different *categories* of information -- they require different *persistence mechanics*. Applying Ebbinghaus decay to factual knowledge is architecturally incorrect. Treating wisdom the same as memory allows a single contradictory observation to destabilize validated heuristics. And persisting inference products (which should be ephemeral) bloats the knowledge store with stale reasoning artifacts.

**Critique of CoALA.** CoALA's taxonomy identifies four memory types (working, episodic, semantic, procedural) but does not assign them distinct persistence semantics. Semantic memory in CoALA -- which stores "general knowledge" -- is subject to the same retrieval and decay mechanisms as episodic memory. This means a well-established causal relationship (e.g., "high funding rates on perps predict spot sell pressure") decays at the same rate as a one-off observation (e.g., "ETH dropped 3% at 14:32 UTC on Tuesday"). The framework provides no mechanism to say "this is a fact that should persist until superseded" versus "this is an experience that should fade naturally."

**Convergence evidence.** Roynard surveys persistence semantics across eight independent convergence points -- from Karpathy's LLM Knowledge Base pattern (which implicitly separates knowledge from memory by using markdown files that persist indefinitely) to the BEAM benchmark's near-zero contradiction-resolution scores (demonstrating that existing systems cannot distinguish between outdated knowledge and actively contradicted knowledge). Six independent community voices articulated the same diagnosis within a single week on Reddit, spanning three distinct concern tiers, suggesting the architectural gap is widely felt even if rarely formalized.

**How each layer maps to the Roko tier system:**

| Roynard Layer | Roko Equivalent | Mapping Rationale |
|---------------|----------------|-------------------|
| Knowledge (indefinite supersession) | `KnowledgeTier::Persistent` | Persistent-tier knowledge in Roko has a 5.0x half-life multiplier and is never automatically deleted -- only demoted. A Persistent insight is replaced only when a better version supersedes it, not because time passed. This is precisely Roynard's "indefinite supersession." |
| Memory (Ebbinghaus decay) | `KnowledgeTier::Transient` / `KnowledgeTier::Working` + demurrage | Transient (0.1x) and Working (0.5x) tiers decay aggressively via the demurrage formula `balance(t) = balance(t0) * exp(-lambda * (t - t0))`. Each query or confirmation resets the decay clock, mirroring Ebbinghaus' finding that each reinforcement resets the forgetting curve. Unreinforced entries decay to garbage collection -- exactly the "attentional relevance" decay Roynard describes. |
| Wisdom (evidence-gated revision) | `KnowledgeTier::Consolidated` + confirmation gates | Consolidated-tier entries require 25 independent confirmations to promote to Persistent. This is an evidence gate: no single observation can promote (or demote) Consolidated knowledge. The confirmation threshold ensures that only entries with broad evidentiary support achieve stable persistence, matching Roynard's "evidence-gated revision." |
| Intelligence (ephemeral inference) | Working memory / LLM context window | The LLM context window in Roko is assembled fresh each cognitive tick by the dynamic context assembly process. Nothing in the context window persists between ticks unless it is explicitly stored as a knowledge entry. This is transient by design -- Roynard's "ephemeral inference" layer. |

**Why this validates the Roko approach.** Roynard's paper argues that most cognitive architectures are missing an explicit separation of persistence semantics -- they treat all stored information with the same update mechanics, leading to either premature decay of valuable knowledge or indefinite retention of stale observations. Roko's tier system with demurrage *already implements* exactly the kind of differentiated persistence semantics that Roynard proposes as missing from CoALA and other frameworks:

- Transient/Working tiers decay aggressively (Ebbinghaus-style), ensuring that unreinforced observations fade naturally.
- Consolidated tier gates promotion behind evidence thresholds, preventing premature elevation of unvalidated claims.
- Persistent tier provides indefinite retention with supersession semantics (demote, never delete).
- The context window is ephemeral by construction -- nothing survives a tick boundary without explicit storage.

**Demurrage vs. ad-hoc garbage collection.** Roynard's framework also strengthens the case for demurrage-based forgetting over the ad-hoc garbage collection mechanisms used by most agent memory systems. Systems like MemGPT, Generative Agents, and LangChain's memory modules rely on either fixed context windows (discard oldest), LRU-style eviction (discard least-recently-used), or periodic manual pruning. None of these approaches encode persistence semantics into the storage layer itself. Demurrage, by contrast, is a continuous, economically-motivated decay function that is intrinsic to the knowledge entry. Each entry's effective half-life is determined by its tier (persistence semantics) and its kind (domain-appropriate decay rate), not by an external garbage collector making ad-hoc eviction decisions. This is precisely the kind of "persistence semantics baked into the layer" that Roynard argues is architecturally necessary.

Where Roynard addresses the *storage* dimension of cognition (what persists and
how), the next section addresses the *learning* dimension -- the theoretical
framework that governs how agents update their beliefs and select actions.

---

### Active Inference: From Theory to Implementation (2024-2025)

Active inference (AIF) -- Karl Friston's framework for understanding adaptive systems as prediction-error minimizers -- was, until recently, a theoretical framework without concrete LLM implementations. That changed in late 2024. Three independent implementations now demonstrate that active inference is not merely compatible with LLM-based agents but may be architecturally superior to reward-engineered alternatives. These implementations also provide striking external validation of several design choices in the Roko cognitive architecture.

#### Three Concrete Implementations

**1. Prakki (2024) -- Active Inference as Cognitive Layer Above LLMs**

Prakki's "Active Inference for Self-Organizing Multi-LLM Systems: A Bayesian Thermodynamic Approach to Adaptation" (arXiv:2412.10425) is the most architecturally detailed implementation. It introduces an active inference agent that functions as a *cognitive layer* above an LLM, dynamically adjusting prompts and search strategies through principled information-seeking behavior.

The generative model uses three state factors:

- **Prompt state** (33 possible configurations) -- which prompt combination is currently in use. The agent explores a structured space of prompt variations rather than relying on a single static prompt.
- **Search state** (11 configurations) -- which information-gathering strategy is active. This covers different search approaches, retrieval methods, and query formulations.
- **Information state** (3 levels: none, basic, detailed) -- how much environmental knowledge the agent has accumulated. This tracks the agent's epistemic progress.

Seven observation modalities provide structured feedback:

- Three prompt-dependent modalities (accuracy, relevance, comprehensiveness of LLM responses)
- Three search-dependent modalities (information relevance, usefulness, source quality)
- One information-dependent modality (direct mapping of information level)

The critical architectural insight: the LLM is *not* the policy. The active inference layer selects policies (which prompts to use, which searches to perform) by minimizing expected free energy. The LLM *executes* those policies and then *evaluates* the results, producing structured quality assessments (JSON with 0.0-1.0 scores) that become observations feeding back into the generative model. This decouples policy selection from execution -- the Bayesian machinery handles "what to try next" while the LLM handles "how to do it and how well did it work."

**2. Beckenbauer et al. (2025) -- Orchestrator for Multi-Agent Coordination**

"Orchestrator: Active Inference for Multi-Agent Systems in Long-Horizon Tasks" (arXiv:2509.05651, NeurIPS 2025) extends active inference to multi-agent coordination. The Orchestrator framework uses free energy as a quantifiable measure of system uncertainty, allocating coordination resources to agents exhibiting the greatest uncertainty. The system employs attention-inspired self-emergent coordination -- agents with higher free energy (more confusion, more uncertainty) receive greater attention allocation, analogous to how biological attention prioritizes surprising stimuli.

Results on maze puzzles of increasing complexity show a threefold improvement in success rate under orchestrator-enabled coordination versus solo LLM baselines, with up to 100% accuracy on medium-difficulty tasks and 76.67% on hard (25x25) mazes.

**3. Wen (2025) -- The Thermodynamic Necessity Argument**

"The Missing Reward: Active Inference in the Era of Experience" (arXiv:2508.05619) makes the strongest theoretical claim: active inference is not merely a useful framework but a *thermodynamic necessity* for AI systems at scale. The argument proceeds from Landauer's principle -- erasing one bit of information necessarily dissipates at least kT ln 2 of heat -- to conclude that information processing has irreducible physical costs. Current foundation models operate far above this theoretical limit, but the scaling trajectory makes thermodynamic efficiency an eventual hard constraint.

Wen's key argument: reward engineering shifts the bottleneck from data curation to reward curation, and each new domain demands fresh reward design -- creating the labor-intensive bottleneck that autonomous systems should eliminate. Active inference resolves this by replacing external reward with intrinsic free energy minimization: the agent's objective is to minimize surprise (prediction error), which requires no external reward signal.

#### Mapping to Roko's Predict-Publish-Correct Loops

The convergence between active inference and Roko's three-timescale learning loops is not superficial -- it reflects a deep structural isomorphism.

**Active inference decomposes expected free energy (EFE) into two terms:**

```
G_pi = -E[D_KL(q(s|o,pi) || q(s|pi))]  -  E[ln p(o|pi)]
        \_________________________/          \____________/
              epistemic value                pragmatic value
         (information gain:                  (goal achievement:
          reduce uncertainty)                match preferred outcomes)
```

**Roko's gamma loop = per-tick prediction error minimization.** Each tick, the agent predicts, observes, and updates confidence on relevant knowledge. This is single-loop learning -- adjusting actions within a fixed strategy. In AIF terms, this is variational free energy minimization: the agent updates its beliefs about the current state given new observations. The Brier score that drives gamma-loop rewards is a direct measure of prediction error -- the same quantity that variational free energy bounds.

**Roko's theta loop = episode-level EFE evaluation.** At episode boundaries, the agent evaluates whether its strategy worked over a sustained horizon. This corresponds to expected free energy evaluation over longer policy horizons -- the agent assesses not just "was this tick's prediction accurate?" but "did this strategy produce the preferred trajectory?" The theta loop creates new episodic memories, updates strategy weights, and generates heuristics -- structural model updates that go beyond parameter adjustment.

**Roko's delta loop = policy optimization via aggregate EFE.** The delta loop aggregates performance across many episodes and asks: should the agent's fundamental orientation change? This is double-loop learning -- changing the values that govern strategy selection. In AIF terms, this is policy-level optimization: evaluating the expected free energy of entire strategy families and reweighting toward those that minimize long-run surprise.

```
AIF Concept              Roko Analog              Shared Mechanism
-----------              -----------              ----------------
Variational FE           Gamma loop               Per-observation belief update
  minimization             (per-tick)               via prediction error

Expected FE              Theta loop               Episode-horizon policy
  evaluation               (per-episode)            evaluation

Policy optimization      Delta loop               Aggregate performance
  via aggregate EFE        (periodic)               drives strategy evolution

Epistemic value          Exploration state         Seek information when
  (info gain term)         + broad retrieval        uncertainty is high

Pragmatic value          Exploitation state        Pursue known-good strategies
  (preference term)        + narrow retrieval       when model is confident
```

#### The LLM-as-EFE-Evaluator vs. LLM-as-Reasoner Architecture

Prakki's implementation reveals an important architectural distinction. In most LLM agent frameworks (including Roko's current design), the LLM is the *reasoner* -- it receives context, deliberates, and outputs decisions. The LLM is the policy.

In Prakki's active inference framework, the LLM serves a different role: it is the *evaluator*. The active inference layer maintains the generative model (A, B, C, D matrices), computes expected free energy for candidate policies, and selects actions. The LLM executes those actions and then evaluates the quality of results, producing structured observations that feed back into the Bayesian model.

This separation has several implications:

- **Policy selection is principled.** Instead of relying on the LLM to implicitly balance exploration and exploitation through prompt engineering, the EFE computation handles this mathematically. The softmax policy selection -- q(pi) = sigma(gamma * G + ln E) -- provides explicit control over the exploration-exploitation tradeoff through the precision parameter gamma.
- **Learning is structured.** The generative model's A matrices (observation likelihoods) and B matrices (transition dynamics) are updated via outer-product accumulation: a^(t+1) = a^t + eta * (o tensor q(s)). This produces interpretable learned structure -- the final observation matrices show which prompt-state combinations produce high-quality outputs.
- **The LLM's role is bounded.** The LLM does what it does well (language understanding, quality assessment) without being asked to do what it does poorly (principled uncertainty quantification, systematic exploration).

Roko's architecture uses the LLM as reasoner, but the three-timescale learning loops perform a function analogous to the AIF generative model -- they accumulate structured knowledge about which strategies work (procedural memory), track prediction accuracy (gamma loop), and optimize policy selection (delta loop). The key difference: Roko distributes across HDC-encoded memory what Prakki concentrates in explicit Bayesian matrices.

#### The Thermodynamic Argument: Surprise Minimization as Necessity

Wen's thermodynamic argument deserves careful attention because it elevates active inference from "useful framework" to "eventual architectural requirement."

The argument chain:

1. **Landauer's principle** establishes that information erasure has irreducible energy cost: at least kT ln 2 per bit.
2. **Current AI systems** operate far above this limit, but the gap is closing as models scale. Training a frontier LLM already consumes megawatt-hours; the energy cost of trial-and-error RL (wholesale parameter updates, heuristic exploration, explicit unlearning) scales poorly.
3. **Active inference offers three efficiency advantages:**
   - *Information gain replaces heuristic exploration.* Instead of epsilon-greedy or other ad-hoc exploration strategies, the epistemic value term in EFE directs exploration toward states that maximally reduce uncertainty. No wasted exploration of already-understood states.
   - *Incremental belief updates replace wholesale parameter changes.* Updating a generative model's sufficient statistics is orders of magnitude cheaper than backpropagating through billions of parameters. The Bayesian update is the thermodynamically minimal learning operation.
   - *Natural memory decay eliminates energy-intensive unlearning.* In AIF, beliefs that are not reinforced by observations naturally decay toward priors. There is no need for explicit "forgetting" mechanisms -- the model's precision naturally concentrates on well-evidenced beliefs.

4. **At sufficient scale, surprise minimization is not optional -- it is the thermodynamically efficient path.** Systems that do not minimize surprise waste energy on prediction errors that could have been avoided, exploration that could have been directed, and unlearning that could have been unnecessary. Landauer's principle guarantees that this waste has real physical cost.

This argument reinforces the Roko architecture's foundation on Friston's free energy principle. The predict-publish-correct loops are not merely a convenient learning framework -- they are, if Wen's argument holds, the thermodynamically natural way for persistent agents to operate.

#### Emergence Finding: Exploration-to-Exploitation Transitions

Perhaps the most striking result from Prakki's implementation is the natural emergence of exploration-to-exploitation transitions -- without any explicit programming of behavioral states.

In the early phase (~40 timesteps), the agent predominantly selects search actions -- information-gathering behaviors that reduce uncertainty about the environment. As the generative model stabilizes (observation matrices develop structured patterns, information state reaches "detailed"), the agent naturally transitions to prompt-testing actions -- exploiting its learned knowledge to optimize output quality.

This emergent transition arises directly from the EFE mathematics:

- When uncertainty is high, the epistemic value term (information gain) dominates EFE. Search actions that reduce uncertainty have high epistemic value, so they are preferentially selected.
- As the model converges and uncertainty decreases, the epistemic term shrinks. The pragmatic value term (achieving preferred outcomes) now dominates. The agent shifts to exploitation -- using its learned model to select high-quality prompt configurations.

This is precisely the behavior that Roko's behavioral state machine implements *explicitly* through the EXPLORE-to-EXPLOIT transition with hysteresis. The AIF result suggests that the behavioral state machine is not an arbitrary design choice but a natural consequence of rational information processing. The hysteresis in Roko's state transitions serves the same function as the gradual EFE rebalancing in Prakki's model -- it prevents premature commitment to exploitation when the model is still uncertain, and prevents reversion to exploration when the model is confident.

The convergence goes deeper. Roko's full six-state behavioral machine (EXPLORE, EXPLOIT, CONSOLIDATE, EMERGENCY, RECOVERY, CAUTIOUS) can be understood as a discrete approximation of the continuous EFE landscape:

- **EXPLORE** = high epistemic value regime (uncertainty dominates)
- **EXPLOIT** = high pragmatic value regime (preferences dominate)
- **CONSOLIDATE** = offline model maintenance (analogous to updating A and B matrices)
- **EMERGENCY/RECOVERY/CAUTIOUS** = high-surprise regimes where the generative model has been violated, requiring model repair before normal EFE evaluation can resume

The AIF framework does not naturally produce the emergency/recovery states -- those require explicit design for risk management. But the core EXPLORE-EXPLOIT-CONSOLIDATE cycle emerges naturally from expected free energy minimization, providing independent theoretical grounding for Roko's behavioral state machine design.

> **References:**
>
> Prakki, R. (2024). "Active Inference for Self-Organizing Multi-LLM Systems: A Bayesian Thermodynamic Approach to Adaptation." arXiv:2412.10425.
>
> Beckenbauer, L., Loewe, J.-L., Zheng, G. & Brintrup, A. (2025). "Orchestrator: Active Inference for Multi-Agent Systems in Long-Horizon Tasks." arXiv:2509.05651. NeurIPS 2025.
>
> Wen, B. (2025). "The Missing Reward: Active Inference in the Era of Experience." arXiv:2508.05619.
