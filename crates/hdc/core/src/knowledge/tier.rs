//! Knowledge retention tiers with promotion/demotion logic.

/// Retention tier. Higher tiers decay more slowly (larger half-life
/// multiplier) and require more confirmations to reach.
///
/// Ordering: Transient < Working < Consolidated < Persistent.
/// The derived PartialOrd is correct because the variants are declared
/// in ascending order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KnowledgeTier {
    /// Multiplier: 0.1x. Default after creation. Promote at 3 confirmations.
    Transient,
    /// Multiplier: 0.5x. Promote at 10 confirmations.
    Working,
    /// Multiplier: 1.0x. Promote at 25 confirmations.
    Consolidated,
    /// Multiplier: 5.0x. Never deleted; demoted to Consolidated if balance < 0.01.
    Persistent,
}

impl KnowledgeTier {
    /// Half-life multiplier. Effective half-life = base_half_life * multiplier().
    pub fn multiplier(&self) -> f64 {
        match self {
            KnowledgeTier::Transient => 0.1,
            KnowledgeTier::Working => 0.5,
            KnowledgeTier::Consolidated => 1.0,
            KnowledgeTier::Persistent => 5.0,
        }
    }

    /// Scoring weight used in the 4-factor importance calculation.
    pub fn weight(&self) -> f64 {
        match self {
            KnowledgeTier::Transient => 0.2,
            KnowledgeTier::Working => 0.4,
            KnowledgeTier::Consolidated => 0.7,
            KnowledgeTier::Persistent => 1.0,
        }
    }

    /// Attempt promotion based on confirmation count.
    /// Returns `Some(new_tier)` if the confirmation threshold is met,
    /// `None` otherwise.
    pub fn try_promote(&self, confirmations: u32) -> Option<KnowledgeTier> {
        match self {
            KnowledgeTier::Transient if confirmations >= 3 => Some(KnowledgeTier::Working),
            KnowledgeTier::Working if confirmations >= 10 => Some(KnowledgeTier::Consolidated),
            KnowledgeTier::Consolidated if confirmations >= 25 => Some(KnowledgeTier::Persistent),
            _ => None,
        }
    }

    /// Demote one level. Transient cannot be demoted further.
    pub fn demote(&self) -> KnowledgeTier {
        match self {
            KnowledgeTier::Persistent => KnowledgeTier::Consolidated,
            KnowledgeTier::Consolidated => KnowledgeTier::Working,
            KnowledgeTier::Working => KnowledgeTier::Transient,
            KnowledgeTier::Transient => KnowledgeTier::Transient,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multiplier_values() {
        assert_eq!(KnowledgeTier::Transient.multiplier(), 0.1);
        assert_eq!(KnowledgeTier::Working.multiplier(), 0.5);
        assert_eq!(KnowledgeTier::Consolidated.multiplier(), 1.0);
        assert_eq!(KnowledgeTier::Persistent.multiplier(), 5.0);
    }

    #[test]
    fn test_weight_values() {
        assert_eq!(KnowledgeTier::Transient.weight(), 0.2);
        assert_eq!(KnowledgeTier::Working.weight(), 0.4);
        assert_eq!(KnowledgeTier::Consolidated.weight(), 0.7);
        assert_eq!(KnowledgeTier::Persistent.weight(), 1.0);
    }

    #[test]
    fn test_try_promote() {
        // Below threshold: no promotion
        assert_eq!(KnowledgeTier::Transient.try_promote(2), None);
        assert_eq!(KnowledgeTier::Working.try_promote(9), None);
        assert_eq!(KnowledgeTier::Consolidated.try_promote(24), None);
        assert_eq!(KnowledgeTier::Persistent.try_promote(100), None);

        // At threshold: promote
        assert_eq!(KnowledgeTier::Transient.try_promote(3), Some(KnowledgeTier::Working));
        assert_eq!(KnowledgeTier::Working.try_promote(10), Some(KnowledgeTier::Consolidated));
        assert_eq!(KnowledgeTier::Consolidated.try_promote(25), Some(KnowledgeTier::Persistent));

        // Above threshold: also promotes
        assert_eq!(KnowledgeTier::Transient.try_promote(5), Some(KnowledgeTier::Working));
    }

    #[test]
    fn test_demote() {
        assert_eq!(KnowledgeTier::Persistent.demote(), KnowledgeTier::Consolidated);
        assert_eq!(KnowledgeTier::Consolidated.demote(), KnowledgeTier::Working);
        assert_eq!(KnowledgeTier::Working.demote(), KnowledgeTier::Transient);
        assert_eq!(KnowledgeTier::Transient.demote(), KnowledgeTier::Transient);
    }

    #[test]
    fn test_tier_ordering() {
        assert!(KnowledgeTier::Transient < KnowledgeTier::Working);
        assert!(KnowledgeTier::Working < KnowledgeTier::Consolidated);
        assert!(KnowledgeTier::Consolidated < KnowledgeTier::Persistent);
    }
}
