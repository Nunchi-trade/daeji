//! Vector search algorithms.
//!
//! This module will contain brute-force and HNSW search implementations.
//! See implementation plan `03-vector-search.md`.

// Re-export core functions needed by search consumers.
pub use crate::vector::hamming_distance;
pub use crate::vector::similarity;
pub use crate::constants::{THRESHOLD_HAMMING, DUPLICATE_THRESHOLD, RESONANCE_THRESHOLD_HAMMING};
