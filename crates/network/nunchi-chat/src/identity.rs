//! Contracts-core identity join primitives for Nunchi Chat.
//!
//! `nunchi-cli` resolves the active agent from contracts-core identity state
//! and signs EVM messages with that account. This module verifies the matching
//! join envelope on the chat side and maps it into room membership.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::transport::{ChatParticipantIdentity, IrohRoomMembership};

/// EVM signed-message domain used by `nunchi-cli` for chat joins.
pub const JOIN_CHALLENGE_DOMAIN: &str = "nunchi-chat.join";

/// Signed join envelope produced by a contracts-core agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatJoinEnvelope {
    /// Agent EVM address resolved by `nunchi-cli`.
    pub agent_address: String,
    /// ERC-8004 passport id for the same agent.
    pub passport_id: String,
    /// Room/topic being joined. For Iroh rooms this is the 0x-prefixed room id.
    pub topic: String,
    /// Client-generated replay nonce.
    pub nonce: String,
    /// Client timestamp in milliseconds.
    pub ts_ms: u64,
    /// Chat transport key or Iroh node routing key for this agent.
    pub transport_pubkey_hex: String,
    /// EIP-191 personal-sign signature over [`ChatJoinEnvelope::message`].
    pub signature: String,
}

impl ChatJoinEnvelope {
    /// Construct the exact UTF-8 message signed by the agent.
    pub fn message(&self) -> String {
        format!("{}\n{}\n{}\n{}", JOIN_CHALLENGE_DOMAIN, self.topic, self.nonce, self.ts_ms)
    }

    /// Verify the EVM signature and return the corresponding participant identity.
    pub fn verify_identity(&self) -> Result<ChatParticipantIdentity, IdentityJoinError> {
        let recovered = recover_personal_sign_address(&self.message(), &self.signature)?;
        let claimed = normalize_hex(&self.agent_address);
        if recovered != claimed {
            return Err(IdentityJoinError::SignerMismatch { claimed, recovered });
        }

        Ok(ChatParticipantIdentity::new(
            &self.agent_address,
            &self.passport_id,
            &self.transport_pubkey_hex,
        ))
    }

    /// Verify signature, freshness, room topic, and membership in one step.
    pub fn verify_room_join(
        &self,
        membership: &IrohRoomMembership,
        now_ms: u64,
        max_age_ms: u64,
    ) -> Result<ChatParticipantIdentity, IdentityJoinError> {
        self.validate_freshness(now_ms, max_age_ms)?;

        let expected_topic = format!("0x{}", hex::encode(membership.room_id));
        if normalize_hex(&self.topic) != normalize_hex(&expected_topic) {
            return Err(IdentityJoinError::TopicMismatch {
                expected: expected_topic,
                actual: self.topic.clone(),
            });
        }

        let identity = self.verify_identity()?;
        if !membership.contains_identity(&identity) {
            return Err(IdentityJoinError::UnexpectedMember {
                agent_address: identity.agent_address,
                passport_id: identity.passport_id,
                transport_pubkey_hex: identity.transport_pubkey_hex,
            });
        }

        Ok(identity)
    }

    const fn validate_freshness(
        &self,
        now_ms: u64,
        max_age_ms: u64,
    ) -> Result<(), IdentityJoinError> {
        let age = now_ms.abs_diff(self.ts_ms);
        if age > max_age_ms {
            return Err(IdentityJoinError::StaleChallenge { age_ms: age, max_age_ms });
        }
        Ok(())
    }
}

/// Errors while verifying a Nunchi Chat identity join.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityJoinError {
    /// Signature is not valid hex or not 65 bytes.
    #[error("invalid EVM signature")]
    InvalidSignature,
    /// Signature recovery failed.
    #[error("failed to recover EVM signer")]
    RecoverSigner,
    /// Recovered signer did not match claimed agent address.
    #[error("join signer mismatch: claimed {claimed}, recovered {recovered}")]
    SignerMismatch {
        /// Claimed agent address.
        claimed: String,
        /// Recovered signer address.
        recovered: String,
    },
    /// Join topic did not match the room membership.
    #[error("join topic mismatch: expected {expected}, got {actual}")]
    TopicMismatch {
        /// Expected topic string.
        expected: String,
        /// Actual topic string.
        actual: String,
    },
    /// Challenge timestamp was outside the allowed replay window.
    #[error("stale join challenge: age {age_ms}ms exceeds {max_age_ms}ms")]
    StaleChallenge {
        /// Challenge age in milliseconds.
        age_ms: u64,
        /// Maximum allowed age in milliseconds.
        max_age_ms: u64,
    },
    /// The verified identity is not expected in the room.
    #[error("verified identity is not an expected room member")]
    UnexpectedMember {
        /// Verified agent address.
        agent_address: String,
        /// Verified passport id.
        passport_id: String,
        /// Verified transport pubkey.
        transport_pubkey_hex: String,
    },
}

fn recover_personal_sign_address(
    message: &str,
    signature_hex: &str,
) -> Result<String, IdentityJoinError> {
    let signature = decode_signature(signature_hex)?;
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();

    let sig =
        Signature::from_slice(&signature[..64]).map_err(|_| IdentityJoinError::InvalidSignature)?;
    let recovery_id = recovery_id(signature[64])?;
    let verifying_key = VerifyingKey::recover_from_prehash(&digest, &sig, recovery_id)
        .map_err(|_| IdentityJoinError::RecoverSigner)?;

    Ok(address_from_verifying_key(&verifying_key))
}

fn decode_signature(signature_hex: &str) -> Result<[u8; 65], IdentityJoinError> {
    let raw = hex::decode(normalize_hex(signature_hex))
        .map_err(|_| IdentityJoinError::InvalidSignature)?;
    let bytes: [u8; 65] = raw.try_into().map_err(|_| IdentityJoinError::InvalidSignature)?;
    Ok(bytes)
}

fn recovery_id(v: u8) -> Result<RecoveryId, IdentityJoinError> {
    let normalized = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        _ => return Err(IdentityJoinError::InvalidSignature),
    };
    RecoveryId::try_from(normalized).map_err(|_| IdentityJoinError::InvalidSignature)
}

fn address_from_verifying_key(verifying_key: &VerifyingKey) -> String {
    let public_key = verifying_key.to_encoded_point(false);
    let public_key = public_key.as_bytes();
    let hash = Keccak256::digest(&public_key[1..]);
    hex::encode(&hash[12..])
}

fn normalize_hex(value: &str) -> String {
    value
        .trim()
        .strip_prefix("0x")
        .or_else(|| value.trim().strip_prefix("0X"))
        .unwrap_or_else(|| value.trim())
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use k256::ecdsa::SigningKey;

    use super::*;
    use crate::{room, transport::IrohRoomMembership};

    fn envelope_for(
        signing_key: &SigningKey,
        room_id: [u8; 32],
        passport_id: &str,
        transport_pubkey_hex: &str,
    ) -> ChatJoinEnvelope {
        let topic = format!("0x{}", hex::encode(room_id));
        let mut envelope = ChatJoinEnvelope {
            agent_address: address_from_verifying_key(signing_key.verifying_key()),
            passport_id: passport_id.into(),
            topic,
            nonce: "nonce-1".into(),
            ts_ms: 1_000,
            transport_pubkey_hex: transport_pubkey_hex.into(),
            signature: String::new(),
        };
        envelope.signature = sign_message(signing_key, &envelope.message());
        envelope
    }

    fn sign_message(signing_key: &SigningKey, message: &str) -> String {
        let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
        let mut hasher = Keccak256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(message.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut out = Vec::from(signature.to_bytes().as_slice());
        out.push(u8::from(recovery_id));
        format!("0x{}", hex::encode(out))
    }

    #[test]
    fn message_matches_nunchi_cli_join_domain() {
        let envelope = ChatJoinEnvelope {
            agent_address: "0xabc".into(),
            passport_id: "7".into(),
            topic: "topic-1".into(),
            nonce: "nonce-1".into(),
            ts_ms: 42,
            transport_pubkey_hex: "0x01".into(),
            signature: "0x".into(),
        };

        assert_eq!(envelope.message(), "nunchi-chat.join\ntopic-1\nnonce-1\n42");
    }

    #[test]
    fn verifies_identity_from_evm_signature() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let envelope = envelope_for(&signing_key, room_id, "9", "0x01");

        let identity = envelope.verify_identity().unwrap();

        assert_eq!(identity.agent_address, envelope.agent_address.trim_start_matches("0x"));
        assert_eq!(identity.passport_id, "9");
        assert_eq!(identity.transport_pubkey_hex, "01");
    }

    #[test]
    fn rejects_signature_for_different_agent() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let mut envelope = envelope_for(&signing_key, room_id, "9", "0x01");
        envelope.agent_address = "0x0000000000000000000000000000000000000001".into();

        assert!(matches!(
            envelope.verify_identity(),
            Err(IdentityJoinError::SignerMismatch { .. })
        ));
    }

    #[test]
    fn room_join_requires_expected_member() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let envelope = envelope_for(&signing_key, room_id, "9", "0x01");
        let membership = IrohRoomMembership::new(
            room_id,
            vec![ChatParticipantIdentity::new(
                &envelope.agent_address,
                &envelope.passport_id,
                &envelope.transport_pubkey_hex,
            )],
        )
        .unwrap();

        let joined = envelope.verify_room_join(&membership, 1_500, 1_000).unwrap();

        assert_eq!(joined.agent_address, envelope.agent_address.trim_start_matches("0x"));
    }

    #[test]
    fn room_join_rejects_unexpected_transport_key() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let envelope = envelope_for(&signing_key, room_id, "9", "0x01");
        let membership = IrohRoomMembership::new(
            room_id,
            vec![ChatParticipantIdentity::new(&envelope.agent_address, "9", "0x02")],
        )
        .unwrap();

        assert!(matches!(
            envelope.verify_room_join(&membership, 1_500, 1_000),
            Err(IdentityJoinError::UnexpectedMember { .. })
        ));
    }
}
