# Cognitive Architecture Implementation

> **STATUS: PARTIALLY IMPLEMENTED**
>
> - **DONE:** ALMA affect model (3-layer PAD with tau values emotion=0.1,
>   mood=0.5, personality=0.9), somatic bias, somatic candidate filter, Mattar-Daw
>   EVB replay prioritization (2-factor product).
> - **PARTIAL:** State machine (6 states defined, `StateModifiers::for_state()`
>   implemented with all 8 parameters for all 6 states, `StateContext::evaluate()`
>   implemented with full transition table -- but transitions share a single
>   `trigger_ticks` counter across divergent paths, see F08).
> - **PARTIAL:** Dream cycle (`DreamConfig`/`DreamReport` structs with defaults
>   defined, but `dream_cycle()` function is **entirely missing** -- no NREM
>   consolidation, no REM cross-binding, no execution logic at all, see F01).
> - **Last audit:** 2026-05-08

> **Scope:** Implement the ALMA affect model, behavioral state machine, somatic
> marker bias, dream cycle, and Mattar-Daw replay inside `crates/hdc/core/src/cognitive/`.
>
> **Depends on:** `kora-hdc` core algebra (HdcVector, BundleAccumulator, bind,
> similarity, complement). See impl doc `02-kora-hdc-core.md`.
>
> **Design spec:** `tmp/HDC/08-cognitive-architecture.md`.

---

## 0. Crate Layout

All code lives under `crates/hdc/core/src/cognitive/`. Create this directory and
a `mod.rs` that re-exports the sub-modules.

```
crates/hdc/core/src/cognitive/
  mod.rs           -- pub mod declarations + re-exports
  affect.rs        -- PadState, AlmaState, update_affect(), mood_to_hdc()
  state_machine.rs -- BehavioralState, StateContext, TickMetrics, StateModifiers
  somatic.rs       -- apply_somatic_bias(), apply_somatic_candidate_filter()
  dream.rs         -- DreamConfig, DreamReport, dream_cycle()
  replay.rs        -- ReplayEntry, mattar_daw_evb(), prioritize_replay()
```

```rust
// crates/hdc/core/src/cognitive/mod.rs

pub mod affect;
pub mod state_machine;
pub mod somatic;
pub mod dream;
pub mod replay;

pub use affect::{PadState, AlmaState};
pub use state_machine::{BehavioralState, StateContext, StateModifiers, TickMetrics};
pub use somatic::{apply_somatic_bias, apply_somatic_candidate_filter};
pub use dream::{DreamConfig, DreamReport, dream_cycle};
pub use replay::{ReplayEntry, mattar_daw_evb, prioritize_replay};
```

Register the module in the parent:

```rust
// crates/hdc/core/src/lib.rs  (add this line among the other pub mod declarations)
pub mod cognitive;
```

---

## 1. ALMA Affect Model (`affect.rs`)

The ALMA (A Layered Model of Affect) system has three temporal layers, each
represented as a point in Pleasure-Arousal-Dominance (PAD) space. Each layer
runs on a different time constant (tau). Lower tau means faster response to
stimuli; higher tau means more inertia.

### 1.1 Structs

```rust
use std::sync::LazyLock;
use crate::{HdcVector, BundleAccumulator};

/// A point in Pleasure-Arousal-Dominance space.
/// Each coordinate is clamped to [-1.0, 1.0].
#[derive(Clone, Debug, PartialEq)]
pub struct PadState {
    pub pleasure:  f64, // valence: negative (-1) to positive (+1)
    pub arousal:   f64, // activation: calm (-1) to excited (+1)
    pub dominance: f64, // control: submissive (-1) to dominant (+1)
}

impl PadState {
    pub fn neutral() -> Self {
        Self { pleasure: 0.0, arousal: 0.0, dominance: 0.0 }
    }

    /// Clamp all dimensions to [-1, 1].
    pub fn clamp(&mut self) {
        self.pleasure  = self.pleasure.clamp(-1.0, 1.0);
        self.arousal   = self.arousal.clamp(-1.0, 1.0);
        self.dominance = self.dominance.clamp(-1.0, 1.0);
    }
}

/// Three-layer affective state (ALMA model, Gebhard 2005).
///
/// - `emotion` (tau=0.1): fast, volatile, triggered by individual events.
/// - `mood`    (tau=0.5): medium, running average of recent emotions.
/// - `personality` (tau=0.9): slow, nearly stable baseline.
#[derive(Clone, Debug)]
pub struct AlmaState {
    pub emotion:     PadState,
    pub mood:        PadState,
    pub personality: PadState,
}

/// Time constants for each ALMA layer.
/// Used in the exponential decay update formula.
const TAU_EMOTION:     f64 = 0.1;
const TAU_MOOD:        f64 = 0.5;
const TAU_PERSONALITY: f64 = 0.9;
```

### 1.2 Update Function

Each tick, the agent receives a stimulus (a `PadState` delta from event
appraisal). All three layers incorporate the stimulus, but at different rates.

**Update formula per layer:**

```
new_layer = (1 - tau) * stimulus + tau * old_layer
```

- Low tau (emotion, 0.1): 90% stimulus, 10% prior state. Snaps to stimuli fast.
- High tau (personality, 0.9): 10% stimulus, 90% prior state. Nearly immovable.

```rust
impl AlmaState {
    pub fn new(personality: PadState) -> Self {
        Self {
            emotion: PadState::neutral(),
            mood: PadState::neutral(),
            personality,
        }
    }

    /// Apply a stimulus to all three layers.
    /// Each layer blends the stimulus with its current state according to tau.
    pub fn update(&mut self, stimulus: &PadState) {
        self.emotion     = blend(stimulus, &self.emotion,     TAU_EMOTION);
        self.mood        = blend(stimulus, &self.mood,        TAU_MOOD);
        self.personality = blend(stimulus, &self.personality,  TAU_PERSONALITY);
    }

    /// Read the mood layer. This is the primary input for somatic bias and
    /// behavioral state decisions.
    pub fn mood(&self) -> &PadState {
        &self.mood
    }
}

/// Exponential blend: `result = (1 - tau) * stimulus + tau * current`.
fn blend(stimulus: &PadState, current: &PadState, tau: f64) -> PadState {
    let inv = 1.0 - tau;
    let mut out = PadState {
        pleasure:  inv * stimulus.pleasure  + tau * current.pleasure,
        arousal:   inv * stimulus.arousal   + tau * current.arousal,
        dominance: inv * stimulus.dominance + tau * current.dominance,
    };
    out.clamp();
    out
}
```

### 1.3 PAD Basis Vectors and `mood_to_hdc()`

Three fixed basis vectors in HDC space, one per PAD dimension. Generated from
deterministic seeds via ChaCha20 so they are identical across all validators.

**Seeds are consensus-critical. They must never change after genesis.**

```rust
const PLEASURE_BASIS_SEED:  u64 = 0xCAFE_0001_0000_0001;
const AROUSAL_BASIS_SEED:   u64 = 0xCAFE_0001_0000_0002;
const DOMINANCE_BASIS_SEED: u64 = 0xCAFE_0001_0000_0003;

static PLEASURE_BASIS:  LazyLock<HdcVector> =
    LazyLock::new(|| HdcVector::random(PLEASURE_BASIS_SEED));
static AROUSAL_BASIS:   LazyLock<HdcVector> =
    LazyLock::new(|| HdcVector::random(AROUSAL_BASIS_SEED));
static DOMINANCE_BASIS: LazyLock<HdcVector> =
    LazyLock::new(|| HdcVector::random(DOMINANCE_BASIS_SEED));
```

`mood_to_hdc()` maps a PAD state into an HDC vector by weighted bundling of the
three basis vectors. Positive PAD values add the raw basis; negative values add
the complement (bitwise NOT), which produces a vector in the opposite direction.

```rust
/// Convert a PAD affective state to an HDC vector.
///
/// For each PAD dimension d with value v in [-1, 1]:
///   - weight = round(|v| * 10)   (0..=10 copies)
///   - if v > 0: add basis[d]     `weight` times
///   - if v < 0: add complement(basis[d]) `weight` times
///   - if v == 0: skip (no contribution)
///
/// If all weights are zero (perfectly neutral mood), returns the zero vector.
pub fn mood_to_hdc(pad: &PadState) -> HdcVector {
    const WEIGHT_SCALE: f64 = 10.0;

    let mut acc = BundleAccumulator::new();

    let dims: [(f64, &HdcVector); 3] = [
        (pad.pleasure,  &*PLEASURE_BASIS),
        (pad.arousal,   &*AROUSAL_BASIS),
        (pad.dominance, &*DOMINANCE_BASIS),
    ];

    for (value, basis) in &dims {
        let weight = (value.abs() * WEIGHT_SCALE).round() as usize;
        if weight == 0 { continue; }

        if *value > 0.0 {
            for _ in 0..weight { acc.add(basis); }
        } else {
            let neg = basis.complement();
            for _ in 0..weight { acc.add(&neg); }
        }
    }

    if acc.count() == 0 {
        return HdcVector::zero();
    }

    acc.finalize()
}
```

### 1.4 PAD Similarity

Cosine similarity in 3D PAD space, rescaled from [-1,1] to [0,1]. Returns 0.5
(no bias) when either vector is zero (neutral mood).

```rust
/// Cosine similarity between two PAD states, rescaled to [0, 1].
/// Returns 0.5 when either input is neutral (zero magnitude).
pub fn pad_similarity(a: &PadState, b: &PadState) -> f64 {
    let dot = a.pleasure * b.pleasure
            + a.arousal * b.arousal
            + a.dominance * b.dominance;
    let mag_a = (a.pleasure.powi(2) + a.arousal.powi(2) + a.dominance.powi(2)).sqrt();
    let mag_b = (b.pleasure.powi(2) + b.arousal.powi(2) + b.dominance.powi(2)).sqrt();

    if mag_a < 1e-9 || mag_b < 1e-9 {
        return 0.5;
    }

    let cosine = dot / (mag_a * mag_b);
    (cosine + 1.0) / 2.0
}
```

---

## 2. Behavioral State Machine (`state_machine.rs`)

### 2.1 The Six States

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BehavioralState {
    Explore,      // broad search, high diversity, speculative publication
    Exploit,      // narrow search, execute best-known strategies
    Consolidate,  // dream cycle: reorganize knowledge offline
    Emergency,    // circuit breaker: damage control, halt publication
    Recovery,     // post-crisis analysis: understand what went wrong
    Cautious,     // elevated skepticism, slow return to normal
}
```

### 2.2 State Diagram

```
                    Normal operation cycle
            +---------------------------------------------+
            |                                             |
            v            (2 ticks)        (5 ticks)       |
     +----------+      +-----------+     +--------------+ |
     | EXPLORE  |----->| EXPLOIT   |---->| CONSOLIDATE  |-+
     |          |      |           |     |              |
     | Search   |<-----| Execute   |     | Dream cycle  |
     | widely   |(5 t) | best      |     | Organize     |
     |          |      | known     |     | knowledge    |
     +--+---+--+      +-----+-----+     +------+-------+
        |   ^               |                   | alarming
   (10  |   |               |                   | patterns?
   ticks)   |               |                   v
        |   |  (3 t, conf   |            +--------------+
        |   |   > 0.9)      |            |   CAUTIOUS*  |
        v   |               |            |              |
     +------+--+            |            | High skepti- |
     | CAUTIOUS|            |            | cism         |
     |         |<-----------+------------+              |
     | Post-   |            |                           |
     | crisis  |            |                           |
     +---------+            |                           |
        ^                   |                           |
        | (1 theta loop)    |                           |
     +--+------+            |                           |
     |RECOVERY |            |                           |
     |         |            |                           |
     | Fix     |            |                           |
     | problems|            |                           |
     +---------+            |                           |
        ^                   |                           |
        | (3 ticks stable)  |                           |
     +--+------------+      |                           |
     |  EMERGENCY    |<-----+---------------------------+
     |               |     Any state (immediate, 0 hysteresis)
     |  Damage       |
     |  control      |
     +--------------+

  PROHIBITED: EMERGENCY --> EXPLORE
  Must traverse: EMERGENCY --> RECOVERY (3t) --> CAUTIOUS (1 theta) --> EXPLORE (10t)
  Minimum ticks from EMERGENCY to EXPLORE: 14 (typically 16-18)

  * CONSOLIDATE --> CAUTIOUS triggers only when dream cycle detects alarming
    patterns (anti-knowledge ratio > 0.3, Brier > 0.4, or 2+ strategy demotions).
    Otherwise CONSOLIDATE --> EXPLORE.
```

### 2.3 Transition Table

| From          | To            | Condition                                             | Hysteresis |
|---------------|---------------|-------------------------------------------------------|------------|
| EXPLORE       | EXPLOIT       | `opportunity_found` for 2 consecutive ticks           | 2 ticks    |
| EXPLOIT       | CONSOLIDATE   | returns plateau (`return_avg < -0.02`) OR KB > 10K entries without consolidation in 500 ticks | 5 ticks |
| EXPLOIT       | EXPLORE       | `return_avg < 0.01` for 5 ticks AND 20+ ticks since last EMERGENCY exit | 5 ticks |
| CONSOLIDATE   | EXPLORE       | dream cycle completes, no alarming patterns            | 0 (immediate) |
| CONSOLIDATE   | CAUTIOUS      | dream cycle completes WITH alarming patterns (anti-knowledge ratio > 0.3, Brier > 0.4, or 2+ demotions) | 0 (immediate) |
| **Any state** | **EMERGENCY** | `drawdown >= 0.10` OR `single_tick_loss >= 0.05`      | **0 (immediate)** |
| EMERGENCY     | RECOVERY      | `bleeding_stopped` for 3 consecutive ticks             | 3 ticks    |
| RECOVERY      | CAUTIOUS      | theta-loop post-mortem analysis completes              | 0 (immediate) |
| CAUTIOUS      | EXPLOIT       | `opportunity_found` AND `confidence > 0.9` for 3 ticks | 3 ticks   |
| CAUTIOUS      | EXPLORE       | `return_avg >= 0.0` AND `mood_pleasure >= -0.2` for 10 ticks | 10 ticks |
| EMERGENCY     | EXPLORE       | **PROHIBITED** -- must go RECOVERY -> CAUTIOUS -> EXPLORE | N/A       |

### 2.4 Completeness Matrix

Every (state, event) pair must be defined. `---` = no transition; remain in
current state. `PROHIBITED` = must never happen.

```
 From \ Event  | opportunity | plateau/ | dream    | crisis    | bleeding | post-   | recovery | alarming
               | found       | KB full  | complete | (loss >   | stopped  | mortem  | perf     | patterns
               |             |          |          | threshold)| (3 ticks)| done    | (10 t)   | in dream
---------------+-------------+----------+----------+-----------+----------+---------+----------+---------
 EXPLORE       | ->EXPLOIT   | ---      | ---      |->EMERGENC | ---      | ---     | ---      | ---
 EXPLOIT       | ---         |->CONSOL  | ---      |->EMERGENC | ---      | ---     | ---      | ---
 CONSOLIDATE   | ---         | ---      |->EXPLORE |->EMERGENC | ---      | ---     | ---      |->CAUTIOUS
 EMERGENCY     | ---         | ---      | ---      | ---       |->RECOVER | ---     | ---      | ---
 RECOVERY      | ---         | ---      | ---      |->EMERGENC | ---      |->CAUT.  | ---      | ---
 CAUTIOUS      |->EXPLOIT*   | ---      | ---      |->EMERGENC | ---      | ---     |->EXPLORE | ---

 *  CAUTIOUS->EXPLOIT requires confidence > 0.9 for 3+ ticks.
    PROHIBITED: EMERGENCY->EXPLORE (must traverse RECOVERY->CAUTIOUS->EXPLORE).
```

### 2.5 Per-State Parameter Modifiers

Each state applies numeric modifiers to the agent's personality-defined
baselines. These values are returned by `StateModifiers::for_state()`.

```rust
/// Numeric operational parameters that vary by behavioral state.
#[derive(Clone, Debug)]
pub struct StateModifiers {
    pub retrieval_top_k:           usize,
    pub retrieval_sim_threshold:   f64,
    pub publication_conf_threshold: f64,
    pub risk_budget_multiplier:    f64,
    pub trust_floor:               f64,
    pub position_size_multiplier:  f64,
    pub somatic_bias_weight_cap:   f64,
    pub exploration_rate:          f64,  // epsilon for epsilon-greedy
}
```

| Parameter                       | EXPLORE | EXPLOIT | CONSOLIDATE | EMERGENCY | RECOVERY | CAUTIOUS |
|---------------------------------|---------|---------|-------------|-----------|----------|----------|
| `retrieval_top_k`               | 20      | 5       | 0 (paused)  | 3         | 10       | 10       |
| `retrieval_sim_threshold`       | 0.55    | 0.75    | N/A         | 0.85      | 0.65     | 0.70     |
| `publication_conf_threshold`    | 0.50    | 0.70    | 1.0 (paused)| 1.0 (off) | 0.85     | 0.80     |
| `risk_budget_multiplier`        | 1.5     | 1.0     | 0.0         | 0.1       | 0.3      | 0.5      |
| `trust_floor`                   | 0.15    | 0.40    | 0.25        | 0.60      | 0.30     | 0.55     |
| `position_size_multiplier`      | 1.2     | 1.0     | 0.0         | 0.0       | 0.2      | 0.5      |
| `somatic_bias_weight_cap`       | 0.30    | 0.15    | 0.05        | 0.05      | 0.10     | 0.10     |
| `exploration_rate`              | 0.30    | 0.05    | 0.00        | 0.00      | 0.10     | 0.10     |

```rust
impl StateModifiers {
    pub fn for_state(state: &BehavioralState) -> Self {
        match state {
            BehavioralState::Explore => Self {
                retrieval_top_k: 20,
                retrieval_sim_threshold: 0.55,
                publication_conf_threshold: 0.50,
                risk_budget_multiplier: 1.5,
                trust_floor: 0.15,
                position_size_multiplier: 1.2,
                somatic_bias_weight_cap: 0.30,
                exploration_rate: 0.30,
            },
            BehavioralState::Exploit => Self {
                retrieval_top_k: 5,
                retrieval_sim_threshold: 0.75,
                publication_conf_threshold: 0.70,
                risk_budget_multiplier: 1.0,
                trust_floor: 0.40,
                position_size_multiplier: 1.0,
                somatic_bias_weight_cap: 0.15,
                exploration_rate: 0.05,
            },
            BehavioralState::Consolidate => Self {
                retrieval_top_k: 0,
                retrieval_sim_threshold: 1.0, // effectively paused
                publication_conf_threshold: 1.0,
                risk_budget_multiplier: 0.0,
                trust_floor: 0.25,
                position_size_multiplier: 0.0,
                somatic_bias_weight_cap: 0.05,
                exploration_rate: 0.00,
            },
            BehavioralState::Emergency => Self {
                retrieval_top_k: 3,
                retrieval_sim_threshold: 0.85,
                publication_conf_threshold: 1.0,
                risk_budget_multiplier: 0.1,
                trust_floor: 0.60,
                position_size_multiplier: 0.0,
                somatic_bias_weight_cap: 0.05,
                exploration_rate: 0.00,
            },
            BehavioralState::Recovery => Self {
                retrieval_top_k: 10,
                retrieval_sim_threshold: 0.65,
                publication_conf_threshold: 0.85,
                risk_budget_multiplier: 0.3,
                trust_floor: 0.30,
                position_size_multiplier: 0.2,
                somatic_bias_weight_cap: 0.10,
                exploration_rate: 0.10,
            },
            BehavioralState::Cautious => Self {
                retrieval_top_k: 10,
                retrieval_sim_threshold: 0.70,
                publication_conf_threshold: 0.80,
                risk_budget_multiplier: 0.5,
                trust_floor: 0.55,
                position_size_multiplier: 0.5,
                somatic_bias_weight_cap: 0.10,
                exploration_rate: 0.10,
            },
        }
    }
}
```

### 2.6 StateContext and Transition Logic

```rust
/// Metrics snapshot used for state transition evaluation.
/// Populated from the agent's tick context and passed into `StateContext::evaluate()`.
pub struct TickMetrics {
    /// Has a high-confidence, positive-EV strategy been found?
    pub opportunity_found: bool,
    /// Strategy confidence (0.0-1.0) for the opportunity.
    pub opportunity_confidence: f64,
    /// Per-tick moving average of returns.
    pub return_avg: f64,
    /// Has the dream cycle just completed?
    pub dream_complete: bool,
    /// Drawdown from recent portfolio peak (0.0-1.0).
    pub drawdown: f64,
    /// Single-tick loss as fraction of portfolio (0.0-1.0).
    pub single_tick_loss: f64,
    /// Has bleeding stopped (no further losses)?
    pub bleeding_stopped: bool,
    /// Has post-mortem (theta-loop) analysis completed?
    pub postmortem_done: bool,
    /// PAD Pleasure dimension of current mood.
    pub mood_pleasure: f64,
    /// Knowledge store size.
    pub kb_size: usize,
    /// Ticks since last consolidation.
    pub ticks_since_consolidation: u64,
    /// Anti-knowledge ratio detected during dream analysis.
    pub anti_knowledge_ratio: f64,
    /// Brier score over last 50 ticks.
    pub brier_score: f64,
    /// Number of strategies demoted in current dream cycle.
    pub strategies_demoted: u32,
}

/// Default thresholds for state transitions.
/// These can be overridden by personality settings.
const MAX_DRAWDOWN: f64 = 0.10;
const CATASTROPHIC_LOSS: f64 = 0.05;
const EXPLOITATION_RETURN_THRESHOLD: f64 = -0.02;
const CONSOLIDATION_KB_TRIGGER: usize = 10_000;
const CONSOLIDATION_TICK_TRIGGER: u64 = 500;
const EXPLORE_RETURN_THRESHOLD: f64 = 0.01;
const CAUTIOUS_TO_EXPLOIT_CONFIDENCE: f64 = 0.9;
```

```rust
/// Hysteresis-tracking context for the behavioral state machine.
pub struct StateContext {
    pub current: BehavioralState,
    /// Consecutive ticks the current transition trigger has been active.
    trigger_ticks: u32,
    /// Tick when the last EMERGENCY state was exited.
    last_emergency_exit_tick: u64,
    /// Current tick number (set externally each tick).
    pub current_tick: u64,
}

impl StateContext {
    pub fn new() -> Self {
        Self {
            current: BehavioralState::Explore,
            trigger_ticks: 0,
            last_emergency_exit_tick: 0,
            current_tick: 0,
        }
    }

    /// Evaluate tick metrics and return the (possibly new) behavioral state.
    /// If a transition fires, `trigger_ticks` is reset to 0.
    /// If no transition fires, `trigger_ticks` either increments (trigger
    /// active but hysteresis not met) or resets to 0 (trigger inactive).
    pub fn evaluate(&mut self, m: &TickMetrics) -> BehavioralState {
        // -------------------------------------------------------
        // EMERGENCY is unconditional and immediate (0-tick hysteresis).
        // Any state can transition here.
        // -------------------------------------------------------
        if m.drawdown >= MAX_DRAWDOWN || m.single_tick_loss >= CATASTROPHIC_LOSS {
            self.trigger_ticks = 0;
            self.current = BehavioralState::Emergency;
            return self.current.clone();
        }

        let next = match &self.current {
            // -- EXPLORE --
            BehavioralState::Explore => {
                if m.opportunity_found {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 2 {
                        BehavioralState::Exploit
                    } else {
                        BehavioralState::Explore
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Explore
                }
            }

            // -- EXPLOIT --
            BehavioralState::Exploit => {
                let plateau = m.return_avg < EXPLOITATION_RETURN_THRESHOLD;
                let kb_full = m.kb_size > CONSOLIDATION_KB_TRIGGER
                    && m.ticks_since_consolidation > CONSOLIDATION_TICK_TRIGGER;

                if plateau || kb_full {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 5 {
                        BehavioralState::Consolidate
                    } else {
                        BehavioralState::Exploit
                    }
                } else if m.return_avg < EXPLORE_RETURN_THRESHOLD
                    && self.current_tick.saturating_sub(self.last_emergency_exit_tick) >= 20
                {
                    // Exploit -> Explore: returns exhausted, no recent emergency
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 5 {
                        BehavioralState::Explore
                    } else {
                        BehavioralState::Exploit
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Exploit
                }
            }

            // -- CONSOLIDATE --
            BehavioralState::Consolidate => {
                if m.dream_complete {
                    if m.anti_knowledge_ratio > 0.3
                        || m.brier_score > 0.4
                        || m.strategies_demoted > 2
                    {
                        BehavioralState::Cautious
                    } else {
                        BehavioralState::Explore
                    }
                } else {
                    BehavioralState::Consolidate
                }
            }

            // -- EMERGENCY --
            // PROHIBITED: EMERGENCY -> EXPLORE.  Must go through RECOVERY.
            BehavioralState::Emergency => {
                if m.bleeding_stopped {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 3 {
                        self.last_emergency_exit_tick = self.current_tick;
                        BehavioralState::Recovery
                    } else {
                        BehavioralState::Emergency
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Emergency
                }
            }

            // -- RECOVERY --
            BehavioralState::Recovery => {
                if m.postmortem_done {
                    BehavioralState::Cautious
                } else {
                    BehavioralState::Recovery
                }
            }

            // -- CAUTIOUS --
            BehavioralState::Cautious => {
                // -> EXPLOIT: high-confidence opportunity for 3 ticks
                if m.opportunity_found
                    && m.opportunity_confidence > CAUTIOUS_TO_EXPLOIT_CONFIDENCE
                {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 3 {
                        BehavioralState::Exploit
                    } else {
                        BehavioralState::Cautious
                    }
                }
                // -> EXPLORE: 10 ticks of normal performance
                else if m.return_avg >= 0.0 && m.mood_pleasure >= -0.2 {
                    self.trigger_ticks += 1;
                    if self.trigger_ticks >= 10 {
                        BehavioralState::Explore
                    } else {
                        BehavioralState::Cautious
                    }
                } else {
                    self.trigger_ticks = 0;
                    BehavioralState::Cautious
                }
            }
        };

        if next != self.current {
            self.trigger_ticks = 0;
        }
        self.current = next.clone();
        next
    }
}
```

---

## 3. Somatic Marker Bias (`somatic.rs`)

Two functions. The first biases the HDC query vector itself; the second filters
the returned candidate list based on mood.

### 3.1 `apply_somatic_bias()` -- Query Bias

Blends the task query with a mood-derived HDC vector. Higher arousal produces
stronger bias (more mood influence on retrieval).

```rust
use crate::{HdcVector, BundleAccumulator};
use super::affect::{PadState, mood_to_hdc};

/// Bias an HDC query vector toward mood-congruent knowledge.
///
/// Arousal controls bias strength:
///   - bias_count = round(|arousal| * 30), clamped to [0, 30]
///   - query_count = 100 - bias_count
///
/// The result is `bundle(query * query_count, mood_hdc * bias_count)`.
///
/// When calm (arousal ~ 0), the query is nearly unmodified.
/// When agitated (|arousal| ~ 1), up to 30% of the bundle is mood bias.
pub fn apply_somatic_bias(query: &HdcVector, mood: &PadState) -> HdcVector {
    let bias_vector = mood_to_hdc(mood);

    // CRITICAL: clamp arousal to [0.0, 1.0] before computing bias_count.
    // Without this clamp, arousal > 1.0 would make bias_count > 30,
    // causing query_count = 100 - bias_count to underflow (usize wraps
    // to usize::MAX).
    let bias_weight = mood.arousal.abs().min(1.0);

    let bias_count = (bias_weight * 30.0) as usize;  // 0..=30
    let query_count = 100 - bias_count;               // 70..=100

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

### 3.2 `apply_somatic_candidate_filter()` -- Result Filtering

After retrieval returns a ranked candidate list, the somatic marker system
adjusts the results based on mood:

- **High arousal** narrows focus: keep fewer candidates (top-N only).
- **Low arousal** broadens search: keep more candidates.
- **Positive pleasure** biases toward familiar (high-similarity entries).
- **Negative pleasure** biases toward novel (lower-similarity entries).

```rust
/// A knowledge entry with its retrieval score.
#[derive(Clone, Debug)]
pub struct ScoredEntry {
    pub id: [u8; 32],
    pub similarity: f64,
    pub confidence: f64,
}

/// Filter and re-rank candidates based on somatic markers (mood).
///
/// - Arousal dimension: controls how many candidates survive.
///   High |arousal| -> fewer candidates (narrow focus).
///   Low |arousal| -> more candidates (broad search).
///
/// - Pleasure dimension: re-scores candidates.
///   Positive pleasure -> boost high-similarity entries (prefer familiar).
///   Negative pleasure -> boost lower-similarity entries (prefer novel).
///
/// `max_candidates` is the state-specific `retrieval_top_k` value.
pub fn apply_somatic_candidate_filter(
    candidates: &mut Vec<ScoredEntry>,
    mood: &PadState,
    max_candidates: usize,
) {
    if candidates.is_empty() || max_candidates == 0 {
        candidates.clear();
        return;
    }

    // --- Arousal: narrow/widen candidate count ---
    // CRITICAL: clamp arousal to min(1.0) before computing bias_count.
    // arousal > 1.0 would make narrowing_factor > 1.0, and then
    // (max * (1.0 - narrowing_factor)) could go negative, wrapping on
    // the as-usize cast.
    let clamped_arousal = mood.arousal.abs().min(1.0);

    // narrowing_factor in [0.0, 0.5]: high arousal removes up to half
    let narrowing_factor = clamped_arousal * 0.5;
    let target_count = ((max_candidates as f64) * (1.0 - narrowing_factor))
        .round()
        .max(1.0) as usize; // always keep at least 1

    // --- Pleasure: re-score candidates ---
    let pleasure = mood.pleasure; // in [-1, 1]
    for entry in candidates.iter_mut() {
        // pleasure > 0 -> multiply similarity (boost familiar/high-sim)
        // pleasure < 0 -> multiply (1 - similarity) (boost novel/low-sim)
        let bonus = if pleasure >= 0.0 {
            pleasure * entry.similarity * 0.1
        } else {
            pleasure.abs() * (1.0 - entry.similarity) * 0.1
        };
        entry.similarity += bonus;
    }

    // Re-sort by adjusted similarity (descending)
    candidates.sort_by(|a, b| {
        b.similarity.partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Truncate to arousal-adjusted count
    candidates.truncate(target_count);
}
```

---

## 4. Dream Cycle (`dream.rs`)

Offline consolidation triggered during the CONSOLIDATE behavioral state, idle
timeout, or knowledge store pressure.

### 4.1 Trigger Conditions

| Trigger                  | Condition                                       |
|--------------------------|-------------------------------------------------|
| CONSOLIDATE state        | Behavioral state machine transitions to CONSOLIDATE |
| Idle timeout             | No pending tasks for 50+ consecutive ticks (~20s) |
| Knowledge store pressure | 10K+ entries OR conflict density > 40%          |

### 4.2 Configuration and Report Structs

```rust
use rand::Rng;
use rand::seq::SliceRandom;

/// Configuration for the dream cycle.
pub struct DreamConfig {
    /// Maximum episodes to process in NREM phase.
    pub max_nrem_episodes: usize,       // default: 500
    /// Number of random cross-bindings in REM phase.
    pub rem_cross_bindings: usize,      // default: 100
    /// Similarity threshold for near-duplicate merging.
    pub merge_threshold: f64,           // default: 0.95
    /// Similarity threshold for REM resonance detection.
    pub resonance_threshold: f64,       // default: 0.65
    /// GC threshold for entry retention.
    pub gc_threshold: f64,              // default: 0.01
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

/// Report produced by a dream cycle, for logging and telemetry.
pub struct DreamReport {
    pub nrem_re_encoded: usize,
    pub nrem_merged: usize,
    pub nrem_promoted: usize,
    pub nrem_garbage_collected: usize,
    pub rem_cross_bindings_attempted: usize,
    pub rem_resonances_found: usize,
    pub rem_insights_created: usize,
    pub duration_ms: u64,
    /// true if alarming patterns were detected -> transition to CAUTIOUS.
    pub alarming: bool,
}
```

### 4.3 NREM Phase (Consolidation)

Processes up to 500 episodes. Steps:

1. **Select** high-replay-value episodes (sort by `query_hits * confidence`, take
   top N). In production, use Mattar-Daw EVB scores from `replay.rs` instead of
   `query_hits * confidence`.
2. **Re-encode** with current projections (knowledge evolves; old encodings go stale).
3. **Merge** near-duplicates (similarity > 0.95). Keep the higher-confidence entry;
   accumulate `confirmation_count` from the merged entry.
4. **Promote** well-confirmed entries up the tier hierarchy
   (`Transient -> Working -> Consolidated`).
5. **Garbage collect** below-threshold entries (effective balance < `gc_threshold`).
6. **Detect alarming patterns**: anti-knowledge ratio > 0.3 sets `report.alarming = true`.

### 4.4 REM Phase (Creative Recombination)

Performs 100 random cross-bindings:

1. Pick two random entries from different domains (mutual similarity < 0.6).
2. Compute `cross = bind(entry_a, entry_b)`.
3. Compare `cross` against all entries for resonance (similarity > 0.65).
4. If resonant: create a new `Transient`-tier `Insight` entry with confidence 0.3.
5. One resonance per cross-binding (break after first match).

### 4.5 Duration Target

- NREM: ~500 episodes at ~0.1ms each = ~50ms.
- REM: 100 cross-bindings at ~0.1ms each = ~10ms.
- **Total: < 100ms.** Agent pauses task execution for the duration.

### 4.6 `dream_cycle()` Signature

```rust
/// Execute a full dream cycle (NREM + REM).
///
/// `store`: mutable reference to the agent's knowledge store.
/// `config`: dream cycle configuration.
/// `now_secs`: current time in seconds (for demurrage calculations).
/// `rng`: random number generator for REM phase.
///
/// Returns a `DreamReport` with telemetry about what happened.
///
/// The caller (cognitive loop) should check `report.alarming` and, if true,
/// transition to CAUTIOUS instead of EXPLORE.
pub fn dream_cycle(
    store: &mut Vec<KnowledgeEntry>,
    config: &DreamConfig,
    now_secs: u64,
    rng: &mut impl Rng,
) -> DreamReport {
    let start = std::time::Instant::now();
    let mut report = DreamReport {
        nrem_re_encoded: 0,
        nrem_merged: 0,
        nrem_promoted: 0,
        nrem_garbage_collected: 0,
        rem_cross_bindings_attempted: 0,
        rem_resonances_found: 0,
        rem_insights_created: 0,
        duration_ms: 0,
        alarming: false,
    };

    // ===== NREM Phase =====

    // Step 1: Sort by replay value (query_hits * confidence), take top N
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

    // Step 3: Merge near-duplicates
    let mut i = 0;
    while i < store.len() {
        let mut j = i + 1;
        while j < store.len() {
            let sim = store[i].vector.similarity(&store[j].vector);
            if sim > config.merge_threshold {
                if store[i].confidence >= store[j].confidence {
                    store[i].confirmation_count += store[j].confirmation_count;
                    store.remove(j);
                } else {
                    store[j].confirmation_count += store[i].confirmation_count;
                    store.remove(i);
                    continue; // re-check at same i
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

    // Step 6: Detect alarming patterns
    let anti_count = store.iter()
        .filter(|e| e.kind == KnowledgeKind::AntiKnowledge)
        .count();
    let anti_ratio = if store.is_empty() { 0.0 }
        else { anti_count as f64 / store.len() as f64 };
    if anti_ratio > 0.3 {
        report.alarming = true;
    }

    // ===== REM Phase =====

    if store.len() >= 2 {
        for _ in 0..config.rem_cross_bindings {
            report.rem_cross_bindings_attempted += 1;

            let idx_a = rng.gen_range(0..store.len());
            let mut idx_b = rng.gen_range(0..store.len());
            while idx_b == idx_a {
                idx_b = rng.gen_range(0..store.len());
            }

            // Only cross-bind entries from different domains
            let mutual_sim = store[idx_a].vector.similarity(&store[idx_b].vector);
            if mutual_sim > 0.6 { continue; }

            let cross = store[idx_a].vector.bind(&store[idx_b].vector);

            // Check for resonance against all entries
            for entry in store.iter() {
                let resonance = cross.similarity(&entry.vector);
                if resonance > config.resonance_threshold {
                    report.rem_resonances_found += 1;

                    let insight = KnowledgeEntry {
                        id: H256::from_slice(&vector_id(&cross)),
                        vector: cross.clone(),
                        content: format!(
                            "REM cross-binding resonance: {} x {}",
                            store[idx_a].content, store[idx_b].content
                        ),
                        kind: KnowledgeKind::Insight,
                        tier: KnowledgeTier::Transient,
                        confidence: 0.3,
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

---

## 5. Mattar-Daw Replay (`replay.rs`)

Prioritizes which memories to replay during the dream cycle's NREM phase.

### 5.1 EVB Formula

**EVB (Expected Value of Backup) is a 2-factor product, NOT 3.**

```
EVB(s, a) = gain(s, a) * need(s)
```

| Factor | Meaning | Computation |
|--------|---------|-------------|
| `gain` | Expected increase in reward from updating this state-action pair. High when the outcome was surprising (large prediction error). | `gain = abs(predicted_reward - actual_reward)` |
| `need` | Discounted expected number of future visits to this state. High when similar situations are likely to recur. | `need = base_recurrence * discount_factor` |

**Why only 2 factors:** The original Mattar & Daw (2018) paper defines EVB as
`gain * need`. Some secondary sources incorrectly add a third factor (e.g.,
priority or relevance). Do not do this. Adding a third factor changes the
mathematical properties of the scoring and does not correspond to the original
formulation.

### 5.2 Structs and Functions

```rust
/// An entry eligible for replay prioritization.
#[derive(Clone, Debug)]
pub struct ReplayEntry {
    /// Index into the knowledge store.
    pub store_index: usize,
    /// Prediction error: |predicted_reward - actual_reward|.
    pub prediction_error: f64,
    /// How often similar situations have occurred recently.
    pub recurrence_count: u32,
    /// Discount factor for future relevance (0.0-1.0).
    /// Situations that occurred long ago have lower discount.
    pub discount_factor: f64,
}

/// Compute the Expected Value of Backup for a single replay candidate.
///
/// EVB = gain * need
///   gain = prediction_error  (how surprising was the outcome?)
///   need = recurrence_count * discount_factor  (how relevant is this going forward?)
///
/// NOTE: This is a 2-factor product. Do NOT add a third factor.
pub fn mattar_daw_evb(entry: &ReplayEntry) -> f64 {
    let gain = entry.prediction_error;
    let need = entry.recurrence_count as f64 * entry.discount_factor;
    gain * need
}

/// Sort replay candidates by descending EVB and return the top N.
///
/// `max_replay` should match `DreamConfig::max_nrem_episodes` (default 500).
pub fn prioritize_replay(
    candidates: &mut Vec<ReplayEntry>,
    max_replay: usize,
) -> Vec<ReplayEntry> {
    candidates.sort_by(|a, b| {
        let evb_a = mattar_daw_evb(a);
        let evb_b = mattar_daw_evb(b);
        evb_b.partial_cmp(&evb_a).unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(max_replay);
    candidates.clone()
}
```

---

## 6. Anti-Patterns

Things that are wrong and must not appear in the implementation.

### 6.1 DO NOT make EVB a 3-factor product

```rust
// WRONG -- do not do this
fn evb_wrong(gain: f64, need: f64, priority: f64) -> f64 {
    gain * need * priority  // <-- 3 factors, not Mattar-Daw
}

// CORRECT -- exactly 2 factors
fn evb_correct(gain: f64, need: f64) -> f64 {
    gain * need
}
```

The original Mattar & Daw (2018) paper defines EVB as `gain * need`. There is no
third factor. Adding one changes the mathematical semantics.

### 6.2 DO NOT forget the arousal clamp in somatic bias

```rust
// WRONG -- arousal can exceed 1.0 from accumulation bugs or test data
let bias_count = (mood.arousal.abs() * 30.0) as usize;  // could be 45 if arousal = 1.5
let query_count = 100 - bias_count;  // 100 - 45 = underflow on usize!

// CORRECT -- clamp first
let bias_weight = mood.arousal.abs().min(1.0);
let bias_count = (bias_weight * 30.0) as usize;  // max 30
let query_count = 100 - bias_count;                // min 70, safe
```

Without the `.min(1.0)` clamp, `bias_count` can exceed 30, and the subtraction
`100 - bias_count` wraps to `usize::MAX`, causing the loop to run for billions
of iterations or panic. This is a real bug, not an edge case.

### 6.3 DO NOT allow EMERGENCY -> EXPLORE

```rust
// WRONG -- skips mandatory recovery path
BehavioralState::Emergency => {
    if conditions_improved {
        return BehavioralState::Explore;  // PROHIBITED
    }
}

// CORRECT -- must go through RECOVERY -> CAUTIOUS -> EXPLORE
BehavioralState::Emergency => {
    if m.bleeding_stopped {
        self.trigger_ticks += 1;
        if self.trigger_ticks >= 3 {
            self.last_emergency_exit_tick = self.current_tick;
            return BehavioralState::Recovery;  // only valid exit
        }
    }
}
```

The mandatory recovery path is: `EMERGENCY -> RECOVERY (3 ticks) -> CAUTIOUS
(1 theta loop) -> EXPLORE (10 ticks)`. Minimum 14 ticks, typically 16-18. Any
code path that transitions directly from EMERGENCY to EXPLORE is a bug.

### 6.4 DO NOT use text embeddings for PAD basis vectors

```rust
// WRONG -- non-deterministic, model-dependent
let pleasure_basis = embed("pleasure positive happy");

// CORRECT -- deterministic from fixed seed via ChaCha20
static PLEASURE_BASIS: LazyLock<HdcVector> =
    LazyLock::new(|| HdcVector::random(0xCAFE_0001_0000_0001));
```

The basis vectors must be identical across all validators. Using LLM text
embeddings introduces model-version dependency and non-determinism.

### 6.5 DO NOT skip hysteresis on non-emergency transitions

```rust
// WRONG -- transitions immediately, causing oscillation
BehavioralState::Explore => {
    if m.opportunity_found {
        return BehavioralState::Exploit;  // no tick counter!
    }
}

// CORRECT -- require sustained trigger
BehavioralState::Explore => {
    if m.opportunity_found {
        self.trigger_ticks += 1;
        if self.trigger_ticks >= 2 {   // 2-tick hysteresis
            return BehavioralState::Exploit;
        }
    } else {
        self.trigger_ticks = 0;  // reset if trigger goes inactive
    }
}
```

Without hysteresis, a single noisy tick can cause EXPLORE -> EXPLOIT, and the
next tick can reverse it. The state machine would oscillate rapidly, disrupting
ongoing strategies and wasting transition overhead.

### 6.6 DO NOT update personality at the same rate as emotion

```rust
// WRONG -- personality should be nearly immovable
self.personality = blend(stimulus, &self.personality, 0.1);  // tau=0.1 like emotion

// CORRECT -- personality changes glacially
self.personality = blend(stimulus, &self.personality, 0.9);  // tau=0.9
```

Tau=0.9 means only 10% of the stimulus reaches the personality layer. This is
by design: personality defines the agent's character and should not shift from a
single event.

---

## 7. Integration: How the Pieces Connect

Each cognitive tick (~400ms / one block), the main loop orchestrates these
components in order:

```
1. Perceive (read chain state, pheromones, new shared insights)
         |
2. Update affect:
         |   stimulus = appraise_events(perceptions)  // -> PadState delta
         |   alma_state.update(&stimulus)
         |
3. Check state machine:
         |   metrics = build_tick_metrics(...)
         |   new_state = state_ctx.evaluate(&metrics)
         |   modifiers = StateModifiers::for_state(&new_state)
         |
4. If new_state == Consolidate:
         |   replay_candidates = build_replay_candidates(store)
         |   prioritized = prioritize_replay(&mut replay_candidates, 500)
         |   report = dream_cycle(&mut store, &config, now, &mut rng)
         |   // report.alarming -> state machine will pick up next tick
         |
5. Otherwise (non-Consolidate states):
         |   query = build_query_vector(task, context)
         |   biased_query = apply_somatic_bias(&query, alma_state.mood())
         |   candidates = search(biased_query, modifiers.retrieval_top_k,
         |                       modifiers.retrieval_sim_threshold)
         |   apply_somatic_candidate_filter(&mut candidates,
         |                                  alma_state.mood(),
         |                                  modifiers.retrieval_top_k)
         |   context = assemble_context(candidates)
         |   decision = llm_call(context)
         |   execute(decision)
         |
6. Learn:
         |   record_episode(situation, action, outcome)
         |   update_predictions(gamma_loop)
```

---

## 8. Implementation Checklist

### Phase 1: Core Types (ALMA Affect) -- DONE

- [x] Create `crates/hdc/core/src/cognitive/` directory
- [x] Create `mod.rs` with sub-module declarations and re-exports
- [x] Register `pub mod cognitive;` in `crates/hdc/core/src/lib.rs`
- [x] Implement `PadState` with `neutral()` and `clamp()` -- `affect.rs`
- [x] Implement `AlmaState` with `new()`, `update()`, `mood()` -- `affect.rs`
- [x] Implement `blend()` helper -- `affect.rs`
- [x] Add `TAU_EMOTION = 0.1`, `TAU_MOOD = 0.5`, `TAU_PERSONALITY = 0.9` constants -- `affect.rs`
- [x] Declare PAD basis seed constants (`0xCAFE_0001_0000_0001/2/3`) -- `affect.rs`
- [x] Declare `LazyLock<HdcVector>` statics for `PLEASURE_BASIS`, `AROUSAL_BASIS`, `DOMINANCE_BASIS` -- `affect.rs`
- [x] Implement `mood_to_hdc()` -- `affect.rs` (minor: uses private `complement()` instead of method, `to_vector()` vs `finalize()`, `default()` vs `zero()`)
- [x] Implement `pad_similarity()` -- `affect.rs` (cosine similarity rescaled to [0,1])

### Phase 2: State Machine -- PARTIAL (transitions stubbed)

- [x] Implement `BehavioralState` enum (6 variants: Explore, Exploit, Consolidate, Emergency, Recovery, Cautious) -- `state_machine.rs`
- [x] Implement `StateModifiers` struct and `for_state()` with all 8 parameters for all 6 states -- `state_machine.rs`
- [x] Implement `TickMetrics` struct with all 13 fields -- `state_machine.rs`
- [x] Implement `StateContext` struct with `new()` and `evaluate()` -- `state_machine.rs`
- [x] Verify EMERGENCY entry is unconditional (0-tick hysteresis) -- CORRECT
- [x] Verify EMERGENCY -> EXPLORE is impossible in all code paths -- CORRECT
- [x] Verify each hysteresis value matches the transition table -- CORRECT
- [x] Verify `trigger_ticks` resets to 0 on state transition -- CORRECT
- [ ] **BUG (F08):** Fix shared `trigger_ticks` counter across divergent paths in Exploit and Cautious states. Exploit has two exit paths (to Consolidate and to Explore) sharing one counter. Cautious has two exit paths (to Exploit and to Explore) sharing one counter. Alternating between paths accumulates ticks across different transition intentions.
- [ ] **IMPROVEMENT (F09):** `StateContext::evaluate()` does not increment `current_tick`. Caller must set it externally. Consider accepting tick number as parameter or auto-incrementing.
- [ ] **IMPROVEMENT (F10):** Add `#[derive(Default)]` on `TickMetrics` to eliminate test `default_metrics()` helper.
- [ ] **IMPROVEMENT (F11):** Add `#[derive(Copy, Hash)]` on `BehavioralState` to eliminate unnecessary `.clone()` calls.

### Phase 3: Somatic Bias -- DONE

- [x] Implement `apply_somatic_bias()` with arousal clamp (0-30% bias strength) -- `somatic.rs`
- [x] Implement `ScoredEntry` struct -- `somatic.rs`
- [x] Implement `apply_somatic_candidate_filter()` with arousal clamp -- `somatic.rs`
- [x] Verify `.min(1.0)` clamp is present in both functions -- CORRECT

**Somatic bias details:**
- Arousal controls bias strength: `bias_count = (|arousal|.min(1.0) * 30.0) as usize` (0..=30 copies of mood vector bundled with query)
- `query_count = 100 - bias_count` (70..=100 copies of original query)
- Positive pleasure: boosts high-similarity entries (`+pleasure * similarity * 0.1`)
- Negative pleasure: boosts low-similarity/novel entries (`+|pleasure| * (1-similarity) * 0.1`)
- High arousal narrows candidate count by up to 50% (`narrowing_factor = |arousal| * 0.5`)

### Phase 4: Dream Cycle -- PARTIAL (config only, no execution logic)

- [x] Implement `DreamConfig` with `Default` -- `dream.rs` (defaults: max_nrem=500, rem_cross_bindings=100, merge_threshold=0.95, resonance_threshold=0.65, gc_threshold=0.01)
- [x] Implement `DreamReport` struct with `empty()` constructor -- `dream.rs`
- [ ] **CRITICAL (F01): Implement `dream_cycle()` function** -- entirely missing. Only data types exist, no logic at all. `mod.rs` re-exports `DreamConfig` and `DreamReport` but omits `dream_cycle`.

**Remaining dream cycle implementation checklist:**

NREM Phase (Consolidation):
- [ ] Step 1: Select high-replay-value episodes (sort by `query_hits * confidence`, take top N=500). Use Mattar-Daw EVB scores from `replay.rs` for production.
- [ ] Step 2: Re-encode with current projections (`encode_text(&entry.content)`)
- [ ] Step 3: Merge near-duplicates (similarity > 0.95). Keep higher-confidence entry, accumulate `confirmation_count`.
- [ ] Step 4: Promote well-confirmed entries up tier hierarchy (`try_promote(confirmation_count)`)
- [ ] Step 5: Garbage collect below-threshold entries (`compute_balance(e, now_hours) >= gc_threshold`)
- [ ] Step 6: Detect alarming patterns -- set `report.alarming = true` when `anti_knowledge_ratio > 0.3`

REM Phase (Creative Recombination):
- [ ] Pick two random entries from different domains (mutual similarity < 0.6)
- [ ] Compute `cross = bind(entry_a, entry_b)`
- [ ] Compare `cross` against all entries for resonance (similarity > 0.65)
- [ ] If resonant: create new `Transient`-tier `Insight` entry with confidence 0.3
- [ ] One resonance per cross-binding (break after first match)
- [ ] Perform max 100 cross-bindings

Integration:
- [ ] Add `dream_cycle` to `mod.rs` re-exports
- [ ] Wire `dream_cycle()` output to `TickMetrics::dream_complete` and `TickMetrics::alarming` fields
- [ ] Duration target: NREM ~50ms (500 episodes x 0.1ms), REM ~10ms (100 bindings x 0.1ms), total < 100ms

### Phase 5: Replay -- DONE

- [x] Implement `ReplayEntry` struct -- `replay.rs`
- [x] Implement `mattar_daw_evb()` as 2-factor product (`gain * need`) -- `replay.rs`
- [x] Implement `prioritize_replay()` -- `replay.rs`
- [x] Verify EVB is NOT a 3-factor product -- CORRECT
- [ ] **IMPROVEMENT (F07):** `prioritize_replay()` mutates input AND clones result. Pick one pattern: mutate in place or return new vec.

---

## 9. Test Plan

### 9.1 ALMA Affect Tests

```rust
#[test]
fn emotion_responds_fast_to_stimulus() {
    let mut alma = AlmaState::new(PadState::neutral());
    let stimulus = PadState { pleasure: 1.0, arousal: 0.0, dominance: 0.0 };
    alma.update(&stimulus);
    // tau=0.1 -> emotion = 0.9 * 1.0 + 0.1 * 0.0 = 0.9
    assert!((alma.emotion.pleasure - 0.9).abs() < 1e-9);
}

#[test]
fn personality_resists_stimulus() {
    let mut alma = AlmaState::new(PadState::neutral());
    let stimulus = PadState { pleasure: 1.0, arousal: 0.0, dominance: 0.0 };
    alma.update(&stimulus);
    // tau=0.9 -> personality = 0.1 * 1.0 + 0.9 * 0.0 = 0.1
    assert!((alma.personality.pleasure - 0.1).abs() < 1e-9);
}

#[test]
fn mood_is_medium_speed() {
    let mut alma = AlmaState::new(PadState::neutral());
    let stimulus = PadState { pleasure: 1.0, arousal: 0.0, dominance: 0.0 };
    alma.update(&stimulus);
    // tau=0.5 -> mood = 0.5 * 1.0 + 0.5 * 0.0 = 0.5
    assert!((alma.mood.pleasure - 0.5).abs() < 1e-9);
}

#[test]
fn pad_values_stay_clamped() {
    let mut alma = AlmaState::new(PadState { pleasure: 0.95, arousal: 0.0, dominance: 0.0 });
    // repeated positive stimuli should not exceed 1.0
    for _ in 0..100 {
        alma.update(&PadState { pleasure: 1.0, arousal: 0.0, dominance: 0.0 });
    }
    assert!(alma.emotion.pleasure <= 1.0);
    assert!(alma.mood.pleasure <= 1.0);
    assert!(alma.personality.pleasure <= 1.0);
}

#[test]
fn neutral_mood_produces_zero_vector() {
    let v = mood_to_hdc(&PadState::neutral());
    // all dimensions zero -> all weights zero -> zero vector
    assert_eq!(v, HdcVector::zero());
}
```

### 9.2 State Machine Tests

```rust
#[test]
fn emergency_is_immediate() {
    let mut ctx = StateContext::new();
    let m = TickMetrics {
        drawdown: 0.15, // exceeds MAX_DRAWDOWN (0.10)
        ..default_metrics()
    };
    let state = ctx.evaluate(&m);
    assert_eq!(state, BehavioralState::Emergency);
}

#[test]
fn explore_to_exploit_requires_2_ticks() {
    let mut ctx = StateContext::new(); // starts in Explore
    let m = TickMetrics { opportunity_found: true, ..default_metrics() };

    // Tick 1: trigger active but hysteresis not met
    assert_eq!(ctx.evaluate(&m), BehavioralState::Explore);
    // Tick 2: hysteresis met -> transition
    assert_eq!(ctx.evaluate(&m), BehavioralState::Exploit);
}

#[test]
fn emergency_cannot_go_directly_to_explore() {
    let mut ctx = StateContext::new();
    ctx.current = BehavioralState::Emergency;

    // Even with perfect metrics, should not jump to Explore
    let m = TickMetrics {
        bleeding_stopped: true,
        return_avg: 1.0,
        mood_pleasure: 1.0,
        ..default_metrics()
    };

    // 3 ticks of stable -> Recovery (not Explore)
    ctx.evaluate(&m);
    ctx.evaluate(&m);
    let state = ctx.evaluate(&m);
    assert_eq!(state, BehavioralState::Recovery);
}

#[test]
fn full_recovery_path() {
    let mut ctx = StateContext::new();

    // Enter emergency
    ctx.current = BehavioralState::Emergency;
    ctx.trigger_ticks = 0;

    // 3 ticks bleeding stopped -> Recovery
    let m_stable = TickMetrics { bleeding_stopped: true, ..default_metrics() };
    ctx.evaluate(&m_stable);
    ctx.evaluate(&m_stable);
    assert_eq!(ctx.evaluate(&m_stable), BehavioralState::Recovery);

    // Postmortem done -> Cautious
    let m_post = TickMetrics { postmortem_done: true, ..default_metrics() };
    assert_eq!(ctx.evaluate(&m_post), BehavioralState::Cautious);

    // 10 ticks normal performance -> Explore
    let m_normal = TickMetrics {
        return_avg: 0.01,
        mood_pleasure: 0.0,
        ..default_metrics()
    };
    for _ in 0..9 {
        assert_eq!(ctx.evaluate(&m_normal), BehavioralState::Cautious);
    }
    assert_eq!(ctx.evaluate(&m_normal), BehavioralState::Explore);
}

#[test]
fn hysteresis_resets_when_trigger_goes_inactive() {
    let mut ctx = StateContext::new(); // Explore
    let m_opp = TickMetrics { opportunity_found: true, ..default_metrics() };
    let m_no_opp = TickMetrics { opportunity_found: false, ..default_metrics() };

    // 1 tick with opportunity
    ctx.evaluate(&m_opp);
    assert_eq!(ctx.trigger_ticks, 1);

    // 1 tick without -> resets
    ctx.evaluate(&m_no_opp);
    assert_eq!(ctx.trigger_ticks, 0);

    // Need 2 consecutive again
    ctx.evaluate(&m_opp);
    assert_eq!(ctx.evaluate(&m_opp), BehavioralState::Exploit);
}
```

### 9.3 Somatic Bias Tests

```rust
#[test]
fn calm_mood_produces_minimal_bias() {
    let query = HdcVector::random(42);
    let calm = PadState { pleasure: 0.0, arousal: 0.0, dominance: 0.0 };
    let biased = apply_somatic_bias(&query, &calm);
    // arousal=0 -> bias_count=0 -> query_count=100
    // result should be very similar to original query
    assert!(query.similarity(&biased) > 0.95);
}

#[test]
fn high_arousal_increases_bias() {
    let query = HdcVector::random(42);
    let agitated = PadState { pleasure: 0.5, arousal: 0.9, dominance: 0.0 };
    let biased = apply_somatic_bias(&query, &agitated);
    // arousal=0.9 -> bias_count=27, query_count=73
    // result should differ from original query
    assert!(query.similarity(&biased) < 0.95);
}

#[test]
fn arousal_clamp_prevents_underflow() {
    let query = HdcVector::random(42);
    // arousal > 1.0 should NOT cause panic or wraparound
    let extreme = PadState { pleasure: 0.0, arousal: 1.5, dominance: 0.0 };
    let biased = apply_somatic_bias(&query, &extreme);
    // should complete without panic -- bias_count capped at 30
    assert!(biased.similarity(&query) < 1.0);
}
```

### 9.4 Mattar-Daw EVB Tests

```rust
#[test]
fn evb_is_two_factor_product() {
    let entry = ReplayEntry {
        store_index: 0,
        prediction_error: 0.5,  // gain
        recurrence_count: 10,   // need component
        discount_factor: 0.8,   // need component
    };
    let evb = mattar_daw_evb(&entry);
    // gain=0.5, need=10*0.8=8.0, EVB=0.5*8.0=4.0
    assert!((evb - 4.0).abs() < 1e-9);
}

#[test]
fn zero_prediction_error_means_zero_evb() {
    let entry = ReplayEntry {
        store_index: 0,
        prediction_error: 0.0,
        recurrence_count: 100,
        discount_factor: 1.0,
    };
    // No surprise -> no value in replaying
    assert!((mattar_daw_evb(&entry) - 0.0).abs() < 1e-9);
}

#[test]
fn prioritize_replay_returns_top_n() {
    let mut candidates = vec![
        ReplayEntry { store_index: 0, prediction_error: 0.1, recurrence_count: 1, discount_factor: 1.0 },
        ReplayEntry { store_index: 1, prediction_error: 0.9, recurrence_count: 10, discount_factor: 0.9 },
        ReplayEntry { store_index: 2, prediction_error: 0.5, recurrence_count: 5, discount_factor: 0.5 },
    ];
    let top = prioritize_replay(&mut candidates, 2);
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].store_index, 1); // highest EVB: 0.9 * 10 * 0.9 = 8.1
    assert_eq!(top[1].store_index, 2); // second: 0.5 * 5 * 0.5 = 1.25
}
```

### 9.5 Dream Cycle Tests

```rust
#[test]
fn dream_cycle_sets_alarming_on_high_anti_knowledge() {
    // Build a store where > 30% of entries are AntiKnowledge
    let mut store = build_test_store(100); // 100 entries
    for entry in store.iter_mut().take(35) {
        entry.kind = KnowledgeKind::AntiKnowledge;
    }
    let config = DreamConfig::default();
    let mut rng = rand::thread_rng();
    let report = dream_cycle(&mut store, &config, 1000, &mut rng);
    assert!(report.alarming);
}

#[test]
fn dream_cycle_completes_under_100ms() {
    let mut store = build_test_store(500);
    let config = DreamConfig::default();
    let mut rng = rand::thread_rng();
    let report = dream_cycle(&mut store, &config, 1000, &mut rng);
    assert!(report.duration_ms < 100);
}

#[test]
fn rem_insights_are_transient_with_low_confidence() {
    let mut store = build_diverse_test_store(50); // low mutual similarity
    let config = DreamConfig { rem_cross_bindings: 200, ..Default::default() };
    let mut rng = rand::thread_rng();
    let initial_len = store.len();
    dream_cycle(&mut store, &config, 1000, &mut rng);
    // Check any new entries are Transient with confidence 0.3
    for entry in store.iter().skip(initial_len) {
        assert_eq!(entry.tier, KnowledgeTier::Transient);
        assert!((entry.confidence - 0.3).abs() < 1e-9);
    }
}
```

---

## Audit Findings

Audit date: 2026-05-08. Comparing spec sections 0-9 against implementation files
in `crates/hdc/core/src/cognitive/`.

### F01 -- `dream_cycle()` function is entirely missing

**Severity: Critical. The dream system is a data-type skeleton with no logic.**

The spec (section 4.6, lines 884-1031) defines a `dream_cycle()` function with
a full NREM (6-step) and REM phase. The implementation in
`crates/hdc/core/src/cognitive/dream.rs` contains only `DreamConfig` and
`DreamReport` structs with their defaults and an `empty()` constructor. The
actual `dream_cycle()` function does not exist anywhere in the cognitive module.

The `mod.rs` re-export line confirms this -- it re-exports `DreamConfig` and
`DreamReport` but conspicuously omits `dream_cycle`:

```rust
// crates/hdc/core/src/cognitive/mod.rs, line 13
pub use dream::{DreamConfig, DreamReport};
// Spec says: pub use dream::{DreamConfig, DreamReport, dream_cycle};
```

Without `dream_cycle()`, the CONSOLIDATE behavioral state is a dead end: the
state machine can enter it but there is no code to execute the consolidation,
set `dream_complete = true` in `TickMetrics`, or compute the `alarming` flag
that drives the Consolidate-to-Cautious transition. The entire offline learning
loop is inert.

Missing implementation includes:
- NREM Step 1: sort by replay value, take top N
- NREM Step 2: re-encode with current projections
- NREM Step 3: merge near-duplicates (similarity > 0.95)
- NREM Step 4: promote well-confirmed entries up tier hierarchy
- NREM Step 5: garbage collect below-threshold entries
- NREM Step 6: detect alarming patterns (anti-knowledge ratio > 0.3)
- REM: random cross-bindings, resonance detection, insight creation
- Duration tracking

### F02 -- `mood_to_hdc()` uses private `complement()` instead of the `HdcVector` API

**Severity: Medium. Correct behavior, wrong abstraction boundary.**

The spec (section 1.3, line 211) calls `basis.complement()` as a method on
`HdcVector`. The implementation (`affect.rs`, lines 140-146) defines a private
free function `complement()` that manually iterates over the internal `v.0`
word array:

```rust
// crates/hdc/core/src/cognitive/affect.rs, lines 140-146
fn complement(v: &HdcVector) -> HdcVector {
    let mut words = [0u64; crate::constants::WORDS];
    for (i, w) in v.0.iter().enumerate() {
        words[i] = !w;
    }
    HdcVector(words)
}
```

This reaches into `HdcVector`'s internal representation (`v.0`), coupling
`affect.rs` to the vector's word-array layout. If `HdcVector` ever changes its
representation (e.g., to use SIMD-aligned storage, or a `Vec<u64>`), this
function breaks silently. The `HdcVector` type should expose a
`pub fn complement(&self) -> HdcVector` method, and `affect.rs` should call it.

### F03 -- `mood_to_hdc()` uses `acc.to_vector()` but spec says `acc.finalize()`

**Severity: Low. No behavioral difference; naming inconsistency only.**

The spec (section 1.3, line 220) calls `acc.finalize()`. The implementation
(`affect.rs`, line 136) calls `acc.to_vector()`. The `BundleAccumulator` type
(`crates/hdc/core/src/bundle.rs`, line 47) only has `to_vector()`, so the
implementation is correct for the actual API. The spec text is wrong or was
written against a planned rename that never happened. Similarly,
`apply_somatic_bias()` in `somatic.rs` line 30 uses `acc.to_vector()` where the
spec (section 3.1, line 708) says `acc.finalize()`.

This is a spec-vs-code naming drift, not a bug.

### F04 -- `mood_to_hdc()` uses `HdcVector::default()` but spec says `HdcVector::zero()`

**Severity: Low. No behavioral difference.**

The spec (section 1.3, line 218) returns `HdcVector::zero()` for the neutral
case. The implementation (`affect.rs`, line 133) returns `HdcVector::default()`.
Inspecting `crates/hdc/core/src/vector.rs` line 18, `Default` produces an
all-zeros vector, so the behavior is identical. `HdcVector` does not have a
`zero()` constructor. Same situation as F03 -- spec drift.

### F05 -- `mood_to_hdc()` does not track `total_added` in the spec

**Severity: Low. Functionally equivalent but different logic.**

The spec (section 1.3, line 216) checks `acc.count() == 0` to detect the
neutral case. The implementation (`affect.rs`, lines 111-133) tracks a manual
`total_added` counter. `BundleAccumulator` does not expose a `count()` method
(`crates/hdc/core/src/bundle.rs` has no such function), so the implementation
works around it with a local variable. This is fine but indicates the spec
assumed a richer `BundleAccumulator` API than what exists.

### F06 -- `apply_somatic_bias()` uses `(bias_weight * 30.0) as usize` truncation

**Severity: Low. Spec says `round()`, implementation truncates.**

The spec (section 3.1, line 698) says `bias_count = round(|arousal| * 30)`.
The implementation (`somatic.rs`, line 20) uses `(bias_weight * 30.0) as usize`,
which truncates (floors) rather than rounding. For arousal = 0.95, spec gives
`round(28.5) = 29` but implementation gives `28`. The difference is at most 1
copy in the bundle, which is negligible for a 100-copy accumulator, but it is a
deviation from the spec.

### F07 -- `prioritize_replay()` mutates the input AND clones it

**Severity: Medium. Wasteful API design.**

`crates/hdc/core/src/cognitive/replay.rs`, lines 30-41:

```rust
pub fn prioritize_replay(
    candidates: &mut Vec<ReplayEntry>,
    max_replay: usize,
) -> Vec<ReplayEntry> {
    candidates.sort_by(|a, b| { ... });
    candidates.truncate(max_replay);
    candidates.clone()  // <-- clones after mutating in place
}
```

This function takes `&mut Vec`, sorts and truncates the original, then clones
the truncated result to return it. The caller loses the original ordering AND
pays for a full clone. Pick one pattern:
- **Mutate in place**: take `&mut Vec`, sort/truncate, return nothing (or `&[ReplayEntry]`).
- **Return new vec**: take `&[ReplayEntry]`, clone internally, sort/truncate the clone, return it.

The current hybrid does both, which is wasteful and surprising.

### F08 -- State machine `evaluate()` has a hysteresis counter bug for multi-path states

**Severity: Medium. Shared counter across divergent transition paths.**

`crates/hdc/core/src/cognitive/state_machine.rs`, `evaluate()` method.

In the `Exploit` branch (lines 171-195), there are two distinct exit paths:
- Exploit -> Consolidate (plateau or KB full, 5-tick hysteresis)
- Exploit -> Explore (returns exhausted, 5-tick hysteresis)

Both paths increment the same `self.trigger_ticks` counter. If the first path
fires for 3 ticks, then the second path fires for 2 ticks, the counter reads 5
and the second path triggers -- even though it only had 2 consecutive qualifying
ticks, not the required 5.

Similarly for `Cautious` (lines 237-257) which has two exit paths (to Exploit
and to Explore) sharing one counter. The `else` branch at line 254 resets the
counter, but alternating between the two `if` paths accumulates ticks across
different transition intentions.

The spec has the same structure, so this may be "spec-faithful," but it is a
design flaw in both. Each transition path should have its own counter, or the
counter should reset when the active transition candidate changes.

### F09 -- `StateContext::evaluate()` does not increment `current_tick`

**Severity: Medium. Caller must remember to do it.**

`crates/hdc/core/src/cognitive/state_machine.rs`, line 134: `current_tick` is
`pub` and must be set externally before each call to `evaluate()`. The
`evaluate()` method reads `self.current_tick` (line 184) but never increments
it. If the caller forgets to update `current_tick`, the
`self.current_tick.saturating_sub(self.last_emergency_exit_tick) >= 20` guard
(line 184) will use stale data, potentially blocking or prematurely enabling the
Exploit -> Explore transition.

The spec also leaves this to the caller ("set externally each tick"), but
`evaluate()` should either accept a tick number as a parameter or auto-increment
to make the contract explicit and hard to misuse.

### F10 -- No `Default` impl for `TickMetrics`

**Severity: Low. Test ergonomics issue.**

The tests define a `default_metrics()` helper function (`state_machine.rs`,
lines 281-298) to construct a zeroed `TickMetrics`. The struct itself does not
derive or implement `Default`. Since `TickMetrics` is a pure data carrier with
obvious zero-values, it should `#[derive(Default)]` or implement `Default`.
This would eliminate the test helper and make the struct easier to use from
calling code.

### F11 -- `BehavioralState` does not derive `Copy` or `Hash`

**Severity: Low. Causes unnecessary `.clone()` calls throughout.**

`crates/hdc/core/src/cognitive/state_machine.rs`, line 5: `BehavioralState`
derives `Clone, Debug, PartialEq, Eq` but not `Copy` or `Hash`. The enum has
no heap-allocated fields -- it is a simple 6-variant fieldless enum. Without
`Copy`, the `evaluate()` method (lines 153, 264-265) must call `.clone()` on
state values instead of copying them implicitly. Deriving `Copy` and `Hash` is
free and removes friction.

---

## Implementation Status

| Component | Spec Section | File | Status | Notes |
|-----------|-------------|------|--------|-------|
| `PadState` | 1.1 | `affect.rs` | **Complete** | Matches spec exactly |
| `AlmaState` | 1.1-1.2 | `affect.rs` | **Complete** | Matches spec exactly |
| `blend()` | 1.2 | `affect.rs` | **Complete** | Matches spec exactly |
| PAD basis vectors | 1.3 | `affect.rs` | **Complete** | Seeds, LazyLock statics match spec |
| `mood_to_hdc()` | 1.3 | `affect.rs` | **Complete, minor deviations** | Uses private `complement()` instead of method (F02); `to_vector()` vs `finalize()` (F03); `default()` vs `zero()` (F04); manual counter vs `acc.count()` (F05) |
| `pad_similarity()` | 1.4 | `affect.rs` | **Complete** | Matches spec exactly |
| `BehavioralState` enum | 2.1 | `state_machine.rs` | **Complete** | All 6 variants present |
| `StateModifiers` | 2.5 | `state_machine.rs` | **Complete** | All 48 values (8 params x 6 states) match spec table exactly |
| `TickMetrics` | 2.6 | `state_machine.rs` | **Complete** | All 13 fields match spec |
| `StateContext` | 2.6 | `state_machine.rs` | **Complete, design flaw** | Shared hysteresis counter across divergent paths (F08); `current_tick` not managed internally (F09) |
| `StateContext::evaluate()` | 2.6 | `state_machine.rs` | **Complete** | All transitions match spec table; EMERGENCY is immediate; EMERGENCY->EXPLORE is prohibited |
| `apply_somatic_bias()` | 3.1 | `somatic.rs` | **Complete, minor deviation** | Arousal clamp present (spec-critical); truncation vs round (F06) |
| `ScoredEntry` | 3.2 | `somatic.rs` | **Complete** | Matches spec |
| `apply_somatic_candidate_filter()` | 3.2 | `somatic.rs` | **Complete** | Arousal clamp present; logic matches spec |
| `DreamConfig` | 4.2 | `dream.rs` | **Complete** | All 5 fields with correct defaults |
| `DreamReport` | 4.2 | `dream.rs` | **Complete** | All 9 fields present |
| `dream_cycle()` | 4.3-4.6 | `dream.rs` | **NOT IMPLEMENTED** | Only structs exist; no NREM, no REM, no logic at all (F01) |
| `ReplayEntry` | 5.2 | `replay.rs` | **Complete** | All 4 fields match spec |
| `mattar_daw_evb()` | 5.1-5.2 | `replay.rs` | **Complete** | Correctly 2-factor; matches spec exactly |
| `prioritize_replay()` | 5.2 | `replay.rs` | **Complete, wasteful API** | Mutate-then-clone anti-pattern (F07) |
| Test coverage | 9.1-9.5 | all files | **Partial** | affect, state_machine, somatic, replay have tests; dream has only struct-level tests; no `dream_cycle()` tests since the function does not exist |

**Overall: ~85% of the type surface is implemented. The critical gap is
`dream_cycle()` -- the entire offline learning subsystem is missing.**

---

## Anti-Patterns & Duct Tape

### AP-01: Reaching into `HdcVector` internals

**File:** `crates/hdc/core/src/cognitive/affect.rs`, lines 140-146

```rust
fn complement(v: &HdcVector) -> HdcVector {
    let mut words = [0u64; crate::constants::WORDS];
    for (i, w) in v.0.iter().enumerate() {
        words[i] = !w;
    }
    HdcVector(words)
}
```

This function directly accesses `v.0` (the internal word array) and constructs
an `HdcVector` from raw words. This couples the cognitive module to the vector's
internal representation. If `HdcVector` fields become private, or the
representation changes, this breaks.

**Fix:** Add `pub fn complement(&self) -> HdcVector` to `HdcVector` in
`crates/hdc/core/src/vector.rs`. Delete the private function in `affect.rs`.

### AP-02: Mutate-then-clone in `prioritize_replay()`

**File:** `crates/hdc/core/src/cognitive/replay.rs`, lines 30-41

The function takes `&mut Vec<ReplayEntry>`, mutates it (sort + truncate), then
returns `candidates.clone()`. The caller loses the original data AND pays for a
heap allocation to get back data it already owns.

**Fix:** Either return `()` (caller reads the mutated vec) or take `&[ReplayEntry]`
and return a new `Vec<ReplayEntry>` without mutating the input.

### AP-03: Empty struct module masquerading as implementation

**File:** `crates/hdc/core/src/cognitive/dream.rs`

The file defines `DreamConfig`, `DreamReport`, and trivial tests for their
constructors. There is no behavioral code. This gives the false impression of
completeness -- the module compiles, the tests pass, and the re-exports work.
But the entire dream cycle is a no-op.

**Pattern name:** "Type-shell stub" -- defining the interface types without any
of the logic that uses them. Dangerous because it silently compiles and passes
CI, hiding the gap.

### AP-04: Single hysteresis counter for multiple transition paths

**File:** `crates/hdc/core/src/cognitive/state_machine.rs`, `evaluate()` method

`self.trigger_ticks` is a single `u32` shared across all possible transitions
from a given state. States with multiple exit paths (`Exploit` has 2, `Cautious`
has 2) can cross-contaminate their counters. See F08 for details.

**Fix:** Either use per-transition counters (e.g., a small enum-keyed map or
named fields), or track which transition candidate is currently accumulating and
reset when it changes.

### AP-05: `current_tick` is a public field with no update contract

**File:** `crates/hdc/core/src/cognitive/state_machine.rs`, line 134

The caller must set `ctx.current_tick = tick_number` before every call to
`evaluate()`. Nothing enforces this. If forgotten, the Exploit -> Explore guard
(`current_tick - last_emergency_exit_tick >= 20`) uses stale data.

**Fix:** Accept `current_tick` as a parameter to `evaluate()`:
```rust
pub fn evaluate(&mut self, m: &TickMetrics, current_tick: u64) -> BehavioralState
```

---

## Recommended Changes Checklist

### Critical (blocks system functionality)

- [ ] **Implement `dream_cycle()` in `dream.rs`** -- the full NREM 6-step + REM
      pipeline per spec section 4.3-4.6. Without this, the CONSOLIDATE state is
      a dead end and offline learning does not exist.
      (`crates/hdc/core/src/cognitive/dream.rs`)

- [ ] **Add `dream_cycle` to mod.rs re-exports** -- once implemented, update
      line 13 of `crates/hdc/core/src/cognitive/mod.rs`:
      `pub use dream::{DreamConfig, DreamReport, dream_cycle};`

- [ ] **Add dream cycle integration tests** -- per spec section 9.5. At minimum:
      alarming detection, duration target (<100ms), REM insight tier/confidence.

### High (design flaws that will cause bugs at scale)

- [ ] **Add `pub fn complement(&self) -> HdcVector` to `HdcVector`**
      (`crates/hdc/core/src/vector.rs`). Delete the private `complement()`
      function in `crates/hdc/core/src/cognitive/affect.rs` lines 140-146.
      Update `mood_to_hdc()` to call `basis.complement()`.

- [ ] **Fix `prioritize_replay()` API** -- either mutate in place and return
      `()` / `&[ReplayEntry]`, or take `&[ReplayEntry]` and return a new vec.
      Do not do both. (`crates/hdc/core/src/cognitive/replay.rs`, lines 30-41)

- [ ] **Fix shared hysteresis counter** -- add per-transition-path counters
      or track which path is active and reset on path change. Affects `Exploit`
      (2 exit paths) and `Cautious` (2 exit paths) in
      `crates/hdc/core/src/cognitive/state_machine.rs`.

- [ ] **Make `current_tick` a parameter** -- change `evaluate()` signature to
      `pub fn evaluate(&mut self, m: &TickMetrics, current_tick: u64)`.
      Remove `pub current_tick` field from `StateContext`.
      (`crates/hdc/core/src/cognitive/state_machine.rs`)

### Medium (correctness and consistency)

- [ ] **Use `round()` in somatic bias** -- change `(bias_weight * 30.0) as usize`
      to `(bias_weight * 30.0).round() as usize` in `apply_somatic_bias()`
      (`crates/hdc/core/src/cognitive/somatic.rs`, line 20) to match the spec's
      `round(|arousal| * 30)` formula.

- [ ] **Derive `Copy, Hash` on `BehavioralState`** -- it is a fieldless enum;
      `Copy` eliminates unnecessary `.clone()` calls throughout `evaluate()`.
      (`crates/hdc/core/src/cognitive/state_machine.rs`, line 5)

- [ ] **Derive or implement `Default` for `TickMetrics`** -- all fields have
      natural zero-values. Eliminates the `default_metrics()` test helper and
      improves ergonomics for callers.
      (`crates/hdc/core/src/cognitive/state_machine.rs`, line 100)

### Low (spec drift, naming)

- [ ] **Reconcile `to_vector()` vs `finalize()` naming** -- the spec says
      `acc.finalize()` in sections 1.3 and 3.1. The actual `BundleAccumulator`
      API is `to_vector()`. Either rename the method or update the spec. Since
      `to_vector()` is used throughout the codebase, updating the spec is the
      right move.

- [ ] **Reconcile `HdcVector::default()` vs `HdcVector::zero()` naming** --
      the spec says `HdcVector::zero()` in section 1.3 line 218. The type only
      has `Default`. Either add a `zero()` alias or update the spec.

- [ ] **Reconcile `acc.count()` vs manual counter** -- the spec assumes
      `BundleAccumulator::count()` exists. It does not. Either add it or update
      the spec to match the `total_added` pattern used in the implementation.

---

## Second-Pass Remediation Detail

This pass turns the first-pass findings into an implementation sequence. The
important correction is that the current codebase already has a real
`KnowledgeStore` in `crates/hdc/core/src/knowledge/store.rs`; the dream-cycle
work should integrate with that store instead of building a parallel
`Vec<KnowledgeEntry>` model.

### Phase 0: Reconcile the Public Surface Before Behavior

**Owned implementation files:** `crates/hdc/core/src/cognitive/mod.rs`,
`crates/hdc/core/src/cognitive/dream.rs`,
`crates/hdc/core/src/cognitive/replay.rs`,
`crates/hdc/core/src/cognitive/somatic.rs`,
`crates/hdc/core/src/knowledge/store.rs`.

Concrete fixes:

- Add the missing `dream_cycle` export in
  `crates/hdc/core/src/cognitive/mod.rs` only after the function exists:
  `pub use dream::{DreamConfig, DreamReport, dream_cycle};`.
- Change the planned `dream_cycle()` signature in
  `crates/hdc/core/src/cognitive/dream.rs` to target the existing store:
  `pub fn dream_cycle(store: &mut KnowledgeStore, config: &DreamConfig, current_tick: u64, rng: &mut impl Rng) -> DreamReport`.
- Add narrow store mutation APIs in
  `crates/hdc/core/src/knowledge/store.rs` instead of exposing private fields:
  `iter_entries()`, `entry_keys()`, `get_mut()`, `replace_vector()`,
  `remove_many()`, and `rebuild_index()`. `replace_vector()` must update
  `KnowledgeEntry::vector`, recompute `KnowledgeEntry::key` via `vector_id()`,
  and keep the `vectors` side index consistent.
- Do not add dream-only `confidence`, `query_hits`, or `confirmation_count`
  fields. The current real fields are `KnowledgeEntry::balance`,
  `confirmations`, `last_accessed`, `last_decay_tick`, and `contradicted`.
  The remediation should map spec concepts onto those fields or add explicit
  telemetry structs if a concept is genuinely missing.

### Phase 1: Affect/PAD Correctness

**Files/functions:** `crates/hdc/core/src/cognitive/affect.rs`
(`PadState::clamp`, `AlmaState::new`, `AlmaState::update`, `mood_to_hdc`,
`pad_similarity`), `crates/hdc/core/src/vector.rs` (`HdcVector`).

Concrete fixes:

- Clamp the personality baseline in `AlmaState::new()` before storing it, so a
  malformed config cannot persist PAD values outside `[-1.0, 1.0]`.
- Add `pub fn complement(&self) -> HdcVector` to `HdcVector` in
  `crates/hdc/core/src/vector.rs`, then delete the private
  `complement(v: &HdcVector)` helper from `affect.rs`.
- Keep the PAD basis seeds unchanged:
  `PLEASURE_BASIS_SEED`, `AROUSAL_BASIS_SEED`, and
  `DOMINANCE_BASIS_SEED`. Treat them as genesis-stable local constants.
- Export `mood_to_hdc` and `pad_similarity` from
  `crates/hdc/core/src/cognitive/mod.rs` if callers outside the somatic module
  need them for appraisal, telemetry, or tests.
- Decide naming once: either add `HdcVector::zero()` and
  `BundleAccumulator::count()` as compatibility helpers, or update the spec
  examples to use the existing `HdcVector::default()` and manual
  `total_added` guard. Do not leave the documentation and API disagreeing.

### Phase 2: Somatic Scoring Integration

**Files/functions:** `crates/hdc/core/src/cognitive/somatic.rs`
(`apply_somatic_bias`, `apply_somatic_candidate_filter`),
`crates/hdc/core/src/knowledge/scoring.rs` (`ScoredEntry`),
`crates/hdc/core/src/cognitive/state_machine.rs`
(`StateModifiers::for_state`).

Concrete fixes:

- Remove or replace `cognitive::somatic::ScoredEntry`; it duplicates the real
  `knowledge::scoring::ScoredEntry` but has incompatible fields
  (`id/similarity/confidence` vs `key/entry/score/hamming_distance`).
- Change `apply_somatic_candidate_filter()` to operate on
  `Vec<crate::knowledge::ScoredEntry>` and adjust `entry.score`, not a
  non-existent `similarity` field from the retrieval path.
- Thread `StateModifiers::somatic_bias_weight_cap` into
  `apply_somatic_bias()`. The fixed formula should be based on the state cap:
  `bias_count = round(abs(arousal).min(1.0) * cap * 100.0)`, with
  `query_count = 100 - bias_count`. This makes Emergency/Consolidate use the
  intended 5 percent cap instead of the current hard-coded 30 percent maximum.
- Match the spec's rounding semantics in `apply_somatic_bias()` by replacing
  truncation with `.round() as usize`.
- Clamp re-scored candidate values to a defensible range. For the real
  `knowledge::ScoredEntry`, use `entry.score = adjusted.clamp(0.0, 1.0)`.
- Preserve deterministic ranking: when adjusted scores tie, sort by
  `hamming_distance` ascending and then `key` ascending so repeated validators
  do not observe unstable order from `partial_cmp()` ties.

### Phase 3: Replay/Mattar-Daw Remediation

**Files/functions:** `crates/hdc/core/src/cognitive/replay.rs`
(`ReplayEntry`, `mattar_daw_evb`, `prioritize_replay`),
`crates/hdc/core/src/cognitive/dream.rs` (`dream_cycle`).

Concrete fixes:

- Keep `mattar_daw_evb()` exactly a two-factor product:
  `prediction_error * (recurrence_count as f64 * discount_factor)`. Do not add
  priority, tier, trust, or confidence as a third factor.
- Replace `ReplayEntry::store_index` with `ReplayEntry::key: [u8; 32]`, or add
  a new key-based replay entry type. A `HashMap`-backed `KnowledgeStore` has no
  stable index, and NREM merge/GC can invalidate positional indices.
- Change `prioritize_replay()` to one clear ownership model. Preferred:
  `pub fn prioritize_replay(candidates: &[ReplayEntry], max_replay: usize) -> Vec<ReplayEntry>`.
  This avoids mutating the caller's replay queue while still returning a sorted
  top-N.
- Add `build_replay_candidates()` in `dream.rs` or `replay.rs` that derives
  replay entries from store keys plus explicit episode telemetry. Do not infer
  prediction error from `KnowledgeEntry::balance`; balance is retention/trust,
  not surprise.
- Add a temporary `ReplaySignals` or `EpisodeOutcome` input if the learning
  loop does not yet persist predicted and actual reward. Use it to compute:
  `prediction_error = abs(predicted_reward - actual_reward)`,
  `recurrence_count = recent_similar_hits`, and
  `discount_factor = gamma.powi(age_or_horizon)`.

### Phase 4: Dream Cycle Execution

**Files/functions:** `crates/hdc/core/src/cognitive/dream.rs`
(`DreamConfig`, `DreamReport`, `dream_cycle`),
`crates/hdc/core/src/knowledge/store.rs` (`insert`, `remove`, `tick`,
`replace_vector`, `rebuild_index`), `crates/hdc/core/src/encode.rs`
(`TrigramEncoder::encode`), `crates/hdc/core/src/vector.rs` (`bind`,
`similarity`), `crates/hdc/core/src/knowledge/tier.rs`
(`KnowledgeTier::try_promote`).

Implementation steps:

1. NREM selection: call `build_replay_candidates()`, then
   `prioritize_replay(..., config.max_nrem_episodes)`. Work from keys, not
   vector positions.
2. Re-encoding: for each selected key, compute
   `TrigramEncoder::encode(&entry.content)` and call
   `KnowledgeStore::replace_vector(old_key, new_vector)`. Report
   `nrem_re_encoded` only after the store and vector index both update.
3. Merge near duplicates: compare selected entries with `similarity(&a.vector,
   &b.vector) > config.merge_threshold`. Keep the entry with stronger
   `balance`, then higher `confirmations`, then lexicographically smaller
   `key`. Merge by adding confirmations with saturating arithmetic and taking
   the higher balance. Remove the losing key through `KnowledgeStore::remove()`.
4. Promote: apply `KnowledgeTier::try_promote(entry.confirmations)` through a
   store method that updates the entry in place. Count only actual tier changes
   in `nrem_promoted`.
5. Garbage collection: prefer reusing `KnowledgeStore::tick(current_tick)` for
   decay, demotion, and GC semantics. If dream-specific GC remains necessary,
   it must respect the existing rule that `KnowledgeTier::Persistent` is
   demoted, not deleted.
6. Alarming detection: compute `anti_knowledge_ratio` from
   `KnowledgeKind::AntiKnowledge`, include contradicted entries if the design
   wants them to count as anti-knowledge pressure, and set
   `DreamReport::alarming` when the ratio exceeds `0.3`.
7. REM cross-binding: choose two different keys with deterministic RNG, skip
   pairs with mutual `similarity > 0.6`, compute `bind(&a.vector, &b.vector)`,
   and scan for resonance over a snapshot of current entries. Snapshot first so
   newly inserted insights do not affect the same REM pass.
8. REM insight insertion: create entries with
   `KnowledgeEntry::new(cross, KnowledgeKind::Insight, content,
   KnowledgeSource::SelfDerived, current_tick)`. Set `balance = 0.3`,
   leave `tier = KnowledgeTier::Transient`, and keep `confirmations = 0`.
9. Duration: set `DreamReport::duration_ms` from one `Instant` covering both
   NREM and REM. The target remains below 100ms for the default config.

### Phase 5: State Machine Integration

**Files/functions:** `crates/hdc/core/src/cognitive/state_machine.rs`
(`StateContext`, `StateContext::evaluate`, `TickMetrics`,
`StateModifiers::for_state`), eventual cognitive loop caller.

Concrete fixes:

- Track the active hysteresis path, not only a shared counter. Add an internal
  enum such as `TransitionIntent::{ExploreToExploit, ExploitToConsolidate,
  ExploitToExplore, EmergencyToRecovery, CautiousToExploit, CautiousToExplore}`
  and reset `trigger_ticks` whenever the intent changes.
- Make tick advancement explicit. Preferred signature:
  `pub fn evaluate(&mut self, m: &TickMetrics, current_tick: u64) -> BehavioralState`.
  Store `current_tick` internally only after accepting the parameter. This
  prevents stale public `current_tick` values from changing transition behavior.
- Add `Default` for `TickMetrics` and derive `Copy, Hash` for
  `BehavioralState`.
- Wire dream completion as a two-step event: the loop enters
  `BehavioralState::Consolidate`, runs `dream_cycle()` once, then builds the
  next `TickMetrics` with `dream_complete = true`,
  `anti_knowledge_ratio = report anti ratio`, and
  `strategies_demoted = report strategies_demoted` if that field is added.
- Keep `Emergency` preemption first in `evaluate()` so dream execution or
  recovery logic cannot mask `drawdown >= 0.10` or
  `single_tick_loss >= 0.05`.
- Ensure Consolidate does not execute normal retrieval or publication:
  `StateModifiers::for_state(&BehavioralState::Consolidate)` already sets
  `retrieval_top_k = 0`, `publication_conf_threshold = 1.0`, and
  `position_size_multiplier = 0.0`; the caller must honor those fields.

### Phase 6: Persistence Boundaries

**Files/functions:** `crates/hdc/core/src/knowledge/store.rs`
(`KnowledgeStore::open`, `insert`, `remove`, `tick`, `reinforce`),
`crates/hdc/core/src/cognitive/affect.rs` (`AlmaState`),
`crates/hdc/core/src/cognitive/state_machine.rs` (`StateContext`),
`crates/hdc/core/src/cognitive/dream.rs` (`DreamReport`).

Concrete boundary rules:

- Persistent/offline memory is `KnowledgeStore` data only:
  `KnowledgeEntry::{key, vector, kind, tier, content, source, confirmations,
  last_accessed, last_decay_tick, balance, contradicted}`.
- Runtime cognitive state is local and should not be published or treated as
  consensus data: `AlmaState`, `PadState` mood/emotion/personality,
  `StateContext`, replay queues, RNG state, and `DreamReport`.
- `DreamReport` is telemetry. It can be logged or used to build the next
  `TickMetrics`, but it should not be inserted into `KnowledgeStore` unless a
  separate `KnowledgeEntry` is explicitly created for an insight.
- `KnowledgeStore::open()` is currently a stub returning an empty store. Do not
  claim dream-cycle persistence is complete until `open()` has a real backing
  format and round-trip tests for entries modified by NREM merge, promotion,
  demotion, and REM insertion.
- On-chain boundaries stay outside `crates/hdc/core/src/cognitive`: dream,
  PAD, replay, and somatic markers must not publish directly. Publication
  remains a caller decision after retrieval, confidence/trust checks, and
  `StateModifiers::publication_conf_threshold`.

### Phase 7: Test Additions and CI Gates

**Files:** unit tests in each cognitive module, plus
`crates/hdc/core/tests/cognitive.rs` for cross-module behavior.

Required tests:

- `affect.rs`: `AlmaState::new()` clamps out-of-range personality PAD values;
  `mood_to_hdc()` uses the new `HdcVector::complement()` path; neutral PAD
  still produces `HdcVector::default()`.
- `somatic.rs`: `apply_somatic_bias()` honors the per-state cap from
  `StateModifiers`; rounding is verified at half increments; candidate
  filtering adjusts `knowledge::ScoredEntry::score` and tie-breaks
  deterministically.
- `replay.rs`: EVB remains two-factor; zero prediction error and zero
  recurrence produce zero; `prioritize_replay()` does not mutate its input in
  the preferred pure API; key-based entries survive store reorder/merge.
- `dream.rs`: NREM re-encoding updates both `KnowledgeEntry::key` and the
  vector index; near-duplicate merge preserves the stronger entry; Persistent
  entries are demoted rather than deleted during GC; anti-knowledge ratio above
  0.3 sets `DreamReport::alarming`; REM insights are `Transient` with
  `balance = 0.3`.
- `state_machine.rs`: alternating `Exploit` exit conditions cannot accumulate
  one another's hysteresis; alternating `Cautious` exit conditions cannot
  accumulate one another's hysteresis; `evaluate(..., current_tick)` enforces
  the 20-tick post-emergency guard; Emergency still preempts every state.
- Integration test: force `Explore -> Exploit -> Consolidate`, run
  `dream_cycle()`, feed the resulting report into `TickMetrics`, and assert
  `Consolidate -> Explore` for normal reports and `Consolidate -> Cautious`
  for alarming reports.
- Persistence test: after a dream cycle mutates a `KnowledgeStore`, reopen the
  store through the real `KnowledgeStore::open()` implementation and verify
  merged keys, tiers, balances, and REM insights survive. Mark this pending
  until persistence stops being a stub.
- Performance gate: deterministic seeded RNG, default `DreamConfig`, and 500
  entries should keep `DreamReport::duration_ms < 100` outside debug-only slow
  paths. If this is flaky under CI, make it an ignored stress test and keep a
  smaller non-ignored functional test.

### Execution Order

1. Phase 0 store/API reconciliation.
2. Phase 1 affect/PAD cleanup.
3. Phase 2 somatic scoring integration.
4. Phase 3 replay keying and EVB API cleanup.
5. Phase 4 `dream_cycle()` implementation.
6. Phase 5 state machine integration.
7. Phase 6 persistence implementation or explicit stub marking.
8. Phase 7 tests, then `cargo nextest run --workspace --all-features` and
   `cargo clippy --all-targets --all-features -- -D warnings`.

---

## Verification

### Source files

| File | Lines | Status | Role |
|------|-------|--------|------|
| `crates/hdc/core/src/cognitive/mod.rs` | ~15 | Complete | Module declarations, re-exports |
| `crates/hdc/core/src/cognitive/affect.rs` | ~250 | Complete | ALMA 3-layer PAD, `mood_to_hdc()`, `pad_similarity()` |
| `crates/hdc/core/src/cognitive/state_machine.rs` | ~500 | Complete (with F08 bug) | 6-state FSM, `StateModifiers`, `StateContext::evaluate()` |
| `crates/hdc/core/src/cognitive/somatic.rs` | ~190 | Complete | `apply_somatic_bias()`, `apply_somatic_candidate_filter()` |
| `crates/hdc/core/src/cognitive/dream.rs` | ~90 | Skeleton only | `DreamConfig`, `DreamReport` -- no `dream_cycle()` |
| `crates/hdc/core/src/cognitive/replay.rs` | ~110 | Complete | `ReplayEntry`, `mattar_daw_evb()`, `prioritize_replay()` |

### Running tests

```bash
# All cognitive tests (40+ tests across 5 modules)
cargo test -p kora-hdc-core cognitive -- --nocapture

# By module
cargo test -p kora-hdc-core cognitive::affect      # 10 tests
cargo test -p kora-hdc-core cognitive::state_machine  # 14 tests
cargo test -p kora-hdc-core cognitive::somatic      # 7 tests
cargo test -p kora-hdc-core cognitive::dream        # 2 tests (config/report only)
cargo test -p kora-hdc-core cognitive::replay       # 5 tests
```

### Actual test names

**affect.rs (10 tests):**
```
emotion_responds_fast_to_stimulus
personality_resists_stimulus
mood_is_medium_speed
pad_values_stay_clamped
neutral_mood_produces_zero_vector
positive_pleasure_produces_similar_to_basis
pad_similarity_identical_states
pad_similarity_opposite_states
pad_similarity_neutral_returns_half
```

**state_machine.rs (14 tests):**
```
starts_in_explore
emergency_is_immediate
emergency_from_catastrophic_loss
explore_to_exploit_requires_2_ticks
hysteresis_resets_when_trigger_goes_inactive
emergency_cannot_go_directly_to_explore
full_recovery_path
consolidate_normal_exit_to_explore
consolidate_alarming_to_cautious
cautious_to_exploit_requires_high_confidence_3_ticks
state_modifiers_explore
state_modifiers_emergency_halts_publication
emergency_from_any_state
```

**somatic.rs (7 tests):**
```
calm_mood_produces_minimal_bias
high_arousal_increases_bias
arousal_clamp_prevents_underflow
filter_empty_candidates
filter_zero_max_clears_candidates
high_arousal_narrows_candidates
filter_with_extreme_arousal_clamps
```

**dream.rs (2 tests):**
```
default_config_values
empty_report
```

**replay.rs (5 tests):**
```
evb_is_two_factor_product
evb_zero_prediction_error
evb_zero_recurrence
prioritize_sorts_by_descending_evb
prioritize_truncates_to_max
```

### Key implementation values

| Constant | Value | File | Notes |
|----------|-------|------|-------|
| `TAU_EMOTION` | 0.1 | `affect.rs` | 90% stimulus, 10% prior |
| `TAU_MOOD` | 0.5 | `affect.rs` | 50/50 blend |
| `TAU_PERSONALITY` | 0.9 | `affect.rs` | 10% stimulus, 90% prior |
| `PLEASURE_BASIS_SEED` | `0xCAFE_0001_0000_0001` | `affect.rs` | Consensus-critical |
| `AROUSAL_BASIS_SEED` | `0xCAFE_0001_0000_0002` | `affect.rs` | Consensus-critical |
| `DOMINANCE_BASIS_SEED` | `0xCAFE_0001_0000_0003` | `affect.rs` | Consensus-critical |
| `MAX_DRAWDOWN` | 0.10 | `state_machine.rs` | Emergency trigger |
| `CATASTROPHIC_LOSS` | 0.05 | `state_machine.rs` | Emergency trigger |
| Somatic bias max strength | 30% | `somatic.rs` | `bias_count` max 30 of 100 |
| DreamConfig `max_nrem_episodes` | 500 | `dream.rs` | Default |
| DreamConfig `rem_cross_bindings` | 100 | `dream.rs` | Default |
| DreamConfig `merge_threshold` | 0.95 | `dream.rs` | Near-duplicate merge |
| DreamConfig `resonance_threshold` | 0.65 | `dream.rs` | REM resonance detection |
| DreamConfig `gc_threshold` | 0.01 | `dream.rs` | GC retention threshold |
