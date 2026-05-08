//! Knowledge store -- local agent memory with decay, tiers, and anti-knowledge.

pub mod anti;
pub mod decay;
pub mod entry;
pub mod kind;
pub mod scoring;
pub mod source;
pub mod store;
pub mod tier;
pub mod trust;

pub use anti::{
    ANTI_SUBSPACE, AntiCheckedResult, AntiClassification, classify_anti_signal, encode_anti,
    is_anti,
};
pub use decay::{GC_THRESHOLD, compute_decayed_balance, ticks_to_hours};
pub use entry::KnowledgeEntry;
pub use kind::KnowledgeKind;
pub use scoring::{RetrievalContext, ScoredEntry};
pub use source::KnowledgeSource;
pub use store::KnowledgeStore;
pub use tier::KnowledgeTier;
pub use trust::{COLD_START_REPUTATION, MIN_TRUST_THRESHOLD, compute_trust};
