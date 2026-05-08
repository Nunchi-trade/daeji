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
//! - [`bundle`] -- majority-vote superposition
//! - [`permute`] -- cyclic bit rotation (sequence encoding)
//! - [`hamming_distance`] -- integer distance (on-chain)
//! - [`similarity`] -- normalized float similarity (off-chain only)

pub mod constants;
pub mod vector;
pub mod bundle;
pub mod encode;
pub mod search;

// Re-export primary types and functions at crate root for convenience.
pub use constants::*;
pub use vector::{HdcVector, bind, hamming_distance, similarity, permute, serialize, deserialize, vector_id};
pub use bundle::{BundleAccumulator, bundle};
pub use encode::{TrigramEncoder, ProjectionEncoder, StructuredEncoder};
