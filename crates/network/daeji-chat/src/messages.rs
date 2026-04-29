use serde::{Deserialize, Serialize};

/// Wire format for symphony-room messages. JSON-encoded over the channel.
/// Phase-1 POC: messages are AEAD-encrypted at the app layer with the room key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomMessage {
    Hello {
        from_pubkey_hex: String,
        wall_clock_ms: u64,
    },
    Status {
        from_pubkey_hex: String,
        phase: String,
        eta_blocks: u64,
    },
    PartialResult {
        from_pubkey_hex: String,
        partial_id: u64,
        content_hash_hex: String,
        content_ref: String,
    },
    Vote {
        from_pubkey_hex: String,
        partial_id: u64,
        agree: bool,
    },
    Final {
        from_pubkey_hex: String,
        result_hash_hex: String,
        signature_hex: String,
    },
}

impl RoomMessage {
    pub fn label(&self) -> &'static str {
        match self {
            RoomMessage::Hello { .. } => "hello",
            RoomMessage::Status { .. } => "status",
            RoomMessage::PartialResult { .. } => "partial_result",
            RoomMessage::Vote { .. } => "vote",
            RoomMessage::Final { .. } => "final",
        }
    }
}
