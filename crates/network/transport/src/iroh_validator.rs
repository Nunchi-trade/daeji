//! Iroh-backed validator transport planning primitives.
//!
//! This module is the first, compile-safe slice of the validator Iroh adapter.
//! It models the identity binding and explicit all-to-all fanout plan that the
//! concrete QUIC transport will use later. It intentionally does not use gossip:
//! consensus broadcasts remain per-validator sends over the current peer set.

use std::{collections::BTreeSet, fmt};

use crate::TransportError;

/// ALPN for validator-network traffic over Iroh.
pub const DEFAULT_VALIDATOR_ALPN: &[u8] = b"nunchi-validator-v1";

const CHANNEL_FRAME_HEADER_LEN: usize = 12;

/// Relay policy for an Iroh-backed validator endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum IrohRelayMode {
    /// Use Iroh's default relay behavior.
    #[default]
    Default,
    /// Disable relays. Useful for local tests and direct-connect smoke tests.
    Disabled,
    /// Use an operator-provided relay endpoint.
    Custom(String),
}

/// Binding from Kora validator identity to Iroh node identity.
///
/// Validator identity remains the consensus identity. The Iroh node id is only
/// the routing identity used by the transport adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct IrohValidatorBinding {
    /// Hex-encoded validator public key used by the current validator set.
    pub validator_public_key_hex: String,
    /// Iroh node id for the validator's transport endpoint.
    pub node_id: String,
}

/// One channel-routed payload carried over an Iroh validator stream.
///
/// This preserves the existing Kora transport model where consensus and marshal
/// consumers register numeric channels. The concrete Iroh adapter can send this
/// frame over QUIC streams while demultiplexing inbound bytes back into the
/// current channel queues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrohChannelFrame {
    /// Numeric Kora transport channel id.
    pub channel_id: u64,
    /// Opaque channel payload.
    pub payload: Vec<u8>,
}

impl IrohChannelFrame {
    /// Create a new channel frame.
    pub fn new(channel_id: u64, payload: impl Into<Vec<u8>>) -> Self {
        Self { channel_id, payload: payload.into() }
    }

    /// Encode as `channel_id_le(8) || payload_len_le(4) || payload`.
    pub fn encode(&self) -> Vec<u8> {
        let payload_len: u32 = self.payload.len().try_into().expect("payload larger than u32::MAX");
        let mut out = Vec::with_capacity(CHANNEL_FRAME_HEADER_LEN + self.payload.len());
        out.extend_from_slice(&self.channel_id.to_le_bytes());
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Decode a channel frame.
    pub fn decode(wire: &[u8]) -> Result<Self, TransportError> {
        if wire.len() < CHANNEL_FRAME_HEADER_LEN {
            return Err(TransportError::InvalidIrohFrameHeader);
        }

        let mut channel = [0u8; 8];
        channel.copy_from_slice(&wire[..8]);
        let channel_id = u64::from_le_bytes(channel);

        let mut len = [0u8; 4];
        len.copy_from_slice(&wire[8..CHANNEL_FRAME_HEADER_LEN]);
        let declared = u32::from_le_bytes(len) as usize;
        let payload = &wire[CHANNEL_FRAME_HEADER_LEN..];

        if declared != payload.len() {
            return Err(TransportError::InvalidIrohFrameLength { declared, actual: payload.len() });
        }

        Ok(Self { channel_id, payload: payload.to_vec() })
    }
}

impl IrohValidatorBinding {
    /// Create a new validator-to-node binding.
    pub fn new(validator_public_key_hex: impl Into<String>, node_id: impl Into<String>) -> Self {
        Self {
            validator_public_key_hex: normalize_hex(validator_public_key_hex.into()),
            node_id: node_id.into(),
        }
    }
}

impl fmt::Debug for IrohValidatorBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohValidatorBinding")
            .field("validator_public_key_hex", &self.validator_public_key_hex)
            .field("node_id", &self.node_id)
            .finish()
    }
}

/// Configuration needed before constructing an Iroh-backed validator transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrohValidatorConfig {
    /// ALPN used to separate validator traffic from any other Iroh protocols.
    pub alpn: Vec<u8>,
    /// Relay behavior for endpoint connectivity.
    pub relay_mode: IrohRelayMode,
    /// Current validator-to-Iroh identity bindings.
    pub bindings: Vec<IrohValidatorBinding>,
}

impl Default for IrohValidatorConfig {
    fn default() -> Self {
        Self {
            alpn: DEFAULT_VALIDATOR_ALPN.to_vec(),
            relay_mode: IrohRelayMode::default(),
            bindings: Vec::new(),
        }
    }
}

impl IrohValidatorConfig {
    /// Set the relay policy.
    #[must_use]
    pub fn with_relay_mode(mut self, relay_mode: IrohRelayMode) -> Self {
        self.relay_mode = relay_mode;
        self
    }

    /// Set the validator-to-node bindings.
    #[must_use]
    pub fn with_bindings(mut self, bindings: Vec<IrohValidatorBinding>) -> Self {
        self.bindings = bindings;
        self
    }

    /// Validate that validator and Iroh node identities are both one-to-one.
    pub fn validate_unique_bindings(&self) -> Result<(), TransportError> {
        let mut validators = BTreeSet::new();
        let mut nodes = BTreeSet::new();

        for binding in &self.bindings {
            if !validators.insert(binding.validator_public_key_hex.clone()) {
                return Err(TransportError::DuplicateIrohValidatorBinding(
                    binding.validator_public_key_hex.clone(),
                ));
            }
            if !nodes.insert(binding.node_id.clone()) {
                return Err(TransportError::DuplicateIrohNodeBinding(binding.node_id.clone()));
            }
        }

        Ok(())
    }

    /// Return the binding for a validator public key.
    pub fn binding_for_validator(
        &self,
        validator_public_key_hex: &str,
    ) -> Result<&IrohValidatorBinding, TransportError> {
        let key = normalize_hex(validator_public_key_hex.to_string());
        self.bindings
            .iter()
            .find(|binding| binding.validator_public_key_hex == key)
            .ok_or(TransportError::MissingIrohValidatorBinding(key))
    }

    /// Build the explicit all-to-all fanout target list for one sender.
    ///
    /// Consensus broadcasts are modeled as direct sends to every other
    /// validator. The eventual Iroh transport can route those sends directly,
    /// via rendezvous, or through relay fallback without changing this target
    /// set.
    pub fn all_to_all_targets(
        &self,
        sender_validator_public_key_hex: &str,
    ) -> Result<Vec<&IrohValidatorBinding>, TransportError> {
        let sender = self.binding_for_validator(sender_validator_public_key_hex)?;
        Ok(self
            .bindings
            .iter()
            .filter(|binding| binding.validator_public_key_hex != sender.validator_public_key_hex)
            .collect())
    }
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

    fn binding(validator: &str, node: &str) -> IrohValidatorBinding {
        IrohValidatorBinding::new(validator, node)
    }

    #[test]
    fn default_config_uses_validator_alpn() {
        let cfg = IrohValidatorConfig::default();
        assert_eq!(cfg.alpn, DEFAULT_VALIDATOR_ALPN);
        assert_eq!(cfg.relay_mode, IrohRelayMode::Default);
    }

    #[test]
    fn bindings_normalize_validator_hex() {
        let b = binding("0xAABB", "node-a");
        assert_eq!(b.validator_public_key_hex, "aabb");
    }

    #[test]
    fn channel_frame_round_trips() {
        let frame = IrohChannelFrame::new(42, b"hello".to_vec());
        let decoded = IrohChannelFrame::decode(&frame.encode()).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn channel_frame_rejects_short_header() {
        assert!(matches!(
            IrohChannelFrame::decode(&[1, 2, 3]),
            Err(TransportError::InvalidIrohFrameHeader)
        ));
    }

    #[test]
    fn channel_frame_rejects_length_mismatch() {
        let mut wire = IrohChannelFrame::new(42, b"hello".to_vec()).encode();
        wire[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert!(matches!(
            IrohChannelFrame::decode(&wire),
            Err(TransportError::InvalidIrohFrameLength { declared: 99, actual: 5 })
        ));
    }

    #[test]
    fn validate_unique_bindings_accepts_one_to_one_mapping() {
        let cfg = IrohValidatorConfig::default()
            .with_bindings(vec![binding("0x01", "node-a"), binding("0x02", "node-b")]);
        cfg.validate_unique_bindings().unwrap();
    }

    #[test]
    fn validate_unique_bindings_rejects_duplicate_validator() {
        let cfg = IrohValidatorConfig::default()
            .with_bindings(vec![binding("0x01", "node-a"), binding("01", "node-b")]);
        assert!(matches!(
            cfg.validate_unique_bindings(),
            Err(TransportError::DuplicateIrohValidatorBinding(key)) if key == "01"
        ));
    }

    #[test]
    fn validate_unique_bindings_rejects_duplicate_node_id() {
        let cfg = IrohValidatorConfig::default()
            .with_bindings(vec![binding("0x01", "node-a"), binding("0x02", "node-a")]);
        assert!(matches!(
            cfg.validate_unique_bindings(),
            Err(TransportError::DuplicateIrohNodeBinding(node)) if node == "node-a"
        ));
    }

    #[test]
    fn binding_lookup_fails_closed_when_missing() {
        let cfg = IrohValidatorConfig::default().with_bindings(vec![binding("0x01", "node-a")]);
        assert!(matches!(
            cfg.binding_for_validator("0x02"),
            Err(TransportError::MissingIrohValidatorBinding(key)) if key == "02"
        ));
    }

    #[test]
    fn all_to_all_targets_excludes_sender() {
        let cfg = IrohValidatorConfig::default().with_bindings(vec![
            binding("0x01", "node-a"),
            binding("0x02", "node-b"),
            binding("0x03", "node-c"),
        ]);

        let targets = cfg.all_to_all_targets("0x01").unwrap();
        let node_ids: Vec<_> = targets.iter().map(|binding| binding.node_id.as_str()).collect();

        assert_eq!(node_ids, vec!["node-b", "node-c"]);
    }
}
