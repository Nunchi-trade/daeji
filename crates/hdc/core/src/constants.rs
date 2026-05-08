//! Canonical constants for the HDC algebra.
//!
//! These values are consensus-critical. Changing any of them produces
//! incompatible vectors. Do not modify without a coordinated migration.

/// Dimensionality of hypervectors in bits.
pub const D: usize = 10_240;

/// Number of u64 words per vector. `D / 64 = 160`.
pub const WORDS: usize = 160;

/// Number of bytes per serialized vector. `D / 8 = 1,280`.
pub const BYTES: usize = 1_280;

/// Hamming-distance threshold corresponding to normalized similarity > 0.526.
pub const THRESHOLD_HAMMING: u32 = 4_854;

/// Hamming distance below which two vectors are considered near-duplicates.
pub const DUPLICATE_THRESHOLD: u32 = 512;

/// Hamming distance below which two vectors are considered to "resonate".
pub const RESONANCE_THRESHOLD_HAMMING: u32 = 1_024;
