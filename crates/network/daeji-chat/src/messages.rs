use serde::{Deserialize, Serialize};

/// Wire format for symphony-room messages. JSON-encoded over the channel.
/// Phase-1 POC: messages are AEAD-encrypted at the app layer with the room key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomMessage {
    /// First message a participant sends after joining the room. Establishes presence
    /// and a wall-clock baseline used for heartbeat / liveness checks.
    Hello {
        /// Sender's transport pubkey (0x-hex).
        from_pubkey_hex: String,
        /// Sender's local wall-clock at send time (ms since UNIX epoch).
        wall_clock_ms: u64,
    },
    /// Periodic progress update from a participant — used by other agents to dedup work
    /// and decide whether to wait or push their own partial.
    Status {
        /// Sender's transport pubkey (0x-hex).
        from_pubkey_hex: String,
        /// Free-form phase tag (e.g. `"reasoning"`, `"voting"`, `"settling"`).
        phase: String,
        /// Sender's estimate of remaining work, expressed in chain blocks.
        eta_blocks: u64,
    },
    /// A partial result. Multiple partials may be in-flight; voters reference them
    /// by `partial_id`.
    PartialResult {
        /// Sender's transport pubkey (0x-hex).
        from_pubkey_hex: String,
        /// Sender-assigned id, unique within `(from_pubkey_hex, room)`.
        partial_id: u64,
        /// keccak256 of the raw content bytes (0x-hex). Voters verify before voting.
        content_hash_hex: String,
        /// URL or other content-address for the raw content bytes.
        content_ref: String,
    },
    /// Approve or reject a peer's `PartialResult`.
    Vote {
        /// Sender's transport pubkey (0x-hex).
        from_pubkey_hex: String,
        /// `partial_id` from the `PartialResult` being voted on.
        partial_id: u64,
        /// `true` to approve, `false` to reject.
        agree: bool,
    },
    /// Authoritative settlement message — the result that gets returned on-chain.
    Final {
        /// Sender's transport pubkey (0x-hex).
        from_pubkey_hex: String,
        /// keccak256 of the final result bytes (0x-hex).
        result_hash_hex: String,
        /// ed25519 signature over `result_hash_hex` by the sender's transport key.
        signature_hex: String,
    },
}

impl RoomMessage {
    /// Stable variant tag — useful for logging without cloning the payload.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::Status { .. } => "status",
            Self::PartialResult { .. } => "partial_result",
            Self::Vote { .. } => "vote",
            Self::Final { .. } => "final",
        }
    }
}
