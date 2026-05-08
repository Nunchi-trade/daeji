//! Knowledge kind classification with decay half-lives.

/// The six kinds of knowledge an agent can store.
/// Each kind has a different base half-life (in hours) that controls how
/// fast it decays via demurrage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KnowledgeKind {
    /// Derived conclusion from observations.
    Insight,
    /// Rule of thumb: "in situation X, do Y."
    Heuristic,
    /// "X causes Y" -- directional causal relationship.
    CausalLink,
    /// Temporary caution signal.
    Warning,
    /// Partial plan or tactic -- composable building block.
    StrategyFragment,
    /// "This is false/harmful" -- meta-cognitive negation.
    AntiKnowledge,
}

impl KnowledgeKind {
    /// Base half-life in hours. This is the half-life at the Consolidated
    /// tier (multiplier 1.0x). Other tiers scale this value.
    pub fn base_half_life_hours(&self) -> f64 {
        match self {
            KnowledgeKind::Insight => 72.0,
            KnowledgeKind::Heuristic => 168.0,
            KnowledgeKind::CausalLink => 240.0,
            KnowledgeKind::Warning => 48.0,
            KnowledgeKind::StrategyFragment => 96.0,
            KnowledgeKind::AntiKnowledge => 336.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_half_life_values() {
        assert_eq!(KnowledgeKind::Insight.base_half_life_hours(), 72.0);
        assert_eq!(KnowledgeKind::Heuristic.base_half_life_hours(), 168.0);
        assert_eq!(KnowledgeKind::CausalLink.base_half_life_hours(), 240.0);
        assert_eq!(KnowledgeKind::Warning.base_half_life_hours(), 48.0);
        assert_eq!(KnowledgeKind::StrategyFragment.base_half_life_hours(), 96.0);
        assert_eq!(KnowledgeKind::AntiKnowledge.base_half_life_hours(), 336.0);
    }
}
