//! `kora-hdc` -- Hyperdimensional Computing core algebra.

/// Canonical constants for the HDC algebra.
pub mod constants;
/// Core hypervector type and operations.
pub mod vector;
/// Bundle (majority-vote superposition) operations.
pub mod bundle;
/// Encoding helpers for mapping data into hypervectors.
pub mod encode;
/// Similarity search over hypervector collections.
pub mod search;
