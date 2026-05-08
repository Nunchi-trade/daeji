//! Decay / demurrage math for knowledge balances.

use super::{KnowledgeKind, KnowledgeTier};

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

/// Convert elapsed ticks to hours.
/// `tick_duration_ms` is the block time in milliseconds.
pub fn ticks_to_hours(elapsed_ticks: u64, tick_duration_ms: u64) -> f64 {
    (elapsed_ticks as f64 * tick_duration_ms as f64) / 3_600_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decay_one_half_life() {
        // Insight at Transient: effective half-life = 72 * 0.1 = 7.2 hours
        let balance =
            compute_decayed_balance(1.0, &KnowledgeKind::Insight, &KnowledgeTier::Transient, 7.2);
        assert!((balance - 0.5).abs() < 1e-6, "expected ~0.5, got {balance}");
    }

    #[test]
    fn test_decay_gc_threshold() {
        // Transient Insight should reach GC threshold (~0.01) around 48 hours
        let balance =
            compute_decayed_balance(1.0, &KnowledgeKind::Insight, &KnowledgeTier::Transient, 48.0);
        assert!(balance < GC_THRESHOLD, "expected < 0.01, got {balance}");
    }

    #[test]
    fn test_decay_zero_elapsed() {
        let balance =
            compute_decayed_balance(0.75, &KnowledgeKind::Heuristic, &KnowledgeTier::Working, 0.0);
        assert!((balance - 0.75).abs() < 1e-10);
    }

    #[test]
    fn test_decay_clamp() {
        // Balance should never exceed 1.0 or drop below 0.0
        let balance =
            compute_decayed_balance(1.0, &KnowledgeKind::Insight, &KnowledgeTier::Persistent, 0.0);
        assert!(balance <= 1.0);
        assert!(balance >= 0.0);

        let balance = compute_decayed_balance(
            1.0,
            &KnowledgeKind::Warning,
            &KnowledgeTier::Transient,
            10000.0,
        );
        assert!(balance >= 0.0);
    }

    #[test]
    fn test_ticks_to_hours() {
        // 9000 ticks at 400 ms/tick = 3_600_000 ms = 1.0 hours
        let hours = ticks_to_hours(9000, 400);
        assert!((hours - 1.0).abs() < 1e-10);
    }
}
