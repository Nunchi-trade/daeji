//! Decoder for the `InsightPosted` event emitted by `InsightBoard.sol`.
//!
//! Mirrors the Solidity event signature at
//! `~/contracts-core/packages/agents/src/InsightBoard.sol`:
//!
//! ```solidity
//! event InsightPosted(
//!     uint256 indexed id,
//!     address indexed poster,
//!     Kind    indexed kind,         // uint8 enum: Insight=0, Heuristic=1, Warning=2, AntiKnowledge=3, CausalLink=4, StrategyFragment=5
//!     bytes32        contentHash,
//!     bytes32        hdcFingerprint,
//!     uint64         postedAt,
//!     uint64         revealAt,
//!     bytes          hdcVector,     // 1,280 bytes
//!     string         uri
//! );
//! ```
//!
//! On `BlockExecutor::on_finalize` we walk every receipt log; for each one
//! whose first topic matches the `InsightPosted` signature we decode the
//! topics + data into [`InsightPostedEvent`] and feed it to
//! [`crate::hdc::HDCState::on_finalize_extend`] to deterministically
//! mutate the HDC index.
//!
//! Spec source: `~/obsidian-vault/research/2026-05-04-wp-agent-chainv2/raw/02-daeji/03-agent-systems.md` line 381 (kind table) + 402-405 (tier multipliers).

use alloy_primitives::{Address, B256, Log, LogData};
use alloy_sol_types::{SolEvent, sol};

use crate::{hdc::InsightPostedEvent, insight_id::InsightId};

sol! {
    /// ABI-shaped twin of the Solidity event for use with `alloy_sol_types`.
    ///
    /// Note: the `kind` field type is `uint8` here because Solidity ABI-encodes
    /// enums as their underlying integer.
    #[derive(Debug)]
    event InsightPosted(
        uint256 indexed id,
        address indexed poster,
        uint8   indexed kind,
        bytes32 contentHash,
        bytes32 hdcFingerprint,
        uint64  postedAt,
        uint64  revealAt,
        bytes   hdcVector,
        string  uri
    );
}

/// Topic[0] for the `InsightPosted` event signature.
pub fn insight_posted_topic0() -> B256 {
    InsightPosted::SIGNATURE_HASH
}

/// Decode an `InsightPosted` log into the precompile's reduced event view.
///
/// Returns `None` if the log topic doesn't match `InsightPosted` or if the
/// data fails ABI decoding. Tier always defaults to `Transient` for new
/// posts (matches the v3 InsightBoard contract behaviour at post time).
pub fn decode_insight_posted(emitter: Address, log: &LogData) -> Option<InsightPostedEvent> {
    let topic0 = *log.topics().first()?;
    if topic0 != insight_posted_topic0() {
        return None;
    }
    let alloy_log = Log { address: emitter, data: log.clone() };
    let parsed = InsightPosted::decode_log(&alloy_log).ok()?;

    // The Solidity contract emits `uint256 id`, but only the bottom 128 bits
    // are meaningful (insight ids are sequential starting from 0). Truncate
    // to the bytes16 InsightId tuple struct, big-endian.
    let id_u128: u128 = u128::try_from(parsed.id).ok()?;
    let insight_id = InsightId(id_u128.to_be_bytes());

    let kind: KnowledgeKindCode = KnowledgeKindCode::from_u8(parsed.kind)?;
    // New posts always start at Tier::Transient (multiplier 100 / 1000 = 0.1×).
    let tier_multiplier_bps: u64 = 100;
    let base_half_life_seconds = kind.half_life_seconds();
    let effective_half_life_seconds =
        (base_half_life_seconds.saturating_mul(tier_multiplier_bps)) / 1000;

    Some(InsightPostedEvent {
        emitter,
        insight_id,
        posted_at: parsed.postedAt,
        effective_half_life_seconds,
        hdc_vector: parsed.hdcVector.clone(),
    })
}

/// Knowledge kind enum mirror — must stay in sync with the Solidity enum
/// `Kind` in `InsightBoard.sol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnowledgeKindCode {
    Insight,
    Heuristic,
    Warning,
    AntiKnowledge,
    CausalLink,
    StrategyFragment,
}

impl KnowledgeKindCode {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Insight),
            1 => Some(Self::Heuristic),
            2 => Some(Self::Warning),
            3 => Some(Self::AntiKnowledge),
            4 => Some(Self::CausalLink),
            5 => Some(Self::StrategyFragment),
            _ => None,
        }
    }

    /// Spec on-chain half-life in seconds. Matches `InsightBoard.halfLifeOf`
    /// at `~/contracts-core/packages/agents/src/InsightBoard.sol`.
    fn half_life_seconds(self) -> u64 {
        match self {
            Self::Warning => 3 * 60,              // 3 minutes
            Self::Insight => 7 * 24 * 60 * 60,    // 7 days
            Self::Heuristic => 15 * 24 * 60 * 60, // 15 days
            Self::AntiKnowledge => 15 * 24 * 60 * 60,
            Self::CausalLink => 15 * 24 * 60 * 60,
            Self::StrategyFragment => 15 * 24 * 60 * 60,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Bytes, U256, address, b256, keccak256};

    use super::*;

    /// Helper: build a synthetic `InsightPosted` log for testing.
    fn synth_log(id: u64, kind: u8, posted_at: u64, hdc_vector: Vec<u8>) -> (Address, LogData) {
        let emitter = address!("0x000000000000000000000000000000000000000A");
        let topic0 = insight_posted_topic0();
        let topic_id = B256::from(U256::from(id));
        let topic_poster =
            B256::from(address!("0x00000000000000000000000000000000beefBeEF").into_word());
        let topic_kind = B256::from(U256::from(kind));

        // Encode the non-indexed body via SolEvent helpers.
        let body = InsightPosted {
            id: U256::from(id),
            poster: address!("0x00000000000000000000000000000000beefBeEF"),
            kind,
            contentHash: B256::ZERO,
            hdcFingerprint: keccak256(&hdc_vector),
            postedAt: posted_at,
            revealAt: 0,
            hdcVector: Bytes::from(hdc_vector),
            uri: "ipfs://abc".to_string(),
        };
        let data = body.encode_data();

        let log = LogData::new(vec![topic0, topic_id, topic_poster, topic_kind], Bytes::from(data))
            .expect("topic count valid");
        (emitter, log)
    }

    #[test]
    fn decode_insight_posted_round_trip_insight_kind() {
        let vec_bytes = vec![0xAA; 1280];
        let (emitter, log) = synth_log(42, 0, 1_700_000_000, vec_bytes.clone());
        let decoded = decode_insight_posted(emitter, &log).expect("decode");
        assert_eq!(decoded.emitter, emitter);
        assert_eq!(decoded.posted_at, 1_700_000_000);
        // Tier::Transient (0.1×) × 7 days = 60480 seconds.
        assert_eq!(decoded.effective_half_life_seconds, 7 * 24 * 60 * 60 / 10);
        assert_eq!(decoded.hdc_vector.as_ref(), &vec_bytes[..]);
    }

    #[test]
    fn decode_warning_kind_uses_three_minute_half_life() {
        let vec_bytes = vec![0x55; 1280];
        let (emitter, log) = synth_log(1, 2, 1, vec_bytes); // kind=Warning
        let decoded = decode_insight_posted(emitter, &log).expect("decode");
        // Warning baseHl = 180s. Transient × 0.1 = 18s.
        assert_eq!(decoded.effective_half_life_seconds, 18);
    }

    #[test]
    fn decode_returns_none_on_topic_mismatch() {
        let bogus_topic =
            b256!("0x1111111111111111111111111111111111111111111111111111111111111111");
        let log = LogData::new(vec![bogus_topic], Bytes::new()).expect("log");
        let emitter = address!("0x0000000000000000000000000000000000000001");
        assert!(decode_insight_posted(emitter, &log).is_none());
    }

    #[test]
    fn decode_returns_none_on_invalid_kind_code() {
        let vec_bytes = vec![0; 1280];
        let (emitter, log) = synth_log(1, 99, 0, vec_bytes); // kind=99 — not a valid enum
        assert!(decode_insight_posted(emitter, &log).is_none());
    }
}
