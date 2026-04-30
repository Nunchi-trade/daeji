//! Predictive-Foraging types + canonical jobType for the autoresearch agent
//! class. Phase η.1.a of the canonical agent-coordination plan.
//!
//! This crate vendors the `prediction` types from
//! `~/evm-specpool-impl/korai-types/src/prediction.rs` (wholesale; no
//! behavior change) so the daeji workspace can build autoresearch jobs
//! without depending on the legacy `korai-types` crate.
//!
//! The matching engine (`PredictionEngine`, `AttentionState`, `ActionGate`)
//! lifted from `~/evm-specpool-impl/specpool-evm/src/networking/prediction.rs`
//! lands in η.1.b as a separate PR — keeping this initial port small and
//! easy to review.
//!
//! # Canonical jobType
//!
//! ```text
//! keccak256("autoresearch") = AUTORESEARCH_JOB_TYPE
//! ```
//!
//! Operators register this jobType in `JobTypeRegistry` once per deployment
//! via `agentctl autoresearch register-job-type`. Agents then bid on
//! `MultiAgentMarket` jobs whose specHash carries the autoresearch payload.

pub mod types;

/// Canonical jobType key (utf-8). Hash this with keccak256 to get the
/// `bytes32` slot used in `JobTypeRegistry`.
pub const AUTORESEARCH_JOB_TYPE_KEY: &[u8] = b"autoresearch";

/// Recommended descriptor passed to `JobTypeRegistry.register` for autoresearch.
pub const AUTORESEARCH_DESCRIPTION: &str = "Autoresearch — predictive foraging via residual-corrected predictions";

/// Recommended min-tier for autoresearch jobs (Standard or higher; bidders
/// must have a track record before being trusted with prediction-driven work).
pub const AUTORESEARCH_MIN_TIER: u8 = 2; // WorkerRegistry.Tier::Standard

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    #[test]
    fn jobtype_key_is_stable() {
        assert_eq!(AUTORESEARCH_JOB_TYPE_KEY, b"autoresearch");
    }

    #[test]
    fn prediction_target_round_trips() {
        let t = PredictionTarget::MarketPrice {
            market_id: [0xAA; 32],
            asset_pair: "ETH-USD".to_string(),
        };
        let bytes = bincode::serialize(&t).unwrap();
        let back: PredictionTarget = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn attention_tier_promote_demote() {
        assert_eq!(AttentionTier::Scanned.promote(), Some(AttentionTier::Watched));
        assert_eq!(AttentionTier::Active.promote(), None);
        assert_eq!(AttentionTier::Active.demote(), Some(AttentionTier::Watched));
        assert_eq!(AttentionTier::Scanned.demote(), None);
    }
}
