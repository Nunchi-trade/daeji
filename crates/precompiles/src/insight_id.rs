//! Content-addressed insight identifier and the six knowledge kinds.
//!
//! Vendored from `roko/apps/mirage-rs/src/chain/insight.rs` (MIT OR Apache-2.0):
//! only `InsightId` + `KnowledgeKind` are pulled in. The full `InsightEntry`
//! state-machine lives off-chain in mirage-rs and the on-chain Solidity
//! `InsightBoard.sol` (see `~/contracts-core/packages/agents/src/InsightBoard.sol`);
//! the precompile only needs the id type for the corpus index.

use serde::{Deserialize, Serialize};

/// Content-addressed identifier for an insight (16 bytes).
///
/// Computed as a FNV-1a64 hash of (author ‖ content ‖ kind_tag). Deterministic
/// across nodes given identical inputs. 128 bits of raw space are emulated by
/// combining two FNV rounds; collisions are astronomically unlikely at realistic
/// entry counts (<1 in 10^10 at 10M entries).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InsightId(pub [u8; 16]);

impl InsightId {
    /// Computes the content-addressed id from the author, content bytes, and knowledge kind.
    #[must_use]
    pub fn derive(author: &[u8], content: &[u8], kind: KnowledgeKind) -> Self {
        let tag = kind.tag_byte();
        let mut lo: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in author.iter().chain(content).chain(std::iter::once(&tag)) {
            lo ^= u64::from(*byte);
            lo = lo.wrapping_mul(0x0100_0000_01b3);
        }
        let mut hi: u64 = lo ^ 0xA5A5_A5A5_5A5A_5A5A;
        for byte in content.iter().chain(author).rev() {
            hi ^= u64::from(*byte);
            hi = hi.wrapping_mul(0x0100_0000_01b3);
        }
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&lo.to_le_bytes());
        out[8..].copy_from_slice(&hi.to_le_bytes());
        Self(out)
    }

    /// Returns a hex representation of the id (32 lowercase characters, no prefix).
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(32);
        for byte in self.0 {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }
}

impl std::fmt::Display for InsightId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "insight:{}", self.to_hex())
    }
}

/// Typed body of a knowledge entry — six variants per Will's `02-daeji/00-INDEX`
/// glossary "Knowledge kinds".
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    /// A factual observation derived from task execution (what IS).
    Insight,
    /// A learned behavioural strategy for a specific context (what to DO).
    Heuristic,
    /// Knowledge about what NOT to do (failure modes, dead ends).
    Warning,
    /// An observed cause-and-effect relationship (mechanism).
    CausalLink,
    /// A reusable partial plan / sequence of steps.
    StrategyFragment,
    /// Explicitly wrong information that was once believed correct.
    AntiKnowledge,
}

impl KnowledgeKind {
    /// Returns a stable single-byte tag for hashing.
    #[must_use]
    pub const fn tag_byte(self) -> u8 {
        match self {
            Self::Insight => 0x01,
            Self::Heuristic => 0x02,
            Self::Warning => 0x03,
            Self::CausalLink => 0x04,
            Self::StrategyFragment => 0x05,
            Self::AntiKnowledge => 0x06,
        }
    }

    /// Default half-life in seconds for on-read decay (informational; the
    /// authoritative half-life table lives in `InsightBoard.sol`).
    #[must_use]
    pub const fn default_half_life_seconds(self) -> u64 {
        match self {
            Self::Warning => 7 * 86_400,            // 7 days
            Self::Insight => 30 * 86_400,           // 30 days
            Self::StrategyFragment => 60 * 86_400,  // 60 days
            Self::Heuristic => 90 * 86_400,         // 90 days
            Self::CausalLink => 180 * 86_400,       // 180 days
            Self::AntiKnowledge => 365 * 86_400,    // 365 days
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic() {
        let a = InsightId::derive(b"alice", b"hello", KnowledgeKind::Insight);
        let b = InsightId::derive(b"alice", b"hello", KnowledgeKind::Insight);
        assert_eq!(a, b);
    }

    #[test]
    fn derive_differs_on_kind() {
        let a = InsightId::derive(b"alice", b"hello", KnowledgeKind::Insight);
        let b = InsightId::derive(b"alice", b"hello", KnowledgeKind::Heuristic);
        assert_ne!(a, b);
    }

    #[test]
    fn to_hex_is_32_chars() {
        let id = InsightId::derive(b"a", b"b", KnowledgeKind::Insight);
        let hex = id.to_hex();
        assert_eq!(hex.len(), 32);
    }
}
