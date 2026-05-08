//! Vector search algorithms for hyperdimensional computing.
//!
//! Provides brute-force linear scan, HNSW approximate nearest neighbor,
//! auto-switching local index, and tiered gas-optimized search pipeline.
//! All operations are consensus-safe (integer-only, deterministic).

use crate::HdcVector;

/// Opaque 32-byte key identifying a stored vector (keccak256 content hash).
pub type H256 = [u8; 32];

mod brute;
mod hnsw;
mod local;
pub(crate) mod simd;
mod tiered;

pub use brute::BruteForceIndex;
pub use hnsw::HnswIndex;
pub use local::LocalIndex;
pub use simd::hamming_distance;
pub use tiered::TieredSearchPipeline;

// Re-export legacy items for backwards compatibility with existing imports.
pub use crate::vector::hamming_distance as vector_hamming_distance;
pub use crate::{
    constants::{DUPLICATE_THRESHOLD, RESONANCE_THRESHOLD_HAMMING, THRESHOLD_HAMMING},
    vector::similarity,
};

/// Result type alias for search operations: vec of (key, distance) pairs.
pub type SearchResult = Vec<(H256, u32)>;

/// Unified trait for all index implementations.
pub trait SearchIndex {
    /// Insert a vector with its key. Returns error on duplicate key.
    fn insert(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError>;

    /// Remove a vector by key. Returns true if it existed.
    fn delete(&mut self, key: &H256) -> bool;

    /// Find the top_k closest vectors to `query`.
    /// Results are sorted by (distance ASC, key ASC) for determinism.
    fn search(&self, query: &HdcVector, top_k: usize) -> Result<SearchResult, HdcIndexError>;

    /// Number of live (non-tombstoned) vectors.
    fn len(&self) -> usize;

    /// Whether the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Errors from index operations.
#[derive(Debug)]
pub enum HdcIndexError {
    /// No vectors in the index.
    EmptyIndex,
    /// Requested top_k > stored vector count.
    InsufficientVectors {
        /// Number of results requested.
        requested: usize,
        /// Number of vectors available.
        available: usize,
    },
    /// Vector data is corrupt or wrong length.
    InvalidVector(String),
    /// A vector with this key already exists.
    DuplicateKey(H256),
    /// Fixed-capacity index is full.
    CapacityExceeded {
        /// Maximum capacity of the index.
        capacity: usize,
    },
    /// HNSW graph invariant violated.
    GraphCorruption(String),
    /// I/O error during persistence.
    StorageError(std::io::Error),
}

impl std::fmt::Display for HdcIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyIndex => write!(f, "cannot search an empty index"),
            Self::InsufficientVectors { requested, available } => {
                write!(f, "requested top-{requested} but only {available} vectors in index")
            }
            Self::InvalidVector(e) => write!(f, "invalid vector: {e}"),
            Self::DuplicateKey(k) => write!(f, "duplicate key: {k:?}"),
            Self::CapacityExceeded { capacity } => write!(f, "index capacity {capacity} exceeded"),
            Self::GraphCorruption(msg) => write!(f, "HNSW graph corruption: {msg}"),
            Self::StorageError(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for HdcIndexError {}

impl From<std::io::Error> for HdcIndexError {
    fn from(e: std::io::Error) -> Self {
        Self::StorageError(e)
    }
}
