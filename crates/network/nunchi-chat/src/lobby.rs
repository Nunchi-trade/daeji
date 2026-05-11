//! Lobby protocol — typed messages broadcast on the well-known lobby channel.
//!
//! Per canonical-plan §14: the lobby is a single commonware channel that all agents
//! subscribe to. It carries control-plane messages that map a chain `JobAwarded`
//! event to a slot-pool channel + room key, so awarded agents can join the right
//! per-job AEAD-encrypted room without further chain-side coordination.
//!
//! Coordinator role varies by job type:
//! - **Symphony / reputation-gated / swarm-open** → requester (the job poster)
//!   broadcasts `JobAnnounce` after their `JobAwarded` tx confirms. They hold the
//!   bounty + selected the winners + can mint the room key.
//! - **Mining-bounty** (winner-takes-all) → claim-first model. Agents broadcast
//!   `MiningClaim` to assert their work; first valid claim settles. No central
//!   coordinator.
//! - **Public-broadcast** (ISFR / autoresearch) → no lobby; agents post estimates
//!   on well-known signal channels directly.
//! - **MEV race** → no lobby; latency budget doesn't allow it.
//!
//! Messages on the lobby are always plaintext JSON (no per-room AEAD). Confidential
//! bits (room keys) are wrapped per-recipient inside `RoomKeyWrap` via X25519 ECDH —
//! that's the access control layer.

use serde::{Deserialize, Serialize};

/// Wire format for one message broadcast on the lobby channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LobbyMessage {
    /// Cooperative job — requester broadcasts after their `JobAwarded` tx confirms.
    /// Maps a chain job id to its slot + participants + per-recipient room-key wraps.
    JobAnnounce {
        /// On-chain job id (decimal string for portability across u64 / U256 paths).
        job_id: String,
        /// Slot index in `0..POOL_SIZE`. Agents independently verify against
        /// `room::slot_for_chain_job(job_id)`; mismatches are dropped.
        slot_index: u32,
        /// Coordinator's pubkey hex (for symphony/reputation-gated/swarm-open: the
        /// requester's transport pubkey).
        coordinator: String,
        /// Awarded winners' transport pubkey hexes (matches
        /// `MultiAgentMarket.JobAwarded.winners` mapped through
        /// `nunchi_chat::registry::Registry`).
        participants: Vec<String>,
        /// One wrap per participant. Wrap order does NOT have to match `participants`
        /// order; recipients identify their own wrap by `recipient_pubkey_hex`.
        room_key_wraps: Vec<RoomKeyWrap>,
        /// Block at which the on-chain `JobAwarded` tx confirmed. Used for
        /// freshness checks + diagnostic logging.
        announced_at_block: u64,
    },
    /// Each winner sends after they unwrap their room key — informational, signals
    /// liveness so the room knows who's actually online.
    RoomJoined {
        job_id: String,
        passport_id: String,
        /// ed25519 signature over `keccak256("NUNCHI_ROOM_JOINED_V1" || room_id)`.
        signature_over_room_id: String,
    },
    /// Coordinator (or any winner, with retry/dedup) signals job done.
    /// Receivers free up the per-job state; the slot becomes immediately reusable.
    JobConcluded { job_id: String, slot_index: u32 },
    /// Competitive job (mining-bounty): an agent broadcasts their first-claim proof
    /// on the lobby. Settlement is on-chain; this is just for visibility + race
    /// dedup hints for other agents.
    MiningClaim {
        job_id: String,
        slot_index: u32,
        agent: String,
        /// Optional branch identifier in the mining-bounty branch DAG.
        #[serde(default)]
        branch_id: Option<String>,
        /// Hash of the work-output the agent claims. The actual on-chain claim
        /// happens via the mining contract; this lobby message is purely advisory.
        claim_proof_hash: String,
    },
}

impl LobbyMessage {
    /// Stable variant tag. Useful for logging without cloning the whole payload.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::JobAnnounce { .. } => "job_announce",
            Self::RoomJoined { .. } => "room_joined",
            Self::JobConcluded { .. } => "job_concluded",
            Self::MiningClaim { .. } => "mining_claim",
        }
    }

    /// Job id this message is associated with. None for messages that don't carry one
    /// (none currently).
    pub fn job_id(&self) -> Option<&str> {
        match self {
            Self::JobAnnounce { job_id, .. }
            | Self::RoomJoined { job_id, .. }
            | Self::JobConcluded { job_id, .. }
            | Self::MiningClaim { job_id, .. } => Some(job_id),
        }
    }
}

/// Per-recipient wrap of a 32-byte symmetric room key. Wrapped via X25519 ECDH
/// from the recipient's ed25519 transport pubkey (converted to x25519 via the
/// standard birational map). v1 ships with **plaintext** wraps (PRE-handshake)
/// for the demo path — full ECDH wrap lands in a follow-up PR (PR-Nunchi-F).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoomKeyWrap {
    /// Recipient's transport pubkey as 0x-prefixed hex.
    pub recipient_pubkey_hex: String,
    /// Either:
    /// - v1 (pre-handshake): the 32-byte room key as 0x-hex (plaintext).
    /// - v2 (post-handshake): X25519-ECDH-derived AEAD ciphertext + nonce as 0x-hex.
    ///
    /// Receivers MUST treat any wrap as opaque bytes and try-parse + try-decrypt.
    pub ciphertext_hex: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_announce_round_trip() {
        let msg = LobbyMessage::JobAnnounce {
            job_id: "42".into(),
            slot_index: 17,
            coordinator: "0xab".repeat(16),
            participants: vec!["0x01".repeat(16), "0x02".repeat(16)],
            room_key_wraps: vec![
                RoomKeyWrap {
                    recipient_pubkey_hex: "0x01".repeat(16),
                    ciphertext_hex: "0x".to_string() + &"77".repeat(32),
                },
                RoomKeyWrap {
                    recipient_pubkey_hex: "0x02".repeat(16),
                    ciphertext_hex: "0x".to_string() + &"88".repeat(32),
                },
            ],
            announced_at_block: 1234,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: LobbyMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(parsed.label(), "job_announce");
        assert_eq!(parsed.job_id(), Some("42"));
    }

    #[test]
    fn room_joined_round_trip() {
        let msg = LobbyMessage::RoomJoined {
            job_id: "42".into(),
            passport_id: "13".into(),
            signature_over_room_id: "0x".to_string() + &"ff".repeat(64),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: LobbyMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(parsed.label(), "room_joined");
    }

    #[test]
    fn job_concluded_round_trip() {
        let msg = LobbyMessage::JobConcluded { job_id: "42".into(), slot_index: 17 };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: LobbyMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(parsed.label(), "job_concluded");
    }

    #[test]
    fn mining_claim_round_trip() {
        let msg = LobbyMessage::MiningClaim {
            job_id: "42".into(),
            slot_index: 17,
            agent: "0x".to_string() + &"aa".repeat(20),
            branch_id: Some("parent-1".into()),
            claim_proof_hash: "0x".to_string() + &"33".repeat(32),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: LobbyMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(parsed.label(), "mining_claim");
    }

    #[test]
    fn mining_claim_branch_id_is_optional() {
        let raw = r#"{
            "type": "mining_claim",
            "job_id": "42",
            "slot_index": 17,
            "agent": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "claim_proof_hash": "0x3333333333333333333333333333333333333333333333333333333333333333"
        }"#;
        let parsed: LobbyMessage = serde_json::from_str(raw).unwrap();
        if let LobbyMessage::MiningClaim { branch_id, .. } = parsed {
            assert!(branch_id.is_none(), "branch_id should default to None");
        } else {
            panic!("expected MiningClaim variant");
        }
    }

    #[test]
    fn unknown_variant_rejected() {
        let raw = r#"{ "type": "made_up_message", "job_id": "1" }"#;
        let parsed: Result<LobbyMessage, _> = serde_json::from_str(raw);
        assert!(parsed.is_err());
    }
}
