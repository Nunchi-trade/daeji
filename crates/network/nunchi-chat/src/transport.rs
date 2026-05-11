//! Chat transport selection and Iroh sub-mesh primitives.
//!
//! This module is the first implementation slice for the Iroh chat sub-mesh.
//! Chat networking is Iroh-native: the module defines the topic, membership,
//! and encrypted wire-frame semantics the Iroh transport implementation will
//! use.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::{messages::RoomMessage, room};

const IROH_LOBBY_DOMAIN: &[u8] = b"NUNCHI_LOBBY_V1";

/// Chat transport backend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    /// Iroh-backed lobby and per-room topic path.
    #[default]
    Iroh,
}

/// Iroh gossip topic id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IrohTopicId([u8; 32]);

impl IrohTopicId {
    /// Well-known lobby topic shared by all chat participants.
    pub fn lobby() -> Self {
        let mut hasher = Keccak256::new();
        hasher.update(IROH_LOBBY_DOMAIN);
        Self(hasher.finalize().into())
    }

    /// Per-job room topic. This is intentionally the full 32-byte room id.
    pub const fn room(room_id: [u8; 32]) -> Self {
        Self(room_id)
    }

    /// Raw 32-byte topic id.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Chain-backed participant identity resolved from `contracts-core`.
///
/// The transport pubkey routes bytes; the agent address and passport id carry
/// the participant identity used by contracts and room membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatParticipantIdentity {
    /// Agent EVM address from `AgentRegistry`.
    pub agent_address: String,
    /// ERC-8004 passport id from `IdentityRegistry`.
    pub passport_id: String,
    /// 32-byte chat transport pubkey, 0x-prefixed or raw hex.
    pub transport_pubkey_hex: String,
}

impl ChatParticipantIdentity {
    /// Construct and normalize a participant identity.
    pub fn new(
        agent_address: impl Into<String>,
        passport_id: impl Into<String>,
        transport_pubkey_hex: impl Into<String>,
    ) -> Self {
        Self {
            agent_address: normalize_hex(agent_address.into()),
            passport_id: passport_id.into(),
            transport_pubkey_hex: normalize_hex(transport_pubkey_hex.into()),
        }
    }
}

/// Expected room membership, expressed as contract identity plus transport key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrohRoomMembership {
    /// Room id that also becomes the Iroh topic id.
    pub room_id: [u8; 32],
    /// Expected room members.
    pub members: Vec<ChatParticipantIdentity>,
}

impl IrohRoomMembership {
    /// Construct membership and reject duplicate transport keys.
    pub fn new(
        room_id: [u8; 32],
        members: Vec<ChatParticipantIdentity>,
    ) -> Result<Self, TransportError> {
        let membership = Self { room_id, members };
        membership.validate_unique_transport_keys()?;
        Ok(membership)
    }

    /// Iroh topic for this room.
    pub const fn topic_id(&self) -> IrohTopicId {
        IrohTopicId::room(self.room_id)
    }

    /// True when the pubkey belongs to an expected member.
    pub fn contains_transport_pubkey(&self, transport_pubkey_hex: &str) -> bool {
        let key = normalize_hex(transport_pubkey_hex.to_string());
        self.members.iter().any(|member| member.transport_pubkey_hex == key)
    }

    /// True when the contracts-core identity and routing key match an expected member.
    pub fn contains_identity(&self, identity: &ChatParticipantIdentity) -> bool {
        self.members.iter().any(|member| {
            member.agent_address == identity.agent_address
                && member.passport_id == identity.passport_id
                && member.transport_pubkey_hex == identity.transport_pubkey_hex
        })
    }

    /// Expected transport pubkeys for the room.
    pub fn transport_pubkeys(&self) -> Vec<&str> {
        self.members.iter().map(|member| member.transport_pubkey_hex.as_str()).collect()
    }

    fn validate_unique_transport_keys(&self) -> Result<(), TransportError> {
        let mut seen = BTreeSet::new();
        for member in &self.members {
            if !seen.insert(member.transport_pubkey_hex.clone()) {
                return Err(TransportError::DuplicateMemberTransportKey(
                    member.transport_pubkey_hex.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// Encrypted room payload carried on an Iroh room topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrohRoomWireFrame {
    /// Room id bound as AEAD AAD and used as the Iroh topic id.
    pub room_id: [u8; 32],
    /// `room::encrypt` output: nonce || ciphertext.
    pub ciphertext: Vec<u8>,
}

impl IrohRoomWireFrame {
    /// Encrypt and serialize a room message for an Iroh room topic.
    pub fn seal(
        room_id: [u8; 32],
        room_key: &[u8; 32],
        msg: &RoomMessage,
    ) -> Result<Self, TransportError> {
        let plaintext = serde_json::to_vec(msg).map_err(TransportError::SerializeRoomMessage)?;
        Ok(Self { room_id, ciphertext: room::encrypt(room_key, &room_id, &plaintext) })
    }

    /// Decrypt and deserialize a room message.
    pub fn open(&self, room_key: &[u8; 32]) -> Result<RoomMessage, TransportError> {
        let plaintext = room::decrypt(room_key, &self.room_id, &self.ciphertext)
            .ok_or(TransportError::DecryptRoomMessage)?;
        serde_json::from_slice(&plaintext).map_err(TransportError::DeserializeRoomMessage)
    }
}

/// Errors from chat transport primitives.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Duplicate room member transport key.
    #[error("duplicate member transport key: {0}")]
    DuplicateMemberTransportKey(String),
    /// Failed to serialize a room message.
    #[error("failed to serialize room message: {0}")]
    SerializeRoomMessage(serde_json::Error),
    /// Failed to decrypt a room message.
    #[error("failed to decrypt room message")]
    DecryptRoomMessage,
    /// Failed to deserialize a room message.
    #[error("failed to deserialize room message: {0}")]
    DeserializeRoomMessage(serde_json::Error),
}

fn normalize_hex(value: String) -> String {
    value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim())
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(pk: &str) -> ChatParticipantIdentity {
        ChatParticipantIdentity::new("0xabc", "7", pk)
    }

    #[test]
    fn transport_kind_defaults_to_iroh() {
        assert_eq!(TransportKind::default(), TransportKind::Iroh);
        let parsed: TransportKind = serde_json::from_str(r#""iroh""#).unwrap();
        assert_eq!(parsed, TransportKind::Iroh);
    }

    #[test]
    fn lobby_topic_preserves_existing_lobby_projection() {
        let topic = IrohTopicId::lobby();
        let mut projected = [0u8; 8];
        projected.copy_from_slice(&topic.as_bytes()[..8]);
        assert_eq!(u64::from_le_bytes(projected), room::lobby_channel_id());
    }

    #[test]
    fn room_topic_is_full_room_id() {
        let room_id = room::room_id_for_chain_job(42);
        assert_eq!(IrohTopicId::room(room_id).as_bytes(), &room_id);
    }

    #[test]
    fn participant_identity_normalizes_hex_fields() {
        let identity = ChatParticipantIdentity::new("0xABCD", "9", "0xAABB");
        assert_eq!(identity.agent_address, "abcd");
        assert_eq!(identity.transport_pubkey_hex, "aabb");
    }

    #[test]
    fn membership_rejects_duplicate_transport_pubkeys() {
        let room_id = room::room_id_for_chain_job(7);
        let err = IrohRoomMembership::new(room_id, vec![member("0x01"), member("01")]).unwrap_err();
        assert!(matches!(
            err,
            TransportError::DuplicateMemberTransportKey(key) if key == "01"
        ));
    }

    #[test]
    fn membership_checks_expected_transport_pubkeys() {
        let room_id = room::room_id_for_chain_job(7);
        let membership =
            IrohRoomMembership::new(room_id, vec![member("0x01"), member("0x02")]).unwrap();
        assert_eq!(membership.topic_id().as_bytes(), &room_id);
        assert!(membership.contains_transport_pubkey("01"));
        assert!(membership.contains_transport_pubkey("0x02"));
        assert!(!membership.contains_transport_pubkey("0x03"));
        assert_eq!(membership.transport_pubkeys(), vec!["01", "02"]);
    }

    #[test]
    fn membership_checks_full_contract_identity() {
        let room_id = room::room_id_for_chain_job(7);
        let expected = ChatParticipantIdentity::new("0xabc", "7", "0x01");
        let membership = IrohRoomMembership::new(room_id, vec![expected.clone()]).unwrap();

        assert!(membership.contains_identity(&expected));
        assert!(!membership.contains_identity(&ChatParticipantIdentity::new("0xdef", "7", "0x01")));
        assert!(!membership.contains_identity(&ChatParticipantIdentity::new("0xabc", "8", "0x01")));
        assert!(!membership.contains_identity(&ChatParticipantIdentity::new("0xabc", "7", "0x02")));
    }

    #[test]
    fn room_wire_frame_seals_and_opens_message() {
        let room_id = room::room_id_for_chain_job(7);
        let key = [3u8; 32];
        let msg = RoomMessage::Hello { from_pubkey_hex: "0x01".into(), wall_clock_ms: 123 };

        let frame = IrohRoomWireFrame::seal(room_id, &key, &msg).unwrap();
        let opened = frame.open(&key).unwrap();

        assert_eq!(opened.label(), msg.label());
    }

    #[test]
    fn room_wire_frame_rejects_wrong_key() {
        let room_id = room::room_id_for_chain_job(7);
        let msg = RoomMessage::Hello { from_pubkey_hex: "0x01".into(), wall_clock_ms: 123 };

        let frame = IrohRoomWireFrame::seal(room_id, &[3u8; 32], &msg).unwrap();
        assert!(matches!(frame.open(&[4u8; 32]), Err(TransportError::DecryptRoomMessage)));
    }
}
