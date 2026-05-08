//! `kora-hdc` -- Hyperdimensional Computing core algebra.
//!
//! This crate provides the BSC (Binary Spatter Code) hyperdimensional
//! computing primitives used throughout Kora's cognitive substrate.
//!
//! # Consensus Safety
//!
//! Functions are annotated as either **CONSENSUS-SAFE** (integer-only,
//! deterministic, safe for on-chain use) or **OFF-CHAIN ONLY** (uses
//! floating-point, for local scoring and display only).
//!
//! # Core Types
//!
//! - [`HdcVector`] -- 10,240-bit binary hypervector
//! - [`BundleAccumulator`] -- streaming majority-vote bundler
//!
//! # Core Operations
//!
//! - [`bind`] -- XOR association (self-inverse)
//! - [`bundle()`] -- majority-vote superposition
//! - [`permute`] -- cyclic bit rotation (sequence encoding)
//! - [`hamming_distance`] -- integer distance (on-chain)
//! - [`similarity`] -- normalized float similarity (off-chain only)

pub mod bundle;
pub mod cognitive;
pub mod constants;
pub mod context;
pub mod encode;
pub mod knowledge;
pub mod search;
pub mod trust;
pub mod vector;

// Re-export primary types and functions at crate root for convenience.
pub use bundle::{BundleAccumulator, bundle};
pub use constants::*;
pub use context::{AssembledContext, ContextCandidate, assemble_context};
pub use encode::{ProjectionEncoder, StructuredEncoder, TrigramEncoder};
pub use knowledge::{KnowledgeEntry, KnowledgeKind, KnowledgeStore, KnowledgeTier};
pub use trust::{TrustPipeline, TrustRegistry, TrustScore};
pub use vector::{
    HdcVector, bind, deserialize, hamming_distance, permute, serialize, similarity, vector_id,
};
