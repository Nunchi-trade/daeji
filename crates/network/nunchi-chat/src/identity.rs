//! Contracts-core identity verification primitives for Nunchi Chat.
//!
//! `nunchi-cli` resolves the active agent from contracts-core identity state
//! and signs EVM messages with that account. This module verifies the matching
//! join and per-message envelopes on the chat side and maps them into room
//! membership.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};

use crate::{
    messages::RoomMessage,
    transport::{ChatParticipantIdentity, IrohRoomMembership},
};

/// EVM signed-message domain used by `nunchi-cli` for chat joins.
pub const JOIN_CHALLENGE_DOMAIN: &str = "nunchi-chat.join";

/// EVM signed-message domain for every chat room message.
pub const MESSAGE_CHALLENGE_DOMAIN: &str = "nunchi-chat.message";

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

/// Protocol-level policy for message signature verification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageSignaturePolicy {
    /// Require a valid EIP-191 signature on every chat message.
    #[default]
    Required,
    /// Allow unsigned messages for local or private testnet v0 builds only.
    ///
    /// This mode is a ship-speed carve-out and must not be used in public or
    /// external deployments.
    LocalTestnetDisabled,
}

impl MessageSignaturePolicy {
    /// True when unsigned messages are allowed by this policy.
    pub const fn allows_unsigned_messages(self) -> bool {
        matches!(self, Self::LocalTestnetDisabled)
    }
}

/// Chat message plus the contracts-core identity signature that authored it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRoomMessage {
    /// Agent EVM address resolved by `nunchi-cli`.
    pub agent_address: String,
    /// ERC-8004 passport id for the same agent.
    pub passport_id: String,
    /// 0x-prefixed room id.
    pub room_id: String,
    /// Client-generated replay nonce.
    pub nonce: String,
    /// Client timestamp in milliseconds.
    pub ts_ms: u64,
    /// Chat transport key or Iroh node routing key for this agent.
    pub transport_pubkey_hex: String,
    /// Signed room payload.
    pub message: RoomMessage,
    /// EIP-191 personal-sign signature over [`SignedRoomMessage::signing_message`].
    pub signature: String,
}

/// A room message that has passed signature and membership checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRoomMessage {
    /// Verified sender identity.
    pub sender: ChatParticipantIdentity,
    /// Verified room message payload.
    pub message: RoomMessage,
}

/// A received room message before signature policy is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "signature", content = "body", rename_all = "kebab-case")]
pub enum ReceivedRoomMessage {
    /// Message carries a contracts-core EVM identity signature.
    Signed(SignedRoomMessage),
    /// Unsigned legacy/local-testnet message.
    Unsigned(RoomMessage),
}

/// A message admitted by the current signature policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptedRoomMessage {
    /// Message was verified against contracts-core identity membership.
    Verified(VerifiedRoomMessage),
    /// Message was admitted only because signature verification is disabled.
    VerificationDisabled(RoomMessage),
}

impl ReceivedRoomMessage {
    /// Apply protocol drop rules. Unsigned or invalid messages return `None`
    /// whenever [`MessageSignaturePolicy::Required`] is active.
    pub fn accept_or_drop(
        &self,
        policy: MessageSignaturePolicy,
        membership: &IrohRoomMembership,
        now_ms: u64,
        max_age_ms: u64,
    ) -> Option<AcceptedRoomMessage> {
        match (policy, self) {
            (MessageSignaturePolicy::Required, Self::Signed(signed)) => signed
                .verify_or_drop(membership, now_ms, max_age_ms)
                .map(AcceptedRoomMessage::Verified),
            (MessageSignaturePolicy::Required, Self::Unsigned(_)) => None,
            (MessageSignaturePolicy::LocalTestnetDisabled, Self::Signed(signed)) => {
                Some(AcceptedRoomMessage::VerificationDisabled(signed.message.clone()))
            }
            (MessageSignaturePolicy::LocalTestnetDisabled, Self::Unsigned(message)) => {
                Some(AcceptedRoomMessage::VerificationDisabled(message.clone()))
            }
        }
    }
}

impl SignedRoomMessage {
    /// Construct the exact UTF-8 message signed by the sender.
    pub fn signing_message(&self) -> Result<String, IdentityVerificationError> {
        Ok(format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}",
            MESSAGE_CHALLENGE_DOMAIN,
            self.room_id,
            self.agent_address,
            self.transport_pubkey_hex,
            self.nonce,
            self.ts_ms,
            self.payload_hash_hex()?
        ))
    }

    /// Verify signature, freshness, room id, and membership.
    pub fn verify(
        &self,
        membership: &IrohRoomMembership,
        now_ms: u64,
        max_age_ms: u64,
    ) -> Result<VerifiedRoomMessage, IdentityVerificationError> {
        validate_freshness(self.ts_ms, now_ms, max_age_ms)?;

        let expected_room = format!("0x{}", hex::encode(membership.room_id));
        if normalize_hex(&self.room_id) != normalize_hex(&expected_room) {
            return Err(IdentityVerificationError::RoomMismatch {
                expected: expected_room,
                actual: self.room_id.clone(),
            });
        }

        let recovered = recover_personal_sign_address(&self.signing_message()?, &self.signature)?;
        let claimed = normalize_hex(&self.agent_address);
        if recovered != claimed {
            return Err(IdentityVerificationError::SignerMismatch { claimed, recovered });
        }

        let sender = ChatParticipantIdentity::new(
            &self.agent_address,
            &self.passport_id,
            &self.transport_pubkey_hex,
        );
        if !membership.contains_identity(&sender) {
            return Err(IdentityVerificationError::UnexpectedMember {
                agent_address: sender.agent_address,
                passport_id: sender.passport_id,
                transport_pubkey_hex: sender.transport_pubkey_hex,
            });
        }

        Ok(VerifiedRoomMessage { sender, message: self.message.clone() })
    }

    /// Verify a message and return `None` for the required silent-drop path.
    pub fn verify_or_drop(
        &self,
        membership: &IrohRoomMembership,
        now_ms: u64,
        max_age_ms: u64,
    ) -> Option<VerifiedRoomMessage> {
        self.verify(membership, now_ms, max_age_ms).ok()
    }

    fn payload_hash_hex(&self) -> Result<String, IdentityVerificationError> {
        let payload =
            serde_json::to_vec(&self.message).map_err(IdentityVerificationError::Payload)?;
        let hash = Keccak256::digest(&payload);
        Ok(hex::encode(hash))
    }
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
        validate_freshness(self.ts_ms, now_ms, max_age_ms)?;

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

/// Errors while verifying a Nunchi Chat signed message.
#[derive(Debug, thiserror::Error)]
pub enum IdentityVerificationError {
    /// Signature is not valid hex or not 65 bytes.
    #[error("invalid EVM signature")]
    InvalidSignature,
    /// Signature recovery failed.
    #[error("failed to recover EVM signer")]
    RecoverSigner,
    /// Failed to serialize the signed room payload.
    #[error("failed to serialize signed room payload: {0}")]
    Payload(serde_json::Error),
    /// Recovered signer did not match claimed agent address.
    #[error("message signer mismatch: claimed {claimed}, recovered {recovered}")]
    SignerMismatch {
        /// Claimed agent address.
        claimed: String,
        /// Recovered signer address.
        recovered: String,
    },
    /// Signed message room id did not match the receiving room.
    #[error("message room mismatch: expected {expected}, got {actual}")]
    RoomMismatch {
        /// Expected room id.
        expected: String,
        /// Actual room id.
        actual: String,
    },
    /// Message timestamp was outside the allowed replay window.
    #[error("stale message challenge: age {age_ms}ms exceeds {max_age_ms}ms")]
    StaleChallenge {
        /// Message age in milliseconds.
        age_ms: u64,
        /// Maximum allowed age in milliseconds.
        max_age_ms: u64,
    },
    /// The verified identity is not expected in the room.
    #[error("verified message identity is not an expected room member")]
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
) -> Result<String, IdentityVerificationError> {
    let signature = decode_signature(signature_hex)?;
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();

    let sig = Signature::from_slice(&signature[..64])
        .map_err(|_| IdentityVerificationError::InvalidSignature)?;
    let recovery_id = recovery_id(signature[64])?;
    let verifying_key = VerifyingKey::recover_from_prehash(&digest, &sig, recovery_id)
        .map_err(|_| IdentityVerificationError::RecoverSigner)?;

    Ok(address_from_verifying_key(&verifying_key))
}

fn decode_signature(signature_hex: &str) -> Result<[u8; 65], IdentityVerificationError> {
    let raw = hex::decode(normalize_hex(signature_hex))
        .map_err(|_| IdentityVerificationError::InvalidSignature)?;
    let bytes: [u8; 65] =
        raw.try_into().map_err(|_| IdentityVerificationError::InvalidSignature)?;
    Ok(bytes)
}

fn recovery_id(v: u8) -> Result<RecoveryId, IdentityVerificationError> {
    let normalized = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        _ => return Err(IdentityVerificationError::InvalidSignature),
    };
    RecoveryId::try_from(normalized).map_err(|_| IdentityVerificationError::InvalidSignature)
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

const fn validate_freshness(
    ts_ms: u64,
    now_ms: u64,
    max_age_ms: u64,
) -> Result<(), IdentityVerificationError> {
    let age = now_ms.abs_diff(ts_ms);
    if age > max_age_ms {
        return Err(IdentityVerificationError::StaleChallenge { age_ms: age, max_age_ms });
    }
    Ok(())
}

impl From<IdentityVerificationError> for IdentityJoinError {
    fn from(err: IdentityVerificationError) -> Self {
        match err {
            IdentityVerificationError::InvalidSignature => Self::InvalidSignature,
            IdentityVerificationError::RecoverSigner => Self::RecoverSigner,
            IdentityVerificationError::SignerMismatch { claimed, recovered } => {
                Self::SignerMismatch { claimed, recovered }
            }
            IdentityVerificationError::StaleChallenge { age_ms, max_age_ms } => {
                Self::StaleChallenge { age_ms, max_age_ms }
            }
            IdentityVerificationError::UnexpectedMember {
                agent_address,
                passport_id,
                transport_pubkey_hex,
            } => Self::UnexpectedMember { agent_address, passport_id, transport_pubkey_hex },
            IdentityVerificationError::Payload(_)
            | IdentityVerificationError::RoomMismatch { .. } => Self::RecoverSigner,
        }
    }
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

    fn signed_room_message_for(
        signing_key: &SigningKey,
        room_id: [u8; 32],
        passport_id: &str,
        transport_pubkey_hex: &str,
    ) -> SignedRoomMessage {
        let mut signed = SignedRoomMessage {
            agent_address: address_from_verifying_key(signing_key.verifying_key()),
            passport_id: passport_id.into(),
            room_id: format!("0x{}", hex::encode(room_id)),
            nonce: "msg-nonce-1".into(),
            ts_ms: 1_000,
            transport_pubkey_hex: transport_pubkey_hex.into(),
            message: RoomMessage::Status {
                from_pubkey_hex: transport_pubkey_hex.into(),
                phase: "working".into(),
                eta_blocks: 3,
            },
            signature: String::new(),
        };
        signed.signature = sign_message(signing_key, &signed.signing_message().unwrap());
        signed
    }

    fn membership_for(room_id: [u8; 32], identity: &SignedRoomMessage) -> IrohRoomMembership {
        IrohRoomMembership::new(
            room_id,
            vec![ChatParticipantIdentity::new(
                &identity.agent_address,
                &identity.passport_id,
                &identity.transport_pubkey_hex,
            )],
        )
        .unwrap()
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
    fn signed_room_message_uses_message_domain_and_payload_hash() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let signed = signed_room_message_for(&signing_key, room_id, "9", "0x01");
        let signing_message = signed.signing_message().unwrap();

        assert!(signing_message.starts_with("nunchi-chat.message\n"));
        assert!(signing_message.contains("\nmsg-nonce-1\n1000\n"));
        assert!(signing_message.ends_with(&signed.payload_hash_hex().unwrap()));
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

    #[test]
    fn verifies_signed_room_message_against_expected_member() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let signed = signed_room_message_for(&signing_key, room_id, "9", "0x01");
        let membership = membership_for(room_id, &signed);

        let verified = signed.verify(&membership, 1_500, 1_000).unwrap();

        assert_eq!(verified.sender.agent_address, signed.agent_address);
        assert_eq!(verified.message, signed.message);
    }

    #[test]
    fn signed_room_message_rejects_wrong_signer() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let mut signed = signed_room_message_for(&signing_key, room_id, "9", "0x01");
        let membership = membership_for(room_id, &signed);
        signed.agent_address = "0x0000000000000000000000000000000000000001".into();

        assert!(matches!(
            signed.verify(&membership, 1_500, 1_000),
            Err(IdentityVerificationError::SignerMismatch { .. })
        ));
        assert!(signed.verify_or_drop(&membership, 1_500, 1_000).is_none());
    }

    #[test]
    fn required_policy_drops_unsigned_and_invalid_messages() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let signed = signed_room_message_for(&signing_key, room_id, "9", "0x01");
        let membership = membership_for(room_id, &signed);
        let unsigned = ReceivedRoomMessage::Unsigned(RoomMessage::Hello {
            from_pubkey_hex: "0x01".into(),
            wall_clock_ms: 42,
        });

        assert!(
            unsigned
                .accept_or_drop(MessageSignaturePolicy::Required, &membership, 1_500, 1_000)
                .is_none()
        );

        let mut invalid = signed;
        invalid.signature = "0x00".into();
        assert!(
            ReceivedRoomMessage::Signed(invalid)
                .accept_or_drop(MessageSignaturePolicy::Required, &membership, 1_500, 1_000)
                .is_none()
        );
    }

    #[test]
    fn received_room_message_serializes_with_signature_tag() {
        let unsigned = ReceivedRoomMessage::Unsigned(RoomMessage::Hello {
            from_pubkey_hex: "0x01".into(),
            wall_clock_ms: 42,
        });

        let value = serde_json::to_value(&unsigned).unwrap();

        assert_eq!(value["signature"], "unsigned");
        assert_eq!(value["body"]["type"], "hello");
    }

    #[test]
    fn local_testnet_policy_can_admit_unsigned_messages() {
        let signing_key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let room_id = room::room_id_for_chain_job(7);
        let signed = signed_room_message_for(&signing_key, room_id, "9", "0x01");
        let membership = membership_for(room_id, &signed);
        let unsigned = ReceivedRoomMessage::Unsigned(RoomMessage::Hello {
            from_pubkey_hex: "0x01".into(),
            wall_clock_ms: 42,
        });

        assert!(matches!(
            unsigned.accept_or_drop(
                MessageSignaturePolicy::LocalTestnetDisabled,
                &membership,
                1_500,
                1_000
            ),
            Some(AcceptedRoomMessage::VerificationDisabled(RoomMessage::Hello { .. }))
        ));
    }
}
