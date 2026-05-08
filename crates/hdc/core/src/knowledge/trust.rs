//! 5-stage trust pipeline for shared substrate knowledge.

use super::{decay::ticks_to_hours, entry::KnowledgeEntry};
use crate::{HdcVector, constants::D, hamming_distance};

/// Cold-start reputation floor for new/unknown agents.
pub const COLD_START_REPUTATION: f64 = 0.1;

/// Minimum trust below which knowledge is not admitted to context.
pub const MIN_TRUST_THRESHOLD: f64 = 0.05;

/// Compute the effective trust score for a shared substrate entry.
///
/// Five multiplicative stages:
///   1. Author reputation (floor 0.1)
///   2. Freshness decay
///   3. Confirmation boost (floor 0.5)
///   4. Stake weight (placeholder, floor 0.5)
///   5. Relevance gating (can produce 0.0)
pub fn compute_trust(
    entry: &KnowledgeEntry,
    source_reputation: f64,
    current_tick: u64,
    tick_duration_ms: u64,
    query_vector: &HdcVector,
) -> f64 {
    let mut trust = 1.0;

    // Stage 1: Author Reputation
    let rep = source_reputation.max(COLD_START_REPUTATION);
    trust *= rep;

    // Stage 2: Freshness Decay
    let age_ticks = current_tick.saturating_sub(entry.last_accessed);
    let age_hours = ticks_to_hours(age_ticks, tick_duration_ms);
    let effective_hl = entry.kind.base_half_life_hours() * entry.tier.multiplier();
    let freshness = (-0.693 * age_hours / effective_hl).exp();
    trust *= freshness;

    // Stage 3: Confirmation Boost
    let conf_boost = (0.5 + 0.05 * entry.confirmations as f64).min(1.0);
    trust *= conf_boost;

    // Stage 4: Stake Weighting (placeholder)
    let stake_weight = 0.5_f64;
    trust *= stake_weight.max(0.5).min(1.0);

    // Stage 5: Relevance Gating
    let dist = hamming_distance(&entry.vector, query_vector);
    let sim = 1.0 - (dist as f64 / D as f64);
    let relevance = ((sim - 0.5) * 2.0).max(0.0);
    trust *= relevance;

    trust.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        super::{KnowledgeKind, KnowledgeSource, entry::KnowledgeEntry},
        *,
    };
    use crate::HdcVector;

    fn make_entry(vector: HdcVector, tick: u64) -> KnowledgeEntry {
        KnowledgeEntry::new(
            vector,
            KnowledgeKind::Insight,
            "test".to_string(),
            KnowledgeSource::SelfDerived,
            tick,
        )
    }

    #[test]
    fn test_cold_start_floor() {
        let v = HdcVector::random(1);
        let entry = make_entry(v.clone(), 0);
        // With reputation=0.0, should be floored to 0.1
        let trust = compute_trust(&entry, 0.0, 0, 400, &v);
        // trust = 1.0 * 0.1 * 1.0 * 0.5 * 0.5 * relevance(1.0 -> 1.0)
        assert!(trust > 0.0, "cold start should still produce nonzero trust");
    }

    #[test]
    fn test_trust_zero_relevance() {
        let v = HdcVector::random(1);
        // Use a maximally different query (all ones vs v)
        let mut opposite = HdcVector::default();
        for i in 0..160 {
            opposite.0[i] = !v.0[i];
        }
        let entry = make_entry(v, 0);
        let trust = compute_trust(&entry, 1.0, 0, 400, &opposite);
        // Similarity ~0.0 -> relevance = max(0, (0.0 - 0.5) * 2) = 0.0
        assert!(trust < 0.001, "irrelevant knowledge should have ~0 trust, got {trust}");
    }

    #[test]
    fn test_trust_clamp() {
        let v = HdcVector::random(1);
        let entry = make_entry(v.clone(), 0);
        let trust = compute_trust(&entry, 1.0, 0, 400, &v);
        assert!(trust >= 0.0 && trust <= 1.0, "trust should be in [0,1], got {trust}");
    }
}
