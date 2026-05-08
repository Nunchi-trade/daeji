//! Encoder types for converting various data forms into hypervectors.

use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

use crate::bundle::BundleAccumulator;
use crate::constants::D;
use crate::vector::{bind, permute, HdcVector};

// ─── TrigramEncoder ───────────────────────────────────────────────────────────

/// Encodes text into a hypervector using character-level trigrams.
///
/// CONSENSUS-SAFE: no floats, deterministic.
pub struct TrigramEncoder;

impl TrigramEncoder {
    pub fn new() -> Self {
        TrigramEncoder
    }

    /// Encode a string into a hypervector via character trigrams.
    pub fn encode(&self, text: &str) -> HdcVector {
        let chars: Vec<char> = text.chars().collect();
        match chars.len() {
            0 => HdcVector::default(),
            1 => HdcVector::symbol(&chars[0].to_string()),
            2 => {
                let s0 = HdcVector::symbol(&chars[0].to_string());
                let s1 = HdcVector::symbol(&chars[1].to_string());
                bind(&permute(&s0, 1), &s1)
            }
            _ => {
                let mut acc = BundleAccumulator::new();
                for window in chars.windows(3) {
                    let s0 = HdcVector::symbol(&window[0].to_string());
                    let s1 = HdcVector::symbol(&window[1].to_string());
                    let s2 = HdcVector::symbol(&window[2].to_string());
                    // bind(permute(s0, 2), bind(permute(s1, 1), s2))
                    let inner = bind(&permute(&s1, 1), &s2);
                    let trigram = bind(&permute(&s0, 2), &inner);
                    acc.add(&trigram);
                }
                acc.to_vector()
            }
        }
    }
}

impl Default for TrigramEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── ProjectionEncoder ────────────────────────────────────────────────────────

/// Projects f32 embeddings into binary hypervectors.
///
/// **OFF-CHAIN ONLY** — uses f32 arithmetic, not consensus-safe.
pub struct ProjectionEncoder {
    input_dim: usize,
    /// D × input_dim matrix of i8 {-1, +1}, stored row-major.
    matrix: Vec<i8>,
}

impl ProjectionEncoder {
    /// Create a new projection encoder with a random D × input_dim matrix.
    pub fn new(input_dim: usize, seed: u64) -> Self {
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let mut matrix = Vec::with_capacity(D * input_dim);
        for _ in 0..(D * input_dim) {
            let val = if rng.next_u32() & 1 == 0 { 1i8 } else { -1i8 };
            matrix.push(val);
        }
        ProjectionEncoder { input_dim, matrix }
    }

    /// Project an f32 embedding into a binary hypervector.
    ///
    /// **OFF-CHAIN ONLY** — uses f32 arithmetic.
    pub fn encode(&self, embedding: &[f32]) -> HdcVector {
        assert_eq!(
            embedding.len(),
            self.input_dim,
            "embedding length {} != input_dim {}",
            embedding.len(),
            self.input_dim
        );
        let mut out = HdcVector::default();
        for row in 0..D {
            let row_start = row * self.input_dim;
            let mut dot = 0.0f32;
            for col in 0..self.input_dim {
                dot += self.matrix[row_start + col] as f32 * embedding[col];
            }
            if dot > 0.0 {
                out.set_bit(row, 1);
            }
        }
        out
    }
}

// ─── StructuredEncoder ────────────────────────────────────────────────────────

/// Encodes role-filler pairs (key-value) into a single hypervector.
///
/// CONSENSUS-SAFE: no floats, deterministic.
pub struct StructuredEncoder {
    fields: Vec<HdcVector>,
}

impl StructuredEncoder {
    pub fn new() -> Self {
        StructuredEncoder { fields: Vec::new() }
    }

    /// Add a string role-filler pair.
    pub fn add_field(&mut self, role: &str, filler: &str) {
        let role_vec = HdcVector::symbol(role);
        let filler_vec = HdcVector::symbol(filler);
        self.fields.push(bind(&role_vec, &filler_vec));
    }

    /// Add a role (string) bound to a pre-computed filler vector.
    pub fn add_field_vec(&mut self, role: &str, filler: &HdcVector) {
        let role_vec = HdcVector::symbol(role);
        self.fields.push(bind(&role_vec, filler));
    }

    /// Bundle all fields into a single hypervector.
    pub fn encode(&self) -> HdcVector {
        if self.fields.is_empty() {
            return HdcVector::default();
        }
        let mut acc = BundleAccumulator::new();
        for f in &self.fields {
            acc.add(f);
        }
        acc.to_vector()
    }
}

impl Default for StructuredEncoder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::THRESHOLD_HAMMING;
    use crate::vector::{bind, hamming_distance};

    #[test]
    fn trigram_encoder_deterministic() {
        let enc = TrigramEncoder::new();
        let a = enc.encode("hello world");
        let b = enc.encode("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn trigram_encoder_similar_strings_are_similar() {
        let enc = TrigramEncoder::new();
        let a = enc.encode("hello world");
        let b = enc.encode("hello world!");
        let dist = hamming_distance(&a, &b);
        assert!(dist < 4000, "similar strings have hamming {dist}, expected < 4000");
    }

    #[test]
    fn trigram_encoder_different_strings_are_dissimilar() {
        let enc = TrigramEncoder::new();
        let a = enc.encode("hello world");
        let b = enc.encode("quantum computing");
        let dist = hamming_distance(&a, &b);
        assert!(dist > 4500, "different strings have hamming {dist}, expected > 4500");
    }

    #[test]
    fn trigram_encoder_empty_returns_zero() {
        let enc = TrigramEncoder::new();
        let result = enc.encode("");
        assert_eq!(result, HdcVector::default());
    }

    #[test]
    fn structured_encoder_role_filler_retrieval() {
        let mut enc = StructuredEncoder::new();
        enc.add_field("name", "alice");
        enc.add_field("role", "engineer");
        let record = enc.encode();

        // Unbind with "name" to retrieve the filler
        let name_key = HdcVector::symbol("name");
        let retrieved = bind(&record, &name_key);

        let alice = HdcVector::symbol("alice");
        let bob = HdcVector::symbol("bob");

        let dist_alice = hamming_distance(&retrieved, &alice);
        let dist_bob = hamming_distance(&retrieved, &bob);

        assert!(
            dist_alice < dist_bob,
            "retrieved should be closer to alice ({dist_alice}) than bob ({dist_bob})"
        );
        assert!(
            dist_alice < THRESHOLD_HAMMING,
            "distance to alice {dist_alice} should be < THRESHOLD_HAMMING {THRESHOLD_HAMMING}"
        );
    }
}
