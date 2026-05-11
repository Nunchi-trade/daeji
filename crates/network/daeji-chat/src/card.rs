//! Off-chain status.json schema and verification — implements spec D1 (the corrected version).
//!
//! Pattern (per `~/contracts-core/packages/agents/src/AgentRegistry.sol`):
//! - On-chain registration carries `capabilities: string` (pipe-delimited, contains `endpoint=URL`)
//!   and `passportHash: bytes32`.
//! - `passportHash` is bound to `keccak256(canonical_json(status_body))` — a content commitment.
//! - The URL inside `capabilities` serves the JSON below; the indexer fetches, verifies the
//!   keccak, then extracts the ed25519 transport pubkey.
//!
//! The 47 agents already registered in contracts-core via `BulkRegisterAgents.s.sol` follow the
//! `endpoint=URL` capabilities convention but their `passportHash` values are opaque random
//! bytes today — Phase 4 retro-fits them with the keccak-of-canonical-json binding.

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusCard {
    pub agent: String,
    pub capabilities: String,
    pub transport: TransportInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransportInfo {
    pub alg: String,
    /// 32-byte ed25519 public key, base64-encoded (or 0x-prefixed hex).
    pub pubkey: String,
    /// One or more `host:port` pairs the operator advertises for inbound dials.
    pub bootstrappable: Vec<String>,
}

impl StatusCard {
    /// Parse the 32-byte ed25519 pubkey from the card. Accepts hex (with or without `0x`) or
    /// base64 (standard or URL-safe).
    pub fn transport_pubkey_bytes(&self) -> Result<[u8; 32], CardError> {
        if self.transport.alg != "ed25519" {
            return Err(CardError::UnsupportedAlg(self.transport.alg.clone()));
        }
        let raw = decode_pubkey(&self.transport.pubkey)?;
        if raw.len() != 32 {
            return Err(CardError::WrongPubkeyLength(raw.len()));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&raw);
        Ok(out)
    }
}

/// Extract the `endpoint=URL` field out of a capabilities string of the form
/// `cap_a|cap_b|endpoint=URL|cap_c`. Returns `None` if no `endpoint=` segment.
pub fn parse_endpoint(capabilities: &str) -> Option<&str> {
    capabilities.split('|').find_map(|seg| seg.strip_prefix("endpoint="))
}

/// `keccak256` of the raw bytes that the operator served. Compared to the on-chain
/// `passportHash` to verify the card hasn't been tampered with mid-flight.
pub fn keccak_passport_hash(raw_body: &[u8]) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(raw_body);
    h.finalize().into()
}

/// Fetch + verify in one shot: parse the JSON, compare its raw bytes' keccak to the expected
/// passport hash, and return the verified card.
pub fn verify_card(
    raw_body: &[u8],
    expected_passport_hash: &[u8; 32],
) -> Result<StatusCard, CardError> {
    let actual = keccak_passport_hash(raw_body);
    if &actual != expected_passport_hash {
        return Err(CardError::PassportHashMismatch {
            expected: hex::encode(expected_passport_hash),
            actual: hex::encode(actual),
        });
    }
    let card: StatusCard = serde_json::from_slice(raw_body).map_err(CardError::Json)?;
    // sanity-check the alg + pubkey decode while we have the body in hand.
    let _ = card.transport_pubkey_bytes()?;
    Ok(card)
}

#[derive(Debug, thiserror::Error)]
pub enum CardError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("passport hash mismatch (expected {expected}, got {actual})")]
    PassportHashMismatch { expected: String, actual: String },
    #[error("unsupported transport alg: {0}")]
    UnsupportedAlg(String),
    #[error("transport pubkey is {0} bytes, expected 32")]
    WrongPubkeyLength(usize),
    #[error("transport pubkey could not be decoded as hex or base64")]
    PubkeyDecode,
}

fn decode_pubkey(raw: &str) -> Result<Vec<u8>, CardError> {
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if let Ok(bytes) = hex::decode(stripped) {
        return Ok(bytes);
    }
    // Fall through to base64 (try standard then URL-safe).
    use base64::Engine as _;
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(trimmed) {
        return Ok(bytes);
    }
    if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE.decode(trimmed) {
        return Ok(bytes);
    }
    Err(CardError::PubkeyDecode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_endpoint_finds_url() {
        let caps = "perps_liquidator|model=korai-8b|version=0.1|endpoint=https://demo.nunchi.trade/agents/agent-00/status.json";
        assert_eq!(
            parse_endpoint(caps),
            Some("https://demo.nunchi.trade/agents/agent-00/status.json")
        );
    }

    #[test]
    fn parse_endpoint_returns_none_when_absent() {
        assert!(parse_endpoint("perps_liquidator|model=korai-8b").is_none());
    }

    #[test]
    fn verify_card_accepts_matching_hash() {
        let card = StatusCard {
            agent: "0xBcd4042DE499D14e55001CcbB24a551F3b954096".into(),
            capabilities: "perps_liquidator|endpoint=https://example/status.json".into(),
            transport: TransportInfo {
                alg: "ed25519".into(),
                pubkey: "0x".to_string() + &"00".repeat(32),
                bootstrappable: vec!["agent-00:9100".into()],
            },
        };
        let body = serde_json::to_vec(&card).unwrap();
        let h = keccak_passport_hash(&body);
        let parsed = verify_card(&body, &h).unwrap();
        assert_eq!(parsed.agent, card.agent);
    }

    #[test]
    fn verify_card_rejects_tampered_body() {
        let card = StatusCard {
            agent: "0xabc".into(),
            capabilities: "endpoint=https://example/status.json".into(),
            transport: TransportInfo {
                alg: "ed25519".into(),
                pubkey: "0x".to_string() + &"11".repeat(32),
                bootstrappable: vec!["x:1".into()],
            },
        };
        let body = serde_json::to_vec(&card).unwrap();
        let mut wrong = [0u8; 32];
        wrong[0] = 0xFF;
        assert!(matches!(verify_card(&body, &wrong), Err(CardError::PassportHashMismatch { .. })));
    }

    #[test]
    fn pubkey_round_trip_hex_and_base64() {
        let mut card = StatusCard {
            agent: "0x".into(),
            capabilities: "endpoint=u".into(),
            transport: TransportInfo {
                alg: "ed25519".into(),
                pubkey: "0x".to_string() + &"ab".repeat(32),
                bootstrappable: vec![],
            },
        };
        let bytes = card.transport_pubkey_bytes().unwrap();
        assert_eq!(bytes, [0xab; 32]);

        // Same key, base64-encoded.
        use base64::Engine as _;
        card.transport.pubkey = base64::engine::general_purpose::STANDARD.encode([0xab; 32]);
        assert_eq!(card.transport_pubkey_bytes().unwrap(), [0xab; 32]);
    }

    #[test]
    fn unsupported_alg_rejected() {
        let card = StatusCard {
            agent: "0x".into(),
            capabilities: "endpoint=u".into(),
            transport: TransportInfo {
                alg: "secp256k1".into(),
                pubkey: "0x".to_string() + &"00".repeat(32),
                bootstrappable: vec![],
            },
        };
        assert!(matches!(card.transport_pubkey_bytes(), Err(CardError::UnsupportedAlg(_))));
    }
}
