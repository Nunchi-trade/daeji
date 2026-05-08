//! Four-factor scoring for knowledge retrieval.
//!
//! Results are ranked by a composite score combining relevance, recency,
//! importance, and emotional resonance.

use super::entry::KnowledgeEntry;
use crate::{
    cognitive::affect::{PadState, pad_similarity},
    constants::D,
};

/// Controls the half-life of the recency exponential decay.
/// At RECENCY_DECAY ticks ago, recency score = 1/e = 0.368.
pub const RECENCY_DECAY: f64 = 100.0;

/// Maximum expected importance value, used for normalization.
/// Corresponds to confidence=1.0, tier_weight=1.0, ln(1025)=6.93.
const IMPORTANCE_NORMALIZER: f64 = 7.0;

/// Default scoring weights.
const W_RELEVANCE: f64 = 0.35;
const W_RECENCY: f64 = 0.20;
const W_IMPORTANCE: f64 = 0.25;
const W_EMOTIONAL: f64 = 0.20;

/// Context passed into search/scoring. Captures the agent's current state.
#[derive(Debug)]
pub struct RetrievalContext {
    /// Current tick number (block height or logical clock).
    pub current_tick: u64,
    /// Agent's current PAD emotional state for emotional resonance scoring.
    pub mood: PadState,
    /// Tick duration in milliseconds (for tick-to-hours conversion).
    pub tick_duration_ms: u64,
}

/// A knowledge entry with its computed composite score.
#[derive(Clone, Debug)]
pub struct ScoredEntry {
    /// The entry's key.
    pub key: [u8; 32],
    /// The full knowledge entry.
    pub entry: KnowledgeEntry,
    /// Composite score in [0.0, 1.0].
    pub score: f64,
    /// Raw Hamming distance from the query vector.
    pub hamming_distance: u32,
}

/// Compute the four-factor composite score for a knowledge entry.
///
/// Factors and weights:
///   - relevance  (0.35): `1.0 - hamming_distance / D`
///   - recency    (0.20): `exp(-age_ticks / RECENCY_DECAY)`
///   - importance (0.25): `balance * tier_weight * ln(confirmations + 1)`, normalized
///   - emotional  (0.20): PAD similarity between entry's emotional tag and query mood
pub fn compute_score(entry: &KnowledgeEntry, hamming: u32, ctx: &RetrievalContext) -> f64 {
    // Factor 1: Relevance
    let relevance = 1.0 - (hamming as f64 / D as f64);

    // Factor 2: Recency
    let age = ctx.current_tick.saturating_sub(entry.last_accessed) as f64;
    let recency = (-age / RECENCY_DECAY).exp();

    // Factor 3: Importance (normalized to [0, 1])
    let raw_importance =
        entry.balance * entry.tier.weight() * (entry.confirmations as f64 + 1.0).ln();
    let importance = (raw_importance / IMPORTANCE_NORMALIZER).min(1.0);

    // Factor 4: Emotional resonance
    let emotional = pad_similarity(&entry.emotional_tag, &ctx.mood);

    W_RELEVANCE * relevance
        + W_RECENCY * recency
        + W_IMPORTANCE * importance
        + W_EMOTIONAL * emotional
}

#[cfg(test)]
mod tests {
    use super::{
        super::{KnowledgeKind, KnowledgeSource, KnowledgeTier},
        *,
    };
    use crate::HdcVector;

    fn make_ctx(tick: u64) -> RetrievalContext {
        RetrievalContext { current_tick: tick, mood: PadState::neutral(), tick_duration_ms: 400 }
    }

    fn make_entry(tick: u64) -> KnowledgeEntry {
        KnowledgeEntry::new(
            HdcVector::random(1),
            KnowledgeKind::Insight,
            "test".to_string(),
            KnowledgeSource::SelfDerived,
            tick,
        )
    }

    #[test]
    fn test_score_weights_sum_to_one() {
        let sum: f64 = W_RELEVANCE + W_RECENCY + W_IMPORTANCE + W_EMOTIONAL;
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_score_perfect_entry() {
        let mut entry = make_entry(100);
        entry.balance = 1.0;
        entry.confirmations = 1000;
        entry.tier = KnowledgeTier::Persistent;

        let ctx = make_ctx(100);
        let score = compute_score(&entry, 0, &ctx);
        // With hamming=0 (relevance=1.0), age=0 (recency=1.0), high importance,
        // neutral mood matching neutral emotional_tag (emotional=0.5)
        assert!(score > 0.7, "perfect entry should score high, got {score}");
    }

    #[test]
    fn test_score_relevance_dominates() {
        let ctx = make_ctx(100);
        let entry = make_entry(100);

        let score_close = compute_score(&entry, 100, &ctx);
        let score_far = compute_score(&entry, 5000, &ctx);
        assert!(score_close > score_far, "closer should score higher");
    }

    #[test]
    fn test_emotional_resonance_boosts_score() {
        let entry = make_entry(100).with_emotional_tag(PadState {
            pleasure: 0.8,
            arousal: 0.3,
            dominance: 0.1,
        });

        // Matching mood should score higher than neutral
        let matching_ctx = RetrievalContext {
            current_tick: 100,
            mood: PadState { pleasure: 0.8, arousal: 0.3, dominance: 0.1 },
            tick_duration_ms: 400,
        };
        let neutral_ctx = make_ctx(100);

        let score_match = compute_score(&entry, 100, &matching_ctx);
        let score_neutral = compute_score(&entry, 100, &neutral_ctx);
        assert!(score_match > score_neutral, "matching mood should score higher");
    }
}
