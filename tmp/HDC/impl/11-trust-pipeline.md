# 11 — Trust Pipeline & Cognitive Immune System

> **Status: PARTIALLY IMPLEMENTED, FRAGMENTED** -- Core trust structures exist
> but are split across two files with known bugs. Immune layers 2-5 are not
> implemented. See [Issues Found](#issues-found) and
> [Fix Checklist](#fix-checklist) below.

> **Design source:** `07-shared-substrate.md` (sections: Trust Pipeline for
> Consumers, Cognitive Immune System, Reputation Registry)
>
> **Crate:** `kora-hdc` (all logic is off-chain, local to each agent)
>
> **On-chain dependency:** `InsightBoard` contract (read-only — confirmations,
> stake amounts, publish blocks), `ReputationRegistry` contract (read-only —
> composite scores, agent history)
>
> **Key constraint:** Every computation here uses `f64`. None of it is
> consensus-critical. If any piece is ever moved on-chain, replace all `f64`
> with fixed-point integer arithmetic (see doc 09).

### Source files

| Component | File |
|-----------|------|
| Trust registry (EMA, pipeline) | `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs` |
| Knowledge-level trust (DUPLICATE) | `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs` |
| Scoring (uses balance not trust) | `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/scoring.rs` |
| Knowledge entry (trust field) | `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/entry.rs` |

### Issues Found

- **G01: Two separate trust implementations** -- `trust.rs` at the crate root AND `knowledge/trust.rs` inside the knowledge module. They define overlapping types and logic. This causes confusion about which is canonical.
- **G02: EMA double-application bug** -- `TrustRegistry::update_trust()` applies EMA, then the caller may also apply EMA via a second path, producing double-smoothing.
- **G03: Hardcoded 0.5 stake weight** -- Stage 4 (stake weighting) uses a hardcoded `0.5` floor instead of the log-scaled formula `0.5 + ln(ratio)/10` specified in this doc.
- **G05: No NaN/Infinity guards** -- `f64` operations like `ln()` and `powf()` can produce NaN or Infinity for edge-case inputs (e.g., zero stake, negative age). No guards exist.
- **G06: `scoring.rs` uses `entry.balance` instead of trust score** -- The scoring module weights entries by their token balance, not their computed trust score, defeating the purpose of the 5-stage pipeline.
- **G07: Missing 7-domain ReputationDomain enum** -- The spec defines 7 domains (Accuracy, Timeliness, Novelty, Reliability, Collaboration, Specialization, Integrity). The implementation has a simpler model without per-domain scores.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Trust Pipeline (5 Stages)](#2-trust-pipeline-5-stages)
3. [Reputation System](#3-reputation-system)
4. [Taint Propagation](#4-taint-propagation)
5. [5-Layer Immune System](#5-5-layer-immune-system)
6. [Known Security Gaps](#6-known-security-gaps)
7. [Anti-Patterns](#7-anti-patterns)
8. [Checklist](#8-checklist)
9. [Test Plan](#9-test-plan)

---

## 1. Overview

When an agent queries the shared substrate, raw results are untrusted. Before
any insight enters the agent's local context window, it passes through:

1. **Trust pipeline** — 5 multiplicative stages that produce a score in `[0.0, 1.0]`
2. **WisdomGate** — threshold-based quality gates that accept or reject
3. **Immune system** — 5 defense layers that detect and contain adversarial knowledge

The trust pipeline and immune system are entirely **off-chain**. Each agent
runs its own instance. Different agents may compute slightly different scores
due to floating-point non-determinism — this is acceptable because trust is a
local policy decision, not a consensus operation.

### Data Flow

```
On-chain (InsightBoard)          Off-chain (agent-local)
========================         =======================

  query results            -->   Trust Pipeline (5 stages)
  + confirmations                    |
  + stake amounts                    v
  + publish blocks               WisdomGate (accept/reject)
  + author addresses                 |
                                     v
  reputation scores        -->   Immune System (5 layers)
  + agent history                    |
                                     v
                                 Agent context window
```

---

## 2. Trust Pipeline (5 Stages)

The pipeline is **multiplicative**: each stage multiplies a running trust
accumulator. The final value is clamped to `[0.0, 1.0]`.

### Constants

```rust
/// Floor reputation for agents with no on-chain history.
const COLD_START_REPUTATION: f64 = 0.1;

/// Minimum stake accepted by InsightBoard (contract-enforced).
/// Used as the denominator for log-scale stake weighting.
const MIN_STAKE: U256 = /* defined in InsightBoard contract */;

/// HDC vector dimensionality.
const D: usize = 10_240;
```

### Struct Definitions

```rust
/// An insight anchor retrieved from the on-chain InsightBoard.
/// Contains all on-chain metadata needed to compute trust.
pub struct InsightAnchor {
    pub id: InsightId,
    pub author: Address,
    pub vector: HdcVector,          // 10,240-bit binary vector
    pub context_vector: HdcVector,  // publication context encoding
    pub publish_block: u64,
    pub tier: InsightTier,          // EPHEMERAL / WORKING / ARCHIVAL
    pub confirmations: u32,
    pub staked_amount: U256,
    pub taint_level: TaintLevel,
}

/// The consumer agent's current task context.
pub struct TaskContext {
    pub query_vector: HdcVector,    // what the agent is looking for
    pub current_block: u64,
}

/// Tier-specific half-lives (in blocks).
#[derive(Clone, Copy)]
pub enum InsightTier {
    Ephemeral,  // half-life ~100 blocks
    Working,    // half-life ~1000 blocks
    Archival,   // half-life ~10000 blocks
}
```

### The Pipeline Function

```rust
/// OFF-CHAIN / LOCAL ONLY.
///
/// Computes a trust score for a shared insight before it enters the
/// agent's context window. Each stage is multiplicative.
///
/// WARNING: Uses f64 throughout. NOT consensus-safe. Do not move on-chain
/// without converting every operation to fixed-point integer arithmetic.
pub fn compute_trust(&self, insight: &InsightAnchor, context: &TaskContext) -> f64 {
    let mut trust = 1.0;

    // ── Stage 1: Author Reputation ──────────────────────────────
    // Composite score across 7 domains, weighted by insight kind.
    // Floor: COLD_START_REPUTATION = 0.1 for new agents.
    let rep = self.reputation_registry.composite_score(insight.author);
    trust *= rep;

    // ── Stage 2: Freshness Decay ────────────────────────────────
    // Exponential decay keyed to the insight's tier half-life.
    // Formula: 0.5^(age / half_life)
    // Never reaches 0.0 for finite age.
    let age = context.current_block.saturating_sub(insight.publish_block);
    let half_life = tier_half_life(insight.tier);
    trust *= 0.5_f64.powf(age as f64 / half_life as f64);

    // ── Stage 3: Confirmation Boost ─────────────────────────────
    // Base: 0.5 (unconfirmed = 50% penalty).
    // Each confirmation adds 5%, linear. Capped at 1.0 (10 confs).
    let conf_factor = (0.5 + 0.05 * insight.confirmations as f64).min(1.0);
    trust *= conf_factor;

    // ── Stage 4: Stake Weighting ────────────────────────────────
    // Logarithmic scaling against MIN_STAKE. Floor: 0.5.
    // Doubling stake adds ~0.07, not 2x. Prevents whale dominance.
    let stake_ratio = insight.staked_amount.as_f64() / MIN_STAKE.as_f64();
    let stake_factor = (0.5 + stake_ratio.ln().max(0.0) / 10.0).min(1.0);
    trust *= stake_factor;

    // ── Stage 5: Relevance Gating ───────────────────────────────
    // Normalized Hamming similarity between insight context and query.
    // CAN produce 0.0 for maximally irrelevant vectors (all bits differ).
    let raw_distance = hamming_distance(&insight.context_vector, &context.query_vector);
    let relevance = 1.0 - (raw_distance as f64 / D as f64);
    trust *= relevance;

    // ── Final Clamp ─────────────────────────────────────────────
    trust.clamp(0.0, 1.0)
}

/// Returns the half-life in blocks for a given insight tier.
fn tier_half_life(tier: InsightTier) -> u64 {
    match tier {
        InsightTier::Ephemeral => 100,
        InsightTier::Working   => 1_000,
        InsightTier::Archival  => 10_000,
    }
}
```

### Worked Example

An insight published 500 blocks ago, WORKING tier, by an agent with
composite reputation 0.7, with 5 confirmations, staked at 3x MIN_STAKE,
context distance 0.15 from the query vector:

```
Stage 0 (init):      trust = 1.000
Stage 1 (rep):       trust *= 0.7                     = 0.700
Stage 2 (freshness): trust *= 0.5^(500/1000)          = 0.700 * 0.707 = 0.495
Stage 3 (confirm):   trust *= (0.5 + 0.05*5) = 0.75   = 0.495 * 0.75  = 0.371
Stage 4 (stake):     trust *= (0.5 + ln(3)/10) = 0.610 = 0.371 * 0.610 = 0.226
Stage 5 (relevance): trust *= (1.0 - 0.15) = 0.85     = 0.226 * 0.85  = 0.192
Clamp:               trust = 0.192 (within [0.0, 1.0], no change)

Final: 0.192 — low/moderate. Cross-reference with local knowledge.
```

### Trust Score Interpretation

| Trust Score | Meaning | Recommended Action |
|-------------|---------|-------------------|
| > 0.8 | High confidence | Use directly in context |
| 0.5 - 0.8 | Moderate | Use with caveats, seek confirmation |
| 0.2 - 0.5 | Low | Cross-reference with local knowledge |
| < 0.2 | Very low | Ignore or flag for investigation |

### Edge Cases

**No stage except Stage 5 can produce exactly 0.0:**

| Stage | Minimum Value | Why |
|-------|--------------|-----|
| 1 (Reputation) | 0.1 | `COLD_START_REPUTATION` floor. EMA decay (`0.9 * old`) asymptotes to zero but never reaches it. Agents below 0.01 are pre-filtered before the pipeline runs. |
| 2 (Freshness) | ~0.03 | `0.5^(age/hl)` never reaches 0 for finite age. Entries transition to ARCHIVED at 5x half-life (~0.03) and PURGED at 10x (~0.001). |
| 3 (Confirmation) | 0.5 | 0 confirmations gives `0.5 + 0.05*0 = 0.5`. Unconfirmed is penalized 50%, not excluded. |
| 4 (Stake) | 0.5 | At exactly MIN_STAKE: `ln(1)/10 = 0`, so `0.5 + 0 = 0.5`. Staking below MIN_STAKE is rejected by the contract. |
| 5 (Relevance) | 0.0 | Distance of 1.0 (every bit differs) gives `1.0 - 1.0 = 0.0`. Astronomically unlikely for non-adversarial vectors. |

**Effective trust floor** for a legitimate new-agent insight:
`0.1 * 0.03 * 0.5 * 0.5 * 0.1 = 0.000075` — well below any behavioral
state's threshold (lowest is EXPLORE at 0.15). New agents must earn
confirmations and build reputation before their insights enter context windows.

---

## 3. Reputation System

### 7 Domains

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReputationDomain {
    Accuracy,       // ratio of confirmed-correct insights to total published
    Timeliness,     // percentile rank of publication block among similar insights
    Novelty,        // avg HDC distance to nearest existing insight at publication time
    Reliability,    // 1 - normalized_variance(accuracy over rolling window)
    Collaboration,  // confirmation rate: issued / opportunities
    Specialization, // entropy of publication vectors in HDC space
    Integrity,      // track record of validated anti-knowledge contributions
}
```

### Reputation Scores Struct

```rust
pub struct ReputationScores {
    /// Per-domain scores. Each in [0.0, 1.0].
    pub scores: HashMap<ReputationDomain, f64>,
    /// Number of update cycles this agent has participated in.
    pub cycles: u64,
    /// Total publications across all time.
    pub total_publications: u64,
    /// Number of prior slashes.
    pub prior_slashes: u32,
    /// Number of anomaly flags raised against this agent.
    pub anomaly_flags: u32,
}
```

### EMA Decay

Reputation decays via Exponential Moving Average with smoothing factor
`alpha = 0.1` per update cycle.

```rust
const EMA_ALPHA: f64 = 0.1;

/// Update a single domain's reputation score.
/// `observation` is the metric from the most recent evaluation window.
/// When no observation exists (agent inactive), pass `observation = 0.0`.
pub fn ema_update(&mut self, domain: ReputationDomain, observation: f64) {
    let old = self.scores.get(&domain).copied().unwrap_or(COLD_START_REPUTATION);
    let new_score = EMA_ALPHA * observation + (1.0 - EMA_ALPHA) * old;
    self.scores.insert(domain, new_score);
}
```

**Decay rate for inactive agents:**

```
new_score = 0.1 * 0.0 + 0.9 * old_score = 0.9 * old_score

After 10 cycles: 0.9^10  = 0.349 of original
After 20 cycles: 0.9^20  = 0.122 of original
After 44 cycles: 0.9^44  < 0.01 (effectively zero)
```

### Composite Score

```rust
/// Compute weighted average across all 7 domains.
///
/// Returns COLD_START_REPUTATION (0.1) for agents with no history.
/// Domain weights depend on the insight kind being evaluated (e.g., for
/// CAUSAL_LINK, weight Accuracy and Novelty more heavily than Timeliness).
pub fn composite_score(&self, agent: &Address) -> f64 {
    let scores = self.get_scores(agent);
    if scores.is_empty() {
        return COLD_START_REPUTATION;
    }
    // Weighted average across 7 domains.
    // Weights are configurable per insight kind.
    let total_weight: f64 = self.weights.values().sum();
    let weighted_sum: f64 = self.weights.iter()
        .map(|(domain, weight)| {
            let score = scores.get(domain).copied().unwrap_or(COLD_START_REPUTATION);
            score * weight
        })
        .sum();
    weighted_sum / total_weight
}
```

### Cold Start

New agents receive `COLD_START_REPUTATION = 0.1` across all 7 domains.

- Low enough to heavily discount their insights (trust pipeline multiplies by 0.1 = 90% penalty)
- Nonzero so their knowledge *can* enter context windows at low priority
- An agent reaches neutral reputation (~0.5) after ~5 successful publications
  with 3+ confirmations each (~50-100 ticks of active participation)

---

## 4. Taint Propagation

### Taint Lattice

Taint is a **monotonic lattice** (one-directional state machine). Once
tainted, an insight can never become less tainted. This prevents adversaries
from "laundering" tainted knowledge through clean intermediaries.

```
           ┌──────────────────────────────┐
           │                              v
        CLEAN ──────> SUSPECT ──────> TAINTED
                         ^                ^
                         │                │
                    (dampening)     (direct taint
                    from TAINTED    or accumulation
                    parent)         from 2+ SUSPECT
                                    parents)

  Ordering: CLEAN < SUSPECT < TAINTED
  Transitions: strictly monotonic (forward only, never backward)
```

### Types

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaintLevel {
    Clean   = 0,
    Suspect = 1,
    Tainted = 2,
}
```

The `Ord` derive gives us the lattice ordering: `Clean < Suspect < Tainted`.
Monotonicity is enforced by the `<=` check in `propagate_taint`.

### Propagation Function

```rust
/// Propagate taint from a source insight to all derived insights.
///
/// "Derived" means: similarity > derivation_threshold AND published
/// after the source insight.
///
/// Rules:
///   - Monotonic: only upgrade taint, never downgrade
///   - Dampening: TAINTED parent -> SUSPECT child (not TAINTED)
///   - Accumulation: 2+ SUSPECT parents -> escalate child to TAINTED
///   - Recursive: bounded by MAX_PROPAGATION_DEPTH
pub fn propagate_taint(&mut self, insight_id: InsightId, new_level: TaintLevel) {
    self.propagate_taint_inner(insight_id, new_level, 0);
}

const MAX_PROPAGATION_DEPTH: usize = 10;

fn propagate_taint_inner(
    &mut self,
    insight_id: InsightId,
    new_level: TaintLevel,
    depth: usize,
) {
    if depth > MAX_PROPAGATION_DEPTH {
        return;
    }

    let insight = match self.insights.get_mut(&insight_id) {
        Some(i) => i,
        None => return,
    };

    // Monotonic: only upgrade, never downgrade
    if new_level <= insight.taint_level {
        return;
    }
    insight.taint_level = new_level;

    // Find all derived insights:
    //   similarity(derived.vector, source.vector) > derivation_threshold
    //   AND derived.publish_block > source.publish_block
    let derived_ids = self.find_derived_insights(insight_id);

    for derived_id in derived_ids {
        // Dampening: TAINTED parent produces SUSPECT child, not TAINTED
        let dampened_level = match new_level {
            TaintLevel::Tainted => TaintLevel::Suspect,
            TaintLevel::Suspect => TaintLevel::Suspect,
            TaintLevel::Clean   => unreachable!("propagation only triggers on Suspect or Tainted"),
        };

        // Apply dampened taint
        self.propagate_taint_inner(derived_id, dampened_level, depth + 1);

        // Check accumulation: 2+ suspect parents -> escalate to TAINTED
        let effective = self.compute_effective_taint(derived_id);
        if effective > self.insights[&derived_id].taint_level {
            self.propagate_taint_inner(derived_id, effective, depth + 1);
        }
    }
}
```

### Effective Taint (Accumulation Check)

```rust
/// Compute effective taint considering parent accumulation.
///
/// If an insight has 2+ parents at SUSPECT or above, it escalates to TAINTED.
/// This prevents an adversary from distributing taint across many
/// slightly-suspect parents to avoid detection.
pub fn compute_effective_taint(&self, insight_id: InsightId) -> TaintLevel {
    let direct_taint = self.insights[&insight_id].taint_level;
    let parent_taints: Vec<TaintLevel> = self.find_parents(insight_id)
        .iter()
        .map(|p| self.insights[p].taint_level)
        .collect();

    let suspect_or_above = parent_taints.iter()
        .filter(|t| **t >= TaintLevel::Suspect)
        .count();

    if suspect_or_above >= 2 || direct_taint == TaintLevel::Tainted {
        TaintLevel::Tainted
    } else if suspect_or_above >= 1 || direct_taint == TaintLevel::Suspect {
        TaintLevel::Suspect
    } else {
        TaintLevel::Clean
    }
}
```

### Helper: Find Derived Insights

```rust
/// Find insights derived from a source.
///
/// "Derived" = HDC similarity above derivation_threshold AND published
/// after the source. Uses the HDC index for efficient neighbor search.
fn find_derived_insights(&self, source_id: InsightId) -> Vec<InsightId> {
    let source = &self.insights[&source_id];
    self.hdc_index
        .search_within_distance(&source.vector, self.derivation_threshold)
        .into_iter()
        .filter(|id| {
            let candidate = &self.insights[id];
            candidate.publish_block > source.publish_block && *id != source_id
        })
        .collect()
}

/// Find parents of an insight (insights it was derived from).
///
/// "Parent" = HDC similarity above derivation_threshold AND published
/// before this insight.
fn find_parents(&self, insight_id: InsightId) -> Vec<InsightId> {
    let insight = &self.insights[&insight_id];
    self.hdc_index
        .search_within_distance(&insight.vector, self.derivation_threshold)
        .into_iter()
        .filter(|id| {
            let candidate = &self.insights[id];
            candidate.publish_block < insight.publish_block && *id != insight_id
        })
        .collect()
}
```

---

## 5. 5-Layer Immune System

The immune system defends the shared substrate against adversarial knowledge.
It assumes up to `f < n/3` adversarial agents (standard BFT threshold).

### Layer 1: Taint Propagation

See [Section 4](#4-taint-propagation) above.

**Summary:** Monotonic lattice (CLEAN < SUSPECT < TAINTED). Propagates through
derived insights. Dampening prevents a single tainted insight from poisoning
the entire graph. Accumulation (2+ suspect parents) escalates to TAINTED.
Irreversible by design.

### Layer 2: Anomaly Detection

Monitors aggregate submission patterns for behavioral anomalies. Does NOT
inspect insight content — that is the role of confirmation and the trust pipeline.

```rust
/// Configuration for anomaly detection thresholds.
pub struct AnomalyConfig {
    /// z-score threshold for publication rate spikes.
    pub rate_spike_z_threshold: f64,     // default: 3.0
    /// Minimum interval (in blocks) between publications before flagging.
    pub min_publication_interval: u64,   // for temporal clustering
    /// DBSCAN epsilon for Sybil vector clustering (Hamming distance).
    pub sybil_cluster_epsilon: u32,
    /// DBSCAN min_points for Sybil clustering.
    pub sybil_cluster_min_points: usize,
    /// Confirmation density ratio for ring detection.
    pub ring_density_threshold: f64,
}

pub struct AnomalyDetector {
    config: AnomalyConfig,
}

/// Flags raised by anomaly detection.
#[derive(Debug)]
pub enum AnomalyFlag {
    /// Publication rate z-score exceeds threshold.
    RateSpike { agent: Address, z_score: f64 },
    /// Multiple insights published in rapid succession.
    TemporalClustering { agent: Address, count: u32, blocks: u64 },
    /// Cluster of low-reputation agents with high vector similarity.
    SybilCluster { agents: Vec<Address>, cluster_size: usize },
    /// Clique of agents with high internal / low external confirmation density.
    ConfirmationRing { agents: Vec<Address>, internal_density: f64 },
}

impl AnomalyDetector {
    /// Run all anomaly checks against current substrate state.
    /// Returns a list of flags to escalate to Layer 3 (quarantine).
    pub fn scan(&self, state: &SubstrateState) -> Vec<AnomalyFlag> {
        let mut flags = Vec::new();
        flags.extend(self.detect_rate_spikes(state));
        flags.extend(self.detect_temporal_clustering(state));
        flags.extend(self.detect_sybil_clusters(state));
        flags.extend(self.detect_confirmation_rings(state));
        flags
    }

    /// Publication rate spikes: z-score > 3.0 against agent's historical mean.
    fn detect_rate_spikes(&self, state: &SubstrateState) -> Vec<AnomalyFlag> {
        let mut flags = Vec::new();
        for (agent, history) in &state.publication_histories {
            let mean = history.mean_rate();
            let stddev = history.stddev_rate();
            if stddev == 0.0 { continue; }
            let current_rate = history.current_rate();
            let z = (current_rate - mean) / stddev;
            if z > self.config.rate_spike_z_threshold {
                flags.push(AnomalyFlag::RateSpike { agent: *agent, z_score: z });
            }
        }
        flags
    }

    /// Temporal clustering: many insights in very few blocks from one agent.
    fn detect_temporal_clustering(&self, state: &SubstrateState) -> Vec<AnomalyFlag> {
        // Check inter-publication intervals per agent.
        // Flag if N insights published within min_publication_interval blocks.
        todo!("implementation")
    }

    /// Sybil detection: DBSCAN on insight vectors cross-referenced with agent identity.
    /// Multiple "different" agents publishing vectors within epsilon distance.
    fn detect_sybil_clusters(&self, state: &SubstrateState) -> Vec<AnomalyFlag> {
        // Run DBSCAN on recently published insight vectors.
        // Group by cluster. For clusters where vectors are within epsilon,
        // check if the publishing agents are all low-reputation.
        // If a cluster has min_points+ agents and all are low-rep, flag.
        todo!("implementation")
    }

    /// Confirmation rings: cliques with high internal, low external confirmation density.
    fn detect_confirmation_rings(&self, state: &SubstrateState) -> Vec<AnomalyFlag> {
        // Build confirmation graph (agent -> agent edges).
        // Detect cliques with high internal/external density ratio.
        // Flag cliques exceeding ring_density_threshold.
        todo!("implementation")
    }
}
```

### Layer 3: Quarantine

Holds unproven knowledge in a visible-but-untrusted state. Insights are NOT
rejected — they are visible in search but heavily penalized by the trust
pipeline and cannot be cited as parents.

```rust
#[derive(Debug, PartialEq)]
pub enum QuarantineStatus {
    /// Agent is established (rep > 0.3, 10+ publications, no slashes).
    /// No quarantine required.
    Exempt,
    /// Quarantine active. Must accumulate `required` confirmations from
    /// agents with reputation > 0.5 before release.
    Active { required: u32 },
    /// Confirmation threshold met. Insight released to active pool.
    Released,
}

/// Determine quarantine status for a given insight.
///
/// Confirmation thresholds by source quality:
///   - Previously slashed agent: 10 confirmations
///   - Anomaly-flagged agent: 5 confirmations
///   - New agent (< 10 publications) or low-rep (< 0.3): 3 confirmations
///   - Established, clean agent: exempt (known gap, see Security Notes)
pub fn quarantine_check(&self, insight: &InsightAnchor) -> QuarantineStatus {
    let author_rep = self.reputation_registry.composite_score(insight.author);
    let history = self.reputation_registry.history(insight.author);

    let required = if history.prior_slashes > 0 {
        10
    } else if history.anomaly_flags > 0 {
        5
    } else if history.total_publications < 10 {
        3
    } else if author_rep < 0.3 {
        3
    } else {
        0  // Exempt — KNOWN GAP (see Section 6)
    };

    if required == 0 {
        QuarantineStatus::Exempt
    } else if insight.confirmations >= required {
        QuarantineStatus::Released
    } else {
        QuarantineStatus::Active { required }
    }
}
```

**Quarantine properties:**
- Visible in search results (agents can find it)
- Marked with `QUARANTINED` flag — trust pipeline penalizes heavily
- Cannot be cited as a parent (prevents taint laundering through quarantined intermediaries)
- Release requires N confirmations from agents with reputation > 0.5

### Layer 4: Incident Response

When confirmed knowledge is proven false, contain the damage.

```rust
/// Slash severity levels.
#[derive(Debug, Clone, Copy)]
pub enum SlashSeverity {
    /// Insight was reasonable but wrong. 50% stake slash.
    HonestMistake,
    /// Insight was poorly supported. 75% stake slash.
    Negligence,
    /// Provably adversarial. 100% stake slash + reputation zeroed.
    Malice,
}

/// Result of an incident response action.
pub struct IncidentReport {
    pub slashed_insight: InsightId,
    pub severity: SlashSeverity,
    pub stake_slashed: U256,
    pub tainted_insights: Vec<InsightId>,
    pub notified_consumers: Vec<Address>,
    pub confirmer_penalties: Vec<(Address, f64)>,
}

impl IncidentResponse {
    /// Execute the full incident response pipeline.
    ///
    /// Steps:
    ///   1. Slash publisher stake (percentage depends on severity)
    ///   2. Recursively taint derived insights (Layer 1)
    ///   3. Notify consumers via KnowledgeRetracted events
    ///   4. Penalize confirmers' Accuracy and Reliability domains
    pub fn respond(&mut self, insight_id: InsightId, severity: SlashSeverity) -> IncidentReport {
        // 1. Slash publisher stake
        let slash_pct = match severity {
            SlashSeverity::HonestMistake => 0.50,
            SlashSeverity::Negligence    => 0.75,
            SlashSeverity::Malice        => 1.00,
        };
        let insight = &self.insights[&insight_id];
        let slash_amount = multiply_u256_f64(insight.staked_amount, slash_pct);
        self.insight_board.slash(insight_id, slash_amount);

        // If malice: zero publisher reputation across all domains
        if matches!(severity, SlashSeverity::Malice) {
            self.reputation_registry.zero_all(insight.author);
        }

        // 2. Taint derived insights (delegates to Layer 1)
        self.propagate_taint(insight_id, TaintLevel::Tainted);
        let tainted = self.collect_tainted_descendants(insight_id);

        // 3. Notify consumers
        let consumers = self.query_log.consumers_of(insight_id);
        for consumer in &consumers {
            self.emit_event(KnowledgeRetracted {
                insight_id,
                severity,
                consumer: *consumer,
            });
        }

        // 4. Penalize confirmers
        //    Early confirmers bear more responsibility.
        let confirmers = self.insight_board.confirmers(insight_id);
        let mut confirmer_penalties = Vec::new();
        for (i, confirmer) in confirmers.iter().enumerate() {
            // Earlier confirmers get larger penalty
            let penalty = 0.1 + 0.05 * (confirmers.len() - i) as f64;
            let penalty = penalty.min(0.5); // cap at 0.5
            self.reputation_registry.penalize(
                *confirmer,
                ReputationDomain::Accuracy,
                penalty,
            );
            self.reputation_registry.penalize(
                *confirmer,
                ReputationDomain::Reliability,
                penalty * 0.5,
            );
            confirmer_penalties.push((*confirmer, penalty));
        }

        IncidentReport {
            slashed_insight: insight_id,
            severity,
            stake_slashed: slash_amount,
            tainted_insights: tainted,
            notified_consumers: consumers,
            confirmer_penalties,
        }
    }
}
```

**Severity classification criteria are underspecified** in the design. The
distinction between honest mistake, negligence, and malice requires off-chain
adjudication. Document this as a known gap (see Section 6).

### Layer 5: Immune Memory

HDC-based pattern storage of past attacks. Future insights are checked against
immune memory before entering the substrate.

```rust
/// Alert raised when a candidate vector matches a known attack pattern.
pub struct ImmuneAlert {
    pub pattern_index: usize,
    pub hamming: u32,             // consensus-safe raw distance
    pub similarity: f64,          // display only, NOT for on-chain branching
    pub recommendation: AlertAction,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AlertAction {
    Quarantine,
    Review,
    Monitor,
}

/// Default match threshold: 1,536 bits (similarity > 0.85).
/// At D=10,240 this is ~70 sigma above the random baseline.
const DEFAULT_IMMUNE_THRESHOLD: u32 = 1_536;

/// Maximum stored attack patterns before consolidation triggers.
const MAX_ATTACK_PATTERNS: usize = 500;

pub struct ImmuneMemory {
    attack_patterns: Vec<HdcVector>,
    match_threshold: u32,
}

impl ImmuneMemory {
    pub fn new() -> Self {
        Self {
            attack_patterns: Vec::new(),
            match_threshold: DEFAULT_IMMUNE_THRESHOLD,
        }
    }

    /// Check a candidate vector against known attack patterns.
    /// Returns Some(alert) if the candidate matches a known pattern.
    pub fn check(&self, candidate: &HdcVector) -> Option<ImmuneAlert> {
        for (i, pattern) in self.attack_patterns.iter().enumerate() {
            let distance = hamming_distance(candidate, pattern);
            if distance < self.match_threshold {
                return Some(ImmuneAlert {
                    pattern_index: i,
                    hamming: distance,
                    similarity: 1.0 - (distance as f64 / D as f64),
                    recommendation: AlertAction::Quarantine,
                });
            }
        }
        None
    }

    /// Learn a new attack pattern. Triggers consolidation if
    /// pattern count exceeds MAX_ATTACK_PATTERNS.
    pub fn learn(&mut self, attack_vector: HdcVector) {
        self.attack_patterns.push(attack_vector);
        if self.attack_patterns.len() > MAX_ATTACK_PATTERNS {
            self.consolidate_patterns();
        }
    }

    /// Consolidate similar attack patterns via HDC bundling.
    ///
    /// Clusters patterns by Hamming distance (single-linkage agglomerative).
    /// Clusters with 3+ members are bundled into a single generalized pattern.
    /// Smaller clusters are kept as-is.
    fn consolidate_patterns(&mut self) {
        let clusters = cluster_by_distance(&self.attack_patterns, self.match_threshold);
        let mut new_patterns = Vec::new();
        for cluster in clusters {
            if cluster.len() >= 3 {
                // Bundle: element-wise majority vote captures commonalities,
                // averages out noise. Detects variants, not just exact replays.
                new_patterns.push(bundle_vectors(&cluster));
            } else {
                new_patterns.extend(cluster);
            }
        }
        self.attack_patterns = new_patterns;
    }
}
```

**Clustering helper** (single-linkage agglomerative via union-find):

```rust
/// Single-linkage agglomerative clustering by Hamming distance.
/// O(N^2) — suitable for immune memory's typical size (< 500 patterns).
fn cluster_by_distance(vectors: &[HdcVector], threshold: u32) -> Vec<Vec<HdcVector>> {
    let n = vectors.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root { root = parent[root]; }
        // Path compression
        let mut cur = i;
        while cur != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }

    for i in 0..n {
        for j in (i + 1)..n {
            if hamming_distance(&vectors[i], &vectors[j]) < threshold {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<HdcVector>> = HashMap::new();
    for i in 0..n {
        groups.entry(find(&mut parent, i)).or_default().push(vectors[i].clone());
    }
    groups.into_values().collect()
}

/// Bundle multiple vectors via element-wise majority vote.
fn bundle_vectors(vectors: &[HdcVector]) -> HdcVector {
    let mut acc = BundleAccumulator::new();
    for v in vectors {
        acc.add(v);
    }
    acc.finalize()
}
```

---

## 6. Known Security Gaps

These are documented in the design and are **not yet resolved**. Any
implementation must preserve them as explicit `// KNOWN GAP` comments so
they are not silently forgotten.

### Gap 1: Patient Reputation-Building (Long-Con Sybil)

Established agents (reputation > 0.3, 10+ publications, no slashes) bypass
quarantine entirely. An attacker can build a clean identity over weeks by
publishing genuine, easily-confirmed insights, then inject a single poisoned
entry that bypasses quarantine.

AGENTPOISON achieves >80% attack success rate with <0.1% poison rate. A
patient attacker needs only 1-2 poisoned entries after establishing trust.

**Possible mitigations (not yet designed):**
- No full quarantine exemption (require at least 1 confirmation for high-impact kinds)
- Behavioral discontinuity detection (content drift from historical centroid)
- Stake scaling by reputation age (newer reputations require higher stake)

### Gap 2: WisdomGate Has No Content Inspection (Prompt Injection)

The WisdomGate checks trust, taint, anti-knowledge, relevance, and diversity
but does NOT inspect the content of insights. An attacker can craft an insight
whose HDC vector is benign (passes all five gates) but whose text contains
prompt injection payloads.

**Possible mitigations (not yet designed):**
- Content sanitization gate (regex heuristic or classifier)
- Content sandboxing (lower-privilege prompt section)
- Content-vector consistency check (re-encode text, compare to stored vector)

### Gap 3: No Commit-Reveal for Insight Submission (Front-Running)

Insight submissions are visible in the mempool before inclusion. A
front-runner can see a valuable insight, copy the vector, and submit their
own version with higher gas to get included first, stealing credit and
timeliness reputation.

### Gap 4: No Eclipse Attack Protection

No mechanism prevents an adversary from surrounding a target agent with
Sybil nodes that control all its peers, filtering the set of insights
the target agent sees.

### Gap 5: Sparse Sybil Rings Evade DBSCAN

Sophisticated attackers can evade clique detection by constructing sparse
confirmation rings: Sybil agents that also confirm some legitimate insights
(adding noise to the graph) while preferentially confirming each other.
The detection threshold for "high internal density" is unspecified.

### Gap 6: Slash Severity Classification is Underspecified

The criteria for distinguishing honest mistake vs. negligence vs. malice
are not defined. This requires off-chain adjudication with no specified
process.

---

## 7. Anti-Patterns

**Do not make these mistakes:**

| Anti-Pattern | Why It Is Wrong | Correct Approach |
|---|---|---|
| Forget to clamp trust output | Floating-point drift can produce values > 1.0 or < 0.0 from chained multiplications, especially with `powf` and `ln` | Always `trust.clamp(0.0, 1.0)` as the final step |
| Use `f64` on-chain | Non-deterministic across platforms. Two validators computing the same trust pipeline will get different `f64` results, breaking consensus | All on-chain arithmetic must be fixed-point integers. `f64` is off-chain only |
| Make taint reversible | Allows adversaries to "launder" tainted knowledge through clean intermediaries. Defeats the entire purpose of taint tracking | Taint lattice is strictly monotonic. Enforce `new_level > current_level` guard |
| Skip dampening in taint propagation | A single tainted insight cascades TAINTED to every derived insight, which cascades to their descendants, eventually poisoning the entire graph | TAINTED parent produces SUSPECT child (one-level dampening). Accumulation (2+ suspect parents) is the only path from SUSPECT to TAINTED in children |
| Return `0.0` from Stage 1 for new agents | Multiplicative pipeline zeros out everything. New agents can never bootstrap into the system | Use `COLD_START_REPUTATION = 0.1` floor |
| Exempt established agents from ALL checks | Patient reputation-building attack. Attacker builds clean history then injects poison that bypasses every defense | At minimum, Layer 2 anomaly detection must still run on established agents. Consider requiring at least 1 confirmation for high-impact insight kinds |
| Use `HashMap` iteration order in consensus path | Iteration order is non-deterministic in Rust's `HashMap`. Taint propagation order must be deterministic for consensus | Use `BTreeMap` or sort before iterating in any consensus-critical path. (The trust pipeline is off-chain so `HashMap` is acceptable there) |

---

## 8. Checklist

### Trust Pipeline

- [ ] Define `InsightAnchor` struct with all fields from InsightBoard
- [ ] Define `TaskContext` struct with query vector and current block
- [ ] Define `InsightTier` enum with half-life values
- [ ] Implement `tier_half_life()` returning block counts per tier
- [ ] Implement `compute_trust()` with all 5 stages
- [ ] Stage 1: read composite score from ReputationRegistry, floor at 0.1
- [ ] Stage 2: exponential decay with tier-adjusted half-life
- [ ] Stage 3: linear confirmation boost (0.5 base + 0.05 per conf, cap 1.0)
- [ ] Stage 4: log-scaled stake weight (0.5 base + ln(ratio)/10, cap 1.0)
- [ ] Stage 5: Hamming similarity between context vectors
- [ ] Final clamp to `[0.0, 1.0]`
- [ ] Implement WisdomGate threshold checks (MIN_TRUST: 0.3, MAX_TAINT: 0.2, MIN_RELEVANCE: 0.55)

### Reputation System

- [ ] Define `ReputationDomain` enum (7 domains)
- [ ] Define `ReputationScores` struct
- [ ] Implement `ema_update()` with alpha = 0.1
- [ ] Implement `composite_score()` with weighted average
- [ ] Handle cold start (return 0.1 for unknown agents)
- [ ] Implement per-domain weight configuration by insight kind

### Taint Propagation

- [ ] Define `TaintLevel` enum with `Ord` derive
- [ ] Implement `propagate_taint()` with monotonic guard
- [ ] Implement dampening (TAINTED parent -> SUSPECT child)
- [ ] Implement accumulation (2+ SUSPECT parents -> TAINTED)
- [ ] Implement `compute_effective_taint()`
- [ ] Implement `find_derived_insights()` (similarity + temporal filter)
- [ ] Implement `find_parents()` (similarity + temporal filter)
- [ ] Add `MAX_PROPAGATION_DEPTH` bound to prevent unbounded recursion

### Immune System

- [ ] **Layer 2:** Implement `AnomalyDetector` with 4 detection methods
- [ ] Rate spike detection (z-score > 3.0)
- [ ] Temporal clustering detection
- [ ] Sybil cluster detection (DBSCAN on vectors)
- [ ] Confirmation ring detection (graph clique analysis)
- [ ] **Layer 3:** Implement `quarantine_check()` with tiered confirmation thresholds
- [ ] **Layer 4:** Implement incident response (slash, taint, notify, penalize confirmers)
- [ ] Define `SlashSeverity` enum with percentage mapping
- [ ] **Layer 5:** Implement `ImmuneMemory` struct
- [ ] Implement `check()` — scan candidate against known patterns
- [ ] Implement `learn()` — add new pattern, trigger consolidation
- [ ] Implement `consolidate_patterns()` — union-find clustering + bundling
- [ ] Implement `cluster_by_distance()` — single-linkage agglomerative
- [ ] Implement `bundle_vectors()` — element-wise majority vote

### Security Gaps

- [ ] Add `// KNOWN GAP: patient reputation-building` comment on quarantine exemption
- [ ] Add `// KNOWN GAP: no content inspection` comment on WisdomGate
- [ ] Add `// KNOWN GAP: no commit-reveal` comment on insight submission
- [ ] Add `// KNOWN GAP: no eclipse protection` comment in anomaly detection
- [ ] Add `// KNOWN GAP: sparse rings evade DBSCAN` comment in ring detection
- [ ] Add `// KNOWN GAP: severity criteria underspecified` comment on slash levels

---

## 9. Test Plan

### Unit Tests

**Trust Pipeline:**

| Test | Input | Expected Output |
|------|-------|----------------|
| `test_new_agent_minimum_trust` | Agent with no history, 0 confs, MIN_STAKE, max distance | Trust near `0.1 * decay * 0.5 * 0.5 * low_relevance` (very low but nonzero) |
| `test_perfect_trust` | Rep 1.0, age 0, 10 confs, 10x stake, distance 0.0 | Trust = 1.0 (clamped) |
| `test_each_stage_independent` | Vary one stage at a time, hold others constant | Each stage's multiplier is independent and within documented range |
| `test_freshness_decay_by_tier` | Same age, different tiers | EPHEMERAL decays fastest, ARCHIVAL slowest |
| `test_confirmation_cap` | 20 confirmations | `(0.5 + 0.05*20) = 1.5` capped to 1.0 |
| `test_stake_at_min` | Staked at exactly MIN_STAKE | `ln(1)/10 = 0`, factor = 0.5 |
| `test_stake_log_scaling` | Staked at 2x, 4x, 8x MIN_STAKE | Sublinear growth: 2x gives ~0.569, 4x gives ~0.639 |
| `test_zero_relevance` | Context distance = 1.0 | Trust = 0.0 (correct — maximally irrelevant) |
| `test_clamp_prevents_overflow` | Contrived inputs that produce trust > 1.0 before clamp | Final output is exactly 1.0 |

**Reputation:**

| Test | Input | Expected Output |
|------|-------|----------------|
| `test_cold_start_score` | Unknown agent | `composite_score() == 0.1` |
| `test_ema_decay_inactive` | 10 cycles, no observations | Score = `0.9^10 * initial` |
| `test_ema_update_with_observation` | observation = 1.0 on initial 0.1 | `0.1 * 1.0 + 0.9 * 0.1 = 0.19` |
| `test_ema_convergence` | Repeated observation = 0.8 | Score converges toward 0.8 |

**Taint:**

| Test | Input | Expected Output |
|------|-------|----------------|
| `test_monotonic_upgrade` | CLEAN -> SUSPECT | Succeeds, level = SUSPECT |
| `test_monotonic_no_downgrade` | SUSPECT -> CLEAN | No-op, level stays SUSPECT |
| `test_dampening` | Taint TAINTED on parent, check child | Child becomes SUSPECT (not TAINTED) |
| `test_accumulation` | 2 SUSPECT parents | Child escalates to TAINTED |
| `test_single_suspect_parent` | 1 SUSPECT parent | Child becomes SUSPECT (no escalation) |
| `test_depth_bound` | Chain of 15 derived insights | Propagation stops at `MAX_PROPAGATION_DEPTH` |

**Immune Memory:**

| Test | Input | Expected Output |
|------|-------|----------------|
| `test_check_no_patterns` | Empty memory, any candidate | Returns `None` |
| `test_check_exact_match` | Learn pattern X, check X | Returns `Some(alert)` with distance 0 |
| `test_check_near_match` | Learn X, check X with 1000 flipped bits | Returns `Some` (within threshold of 1536) |
| `test_check_no_match` | Learn X, check random Y | Returns `None` (distance ~5120, well above threshold) |
| `test_consolidation` | Learn 501 similar patterns | Triggers `consolidate_patterns`, reduces count |
| `test_bundle_detects_variant` | Learn 3 attack variants, consolidate, check 4th variant | Bundled pattern matches the unseen variant |

### Integration Tests

| Test | Description |
|------|-------------|
| `test_full_pipeline_with_quarantine` | Submit insight from new agent -> check quarantine_check returns Active{3} -> add 3 confirmations -> check returns Released -> run trust pipeline -> verify score reflects quarantine release |
| `test_incident_response_end_to_end` | Publish insight -> confirm it -> slash it as Malice -> verify: stake gone, reputation zeroed, derived insights tainted, consumers notified |
| `test_taint_propagation_chain` | Publish A -> publish B derived from A -> publish C derived from B -> taint A as TAINTED -> verify B is SUSPECT, C is SUSPECT (dampened twice) |
| `test_immune_memory_learns_from_incident` | Slash an insight -> verify its vector is learned by immune memory -> submit similar vector -> verify immune check triggers alert |
| `test_anomaly_flag_escalates_to_quarantine` | Agent publishes at z-score > 3.0 -> anomaly detector flags -> next insight from that agent requires 5 confirmations |

---

## Audit Findings

Audit performed 2026-05-08. Compared spec (this document, sections 1-9) against
implementation files. Two separate implementation sites exist:

- **Site A** (`trust.rs` in `crates/hdc/core/src/`): `TrustRegistry`, `TrustPipeline`
  (immune system layers), `TaintTracker`.
- **Site B** (`knowledge/` submodules in `crates/hdc/core/src/knowledge/`): `compute_trust`
  (5-stage pipeline), `compute_score` (4-factor retrieval scoring), `compute_decayed_balance`
  (decay math), anti-knowledge (`encode_anti`, `is_anti`).

### F01 — Trust Computation Correctness

#### Stage 1: Author Reputation

- **Spec:** `trust *= self.reputation_registry.composite_score(insight.author)` --
  weighted average across 7 `ReputationDomain` domains, floor `0.1`.
- **Impl (`knowledge/trust.rs:32`):** `let rep = source_reputation.max(COLD_START_REPUTATION);`
  -- correct floor, but the reputation is received as a bare `f64` parameter. There is
  **no `ReputationDomain` enum, no 7-domain composite calculation, no per-domain
  weighting, no `ReputationScores` struct** anywhere in the codebase. The reputation
  system from spec section 3 is entirely unimplemented.
- **Severity:** HIGH. The trust pipeline's first stage has no real backing data.

#### Stage 2: Freshness Decay

- **Spec:** `0.5_f64.powf(age as f64 / half_life as f64)` -- half-life decay using
  `0.5^(age/hl)`.
- **Impl (`knowledge/trust.rs:39`):** `(-0.693 * age_hours / effective_hl).exp()` --
  mathematically equivalent (`e^(-ln2 * t/hl) = 0.5^(t/hl)`). **Correct.**
- **Spec uses blocks** for age units; **impl uses hours** (via `ticks_to_hours`).
  This is a unit-system divergence, not a bug, since the spec's `InsightTier`
  half-lives are in blocks while the impl's `KnowledgeKind` half-lives are in hours.
  The mapping is consistent within each system.
- **Hardcoded `0.693`:** should be `(2.0_f64).ln()` for clarity and precision
  (`ln(2) = 0.6931471805599453`). Truncation to `0.693` introduces a relative error
  of ~0.02%, which is negligible for off-chain `f64` use but sloppy.
  File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, line 39.

#### Stage 3: Confirmation Boost

- **Spec:** `(0.5 + 0.05 * confirmations).min(1.0)` -- floor 0.5, cap 1.0.
- **Impl (`knowledge/trust.rs:43`):** `(0.5 + 0.05 * entry.confirmations as f64).min(1.0)`
  -- **Correct.** Matches spec exactly.

#### Stage 4: Stake Weighting

- **Spec:** Logarithmic: `(0.5 + ln(stake_ratio).max(0.0) / 10.0).min(1.0)`.
  Reads actual staked amount from `InsightAnchor.staked_amount`. Floor 0.5.
- **Impl (`knowledge/trust.rs:47-48`):**
  ```rust
  let stake_weight = 0.5_f64;
  trust *= stake_weight.max(0.5).min(1.0);
  ```
  This is a **hardcoded 0.5 constant**. The `.max(0.5).min(1.0)` on a literal `0.5`
  is a no-op. The log-scaled stake weighting from the spec is **completely missing**.
  The comment says `(placeholder)` which is accurate, but the spec mandates a
  functional implementation.
- **Severity:** HIGH. Every entry is penalized by a flat 50% regardless of stake,
  making the trust pipeline insensitive to economic commitment.

#### Stage 5: Relevance Gating

- **Spec:** `1.0 - (raw_distance as f64 / D as f64)` -- raw normalized Hamming
  similarity, range `[0.0, 1.0]`.
- **Impl (`knowledge/trust.rs:51-53`):**
  ```rust
  let sim = 1.0 - (dist as f64 / D as f64);
  let relevance = ((sim - 0.5) * 2.0).max(0.0);
  ```
  The impl applies an **additional linear rescaling** `(sim - 0.5) * 2.0` that is
  not in the spec. This maps similarity from `[0.5, 1.0]` to `[0.0, 1.0]` and
  clamps everything below 0.5 similarity to 0.0.
- **Effect:** Far more aggressive gating than the spec. Any entry with similarity
  below 0.5 gets zero trust (killed). The spec would give it 0.5 trust from this
  stage. This is a **behavioral divergence** from the spec that changes the
  pipeline's selectivity significantly.
- **Severity:** MEDIUM. The rescaling may be intentional (stricter gating), but it
  is undocumented and deviates from the spec formula.

### F02 — Decay Model Accuracy

- **Spec (implicit):** Decay is `0.5^(age/hl)` with tier-adjusted half-life.
- **Impl (`knowledge/decay.rs:19-29`):**
  ```rust
  let lambda = (2.0_f64).ln() / effective_hl;
  let decayed = balance_at_t0 * (-lambda * elapsed_hours).exp();
  ```
  Mathematically correct: `e^(-ln2 * t/hl) = 0.5^(t/hl)`. Uses `(2.0_f64).ln()`
  rather than the hardcoded `0.693` in `trust.rs`. **Inconsistency between the two
  files**: `decay.rs` uses the precise constant, `trust.rs` uses the truncated one.
- **`ticks_to_hours` (`decay.rs:33-35`):** Correct arithmetic:
  `elapsed_ticks * tick_duration_ms / 3_600_000`.
- **GC threshold (`decay.rs:7`):** `0.01` -- matches spec's implicit lifecycle
  (entries below 0.01 are garbage collected or demoted).
- **Severity:** LOW. Decay math is correct. Only issue is the `0.693` vs
  `(2.0_f64).ln()` inconsistency between `trust.rs` and `decay.rs`.

### F03 — Anti-Knowledge Handling

- **Spec (section 5, Layer 5):** Immune memory stores attack patterns, checks
  candidates against them via Hamming distance < 1,536, supports consolidation
  via single-linkage agglomerative clustering + bundle vectors.
- **Impl (`knowledge/anti.rs`):** Implements a **different concept**: anti-knowledge
  subspace encoding via XOR binding with a fixed `ANTI_SUBSPACE` vector.
  - `encode_anti(v) = bind(v, ANTI_SUBSPACE)` -- XOR-based negation
  - `is_anti(candidate, knowledge) = similarity(unbind(candidate), knowledge) > 0.90`
  - Self-inverse: `anti(anti(X)) = X` (correct, since XOR is self-inverse)
  - Deterministic seed: `ANTI_SUBSPACE_SEED = 0xAE71_5B8C_0000_0001`

  This is **correct and well-designed** for its purpose (detecting contradictory
  knowledge), but it is **not the immune memory system** from the spec. The spec's
  `ImmuneMemory` (Layer 5) with attack pattern storage, consolidation, and
  `ImmuneAlert` is **not implemented**.

- **`ANTI_RESONANCE_THRESHOLD = 0.90`** (`anti.rs:19`): Reasonable threshold. With
  D=10,240, random vectors have similarity ~0.5 +/- ~0.005, so 0.90 is ~80 sigma
  above baseline. False positive rate is astronomically low.
- **Severity:** The anti-knowledge encoding is sound. The gap is that it covers only
  one aspect (contradiction detection) while the spec's immune memory (attack pattern
  matching) is entirely absent.

### F04 — Duplicate / Divergent Trust Infrastructure

Two separate trust systems exist that do not interoperate:

| Aspect | Site A (`trust.rs`) | Site B (`knowledge/trust.rs`) |
|--------|---------------------|-------------------------------|
| Location | `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs` | `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs` |
| Trust computation | `TrustPipeline::validate()` -- 5-layer immune system (accept/quarantine/reject) | `compute_trust()` -- 5-stage multiplicative pipeline |
| Score model | Binary gate: accept, quarantine, or reject | Continuous `f64` score in `[0.0, 1.0]` |
| Reputation | `TrustRegistry` with EMA-updated per-agent scores and `TrustOutcome` events | Bare `f64` parameter passed in |
| Taint | `TaintTracker` (binary tainted/clean `HashSet`) | Not addressed |
| Constants | `COLD_START_REPUTATION = 0.1`, `MIN_TRUST_THRESHOLD = 0.05` | Same constants, redeclared |

- Site A's `TrustPipeline` is called "5-layer immune system" but is actually a
  sequential gate chain, not the spec's 5-stage multiplicative trust pipeline.
- Site B's `compute_trust` implements the spec's 5-stage multiplicative pipeline
  but lacks the surrounding infrastructure (reputation registry, taint, quarantine).
- Neither site implements the full spec. Together they partially cover it, but they
  are not connected.
- **Severity:** HIGH. Two parallel, incompatible trust subsystems create confusion
  about which one is canonical and neither is complete.

### F05 — Scoring System (`knowledge/scoring.rs`)

The 4-factor scoring system is **not part of the trust pipeline spec** (section 2)
but is used for knowledge retrieval ranking. Observations:

- **Factor 4 (Trust):** `let trust = entry.balance;` (`scoring.rs:56`). Uses the
  entry's decay balance as a proxy for source trust. This is **not trust** -- it is a
  decay-adjusted balance. The comment says "use balance as proxy for source trust."
  This is a known shortcut documented inline but conflates two distinct concepts.
- **Weight distribution (0.35/0.25/0.25/0.15):** Hardcoded magic numbers with no
  configuration. The spec does not define these weights, so this is ad-hoc.
- **`RECENCY_DECAY = 100.0`** (`scoring.rs:8`): Uses ticks as the unit but the
  actual decay rate depends on tick duration, which is not factored in. An entry 100
  ticks old has recency `1/e` regardless of whether ticks are 400ms or 12s. This may
  be intentional (tick-local reasoning) but is inconsistent with `trust.rs` which
  converts ticks to hours.
- **`IMPORTANCE_NORMALIZER = 7.0`** (`scoring.rs:11`): Arbitrary magic number with
  no derivation or documentation.

### F06 — Security Concerns

1. **No `KNOWN GAP` comments anywhere in the codebase.** The spec (section 8,
   Security Gaps checklist) requires 6 specific `// KNOWN GAP: ...` comments.
   Zero are present. This means security gaps are silently invisible to developers
   reading the code.

2. **`TrustRegistry::update_trust` double-application bug** (`trust.rs:95-128`):
   The method computes an `outcome_score` by adding/subtracting from `old_trust`,
   then applies EMA: `alpha * outcome_score + (1 - alpha) * old_trust`. But
   `outcome_score` already incorporates `old_trust` (e.g., `old_trust + weight * 0.1`).
   So the EMA blends `old_trust` with a value that already contains `old_trust`.
   Mathematically: `new = 0.1 * (old + delta) + 0.9 * old = old + 0.1 * delta`.
   This means the EMA alpha has no smoothing effect on the direction of change --
   it merely scales the step size by `alpha`. The spec's EMA formula
   (`alpha * observation + (1-alpha) * old`) expects `observation` to be the raw
   metric (e.g., 1.0 for success, 0.0 for failure), not `old_trust + delta`.
   File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 103-127.
   **Severity:** MEDIUM. The trust updates work directionally (positive increases,
   negative decreases) but the dynamics are wrong. The EMA is not actually performing
   exponential smoothing.

3. **No input validation on `TrustOutcome` weights** (`trust.rs:104,109`):
   `TrustOutcome::Positive(weight)` and `TrustOutcome::Negative(weight)` accept
   arbitrary `f64` values. A caller can pass `Negative(f64::INFINITY)` or
   `Positive(f64::NAN)`, corrupting the trust score. No bounds checking or NaN
   guard exists.
   File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 28-37.

4. **`TaintTracker` is binary, not a lattice** (`trust.rs:238-265`): The spec
   defines a 3-level taint lattice (CLEAN < SUSPECT < TAINTED) with monotonic
   transitions, dampening, and accumulation. The implementation is a bare
   `HashSet<[u8; 32]>` with only two states: present (tainted) or absent (clean).
   No `TaintLevel` enum, no dampening, no accumulation, no monotonicity guard.
   File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 238-265.
   **Severity:** HIGH. The taint system cannot represent SUSPECT state, cannot
   dampen propagation, and has no accumulation logic. A single taint call cascades
   binary taint without attenuation.

5. **`ANTI_SUBSPACE_SEED` is consensus-critical but has no test for cross-platform
   determinism** (`anti.rs:9`): The comment says "Must never change after genesis"
   but the seed is fed to `HdcVector::random()` whose PRNG implementation
   determines the actual vector. If the PRNG changes (e.g., library update), all
   existing anti-knowledge entries become invalid. No integration test verifies
   that the produced vector matches a known-good byte sequence.

6. **`TrustPipeline` Layer 5 (Meta) is empty** (`trust.rs:228`): The comment says
   "cross-layer consistency" but the implementation just falls through to `Accept`
   after Layer 4. No cross-layer validation logic exists.

---

## Implementation Status

Status legend: DONE = matches spec, PARTIAL = partially implemented, STUB = placeholder,
MISSING = not implemented.

### Trust Pipeline (5 Stages)

| Item | Status | Location | Notes |
|------|--------|----------|-------|
| `InsightAnchor` struct | MISSING | -- | `TrustCandidate` in `trust.rs` is a simplified substitute with fewer fields |
| `TaskContext` struct | MISSING | -- | `RetrievalContext` in `scoring.rs` is a partial substitute |
| `InsightTier` enum | PARTIAL | `knowledge/tier.rs` | Implemented as `KnowledgeTier` with 4 tiers (not 3). Multiplier-based, not block-based half-lives |
| `tier_half_life()` | PARTIAL | `knowledge/kind.rs:25-34` | Defined on `KnowledgeKind` not `InsightTier`. Hours not blocks |
| `compute_trust()` | PARTIAL | `knowledge/trust.rs:22-57` | Stages 1,2,3 correct. Stage 4 stubbed. Stage 5 diverges from spec |
| Stage 1: Author reputation | PARTIAL | `knowledge/trust.rs:32-33` | Floor correct, but no composite score / 7-domain system |
| Stage 2: Freshness decay | DONE | `knowledge/trust.rs:36-40` | Math correct (uses `0.693` approximation) |
| Stage 3: Confirmation boost | DONE | `knowledge/trust.rs:43` | Exact match to spec |
| Stage 4: Stake weighting | STUB | `knowledge/trust.rs:47-48` | Hardcoded `0.5`, no log-scale formula |
| Stage 5: Relevance gating | PARTIAL | `knowledge/trust.rs:51-54` | Extra `(sim - 0.5) * 2.0` rescaling not in spec |
| Final clamp | DONE | `knowledge/trust.rs:56` | `.clamp(0.0, 1.0)` |
| WisdomGate thresholds | MISSING | -- | No MIN_TRUST/MAX_TAINT/MIN_RELEVANCE threshold checks in pipeline |

### Reputation System

| Item | Status | Location | Notes |
|------|--------|----------|-------|
| `ReputationDomain` enum (7 domains) | MISSING | -- | Not implemented |
| `ReputationScores` struct | MISSING | -- | Not implemented |
| `ema_update()` | PARTIAL | `trust.rs:95-128` | Exists but has double-application bug |
| `composite_score()` | MISSING | -- | Not implemented |
| Cold start handling | DONE | `trust.rs:14`, `knowledge/trust.rs:9` | `COLD_START_REPUTATION = 0.1` |
| Per-domain weight config | MISSING | -- | Not implemented |

### Taint Propagation

| Item | Status | Location | Notes |
|------|--------|----------|-------|
| `TaintLevel` enum (3 levels) | MISSING | -- | Binary `HashSet` instead |
| `propagate_taint()` with monotonic guard | MISSING | -- | `TaintTracker::propagate()` has no monotonicity |
| Dampening (TAINTED -> SUSPECT) | MISSING | -- | Binary taint, no dampening |
| Accumulation (2+ SUSPECT -> TAINTED) | MISSING | -- | No accumulation logic |
| `compute_effective_taint()` | MISSING | -- | Not implemented |
| `find_derived_insights()` | MISSING | -- | Not implemented |
| `find_parents()` | MISSING | -- | Not implemented |
| `MAX_PROPAGATION_DEPTH` | MISSING | -- | No depth bound |

### Immune System (5 Layers)

| Item | Status | Location | Notes |
|------|--------|----------|-------|
| Layer 1: Taint propagation | PARTIAL | `trust.rs:238-265` | Binary only, see above |
| Layer 2: Anomaly detection | MISSING | -- | No `AnomalyDetector`, no rate spike / Sybil / ring detection |
| Layer 3: Quarantine | PARTIAL | `trust.rs:216-220` | Basic reputation threshold gate, no tiered confirmation requirements |
| Layer 4: Incident response | MISSING | -- | No slash severity, no confirmer penalties, no `KnowledgeRetracted` events |
| Layer 5: Immune memory | MISSING | -- | No `ImmuneMemory`, no attack pattern storage, no consolidation |

### Anti-Knowledge

| Item | Status | Location | Notes |
|------|--------|----------|-------|
| `ANTI_SUBSPACE` vector | DONE | `knowledge/anti.rs:13-15` | Deterministic seed, `LazyLock` |
| `encode_anti()` | DONE | `knowledge/anti.rs:27-29` | XOR binding, self-inverse verified by test |
| `is_anti()` | DONE | `knowledge/anti.rs:35-40` | Unbind + similarity check, threshold 0.90 |

### Security Gap Comments

| Gap | Status | Notes |
|-----|--------|-------|
| Gap 1: Patient reputation-building | MISSING | No `// KNOWN GAP` comment |
| Gap 2: No content inspection | MISSING | No `// KNOWN GAP` comment |
| Gap 3: No commit-reveal | MISSING | No `// KNOWN GAP` comment |
| Gap 4: No eclipse protection | MISSING | No `// KNOWN GAP` comment |
| Gap 5: Sparse Sybil rings | MISSING | No `// KNOWN GAP` comment |
| Gap 6: Severity underspecified | MISSING | No `// KNOWN GAP` comment |

---

## Anti-Patterns & Duct Tape

### AP01 — Hardcoded `0.693` Instead of `ln(2)`

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, line 39.
**Code:** `(-0.693 * age_hours / effective_hl).exp()`
**Problem:** Magic number. `decay.rs:26` uses `(2.0_f64).ln()` correctly for the same
constant. This inconsistency between two files in the same module is a maintenance risk.
**Fix:** Replace `0.693` with `(2.0_f64).ln()` or define a shared `LN_2` constant.

### AP02 — Hardcoded Stake Placeholder Disguised as Logic

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, lines 47-48.
**Code:**
```rust
let stake_weight = 0.5_f64;
trust *= stake_weight.max(0.5).min(1.0);
```
**Problem:** `.max(0.5).min(1.0)` on a literal `0.5` is dead code -- it always
evaluates to `0.5`. This looks like it was copy-pasted from a template where
`stake_weight` would be computed dynamically. The no-op clamping creates the illusion
of bounds checking where none is needed.
**Fix:** Either implement the spec's log-scaled formula or simplify to `trust *= 0.5;`
with a clear `// TODO: implement log-scaled stake weighting per spec section 2`.

### AP03 — EMA Applied to Already-Shifted Value

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 103-127.
**Code:**
```rust
let outcome_score = match outcome {
    TrustOutcome::Positive(weight) => (old_trust + weight * 0.1).min(1.0),
    // ...
};
score.reputation = EMA_ALPHA * outcome_score + (1.0 - EMA_ALPHA) * old_trust;
```
**Problem:** `outcome_score` is computed as `old_trust + delta`, then EMA blends it
with `old_trust`. The spec's EMA expects a raw observation (independent of current
state). This produces: `0.1 * (old + delta) + 0.9 * old = old + 0.1 * delta`, which
is just additive stepping with a scaled step size, not exponential smoothing.
**Fix:** Make `outcome_score` a raw observation in `[0.0, 1.0]` independent of
`old_trust`. For `Positive(weight)`: `outcome_score = weight.min(1.0)`.
For `Negative(weight)`: `outcome_score = 0.0` (or `(1.0 - weight).max(0.0)`).

### AP04 — Duplicate Constants Across Files

**Files:**
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 14, 17
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, lines 9, 12

**Problem:** `COLD_START_REPUTATION` and `MIN_TRUST_THRESHOLD` are defined identically
in two files. If one is updated without the other, the system silently uses different
thresholds depending on which code path is invoked.
**Fix:** Define once in a shared `constants` module and re-export.

### AP05 — Binary Taint Tracker vs. Spec's 3-Level Lattice

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 238-265.
**Problem:** The spec requires a 3-level monotonic lattice (CLEAN/SUSPECT/TAINTED)
with dampening and accumulation rules. The implementation is a bare `HashSet<[u8; 32]>`
that only tracks "tainted or not". This is the simplest possible stub that cannot
support the spec's propagation semantics.
**Fix:** Replace `HashSet` with `HashMap<[u8; 32], TaintLevel>`. Implement the
`TaintLevel` enum with `Ord` derive. Add monotonic guard, dampening, and accumulation
logic per spec section 4.

### AP06 — `TrustPipeline::validate` Conflates Immune System with Trust Pipeline

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 180-231.
**Problem:** The struct is called `TrustPipeline` and the doc comment says "5-layer
cognitive immune system" but it is architecturally a sequential gate chain that
returns Accept/Quarantine/Reject. The spec separates:
1. Trust pipeline (5-stage multiplicative score) -- returns `f64`
2. WisdomGate (threshold checks) -- returns accept/reject
3. Immune system (5 defense layers) -- runs post-WisdomGate

The implementation merges all three into a single `validate()` method that conflates
size validation (innate immunity) with trust score checking (pipeline) with reputation
gating (social layer) with on-chain verification (consensus layer).
**Fix:** Separate into three distinct components matching the spec's data flow diagram.

### AP07 — `scoring.rs` Uses Balance as Trust Proxy

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/scoring.rs`, line 56.
**Code:** `let trust = entry.balance;`
**Problem:** Balance is a decay-adjusted energy level that starts at 1.0 and decreases
over time. It is not a trust score. A freshly created entry from an untrusted agent
has `balance = 1.0`, giving it maximum "trust" in the scoring function.
**Fix:** Accept a real trust score as a parameter (from `compute_trust` or the
`TrustRegistry`) rather than using balance as a proxy.

### AP08 — No NaN / Infinity Guards on Trust Computation

**Files:**
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 28-37 (TrustOutcome)
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, line 22 (compute_trust)
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/scoring.rs`, line 37 (compute_score)

**Problem:** All trust computation functions accept `f64` inputs with no validation.
`f64::NAN`, `f64::INFINITY`, or `f64::NEG_INFINITY` will silently propagate through
the pipeline and produce corrupt scores. The final `.clamp(0.0, 1.0)` does NOT catch
NaN (in Rust, `f64::NAN.clamp(0.0, 1.0)` returns `NaN`).
**Fix:** Add `debug_assert!` or runtime checks for `is_finite()` on all `f64`
inputs. Consider returning `Result<f64, TrustError>` instead of bare `f64`.

### AP09 — Stage 5 Relevance Rescaling Undocumented

**File:** `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, lines 51-53.
**Code:**
```rust
let sim = 1.0 - (dist as f64 / D as f64);
let relevance = ((sim - 0.5) * 2.0).max(0.0);
```
**Problem:** The `(sim - 0.5) * 2.0` transformation maps `[0.5, 1.0]` to `[0.0, 1.0]`
and kills everything below 0.5 similarity. The spec uses raw `1.0 - distance/D` with
no rescaling. This is a deliberate tightening of the relevance gate but is
undocumented in the code, the spec, or any comment.
**Fix:** Either document the rationale for the 0.5-threshold rescaling with a comment
explaining why random-baseline similarity (0.5 for binary HDC vectors) should map to
zero trust, or revert to the spec's raw formula.

---

## Recommended Changes Checklist

Priority: P0 = blocking, P1 = high, P2 = medium, P3 = low.

### P0 — Architectural

- [ ] **Unify the two trust systems.** Decide whether `trust.rs` (Site A) or
  `knowledge/trust.rs` (Site B) is canonical. Deprecate the other. Wire the winner
  into the actual data flow.
  Files: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`,
  `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`

- [ ] **Implement `TaintLevel` enum with 3-level lattice.** Replace
  `TaintTracker`'s `HashSet<[u8; 32]>` with `HashMap<[u8; 32], TaintLevel>`.
  Implement monotonic guard, dampening, and accumulation per spec section 4.
  File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 238-265.

### P1 — Correctness

- [ ] **Fix EMA double-application bug in `TrustRegistry::update_trust`.** Make
  `outcome_score` independent of `old_trust`.
  File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 103-127.

- [ ] **Implement log-scaled stake weighting** (Stage 4). Replace hardcoded `0.5`
  with `(0.5 + ln(stake_ratio).max(0.0) / 10.0).min(1.0)`.
  File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, lines 47-48.

- [ ] **Document or revert Stage 5 relevance rescaling.** The `(sim - 0.5) * 2.0`
  transformation deviates from spec. Add rationale comment or match spec formula.
  File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`, lines 51-53.

- [ ] **Add NaN/Infinity guards** to `compute_trust`, `compute_score`, and
  `TrustRegistry::update_trust`. At minimum, validate that all `f64` inputs are
  finite. Ensure output is also finite (NaN survives `.clamp()`).
  Files: `trust.rs:95`, `knowledge/trust.rs:22`, `knowledge/scoring.rs:37`.

- [ ] **Fix `scoring.rs` trust factor.** Replace `entry.balance` with an actual
  trust score parameter.
  File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/scoring.rs`, line 56.

### P2 — Spec Compliance

- [ ] **Implement 7-domain `ReputationDomain` enum and `ReputationScores` struct.**
  Wire `composite_score()` into the trust pipeline's Stage 1.

- [ ] **Implement quarantine with tiered confirmation thresholds** (3/5/10
  confirmations based on agent history). Current implementation has only a
  single-threshold reputation gate.
  File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`, lines 216-220.

- [ ] **Add all 6 `// KNOWN GAP` comments** per spec section 8 checklist.

- [ ] **Deduplicate constants.** Move `COLD_START_REPUTATION` and
  `MIN_TRUST_THRESHOLD` to a single location.

- [ ] **Replace `0.693` with `(2.0_f64).ln()`** in `knowledge/trust.rs:39` for
  consistency with `decay.rs`.

### P3 — Future Work

- [ ] Implement `AnomalyDetector` (Layer 2): rate spikes, temporal clustering,
  Sybil detection, confirmation rings.

- [ ] Implement `IncidentResponse` (Layer 4): slash severity, taint propagation
  from incidents, consumer notification, confirmer penalties.

- [ ] Implement `ImmuneMemory` (Layer 5): attack pattern storage, Hamming-based
  matching, single-linkage consolidation, vector bundling.

- [ ] Add cross-platform determinism test for `ANTI_SUBSPACE_SEED` -- verify the
  produced vector bytes against a pinned expected value.

- [ ] Add `WisdomGate` threshold integration into the trust pipeline (MIN_TRUST,
  MAX_TAINT, MIN_RELEVANCE gates per spec section 1).

---

## Second-Pass Remediation Detail

Second pass performed 2026-05-08 against:

- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/scoring.rs`
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/anti.rs`
- `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/store.rs`

### Target Unified Trust Architecture

The codebase needs one canonical trust stack. Today `trust.rs` owns a gate-style
`TrustPipeline` plus a binary `TaintTracker`, while `knowledge/trust.rs` owns the
5-stage multiplicative score. Remediation should make `crates/hdc/core/src/trust.rs`
the public facade and move the score implementation behind it, with
`knowledge/trust.rs` retained only as a compatibility wrapper during migration.

Canonical data flow:

```text
KnowledgeEntry or InsightAnchor
  + ChainMetadataSnapshot
  + ReputationSnapshot
  + RetrievalContext
        |
        v
TrustEngine::evaluate(...)
        |
        +--> TrustReport { stage factors, final_trust, taint, quarantine, decision }
        |
        v
KnowledgeStore ranking / WisdomGate / context selection
```

Concrete API shape:

```rust
pub struct TrustEngine {
    pub registry: TrustRegistry,
    pub taint: TaintRegistry,
    pub immune: ImmuneSystem,
    pub config: TrustConfig,
}

pub struct TrustEvaluation<'a> {
    pub entry: &'a KnowledgeEntry,
    pub author: AgentId,
    pub current_tick: u64,
    pub tick_duration_ms: u64,
    pub query_vector: &'a HdcVector,
    pub chain: Option<ChainMetadataSnapshot>,
    pub reputation: ReputationSnapshot,
}

pub struct ChainMetadataSnapshot {
    pub on_chain_verified: bool,
    pub confirmations: u32,
    pub staked_amount: U256,
    pub min_stake: U256,
    pub publish_tick: u64,
}

pub struct TrustReport {
    pub final_trust: f64,
    pub reputation_factor: f64,
    pub freshness_factor: f64,
    pub confirmation_factor: f64,
    pub stake_factor: f64,
    pub relevance_factor: f64,
    pub taint_level: TaintLevel,
    pub decision: TrustDecision,
    pub flags: Vec<TrustFlag>,
}
```

Rules:

- `TrustEngine::evaluate` is the only place that computes final trust.
- `knowledge/scoring.rs::compute_score` must receive a trust score or
  `TrustReport`; it must not infer trust from `entry.balance`.
- `TrustPipeline::validate` should become a thin WisdomGate wrapper over
  `TrustReport`, not a separate trust implementation.
- `knowledge/trust.rs::compute_trust` should call `TrustEngine` or a shared
  `compute_trust_factors` helper and be marked as legacy until all callers move.

### Stake Source and Stage 4

Stage 4 must not use `KnowledgeEntry::balance`. Balance is local retention energy,
starts at `1.0`, and decays over time; it is not economic stake or source trust.

Authoritative stake source:

- Shared substrate entries: read `staked_amount`, `min_stake`, confirmations, and
  publish tick from the on-chain `InsightBoard` snapshot used to build
  `ChainMetadataSnapshot`.
- Local/self-derived entries: no on-chain stake exists. Use a source-specific
  policy, not fake stake. Recommended default is `stake_factor = 1.0` for
  `KnowledgeSource::SelfDerived` and `stake_factor = 0.5` plus quarantine for
  external entries with missing chain metadata.
- Anonymous or unverifiable external entries: `stake_factor = 0.5` and
  `TrustDecision::Quarantine("missing stake snapshot")`.

Bounded formula:

```rust
const STAKE_FACTOR_FLOOR: f64 = 0.5;
const STAKE_LOG_DIVISOR: f64 = 10.0;
const STAKE_FULL_TRUST_LN_RATIO: f64 = 5.0; // factor reaches 1.0 at e^5 x min stake

fn stake_factor(staked: U256, min_stake: U256) -> Result<f64, TrustError> {
    if min_stake.is_zero() {
        return Err(TrustError::InvalidStakeConfig);
    }
    if staked < min_stake {
        return Err(TrustError::StakeBelowMinimum);
    }

    // Cap before f64 conversion so huge U256 values cannot become infinity.
    let full_trust_stake = min_stake.saturating_mul(U256::from(149_u64));
    let effective = staked.min(full_trust_stake);
    let ratio = u256_ratio_to_f64(effective, min_stake)?;
    finite_unit(STAKE_FACTOR_FLOOR + ratio.ln().max(0.0) / STAKE_LOG_DIVISOR)
}
```

The `149` cap is `ceil(e^5)`. Any larger stake already maps to `1.0`, so preserving
the exact larger value adds no scoring information and only increases overflow risk.

### Taint Lattice

Replace binary `TaintTracker { HashSet<[u8; 32]> }` with a 3-level lattice:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaintLevel {
    Clean = 0,
    Suspect = 1,
    Tainted = 2,
}
```

State ownership:

- `TaintRegistry` stores `HashMap<KnowledgeId, TaintRecord>`.
- Missing map entry means `Clean`; explicit records are used for `Suspect` and
  `Tainted`.
- Direct incident response and strong anti-knowledge contradictions can set
  `Tainted`.
- Anomaly flags, immune-memory near matches, moderate anti-knowledge resonance,
  and derivation from a tainted parent set at least `Suspect`.

Monotonic update rule:

```rust
fn raise_taint(&mut self, id: KnowledgeId, next: TaintLevel, cause: TaintCause) -> bool {
    let current = self.level(id);
    if next <= current {
        return false;
    }
    self.records.insert(id, TaintRecord { level: next, cause });
    true
}
```

Propagation rules:

| Input | Child Effect |
|---|---|
| Clean parent | No effect |
| Suspect parent | Child becomes Suspect |
| Tainted parent | Child becomes Suspect, not Tainted |
| Two or more Suspect-or-higher parents | Child becomes Tainted |
| Direct malice incident | Target becomes Tainted |

Implementation requirements:

- Propagation must be bounded by `MAX_PROPAGATION_DEPTH`.
- Derived-parent discovery must use HDC similarity plus temporal ordering:
  parent publish tick < child publish tick.
- Iterate derived IDs in sorted order for stable tests and reproducible local
  behavior.
- Quarantine release must not downgrade taint. If a false positive is adjudicated,
  record a separate `exonerated` or `released` field; do not mutate
  `Tainted -> Suspect` or `Suspect -> Clean`.

### Immune Layers

The remediated implementation should keep the five scoring stages separate from
the immune layers. Recommended order:

| Layer | Name | Input | Output |
|---|---|---|---|
| 0 | Innate validation | Payload size, vector shape, chain snapshot completeness, finite numbers | Reject malformed candidates |
| 1 | Taint and contradiction | `TaintRegistry`, `anti::is_anti`, anti-search from `KnowledgeStore` | Raise taint, quarantine/reject contradicted entries |
| 2 | Anomaly/social | publication rate, temporal clustering, vector clusters, confirmation graph | Raise flags; increase confirmation requirements |
| 3 | Economic/consensus | `InsightBoard` snapshot: stake, confirmations, verification status | Stake factor; reject below minimum stake; quarantine unverified |
| 4 | Meta immune memory | stored attack vectors, cross-layer consistency checks | Quarantine near matches; learn from incidents |

Decision policy:

- `Tainted` means reject for context admission.
- `Suspect` means quarantine unless an explicit review policy allows monitored use.
- Strong anti-knowledge resonance (`> ANTI_STRONG_THRESHOLD`, currently `0.90`) is
  equivalent to direct `Tainted` for the candidate pair.
- Moderate resonance (`> ANTI_MODERATE_THRESHOLD`, currently hardcoded as `0.70`
  in `KnowledgeStore::search_with_anti_check`) sets `Suspect` and halves confidence
  only through a named constant.
- Established reputation must never skip Layers 1, 2, or 4. It may reduce
  confirmation requirements, but it cannot bypass taint, anomaly, or immune-memory
  checks.

### NaN and Infinity Guards

Rust `f64::clamp(0.0, 1.0)` does not sanitize `NaN`; a `NaN` input can survive
to sorting and ranking. Add runtime guards at every boundary where `f64` enters
trust or scoring.

Required helpers:

```rust
fn finite_unit(value: f64) -> Result<f64, TrustError> {
    if !value.is_finite() {
        return Err(TrustError::NonFiniteFloat);
    }
    Ok(value.clamp(0.0, 1.0))
}

fn finite_nonnegative(value: f64) -> Result<f64, TrustError> {
    if !value.is_finite() || value < 0.0 {
        return Err(TrustError::InvalidFloat);
    }
    Ok(value)
}
```

Guard points:

- `TrustOutcome::Positive(weight)` and `TrustOutcome::Negative(weight)` before
  updating reputation.
- `source_reputation`, `entry.balance`, `entry.tier.weight()`, decay half-life,
  freshness, confirmation factor, stake ratio, relevance, anti similarity, and
  final trust.
- `tick_duration_ms == 0` should return `TrustError::InvalidTickDuration` rather
  than feeding invalid age math.
- `knowledge/scoring.rs` should return `Result<f64, ScoreError>` or drop invalid
  candidates before sort. Once finite scores are guaranteed, sort with
  `total_cmp`.

Fail-closed behavior:

- Non-finite trust input from external or chain-derived data: quarantine or reject.
- Non-finite local scoring artifact: score candidate as `0.0` only if the error is
  logged and the candidate is not admitted to context as trusted.
- Non-finite reputation update weight: reject the update; do not modify stored
  reputation.

### Constants Ownership

Deduplicate constants into one owner. Recommended owner:
`crates/hdc/core/src/constants.rs` for cross-module constants, with
`trust.rs` re-exporting only the trust-specific public API.

Move or centralize:

| Constant | Current Location | Owner |
|---|---|---|
| `COLD_START_REPUTATION` | duplicated in `trust.rs` and `knowledge/trust.rs` | `constants.rs` |
| `MIN_TRUST_THRESHOLD` | duplicated in `trust.rs` and `knowledge/trust.rs` | `constants.rs` |
| `EMA_ALPHA` | `trust.rs` | `constants.rs` or `TrustConfig` |
| `RECENCY_DECAY` | `knowledge/scoring.rs` | `TrustConfig` if policy, `constants.rs` if fixed |
| `IMPORTANCE_NORMALIZER` | `knowledge/scoring.rs` | `TrustConfig` |
| `ANTI_SUBSPACE_SEED` | `knowledge/anti.rs` | `constants.rs`, pinned by genesis/version |
| `ANTI_RESONANCE_THRESHOLD` | `knowledge/anti.rs` | `constants.rs` |
| moderate anti threshold `0.70` | hardcoded in `knowledge/store.rs` | `constants.rs` |
| strong anti threshold `0.90` | hardcoded in `knowledge/store.rs` and `anti.rs` | `constants.rs` |
| `0.693` | `knowledge/trust.rs` | replace with `(2.0_f64).ln()` or `LN_2` |
| stage weights `0.35/0.25/0.25/0.15` | `knowledge/scoring.rs` | `TrustConfig` |
| `quarantine_threshold = 0.3` | `trust.rs` | `TrustConfig` |
| `max_payload_size = 1_048_576` | `trust.rs` | `TrustConfig` |

`ANTI_SUBSPACE_SEED` is special: moving it must not change the generated vector.
Add a pinned hash or byte-prefix test for `ANTI_SUBSPACE` before moving it.

### Migration Plan

1. Add `TrustConfig`, `TrustError`, `TrustReport`, `ChainMetadataSnapshot`, and
   finite guard helpers. Keep existing public functions compiling.
2. Centralize constants and re-export legacy names so downstream code does not
   break in the same patch.
3. Extract the 5-stage math from `knowledge/trust.rs` into the canonical trust
   facade. Make the legacy `knowledge::compute_trust` call the shared helper.
4. Replace `knowledge/scoring.rs` trust proxy with an explicit `source_trust`
   or `TrustReport` input. Preserve the old `search` method temporarily by
   passing cold-start trust and documenting it as compatibility behavior.
5. Add `ChainMetadataSnapshot` plumbing for shared substrate entries. Do not use
   `KnowledgeEntry::balance` as stake. External entries without a stake snapshot
   must quarantine.
6. Replace `TaintTracker` with `TaintRegistry` and migrate old state:
   previously tainted IDs become `Tainted`; missing IDs remain `Clean`.
7. Wire `KnowledgeStore::search_with_anti_check` to constants and `TaintRegistry`
   so strong contradictions reject and moderate contradictions produce `Suspect`.
8. Implement quarantine thresholds with no full bypass for established agents on
   high-impact entries.
9. Add immune memory as an independent Layer 4 component and teach it from
   incident response. Keep attack-pattern matching off-chain and deterministic
   for tests.
10. Remove or deprecate the old `TrustPipeline::validate` semantics once callers
    consume `TrustReport.decision`.

### Required Tests

Unit tests:

- `trust_rejects_nan_reputation`: `source_reputation = f64::NAN` returns an error
  or quarantine; no `NaN` final trust.
- `trust_rejects_infinite_outcome_weight`: `TrustOutcome::Positive(INFINITY)` and
  `Negative(NAN)` leave stored reputation unchanged.
- `trust_rejects_zero_tick_duration`: `tick_duration_ms = 0` fails closed.
- `stake_factor_at_minimum`: `staked_amount == min_stake` returns `0.5`.
- `stake_factor_log_scaled`: `2x`, `4x`, and `8x` min stake produce increasing
  sublinear factors.
- `stake_factor_caps_before_float_conversion`: huge `U256` stake returns finite
  `1.0`.
- `stake_below_minimum_rejects`: shared substrate entry below min stake is rejected
  or quarantined before scoring.
- `relevance_policy_is_explicit`: random-baseline HDC similarity behavior is tested
  for whichever formula is chosen, raw similarity or `(sim - 0.5) * 2.0`.
- `score_uses_real_trust`: two entries with equal balance but different trust rank
  according to trust, not balance.

Taint and immune tests:

- `taint_monotonic_no_downgrade`: `Tainted -> Clean` and `Suspect -> Clean` are
  no-ops.
- `tainted_parent_dampens_to_suspect_child`: direct taint does not poison all
  descendants as `Tainted`.
- `two_suspect_parents_escalate_child`: accumulation raises child to `Tainted`.
- `propagation_depth_is_bounded`: chain longer than `MAX_PROPAGATION_DEPTH` stops.
- `derived_iteration_is_stable`: propagation over the same graph yields the same
  ordered event list.
- `strong_anti_resonance_taints_or_rejects`: anti similarity above strong threshold
  is not merely a confidence modifier.
- `moderate_anti_resonance_marks_suspect`: threshold between moderate and strong
  halves confidence and sets `Suspect`.
- `anti_subspace_pinned`: generated `ANTI_SUBSPACE` matches a pinned hash so seed
  or PRNG drift is detected.

Integration tests:

- `shared_entry_full_trust_report`: chain metadata plus reputation produces all
  five stage factors and a finite final trust.
- `missing_chain_metadata_quarantines_external_entry`: external shared entry with
  no stake snapshot cannot enter context as trusted.
- `established_agent_still_runs_immune_layers`: high reputation does not bypass
  anti-knowledge, anomaly, or immune-memory checks.
- `incident_response_taints_and_teaches_memory`: malicious incident slashes, raises
  taint, penalizes confirmers, and stores an immune pattern.
- `retrieval_sort_has_no_nan_equal_fallback`: scoring rejects or drops invalid
  candidates before sort; no `partial_cmp(...).unwrap_or(Equal)` masking remains.

---

## Fix Checklist

Prioritized fixes for the trust pipeline and immune system, with file paths.

### P0 -- Critical (blocks correct behavior)

- [ ] **Unify trust at `crates/hdc/core/src/trust.rs`** -- Delete or merge `crates/hdc/core/src/knowledge/trust.rs` into the canonical `trust.rs`. The crate must have exactly one trust computation path. Currently:
  - `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs` -- `TrustRegistry`, `TrustPipeline` (immune layers)
  - `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs` -- `compute_trust()` (5-stage pipeline)
  - These must be merged. Keep `compute_trust()` as the pipeline, integrate it into `TrustPipeline::validate()`.

- [ ] **Add `TaintLevel` enum `{Clean, Suspect, Tainted}` with monotonic lattice** -- Currently `TaintTracker` in `trust.rs` uses a binary `HashSet` (tainted or not). Replace with the 3-level lattice specified in Section 4 of this doc:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
  pub enum TaintLevel {
      Clean   = 0,
      Suspect = 1,
      Tainted = 2,
  }
  ```
  Enforce monotonicity: `new_level > current_level` guard on every taint update.

### P1 -- High (correctness bugs)

- [ ] **Fix EMA double-application** in `TrustRegistry::update_trust()` at `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs` -- Ensure EMA is applied exactly once per update cycle, not double-smoothed by caller + callee both applying it.

- [ ] **Implement log-scaled stake weighting** at `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs` line ~47 -- Replace:
  ```rust
  let stake_weight = 0.5_f64;  // (placeholder)
  ```
  With:
  ```rust
  let stake_ratio = staked_amount as f64 / min_stake as f64;
  let stake_weight = (0.5 + stake_ratio.ln().max(0.0) / 10.0).min(1.0);
  ```

- [ ] **Add `finite_unit()` guard for NaN/Infinity at all f64 boundaries** -- Add a helper:
  ```rust
  fn finite_unit(x: f64) -> f64 {
      if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
  }
  ```
  Apply at: Stage 2 (`powf` result), Stage 4 (`ln` result), composite score output, EMA update output. Files:
  - `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`
  - `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs`
  - `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/scoring.rs`

- [ ] **Fix `scoring.rs` to use trust score, not `entry.balance`** at `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/scoring.rs` -- The scoring function should weight by the entry's computed trust score from the 5-stage pipeline, not by its raw token balance.

### P2 -- Medium (spec compliance)

- [ ] **Implement 7-domain `ReputationDomain` enum** -- Add to `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/trust.rs`:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum ReputationDomain {
      Accuracy,
      Timeliness,
      Novelty,
      Reliability,
      Collaboration,
      Specialization,
      Integrity,
  }
  ```
  Add `ReputationScores` struct with per-domain `HashMap<ReputationDomain, f64>` and implement `composite_score()` with weighted average.

- [ ] **Add quarantine with tiered confirmation thresholds** -- Implement `quarantine_check()` per Section 5 Layer 3:
  - Previously slashed: 10 confirmations
  - Anomaly-flagged: 5 confirmations
  - New agent (< 10 publications) or low-rep (< 0.3): 3 confirmations
  - Established, clean agent: exempt

- [ ] **Fix hardcoded `0.693` in `trust.rs`** -- Replace with `(2.0_f64).ln()` for consistency with `decay.rs`. File: `/Users/will/dev/nunchi/daeji/crates/hdc/core/src/knowledge/trust.rs` line ~39.

- [ ] **Document Stage 5 rescaling divergence** -- The impl applies `(sim - 0.5) * 2.0` which is stricter than the spec's raw similarity. Either align with spec or add a comment explaining the intentional deviation.

### Immune Layers 2-5 Implementation Checklist

These layers are specified in Section 5 but **none are implemented**.

**Layer 2: Anomaly Detection**
- [ ] Create `AnomalyDetector` struct with `AnomalyConfig`
- [ ] Implement `detect_rate_spikes()` -- z-score > 3.0 against agent's historical mean
- [ ] Implement `detect_temporal_clustering()` -- many insights within `min_publication_interval` blocks
- [ ] Implement `detect_sybil_clusters()` -- DBSCAN on insight vectors cross-referenced with agent identity
- [ ] Implement `detect_confirmation_rings()` -- clique detection with high internal/low external confirmation density

**Layer 3: Quarantine**
- [ ] Implement `QuarantineStatus` enum (`Exempt`, `Active { required }`, `Released`)
- [ ] Implement tiered confirmation thresholds per agent category (see P2 above)
- [ ] Integrate quarantine status into `WisdomGate` filter

**Layer 4: Incident Response**
- [ ] Implement `SlashSeverity` enum (`HonestMistake` 50%, `Negligence` 75%, `Malice` 100%)
- [ ] Implement `IncidentResponse::respond()` -- slash, taint, notify consumers, penalize confirmers
- [ ] Wire incident response to on-chain slash mechanism

**Layer 5: Immune Memory**
- [ ] Implement `ImmuneMemory` struct with attack pattern storage
- [ ] Implement `check()` -- scan candidate against known attack patterns (Hamming distance < 1,536)
- [ ] Implement `learn()` -- add new pattern, trigger consolidation at MAX_ATTACK_PATTERNS (500)
- [ ] Implement `consolidate_patterns()` -- single-linkage agglomerative clustering + majority-vote bundling

### Verification commands

```bash
# Check trust-related files compile
cargo check -p kora-hdc

# Run trust tests
cargo test -p kora-hdc trust
cargo test -p kora-hdc knowledge

# Find duplicate trust code
grep -rn "compute_trust\|TrustPipeline\|TrustRegistry\|COLD_START" crates/hdc/core/src/

# Find NaN hazards (powf, ln, exp without guards)
grep -rn "\.powf\|\.ln()\|\.exp()\|\.log(" crates/hdc/core/src/

# Find hardcoded stake weight
grep -rn "0.5_f64\|stake_weight.*0.5" crates/hdc/core/src/

# Check for balance-instead-of-trust in scoring
grep -rn "entry\.balance\|\.balance" crates/hdc/core/src/knowledge/scoring.rs
```
