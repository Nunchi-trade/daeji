//! Knowledge source classification for trust weighting.

/// Where a piece of knowledge came from. Used for trust weighting
/// in the five-stage trust pipeline.
#[derive(Clone, Debug, PartialEq)]
pub enum KnowledgeSource {
    /// Derived from the agent's own observations or reasoning.
    SelfDerived,
    /// Received from the shared on-chain substrate.
    SharedSubstrate {
        /// The publisher agent's 32-byte identifier.
        publisher: [u8; 32],
    },
    /// Confirmed by multiple external agents.
    MultiAgentConfirmed {
        /// Number of confirming agents.
        count: u32,
    },
    /// Received from a single external agent.
    SingleAgent {
        /// The source agent's 32-byte identifier.
        agent_id: [u8; 32],
    },
    /// Anonymous on-chain submission (lowest trust).
    Anonymous,
}
