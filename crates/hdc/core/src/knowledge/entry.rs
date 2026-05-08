//! Knowledge entry -- a single piece of knowledge in the local store.

use super::{KnowledgeKind, KnowledgeSource, KnowledgeTier};
use crate::{HdcVector, cognitive::affect::PadState};

/// A single knowledge entry in the local store.
#[derive(Clone, Debug)]
pub struct KnowledgeEntry {
    /// Unique identifier. Computed as `vector_id(&vector)` at creation time.
    pub key: [u8; 32],
    /// The 10,240-bit HDC vector encoding this knowledge.
    pub vector: HdcVector,
    /// Which of the six knowledge kinds.
    pub kind: KnowledgeKind,
    /// Current retention tier.
    pub tier: KnowledgeTier,
    /// Human-readable content (the text that was encoded into the vector).
    pub content: String,
    /// Origin of this knowledge.
    pub source: KnowledgeSource,
    /// Number of independent confirmations received.
    pub confirmations: u32,
    /// Tick when this entry was last accessed (queried or confirmed).
    pub last_accessed: u64,
    /// Tick when decay was last applied to this entry.
    pub last_decay_tick: u64,
    /// Current balance in [0.0, 1.0]. Decays exponentially over time.
    pub balance: f64,
    /// PAD emotional state captured at creation time.
    pub emotional_tag: PadState,
    /// True if anti-knowledge resonance was detected against this entry.
    pub contradicted: bool,
}

impl KnowledgeEntry {
    /// Create a new entry with sensible defaults.
    ///
    /// The emotional tag defaults to neutral PAD state. Use
    /// [`KnowledgeEntry::with_emotional_tag`] to attach a specific mood.
    pub fn new(
        vector: HdcVector,
        kind: KnowledgeKind,
        content: String,
        source: KnowledgeSource,
        current_tick: u64,
    ) -> Self {
        let key = crate::vector_id(&vector);
        Self {
            key,
            vector,
            kind,
            tier: KnowledgeTier::Transient,
            content,
            source,
            confirmations: 0,
            last_accessed: current_tick,
            last_decay_tick: current_tick,
            balance: 1.0,
            emotional_tag: PadState::neutral(),
            contradicted: false,
        }
    }

    /// Set the emotional tag on this entry (builder pattern).
    pub fn with_emotional_tag(mut self, pad: PadState) -> Self {
        self.emotional_tag = pad;
        self
    }
}
