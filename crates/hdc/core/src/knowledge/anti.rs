//! Anti-knowledge subspace and detection.
//!
//! Anti-knowledge encodes "this is false/harmful" in a structurally distinct
//! subspace. It uses algebraic binding with a fixed subspace vector so that
//! anti-knowledge vectors are quasi-orthogonal to the knowledge they negate.

use std::sync::LazyLock;

use crate::{HdcVector, bind, constants::D, hamming_distance, search::H256};

/// Consensus-critical seed. Changing this invalidates all existing
/// anti-knowledge entries. Must never change after genesis.
const ANTI_SUBSPACE_SEED: u64 = 0xAE71_5B8C_0000_0001;

/// A fixed random vector that defines the anti-knowledge subspace.
/// All agents derive the identical vector from the same seed.
pub static ANTI_SUBSPACE: LazyLock<HdcVector> =
    LazyLock::new(|| HdcVector::random(ANTI_SUBSPACE_SEED));

/// Resonance threshold for strong anti-knowledge detection.
/// Similarity > 0.90 = strong contradiction (reject entirely).
const ANTI_STRONG_THRESHOLD: f64 = 0.90;

/// Threshold for moderate anti-knowledge detection.
/// Similarity in (0.70, 0.90] = moderate contradiction (halve confidence).
const ANTI_MODERATE_THRESHOLD: f64 = 0.70;

/// Threshold for weak anti-knowledge signal.
/// Similarity in (0.50, 0.70] = weak signal (warning only).
const ANTI_WEAK_THRESHOLD: f64 = 0.50;

/// Encode anti-knowledge: bind the knowledge vector with ANTI_SUBSPACE.
///
/// Properties:
///   - `anti(X)` is quasi-orthogonal to X (similarity ~ 0.5)
///   - `anti(anti(X)) = X` (self-inverse, because bind is XOR)
///   - `anti(X)` will NOT be accidentally retrieved by a search for X
pub fn encode_anti(knowledge_vector: &HdcVector) -> HdcVector {
    bind(knowledge_vector, &ANTI_SUBSPACE)
}

/// Check whether `candidate` is anti-knowledge for `knowledge_vector`.
///
/// Unbinds the candidate from ANTI_SUBSPACE, then checks similarity
/// to the target knowledge vector. Returns true only for strong matches
/// (similarity > 0.90).
pub fn is_anti(candidate: &HdcVector, knowledge_vector: &HdcVector) -> bool {
    let unbound = bind(candidate, &ANTI_SUBSPACE);
    let dist = hamming_distance(&unbound, knowledge_vector);
    let sim = 1.0 - (dist as f64 / D as f64);
    sim > ANTI_STRONG_THRESHOLD
}

/// Result of anti-knowledge checking for a search candidate.
///
/// Produced by the anti-knowledge pipeline in `KnowledgeStore::search_with_anti_check`.
/// Contains the original search result metadata plus anti-knowledge annotations.
#[derive(Clone, Debug)]
pub struct AntiCheckedResult {
    /// The entry's key.
    pub key: H256,
    /// Normalized similarity to the query: `1.0 - (hamming / D)`.
    pub similarity: f64,
    /// True if moderate anti-knowledge resonance (0.7-0.9) was detected.
    pub contradicted: bool,
    /// Multiplicative confidence modifier.
    /// 1.0 = no anti-knowledge. 0.5 = moderate contradiction.
    pub confidence_modifier: f64,
    /// Human-readable warnings (e.g., weak signal notifications).
    pub warnings: Vec<String>,
}

/// Classify the anti-knowledge similarity into a severity level.
///
/// Returns `(contradicted, confidence_modifier, warning)` based on the
/// anti-similarity value:
///   - > 0.90: strong contradiction (caller should reject entirely)
///   - 0.70-0.90: moderate contradiction (halve confidence)
///   - 0.50-0.70: weak signal (warning only)
///   - <= 0.50: chance level, no concern
pub fn classify_anti_signal(anti_sim: f64) -> AntiClassification {
    if anti_sim > ANTI_STRONG_THRESHOLD {
        AntiClassification::Strong
    } else if anti_sim > ANTI_MODERATE_THRESHOLD {
        AntiClassification::Moderate
    } else if anti_sim > ANTI_WEAK_THRESHOLD {
        AntiClassification::Weak
    } else {
        AntiClassification::None
    }
}

/// Classification of anti-knowledge signal strength.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AntiClassification {
    /// Similarity > 0.90: reject entirely.
    Strong,
    /// Similarity 0.70-0.90: halve confidence.
    Moderate,
    /// Similarity 0.50-0.70: warning only.
    Weak,
    /// Similarity <= 0.50: no concern (chance level).
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::similarity;

    #[test]
    fn test_anti_subspace_deterministic() {
        let a = &*ANTI_SUBSPACE;
        let b = &*ANTI_SUBSPACE;
        assert_eq!(a, b);
    }

    #[test]
    fn test_encode_anti_self_inverse() {
        let v = HdcVector::random(42);
        let anti = encode_anti(&v);
        let recovered = encode_anti(&anti);
        assert_eq!(recovered, v);
    }

    #[test]
    fn test_anti_orthogonal() {
        let v = HdcVector::random(100);
        let anti = encode_anti(&v);
        let sim = similarity(&v, &anti);
        // Anti-knowledge should be quasi-orthogonal (~0.5 similarity)
        assert!((sim - 0.5).abs() < 0.02, "expected similarity ~0.5, got {sim}");
    }

    #[test]
    fn test_is_anti_true_positive() {
        let v = HdcVector::random(200);
        let anti = encode_anti(&v);
        assert!(is_anti(&anti, &v), "should detect anti-knowledge");
    }

    #[test]
    fn test_is_anti_true_negative() {
        let v = HdcVector::random(300);
        let random = HdcVector::random(301);
        assert!(!is_anti(&random, &v), "random vector should not be anti-knowledge");
    }

    #[test]
    fn test_classify_strong() {
        assert_eq!(classify_anti_signal(0.95), AntiClassification::Strong);
    }

    #[test]
    fn test_classify_moderate() {
        assert_eq!(classify_anti_signal(0.80), AntiClassification::Moderate);
    }

    #[test]
    fn test_classify_weak() {
        assert_eq!(classify_anti_signal(0.60), AntiClassification::Weak);
    }

    #[test]
    fn test_classify_none() {
        assert_eq!(classify_anti_signal(0.40), AntiClassification::None);
    }
}
