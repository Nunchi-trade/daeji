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
        /// Contribution score in bps (0..=10000) declared by this winner. Sums across
        /// all winners are passed to `MultiAgentMarket.submitMultiWithScores` for a
        /// proportional bounty split. Sum == 0 → equal split. Pre-bump senders that
        /// don't include this field default to 0 via `serde(default)`, preserving
        /// soft compat: a room with mixed-version agents falls back to equal split.
        ///
        /// For symphony ISFR, agents populate this with their class governance
        /// weight × 100 (lending → 6000, structured → 2500, funding → 1000, staking → 500).
        #[serde(default)]
        contribution_score_bps: u16,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_with_score_round_trips() {
        let m = RoomMessage::Final {
            from_pubkey_hex: "0xabcd".into(),
            result_hash_hex: "0xbeef".into(),
            signature_hex: "0xfeed".into(),
            contribution_score_bps: 6000,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: RoomMessage = serde_json::from_str(&json).unwrap();
        if let RoomMessage::Final { contribution_score_bps, .. } = back {
            assert_eq!(contribution_score_bps, 6000);
        } else {
            panic!("decoded wrong variant");
        }
    }

    #[test]
    fn final_without_score_field_defaults_to_zero_for_soft_compat() {
        // Pre-bump wire format: no `contribution_score_bps` field present.
        let legacy_json = r#"{"type":"final","from_pubkey_hex":"0xa","result_hash_hex":"0xb","signature_hex":"0xc"}"#;
        let m: RoomMessage = serde_json::from_str(legacy_json).unwrap();
        if let RoomMessage::Final { contribution_score_bps, .. } = m {
            assert_eq!(contribution_score_bps, 0);
        } else {
            panic!("decoded wrong variant");
        }
    }
}
