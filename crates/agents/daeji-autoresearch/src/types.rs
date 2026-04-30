//! Predictive Foraging types — vendored from
//! `~/evm-specpool-impl/korai-types/src/prediction.rs` (Apr 2026 snapshot).
//!
//! No behavior change relative to the source. Module renames only:
//! - `korai_types::prediction` → `daeji_autoresearch::types`.
//!
//! When the legacy korai-types crate is sunset, downstream crates can
//! migrate to this module by changing only the `use` paths.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// PredictionTarget
// ---------------------------------------------------------------------------

/// What the prediction is about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PredictionTarget {
    /// Market price prediction (e.g. ETH-USD spot).
    MarketPrice {
        /// 32-byte market identifier.
        market_id: [u8; 32],
        /// Human-readable pair tag, e.g. "ETH-USD".
        asset_pair: String,
    },
    /// Funding rate prediction.
    FundingRate {
        /// 32-byte market identifier.
        market_id: [u8; 32],
    },
    /// Transaction latency prediction (milliseconds).
    Latency {
        /// Endpoint URL the latency claim refers to.
        endpoint: String,
    },
    /// Gas price prediction (in gwei-equivalent bps).
    GasPrice {
        /// EVM chain identifier.
        chain_id: u64,
    },
    /// Custom prediction target.
    Custom {
        /// Stable key for corrector bucketing.
        target_key: String,
        /// Free-form description.
        description: String,
    },
}

impl PredictionTarget {
    /// Return a stable key for this target, used for corrector bucketing.
    #[must_use]
    pub fn target_key(&self) -> String {
        match self {
            Self::MarketPrice { asset_pair, .. } => format!("market:{}", asset_pair),
            Self::FundingRate { market_id } => format!(
                "funding:{:02x}{:02x}{:02x}{:02x}",
                market_id[0], market_id[1], market_id[2], market_id[3]
            ),
            Self::Latency { endpoint } => format!("latency:{}", endpoint),
            Self::GasPrice { chain_id } => format!("gas:{}", chain_id),
            Self::Custom { target_key, .. } => format!("custom:{}", target_key),
        }
    }
}

// ---------------------------------------------------------------------------
// ResolutionMethod
// ---------------------------------------------------------------------------

/// How a prediction is resolved against ground truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResolutionMethod {
    /// Resolved by an on-chain oracle (e.g. Chainlink, Pyth).
    Oracle {
        /// Oracle identifier (e.g. "chainlink-eth-usd").
        oracle_id: String,
    },
    /// Resolved by reading on-chain state directly.
    OnChain {
        /// 20-byte EVM address holding the ground truth.
        contract_address: [u8; 20],
    },
    /// Resolved by peer consensus (majority of agents agree).
    PeerConsensus {
        /// Min number of peers that must agree.
        min_peers: u32,
    },
    /// Resolved by the agent's own observation.
    SelfObserved,
}

// ---------------------------------------------------------------------------
// AttentionTier
// ---------------------------------------------------------------------------

/// Three-tier attention hierarchy for prediction targets.
///
/// Active targets get the most compute and monitoring; Scanned targets are
/// checked periodically with minimal resources.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AttentionTier {
    /// High-frequency monitoring, full compute allocation (~15 targets max).
    Active,
    /// Medium-frequency monitoring (~50 targets max).
    Watched,
    /// Low-frequency scanning (~200 targets max).
    Scanned,
}

impl AttentionTier {
    /// Return the tier one level higher (more attention), if possible.
    #[must_use]
    pub const fn promote(&self) -> Option<Self> {
        match self {
            Self::Scanned => Some(Self::Watched),
            Self::Watched => Some(Self::Active),
            Self::Active => None,
        }
    }

    /// Return the tier one level lower (less attention), if possible.
    #[must_use]
    pub const fn demote(&self) -> Option<Self> {
        match self {
            Self::Active => Some(Self::Watched),
            Self::Watched => Some(Self::Scanned),
            Self::Scanned => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Prediction
// ---------------------------------------------------------------------------

/// A falsifiable prediction about future state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prediction {
    /// Unique prediction identifier (SHA256 hash).
    pub prediction_id: [u8; 32],
    /// The agent that made this prediction.
    pub agent_id: [u8; 32],
    /// What is being predicted.
    pub target: PredictionTarget,
    /// Predicted value in basis points.
    pub predicted_value_bps: i64,
    /// Confidence interval half-width in basis points.
    /// Claim: actual ∈ [predicted − interval, predicted + interval].
    pub confidence_interval_bps: u64,
    /// How this prediction should be resolved.
    pub resolution_method: ResolutionMethod,
    /// When the prediction was made (unix ms).
    pub created_at_ms: u64,
    /// When the prediction expires / should be resolved by (unix ms).
    pub resolve_by_ms: u64,
    /// The attention tier of the target at prediction time.
    pub attention_tier: AttentionTier,
}

// ---------------------------------------------------------------------------
// PredictionResolution
// ---------------------------------------------------------------------------

/// The outcome of resolving a prediction against external state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PredictionResolution {
    /// The prediction that was resolved.
    pub prediction_id: [u8; 32],
    /// Actual observed value in basis points.
    pub actual_value_bps: i64,
    /// Residual = actual − predicted (in basis points).
    pub residual_bps: i64,
    /// Was the actual value within the confidence interval?
    pub within_interval: bool,
    /// Source of the resolution data.
    pub resolution_source: String,
    /// When the resolution occurred (unix ms).
    pub resolved_at_ms: u64,
}

// ---------------------------------------------------------------------------
// ResidualCorrection
// ---------------------------------------------------------------------------

/// A bias correction derived from accumulated prediction residuals. Broadcast
/// to peers so they can benefit from observed systematic errors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResidualCorrection {
    /// Agent broadcasting this correction.
    pub agent_id: [u8; 32],
    /// Target key this correction applies to.
    pub target_key: String,
    /// Estimated bias in basis points (mean of residuals).
    pub bias_bps: i64,
    /// Coverage rate in basis points (fraction within interval).
    pub coverage_rate_bps: u32,
    /// Number of samples used to compute this correction.
    pub sample_count: u32,
    /// When this correction was computed (unix ms).
    pub computed_at_ms: u64,
}

// ---------------------------------------------------------------------------
// AttentionShift
// ---------------------------------------------------------------------------

/// A promotion or demotion event in the attention hierarchy. Broadcast to
/// peers so they can adjust their own attention allocations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttentionShift {
    /// Agent reporting the attention shift.
    pub agent_id: [u8; 32],
    /// Target key that shifted.
    pub target_key: String,
    /// Previous attention tier.
    pub from_tier: AttentionTier,
    /// New attention tier.
    pub to_tier: AttentionTier,
    /// Reason for the shift (e.g. "5 consecutive errors > 200bps").
    pub reason: String,
    /// When the shift occurred (unix ms).
    pub shifted_at_ms: u64,
}
