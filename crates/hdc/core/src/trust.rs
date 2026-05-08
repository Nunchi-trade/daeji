//! Trust pipeline, reputation registry, taint tracking, and cognitive immune system.
//!
//! **OFF-CHAIN ONLY** — all computations use `f64`. None of this is
//! consensus-critical. If any piece is ever moved on-chain, replace all
//! `f64` with fixed-point integer arithmetic.

use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Floor reputation for agents with no on-chain history.
pub const COLD_START_REPUTATION: f64 = 0.1;

/// Minimum trust score to be considered "trusted".
pub const MIN_TRUST_THRESHOLD: f64 = 0.05;

/// EMA smoothing factor — slow learning.
const EMA_ALPHA: f64 = 0.1;

// ---------------------------------------------------------------------------
// TrustOutcome
// ---------------------------------------------------------------------------

/// Outcome of an interaction used to update an agent's trust score.
#[derive(Debug, Clone, Copy)]
pub enum TrustOutcome {
    /// Successful interaction with a weight factor.
    Positive(f64),
    /// Failed or malicious interaction with a weight factor.
    Negative(f64),
    /// Insight was challenged by another agent.
    Challenge,
    /// Insight was accepted into the knowledge base.
    Accepted,
}

// ---------------------------------------------------------------------------
// TrustScore
// ---------------------------------------------------------------------------

/// Per-agent trust score with domain-level breakdown.
#[derive(Debug, Clone)]
pub struct TrustScore {
    /// Global reputation in `[0.0, 1.0]`.
    pub reputation: f64,
    /// Per-domain reputation scores.
    pub domain_scores: HashMap<String, f64>,
    /// Total number of interactions observed.
    pub total_interactions: u64,
    /// Number of successful (positive) interactions.
    pub successful_interactions: u64,
}

impl TrustScore {
    fn new() -> Self {
        Self {
            reputation: COLD_START_REPUTATION,
            domain_scores: HashMap::new(),
            total_interactions: 0,
            successful_interactions: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// TrustRegistry
// ---------------------------------------------------------------------------

/// Registry mapping agent addresses to their trust scores.
#[derive(Debug)]
pub struct TrustRegistry {
    /// Map from 20-byte agent address to trust score.
    agents: HashMap<[u8; 20], TrustScore>,
}

impl TrustRegistry {
    /// Create a new empty trust registry.
    pub fn new() -> Self {
        Self { agents: HashMap::new() }
    }

    /// Returns the agent's reputation, or [`COLD_START_REPUTATION`] if unknown.
    pub fn get_trust(&self, agent: &[u8; 20]) -> f64 {
        self.agents.get(agent).map(|s| s.reputation).unwrap_or(COLD_START_REPUTATION)
    }

    /// Update an agent's trust score based on an interaction outcome.
    ///
    /// Uses exponential moving average:
    ///   `new_trust = alpha * outcome_score + (1 - alpha) * old_trust`
    pub fn update_trust(&mut self, agent: &[u8; 20], outcome: TrustOutcome) {
        let score = self.agents.entry(*agent).or_insert_with(TrustScore::new);

        let old_trust = score.reputation;

        let outcome_score = match outcome {
            TrustOutcome::Positive(weight) => {
                score.total_interactions += 1;
                score.successful_interactions += 1;
                (old_trust + weight * 0.1).min(1.0)
            }
            TrustOutcome::Negative(weight) => {
                score.total_interactions += 1;
                (old_trust - weight * 0.2).max(0.0)
            }
            TrustOutcome::Challenge => {
                score.total_interactions += 1;
                // Challenge is a mild negative signal.
                (old_trust - 0.05).max(0.0)
            }
            TrustOutcome::Accepted => {
                score.total_interactions += 1;
                score.successful_interactions += 1;
                // Acceptance is a mild positive signal.
                (old_trust + 0.05).min(1.0)
            }
        };

        // EMA update: new = alpha * outcome + (1 - alpha) * old
        score.reputation = EMA_ALPHA * outcome_score + (1.0 - EMA_ALPHA) * old_trust;
    }

    /// Returns `true` if the agent's trust is at or above [`MIN_TRUST_THRESHOLD`].
    pub fn is_trusted(&self, agent: &[u8; 20]) -> bool {
        self.get_trust(agent) >= MIN_TRUST_THRESHOLD
    }
}

impl Default for TrustRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TrustCandidate / TrustDecision — pipeline input/output
// ---------------------------------------------------------------------------

/// A candidate entry to be evaluated by the trust pipeline.
#[derive(Debug)]
pub struct TrustCandidate {
    /// Address of the agent that produced this entry.
    pub author: [u8; 20],
    /// Unique identifier for the content (e.g. vector ID).
    pub content_id: [u8; 32],
    /// Size of the payload in bytes.
    pub payload_size: usize,
    /// Whether this entry has been verified on-chain.
    pub on_chain_verified: bool,
}

/// Decision produced by the trust pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum TrustDecision {
    /// Entry is accepted into the knowledge base.
    Accept,
    /// Entry is held for further review.
    Quarantine(String),
    /// Entry is rejected outright.
    Reject(String),
}

// ---------------------------------------------------------------------------
// TrustPipeline — 5-layer immune system (simplified)
// ---------------------------------------------------------------------------

/// Simplified 5-layer cognitive immune system.
///
/// - Layer 1 (Innate): Basic format/size validation
/// - Layer 2 (Adaptive): Trust score check
/// - Layer 3 (Social): Source reputation
/// - Layer 4 (Consensus): On-chain verification status
/// - Layer 5 (Meta): Cross-layer consistency
#[derive(Debug)]
pub struct TrustPipeline {
    /// The backing trust registry.
    pub registry: TrustRegistry,
    /// Maximum allowed payload size in bytes.
    max_payload_size: usize,
    /// Minimum reputation required to bypass quarantine.
    quarantine_threshold: f64,
}

impl TrustPipeline {
    /// Create a new trust pipeline with the given registry and default thresholds.
    pub const fn new(registry: TrustRegistry) -> Self {
        Self {
            registry,
            max_payload_size: 1_048_576, // 1 MiB default
            quarantine_threshold: 0.3,
        }
    }

    /// Run the 5-layer immune system on a candidate entry.
    pub fn validate(&self, entry: &TrustCandidate) -> TrustDecision {
        // Layer 1 — Innate: basic format/size validation
        if entry.payload_size == 0 {
            return TrustDecision::Reject("empty payload".into());
        }
        if entry.payload_size > self.max_payload_size {
            return TrustDecision::Reject("payload exceeds max size".into());
        }

        // Layer 2 — Adaptive: trust score check
        let trust = self.registry.get_trust(&entry.author);
        if trust < MIN_TRUST_THRESHOLD {
            return TrustDecision::Reject(format!(
                "trust {trust:.4} below minimum {MIN_TRUST_THRESHOLD}"
            ));
        }

        // Layer 3 — Social: source reputation
        if trust < self.quarantine_threshold {
            return TrustDecision::Quarantine(format!(
                "low reputation {trust:.4}, requires review"
            ));
        }

        // Layer 4 — Consensus: on-chain verification
        if !entry.on_chain_verified {
            return TrustDecision::Quarantine("not yet verified on-chain".into());
        }

        // Layer 5 — Meta: cross-layer consistency
        // All layers passed — accept.
        TrustDecision::Accept
    }
}

// ---------------------------------------------------------------------------
// TaintTracker
// ---------------------------------------------------------------------------

/// Tracks tainted vector IDs and propagates taint through derivation links.
#[derive(Debug)]
pub struct TaintTracker {
    /// Set of tainted 32-byte vector IDs.
    tainted: HashSet<[u8; 32]>,
}

impl TaintTracker {
    /// Create a new empty taint tracker.
    pub fn new() -> Self {
        Self { tainted: HashSet::new() }
    }

    /// Mark a vector ID as tainted.
    pub fn taint(&mut self, id: [u8; 32]) {
        self.tainted.insert(id);
    }

    /// Check whether a vector ID is tainted.
    pub fn is_tainted(&self, id: &[u8; 32]) -> bool {
        self.tainted.contains(id)
    }

    /// Propagate taint: if `from` is tainted, mark `to` as tainted too.
    pub fn propagate(&mut self, from: &[u8; 32], to: &[u8; 32]) {
        if self.tainted.contains(from) {
            self.tainted.insert(*to);
        }
    }
}

impl Default for TaintTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_addr(byte: u8) -> [u8; 20] {
        [byte; 20]
    }

    fn vector_id(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    // -- TrustRegistry --

    #[test]
    fn cold_start_reputation() {
        let registry = TrustRegistry::new();
        let unknown = agent_addr(0xFF);
        assert_eq!(registry.get_trust(&unknown), COLD_START_REPUTATION);
    }

    #[test]
    fn cold_start_is_trusted() {
        let registry = TrustRegistry::new();
        let unknown = agent_addr(0xAA);
        // COLD_START_REPUTATION (0.1) >= MIN_TRUST_THRESHOLD (0.05)
        assert!(registry.is_trusted(&unknown));
    }

    #[test]
    fn trust_update_positive() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x01);

        let before = registry.get_trust(&agent);
        registry.update_trust(&agent, TrustOutcome::Positive(1.0));
        let after = registry.get_trust(&agent);

        assert!(after > before, "positive outcome should increase trust");
    }

    #[test]
    fn trust_update_negative() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x02);

        // First give some positive trust so there's room to decrease.
        for _ in 0..5 {
            registry.update_trust(&agent, TrustOutcome::Positive(1.0));
        }
        let before = registry.get_trust(&agent);

        registry.update_trust(&agent, TrustOutcome::Negative(1.0));
        let after = registry.get_trust(&agent);

        assert!(after < before, "negative outcome should decrease trust");
    }

    #[test]
    fn trust_clamped_to_valid_range() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x03);

        // Many positive outcomes should not exceed 1.0.
        for _ in 0..100 {
            registry.update_trust(&agent, TrustOutcome::Positive(10.0));
        }
        assert!(registry.get_trust(&agent) <= 1.0);

        // Many negative outcomes should not go below 0.0.
        let agent2 = agent_addr(0x04);
        for _ in 0..100 {
            registry.update_trust(&agent2, TrustOutcome::Negative(10.0));
        }
        assert!(registry.get_trust(&agent2) >= 0.0);
    }

    #[test]
    fn min_trust_threshold_enforcement() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x05);

        // Drive trust down below threshold.
        for _ in 0..200 {
            registry.update_trust(&agent, TrustOutcome::Negative(1.0));
        }

        assert!(!registry.is_trusted(&agent));
        assert!(registry.get_trust(&agent) < MIN_TRUST_THRESHOLD);
    }

    #[test]
    fn trust_update_challenge_and_accepted() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x06);

        let initial = registry.get_trust(&agent);

        registry.update_trust(&agent, TrustOutcome::Accepted);
        let after_accept = registry.get_trust(&agent);
        assert!(after_accept > initial, "accepted should increase trust");

        registry.update_trust(&agent, TrustOutcome::Challenge);
        let after_challenge = registry.get_trust(&agent);
        assert!(after_challenge < after_accept, "challenge should decrease trust");
    }

    #[test]
    fn interaction_counts() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x07);

        registry.update_trust(&agent, TrustOutcome::Positive(1.0));
        registry.update_trust(&agent, TrustOutcome::Negative(0.5));
        registry.update_trust(&agent, TrustOutcome::Accepted);

        let score = registry.agents.get(&agent).unwrap();
        assert_eq!(score.total_interactions, 3);
        assert_eq!(score.successful_interactions, 2); // Positive + Accepted
    }

    // -- TaintTracker --

    #[test]
    fn taint_basic() {
        let mut tracker = TaintTracker::new();
        let id = vector_id(0x01);

        assert!(!tracker.is_tainted(&id));
        tracker.taint(id);
        assert!(tracker.is_tainted(&id));
    }

    #[test]
    fn taint_propagation() {
        let mut tracker = TaintTracker::new();
        let parent = vector_id(0x01);
        let child = vector_id(0x02);
        let grandchild = vector_id(0x03);

        tracker.taint(parent);

        // Propagate parent -> child.
        tracker.propagate(&parent, &child);
        assert!(tracker.is_tainted(&child));

        // Propagate child -> grandchild.
        tracker.propagate(&child, &grandchild);
        assert!(tracker.is_tainted(&grandchild));
    }

    #[test]
    fn taint_no_propagation_from_clean() {
        let mut tracker = TaintTracker::new();
        let clean = vector_id(0x10);
        let target = vector_id(0x11);

        tracker.propagate(&clean, &target);
        assert!(!tracker.is_tainted(&target));
    }

    // -- TrustPipeline --

    #[test]
    fn pipeline_reject_empty_payload() {
        let pipeline = TrustPipeline::new(TrustRegistry::new());
        let candidate = TrustCandidate {
            author: agent_addr(0x01),
            content_id: vector_id(0x01),
            payload_size: 0,
            on_chain_verified: true,
        };
        assert_eq!(pipeline.validate(&candidate), TrustDecision::Reject("empty payload".into()));
    }

    #[test]
    fn pipeline_reject_oversized_payload() {
        let pipeline = TrustPipeline::new(TrustRegistry::new());
        let candidate = TrustCandidate {
            author: agent_addr(0x01),
            content_id: vector_id(0x01),
            payload_size: 10_000_000,
            on_chain_verified: true,
        };
        match pipeline.validate(&candidate) {
            TrustDecision::Reject(reason) => {
                assert!(reason.contains("max size"));
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_reject_untrusted_agent() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x01);

        // Drive trust below MIN_TRUST_THRESHOLD.
        for _ in 0..200 {
            registry.update_trust(&agent, TrustOutcome::Negative(1.0));
        }

        let pipeline = TrustPipeline::new(registry);
        let candidate = TrustCandidate {
            author: agent,
            content_id: vector_id(0x01),
            payload_size: 100,
            on_chain_verified: true,
        };
        match pipeline.validate(&candidate) {
            TrustDecision::Reject(reason) => {
                assert!(reason.contains("below minimum"));
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_quarantine_low_reputation() {
        // Cold start reputation (0.1) is below quarantine threshold (0.3).
        let pipeline = TrustPipeline::new(TrustRegistry::new());
        let candidate = TrustCandidate {
            author: agent_addr(0x01),
            content_id: vector_id(0x01),
            payload_size: 100,
            on_chain_verified: true,
        };
        match pipeline.validate(&candidate) {
            TrustDecision::Quarantine(reason) => {
                assert!(reason.contains("low reputation"));
            }
            other => panic!("expected Quarantine, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_quarantine_unverified() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x01);

        // Build reputation above quarantine threshold.
        for _ in 0..100 {
            registry.update_trust(&agent, TrustOutcome::Positive(1.0));
        }

        let pipeline = TrustPipeline::new(registry);
        let candidate = TrustCandidate {
            author: agent,
            content_id: vector_id(0x01),
            payload_size: 100,
            on_chain_verified: false,
        };
        match pipeline.validate(&candidate) {
            TrustDecision::Quarantine(reason) => {
                assert!(reason.contains("not yet verified"));
            }
            other => panic!("expected Quarantine, got {other:?}"),
        }
    }

    #[test]
    fn pipeline_accept_trusted_verified() {
        let mut registry = TrustRegistry::new();
        let agent = agent_addr(0x01);

        // Build high reputation.
        for _ in 0..100 {
            registry.update_trust(&agent, TrustOutcome::Positive(1.0));
        }

        let pipeline = TrustPipeline::new(registry);
        let candidate = TrustCandidate {
            author: agent,
            content_id: vector_id(0x01),
            payload_size: 100,
            on_chain_verified: true,
        };
        assert_eq!(pipeline.validate(&candidate), TrustDecision::Accept);
    }
}
