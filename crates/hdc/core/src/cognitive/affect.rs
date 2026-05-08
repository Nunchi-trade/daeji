//! ALMA (A Layered Model of Affect) — three-layer affective state in PAD space.
//!
//! Three temporal layers, each a point in Pleasure-Arousal-Dominance space:
//! - `emotion` (tau=0.1): fast, volatile, triggered by individual events.
//! - `mood`    (tau=0.5): medium, running average of recent emotions.
//! - `personality` (tau=0.9): slow, nearly stable baseline.

use std::sync::LazyLock;

use crate::{BundleAccumulator, HdcVector};

// ─── PAD State ──────────────────────────────────────────────────────────────

/// A point in Pleasure-Arousal-Dominance space.
/// Each coordinate is clamped to [-1.0, 1.0].
#[derive(Clone, Debug, PartialEq)]
pub struct PadState {
    /// Pleasure dimension in `[-1.0, 1.0]`.
    pub pleasure: f64,
    /// Arousal dimension in `[-1.0, 1.0]`.
    pub arousal: f64,
    /// Dominance dimension in `[-1.0, 1.0]`.
    pub dominance: f64,
}

impl PadState {
    /// Create a neutral PAD state at the origin (0, 0, 0).
    pub const fn neutral() -> Self {
        Self { pleasure: 0.0, arousal: 0.0, dominance: 0.0 }
    }

    /// Clamp all dimensions to [-1, 1].
    pub const fn clamp(&mut self) {
        self.pleasure = self.pleasure.clamp(-1.0, 1.0);
        self.arousal = self.arousal.clamp(-1.0, 1.0);
        self.dominance = self.dominance.clamp(-1.0, 1.0);
    }
}

// ─── ALMA State ─────────────────────────────────────────────────────────────

/// Time constants for each ALMA layer.
const TAU_EMOTION: f64 = 0.1;
const TAU_MOOD: f64 = 0.5;
const TAU_PERSONALITY: f64 = 0.9;

/// Three-layer affective state (ALMA model, Gebhard 2005).
#[derive(Clone, Debug)]
pub struct AlmaState {
    /// Fast-decaying emotion layer (tau=0.1).
    pub emotion: PadState,
    /// Medium-speed mood layer (tau=0.5).
    pub mood: PadState,
    /// Slow personality baseline (tau=0.9).
    pub personality: PadState,
}

impl AlmaState {
    /// Create a new ALMA state with given personality baseline.
    pub const fn new(personality: PadState) -> Self {
        Self { emotion: PadState::neutral(), mood: PadState::neutral(), personality }
    }

    /// Apply a stimulus to all three layers.
    /// Each layer blends the stimulus with its current state according to tau.
    pub fn update(&mut self, stimulus: &PadState) {
        self.emotion = blend(stimulus, &self.emotion, TAU_EMOTION);
        self.mood = blend(stimulus, &self.mood, TAU_MOOD);
        self.personality = blend(stimulus, &self.personality, TAU_PERSONALITY);
    }

    /// Read the mood layer. Primary input for somatic bias and state decisions.
    pub const fn mood(&self) -> &PadState {
        &self.mood
    }
}

/// Exponential blend: `result = (1 - tau) * stimulus + tau * current`.
fn blend(stimulus: &PadState, current: &PadState, tau: f64) -> PadState {
    let inv = 1.0 - tau;
    let mut out = PadState {
        pleasure: inv * stimulus.pleasure + tau * current.pleasure,
        arousal: inv * stimulus.arousal + tau * current.arousal,
        dominance: inv * stimulus.dominance + tau * current.dominance,
    };
    out.clamp();
    out
}

// ─── PAD Basis Vectors ──────────────────────────────────────────────────────

const PLEASURE_BASIS_SEED: u64 = 0xCAFE_0001_0000_0001;
const AROUSAL_BASIS_SEED: u64 = 0xCAFE_0001_0000_0002;
const DOMINANCE_BASIS_SEED: u64 = 0xCAFE_0001_0000_0003;

static PLEASURE_BASIS: LazyLock<HdcVector> =
    LazyLock::new(|| HdcVector::random(PLEASURE_BASIS_SEED));
static AROUSAL_BASIS: LazyLock<HdcVector> = LazyLock::new(|| HdcVector::random(AROUSAL_BASIS_SEED));
static DOMINANCE_BASIS: LazyLock<HdcVector> =
    LazyLock::new(|| HdcVector::random(DOMINANCE_BASIS_SEED));

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
    let mut total_added = 0usize;

    let dims: [(f64, &HdcVector); 3] = [
        (pad.pleasure, &*PLEASURE_BASIS),
        (pad.arousal, &*AROUSAL_BASIS),
        (pad.dominance, &*DOMINANCE_BASIS),
    ];

    for (value, basis) in &dims {
        let weight = (value.abs() * WEIGHT_SCALE).round() as usize;
        if weight == 0 {
            continue;
        }

        if *value > 0.0 {
            for _ in 0..weight {
                acc.add(basis);
            }
        } else {
            let neg = complement(basis);
            for _ in 0..weight {
                acc.add(&neg);
            }
        }
        total_added += weight;
    }

    if total_added == 0 {
        return HdcVector::default();
    }

    acc.to_vector()
}

/// Bitwise complement of an HDC vector.
fn complement(v: &HdcVector) -> HdcVector {
    let mut words = [0u64; crate::constants::WORDS];
    for (i, w) in v.0.iter().enumerate() {
        words[i] = !w;
    }
    HdcVector(words)
}

/// Cosine similarity between two PAD states, rescaled to [0, 1].
/// Returns 0.5 when either input is neutral (zero magnitude).
pub fn pad_similarity(a: &PadState, b: &PadState) -> f64 {
    let dot = a.pleasure * b.pleasure + a.arousal * b.arousal + a.dominance * b.dominance;
    let mag_a = (a.pleasure.powi(2) + a.arousal.powi(2) + a.dominance.powi(2)).sqrt();
    let mag_b = (b.pleasure.powi(2) + b.arousal.powi(2) + b.dominance.powi(2)).sqrt();

    if mag_a < 1e-9 || mag_b < 1e-9 {
        return 0.5;
    }

    let cosine = dot / (mag_a * mag_b);
    (cosine + 1.0) / 2.0
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(v, HdcVector::default());
    }

    #[test]
    fn positive_pleasure_produces_similar_to_basis() {
        let v = mood_to_hdc(&PadState { pleasure: 0.8, arousal: 0.0, dominance: 0.0 });
        let sim = crate::similarity(&v, &*PLEASURE_BASIS);
        assert!(sim > 0.7, "similarity to pleasure basis should be high, got {sim}");
    }

    #[test]
    fn pad_similarity_identical_states() {
        let a = PadState { pleasure: 0.5, arousal: 0.3, dominance: -0.2 };
        let sim = pad_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pad_similarity_opposite_states() {
        let a = PadState { pleasure: 1.0, arousal: 0.0, dominance: 0.0 };
        let b = PadState { pleasure: -1.0, arousal: 0.0, dominance: 0.0 };
        let sim = pad_similarity(&a, &b);
        assert!(sim < 0.01, "opposite states should have ~0 similarity, got {sim}");
    }

    #[test]
    fn pad_similarity_neutral_returns_half() {
        let a = PadState { pleasure: 0.5, arousal: 0.3, dominance: 0.1 };
        let b = PadState::neutral();
        assert!((pad_similarity(&a, &b) - 0.5).abs() < 1e-9);
    }
}
