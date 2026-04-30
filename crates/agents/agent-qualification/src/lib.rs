//! Self-qualification + profitability assessment for Daeji agents.
//!
//! Each agent runs `should_bid_on_job` against a posted [`JobOffer`] before
//! bidding, joining a chat room, or submitting a result. The function combines
//! hard gates (deadline, capabilities, tier, reputation) with a soft
//! profitability projection (expected reward minus expected gas minus expected
//! slashing). The caller decides what to do with disqualifications: log + skip,
//! flag for the operator, or surface to a UI.
//!
//! Lifted from `~/marketplace/agent-runtime/src/qualification.rs` and extended
//! per the Phase β plan with gas estimation, slashing risk, capability bitfield
//! match, and tier gating. Domain-specific marketplace types (MiningJob,
//! SwarmQualification) are intentionally dropped — this lib speaks the
//! Daeji JobTypeRegistry / MultiAgentMarket vocabulary instead.

#![cfg_attr(docsrs, feature(doc_cfg))]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Agent tier as reported by `WorkerRegistry` on-chain.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    /// Not registered / inactive.
    None = 0,
    /// Bronze — registered with bond, no track record yet.
    Bronze = 1,
    /// Silver — accumulated reputation.
    Silver = 2,
    /// Trusted — eligible for ISFR/structured jobs.
    Trusted = 3,
    /// Elite — eligible for direct-pick across all job types.
    Elite = 4,
}

impl Tier {
    /// Numeric representation matching the on-chain encoding.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Build a `Tier` from a byte. Out-of-range values clamp to `None`.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Bronze,
            2 => Self::Silver,
            3 => Self::Trusted,
            4 => Self::Elite,
            _ => Self::None,
        }
    }
}

/// Local agent capabilities and state. Snapshot of what we know about ourselves
/// before answering the question "should I bid on this job?".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// Stable identity (typically `keccak256(pubkey)` truncated to 32 bytes).
    pub agent_id: [u8; 32],
    /// Capability bitfield. Job's `required_capabilities` must be a subset.
    pub capabilities: u64,
    /// Reputation in bps (0..=10000), reflecting WorkerRegistry's EWMA.
    pub reputation_bps: u32,
    /// DAEJI bond, in pu18 (token base units, 18 decimals).
    pub stake_pu18: u128,
    /// Tier as reported by WorkerRegistry.
    pub tier: Tier,
    /// Whether we have a verified TEE attestation on file.
    pub is_tee_attested: bool,
    /// Jobs currently in flight (awarded but not resolved).
    pub in_flight_jobs: u32,
    /// Hard cap on concurrency — prevents over-commitment.
    pub max_concurrent_jobs: u32,
    /// Operator-set floor below which we never bid, regardless of profit margin.
    pub min_reward_pu18: u128,
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            agent_id: [0u8; 32],
            capabilities: 0,
            reputation_bps: 5_000,
            stake_pu18: 1_000_000_000_000_000_000_000, // 1000 DAEJI
            tier: Tier::Bronze,
            is_tee_attested: false,
            in_flight_jobs: 0,
            max_concurrent_jobs: 5,
            min_reward_pu18: 0,
        }
    }
}

/// A job offer, rendered into a form `should_bid_on_job` can reason about.
/// Built from `MultiAgentMarket.jobs(id)` plus job-type metadata pulled from
/// `JobTypeRegistry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOffer {
    /// On-chain job id (informational; not used in scoring).
    pub job_id: u64,
    /// Total bounty escrowed in pu18.
    pub bounty_pu18: u128,
    /// Fraction we expect to receive in bps (0..=10000). For symmetric splits
    /// with `n_winners`, pass `10000 / n_winners`. For symphony ISFR with
    /// per-class governance weights, pass the matching weight × 100.
    pub expected_share_bps: u16,
    /// Hard deadline (unix ms).
    pub deadline_ms: u64,
    /// Capability bitfield required by the job.
    pub required_capabilities: u64,
    /// Min worker tier (`u8` to match contract encoding).
    pub min_tier: u8,
    /// Min reputation in bps.
    pub min_reputation_bps: u32,
    /// Min stake in pu18.
    pub min_stake_pu18: u128,
    /// Whether a verified TEE attestation is required.
    pub required_tee: bool,
    /// Expected number of on-chain transactions we'll send (bid + submitMulti
    /// + ack, etc.). Multiplied with `expected_gas_per_tx × current_basefee_wei`
    /// for the gas projection.
    pub expected_tx_count: u32,
    /// Expected gas per transaction.
    pub expected_gas_per_tx: u64,
    /// Current basefee in wei (caller is responsible for sampling).
    pub current_basefee_wei: u128,
    /// Conversion factor: how many pu18 of bounty token equal 1 pu18 of native
    /// gas-token. Set to `1` if bounty is paid in the native gas token; pass
    /// the appropriate ratio if denominated in DAEJI vs ETH.
    pub bounty_per_native_pu18_x18: u128,
    /// Prior probability the resolver rejects, in bps. Heuristic — a starting
    /// point is `(10000 - reputation_bps) / 4`.
    pub reject_probability_bps: u16,
    /// Bond burned on reject (in pu18). Typically `MIN_BOND` from
    /// WorkerRegistry, possibly scaled.
    pub slashing_amount_on_reject_pu18: u128,
}

/// Numeric breakdown produced when an agent does qualify. Net is signed: a
/// negative value means the gas + slashing projection outweighs the expected
/// reward. The hard gate for "should I actually bid?" is `Ok(score)` with
/// `score.net_pu18 >= 0`; consumers can also choose to bid on slim margins if
/// they value the reputation delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitabilityScore {
    /// `bounty × share_bps / 10000`.
    pub expected_reward_pu18: u128,
    /// `tx_count × gas_per_tx × basefee × bounty_per_native_pu18_x18 / 10^18`,
    /// converted into bounty-token terms.
    pub expected_gas_cost_pu18: u128,
    /// `reject_probability_bps × slashing_amount_on_reject_pu18 / 10000`.
    pub expected_slashing_pu18: u128,
    /// `expected_reward - expected_gas - expected_slashing`. Signed; negative
    /// values indicate the projection is net-unprofitable (and the function
    /// will already have returned `Err(NetUnprofitable)`).
    pub net_pu18: i128,
    /// 0..=10000. Heuristic: scales reputation against reject probability.
    pub confidence: u16,
}

/// Reasons an agent might decline to bid. Each variant is human-readable and
/// persists into a structured log line for ops to triage.
#[allow(missing_docs)] // field names self-document; variant docstrings cover meaning
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum DisqualificationReason {
    /// Reputation EWMA below the job's floor.
    #[error("insufficient reputation: required {required_bps} bps, have {actual_bps} bps")]
    InsufficientReputation { required_bps: u32, actual_bps: u32 },

    /// Posted bond below the job's floor.
    #[error("insufficient stake: required {required_pu18} pu18, have {actual_pu18} pu18")]
    InsufficientStake { required_pu18: u128, actual_pu18: u128 },

    /// `(profile.capabilities & required) != required`.
    #[error("capability mismatch: required 0b{required:064b}, have 0b{actual:064b}")]
    CapabilityMismatch { required: u64, actual: u64 },

    /// Tier below `min_tier`.
    #[error("insufficient tier: required {required}, have {actual}")]
    InsufficientTier { required: u8, actual: u8 },

    /// Job demands a verified TEE attestation; we don't have one.
    #[error("TEE attestation required")]
    TeeRequired,

    /// At our self-imposed concurrency cap.
    #[error("at capacity: {in_flight} in flight, max {max}")]
    AtCapacity { in_flight: u32, max: u32 },

    /// Deadline passed by the time we evaluated.
    #[error("deadline passed: deadline_ms={deadline_ms}, now_ms={now_ms}")]
    DeadlinePassed { deadline_ms: u64, now_ms: u64 },

    /// Even at full share + zero costs, our floor isn't met.
    #[error("reward too low: {reward_pu18} pu18 < min {min_acceptable_pu18} pu18")]
    RewardTooLow { reward_pu18: u128, min_acceptable_pu18: u128 },

    /// Expected reward is positive, but gas + slashing exceeds it.
    #[error("net unprofitable: net {net_pu18} pu18")]
    NetUnprofitable { net_pu18: i128 },
}

/// Decide whether to bid. Hard gates first (deadline → capacity → tier →
/// reputation → stake → tee → capabilities → reward floor), then the
/// profitability projection. Returns `Ok(ProfitabilityScore)` if the agent
/// should bid; `Err(DisqualificationReason)` otherwise.
///
/// `now_ms` is injected to keep this pure — caller passes
/// `SystemTime::now().duration_since(UNIX_EPOCH).as_millis() as u64`.
pub fn should_bid_on_job(
    profile: &AgentProfile,
    job: &JobOffer,
    now_ms: u64,
) -> Result<ProfitabilityScore, DisqualificationReason> {
    if job.deadline_ms <= now_ms {
        return Err(DisqualificationReason::DeadlinePassed {
            deadline_ms: job.deadline_ms,
            now_ms,
        });
    }

    if profile.in_flight_jobs >= profile.max_concurrent_jobs {
        return Err(DisqualificationReason::AtCapacity {
            in_flight: profile.in_flight_jobs,
            max: profile.max_concurrent_jobs,
        });
    }

    if profile.tier.as_u8() < job.min_tier {
        return Err(DisqualificationReason::InsufficientTier {
            required: job.min_tier,
            actual: profile.tier.as_u8(),
        });
    }

    if profile.reputation_bps < job.min_reputation_bps {
        return Err(DisqualificationReason::InsufficientReputation {
            required_bps: job.min_reputation_bps,
            actual_bps: profile.reputation_bps,
        });
    }

    if profile.stake_pu18 < job.min_stake_pu18 {
        return Err(DisqualificationReason::InsufficientStake {
            required_pu18: job.min_stake_pu18,
            actual_pu18: profile.stake_pu18,
        });
    }

    if job.required_tee && !profile.is_tee_attested {
        return Err(DisqualificationReason::TeeRequired);
    }

    if (profile.capabilities & job.required_capabilities) != job.required_capabilities {
        return Err(DisqualificationReason::CapabilityMismatch {
            required: job.required_capabilities,
            actual: profile.capabilities,
        });
    }

    let expected_reward_pu18 = mul_div(
        job.bounty_pu18,
        u128::from(job.expected_share_bps),
        10_000,
    );
    if expected_reward_pu18 < profile.min_reward_pu18 {
        return Err(DisqualificationReason::RewardTooLow {
            reward_pu18: expected_reward_pu18,
            min_acceptable_pu18: profile.min_reward_pu18,
        });
    }

    let expected_gas_native_pu18 = u128::from(job.expected_tx_count)
        .saturating_mul(u128::from(job.expected_gas_per_tx))
        .saturating_mul(job.current_basefee_wei);
    // Convert native gas (wei) into bounty-token pu18 by multiplying by the
    // ratio and dividing back out the 18-decimal scale.
    let expected_gas_cost_pu18 = mul_div(
        expected_gas_native_pu18,
        job.bounty_per_native_pu18_x18,
        1_000_000_000_000_000_000, // 1e18
    );

    let expected_slashing_pu18 = mul_div(
        job.slashing_amount_on_reject_pu18,
        u128::from(job.reject_probability_bps),
        10_000,
    );

    let net_pu18 = i128::try_from(expected_reward_pu18).unwrap_or(i128::MAX)
        - i128::try_from(expected_gas_cost_pu18).unwrap_or(i128::MAX)
        - i128::try_from(expected_slashing_pu18).unwrap_or(i128::MAX);

    if net_pu18 < 0 {
        return Err(DisqualificationReason::NetUnprofitable { net_pu18 });
    }

    let confidence = compute_confidence(profile.reputation_bps, job.reject_probability_bps);

    Ok(ProfitabilityScore {
        expected_reward_pu18,
        expected_gas_cost_pu18,
        expected_slashing_pu18,
        net_pu18,
        confidence,
    })
}

/// `(rep_bps × (10_000 - reject_prob_bps)) / 10_000`, clamped to `[0, 10_000]`.
fn compute_confidence(rep_bps: u32, reject_prob_bps: u16) -> u16 {
    let inv = 10_000_u32.saturating_sub(u32::from(reject_prob_bps));
    let scaled = u64::from(rep_bps).saturating_mul(u64::from(inv)) / 10_000;
    scaled.min(10_000) as u16
}

/// `(a × b) / d` using u256-equivalent intermediate to avoid overflow on
/// large pu18 numbers. We only need 256-bit math when `a × b` overflows u128;
/// in practice all of bounty/share/basefee fit comfortably. Saturating fallback
/// is correct because all callers want to clamp at u128::MAX (the universe's
/// total token supply is well below this).
fn mul_div(a: u128, b: u128, d: u128) -> u128 {
    if d == 0 {
        return 0;
    }
    match a.checked_mul(b) {
        Some(prod) => prod / d,
        None => {
            // Fall back to u256-style chunked math when the product overflows.
            let a_hi = a >> 64;
            let a_lo = a & ((1u128 << 64) - 1);
            let prod_hi = a_hi.saturating_mul(b);
            let prod_lo = a_lo.saturating_mul(b);
            // (prod_hi << 64 + prod_lo) / d, with rough saturation.
            (prod_hi / d)
                .saturating_mul(1u128 << 64)
                .saturating_add(prod_lo / d)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PU18: u128 = 1_000_000_000_000_000_000;

    fn good_profile() -> AgentProfile {
        AgentProfile {
            agent_id: [1u8; 32],
            capabilities: 0b1111,
            reputation_bps: 7_500,
            stake_pu18: 1_000 * PU18,
            tier: Tier::Trusted,
            is_tee_attested: true,
            in_flight_jobs: 0,
            max_concurrent_jobs: 5,
            min_reward_pu18: PU18, // 1 DAEJI floor
        }
    }

    fn good_job() -> JobOffer {
        JobOffer {
            job_id: 42,
            bounty_pu18: 100 * PU18,
            expected_share_bps: 2_500, // 1 of 4 winners
            deadline_ms: 10_000_000_000_000, // far future
            required_capabilities: 0b0011,
            min_tier: 3,
            min_reputation_bps: 5_000,
            min_stake_pu18: 100 * PU18,
            required_tee: false,
            expected_tx_count: 2,
            expected_gas_per_tx: 200_000,
            current_basefee_wei: 1_000_000_000, // 1 gwei
            bounty_per_native_pu18_x18: 1, // 1:1 bounty-token == native
            reject_probability_bps: 500, // 5%
            slashing_amount_on_reject_pu18: 10 * PU18,
        }
    }

    #[test]
    fn happy_path_returns_profitability_score() {
        let p = good_profile();
        let j = good_job();
        let s = should_bid_on_job(&p, &j, 0).expect("should qualify");
        // Expected reward = 100 DAEJI × 25% = 25 DAEJI
        assert_eq!(s.expected_reward_pu18, 25 * PU18);
        // Expected slashing = 10 DAEJI × 5% = 0.5 DAEJI
        assert_eq!(s.expected_slashing_pu18, PU18 / 2);
        assert!(s.net_pu18 > 0);
        // confidence = 7500 × (10000 - 500) / 10000 = 7125
        assert_eq!(s.confidence, 7_125);
    }

    #[test]
    fn deadline_passed_disqualifies() {
        let p = good_profile();
        let mut j = good_job();
        j.deadline_ms = 1_000;
        let now = 2_000;
        let err = should_bid_on_job(&p, &j, now).unwrap_err();
        assert!(matches!(
            err,
            DisqualificationReason::DeadlinePassed { deadline_ms: 1_000, now_ms: 2_000 }
        ));
    }

    #[test]
    fn at_capacity_disqualifies() {
        let mut p = good_profile();
        p.in_flight_jobs = p.max_concurrent_jobs;
        let j = good_job();
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert!(matches!(err, DisqualificationReason::AtCapacity { .. }));
    }

    #[test]
    fn insufficient_tier_disqualifies() {
        let mut p = good_profile();
        p.tier = Tier::Bronze;
        let j = good_job(); // requires tier 3 (Trusted)
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert_eq!(
            err,
            DisqualificationReason::InsufficientTier { required: 3, actual: 1 }
        );
    }

    #[test]
    fn insufficient_reputation_disqualifies() {
        let mut p = good_profile();
        p.reputation_bps = 100;
        let j = good_job();
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert_eq!(
            err,
            DisqualificationReason::InsufficientReputation {
                required_bps: 5_000,
                actual_bps: 100,
            }
        );
    }

    #[test]
    fn insufficient_stake_disqualifies() {
        let mut p = good_profile();
        p.stake_pu18 = PU18; // way below 100 DAEJI floor
        let j = good_job();
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert!(matches!(
            err,
            DisqualificationReason::InsufficientStake { .. }
        ));
    }

    #[test]
    fn tee_required_disqualifies() {
        let mut p = good_profile();
        p.is_tee_attested = false;
        let mut j = good_job();
        j.required_tee = true;
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert_eq!(err, DisqualificationReason::TeeRequired);
    }

    #[test]
    fn capability_mismatch_disqualifies() {
        let mut p = good_profile();
        p.capabilities = 0b0001; // missing 0b0010
        let j = good_job(); // requires 0b0011
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert_eq!(
            err,
            DisqualificationReason::CapabilityMismatch {
                required: 0b0011,
                actual: 0b0001,
            }
        );
    }

    #[test]
    fn reward_too_low_disqualifies() {
        let mut p = good_profile();
        p.min_reward_pu18 = 50 * PU18; // floor higher than 25 DAEJI share
        let j = good_job();
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert_eq!(
            err,
            DisqualificationReason::RewardTooLow {
                reward_pu18: 25 * PU18,
                min_acceptable_pu18: 50 * PU18,
            }
        );
    }

    #[test]
    fn net_unprofitable_disqualifies_when_slashing_dominates() {
        let p = good_profile();
        let mut j = good_job();
        j.reject_probability_bps = 9_000; // 90%
        j.slashing_amount_on_reject_pu18 = 1_000 * PU18; // 90% × 1000 = 900 DAEJI > 25 DAEJI reward
        let err = should_bid_on_job(&p, &j, 0).unwrap_err();
        assert!(matches!(
            err,
            DisqualificationReason::NetUnprofitable { net_pu18 } if net_pu18 < 0
        ));
    }

    #[test]
    fn confidence_zero_at_max_reject_probability() {
        let p = good_profile();
        let mut j = good_job();
        j.reject_probability_bps = 10_000;
        // Will fail on NetUnprofitable due to slashing; sidestep that.
        j.slashing_amount_on_reject_pu18 = 0;
        let s = should_bid_on_job(&p, &j, 0).expect("should still qualify");
        assert_eq!(s.confidence, 0);
    }

    #[test]
    fn check_order_deadline_before_capacity() {
        // If both deadline and capacity fail, we report deadline first.
        let mut p = good_profile();
        p.in_flight_jobs = p.max_concurrent_jobs;
        let mut j = good_job();
        j.deadline_ms = 1_000;
        let err = should_bid_on_job(&p, &j, 2_000).unwrap_err();
        assert!(matches!(err, DisqualificationReason::DeadlinePassed { .. }));
    }
}
