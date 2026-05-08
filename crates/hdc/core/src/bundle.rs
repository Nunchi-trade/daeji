//! BundleAccumulator — streaming majority-vote bundling for HDC vectors.

use crate::{constants::D, vector::HdcVector};

// ─── BundleAccumulator ────────────────────────────────────────────────────────

/// Streaming majority-vote accumulator for bundling HDC vectors.
#[derive(Debug)]
pub struct BundleAccumulator {
    counts: Vec<i32>, // one counter per bit position
}

impl BundleAccumulator {
    /// Create a new zero-initialized accumulator.
    pub fn new() -> Self {
        Self { counts: vec![0i32; D] }
    }

    /// Add a vector: +1 per set bit, -1 per cleared bit.
    pub fn add(&mut self, vector: &HdcVector) {
        for i in 0..D {
            if vector.bit(i) == 1 {
                self.counts[i] += 1;
            } else {
                self.counts[i] -= 1;
            }
        }
    }

    /// Add a vector with a positive integer weight.
    pub fn add_weighted(&mut self, vector: &HdcVector, weight: u32) {
        let w: i32 = weight.try_into().expect("weight exceeds i32::MAX");
        for i in 0..D {
            if vector.bit(i) == 1 {
                self.counts[i] += w;
            } else {
                self.counts[i] -= w;
            }
        }
    }

    /// Convert accumulated counts to a binary vector.
    /// Bit = 1 if count > 0, else 0 (ties break to 0).
    pub fn to_vector(&self) -> HdcVector {
        let mut out = HdcVector::default();
        for i in 0..D {
            if self.counts[i] > 0 {
                out.set_bit(i, 1);
            }
        }
        out
    }
}

impl Default for BundleAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Free function ────────────────────────────────────────────────────────────

/// Bundle a slice of vectors into one via majority vote.
pub fn bundle(vectors: &[&HdcVector]) -> HdcVector {
    let mut acc = BundleAccumulator::new();
    for v in vectors {
        acc.add(v);
    }
    acc.to_vector()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::THRESHOLD_HAMMING,
        vector::{HdcVector, hamming_distance},
    };

    #[test]
    fn bundle_single_vector_returns_same() {
        let a = HdcVector::random(1);
        let result = bundle(&[&a]);
        assert_eq!(result, a);
    }

    #[test]
    fn bundle_result_is_similar_to_all_inputs() {
        let a = HdcVector::random(10);
        let b = HdcVector::random(20);
        let c = HdcVector::random(30);
        let result = bundle(&[&a, &b, &c]);
        assert!(hamming_distance(&result, &a) < THRESHOLD_HAMMING);
        assert!(hamming_distance(&result, &b) < THRESHOLD_HAMMING);
        assert!(hamming_distance(&result, &c) < THRESHOLD_HAMMING);
    }

    #[test]
    fn bundle_tie_breaks_to_zero() {
        let a = HdcVector::random(42);
        let mut comp_words = [0u64; crate::constants::WORDS];
        for (i, w) in a.0.iter().enumerate() {
            comp_words[i] = !w;
        }
        let comp = HdcVector(comp_words);
        let result = bundle(&[&a, &comp]);
        assert_eq!(result, HdcVector::default());
    }

    #[test]
    fn bundle_accumulator_weighted() {
        let a = HdcVector::random(100);
        let b = HdcVector::random(200);
        let mut acc = BundleAccumulator::new();
        acc.add_weighted(&a, 10);
        acc.add(&b);
        let result = acc.to_vector();
        let dist_a = hamming_distance(&result, &a);
        let dist_b = hamming_distance(&result, &b);
        assert!(dist_a < dist_b, "dist_a={dist_a} should be < dist_b={dist_b}");
        assert!(dist_a < 1000, "dist_a={dist_a} should be < 1000");
    }
}
