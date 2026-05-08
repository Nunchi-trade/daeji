//! HdcVector — 10,240-bit binary hypervector and core algebraic operations.

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use tiny_keccak::{Hasher, Keccak};

use crate::constants::{BYTES, D, WORDS};

// ─── Type ────────────────────────────────────────────────────────────────────

/// A 10,240-bit binary hypervector stored as 160 `u64` words.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[repr(C, align(64))]
pub struct HdcVector(pub [u64; WORDS]);

impl Default for HdcVector {
    fn default() -> Self {
        Self([0u64; WORDS])
    }
}

// ─── Bit access helpers ───────────────────────────────────────────────────────

impl HdcVector {
    /// Returns bit `i` as 0 or 1. Panics if `i >= D`.
    pub fn bit(&self, i: usize) -> u64 {
        assert!(i < D, "bit index {i} out of range (D={D})");
        let word = i / 64;
        let offset = i % 64;
        (self.0[word] >> offset) & 1
    }

    /// Sets bit `i` to `val` (0 or 1). Panics if `i >= D` or `val > 1`.
    pub fn set_bit(&mut self, i: usize, val: u64) {
        assert!(i < D, "bit index {i} out of range (D={D})");
        assert!(val <= 1, "val must be 0 or 1, got {val}");
        let word = i / 64;
        let offset = i % 64;
        self.0[word] = (self.0[word] & !(1u64 << offset)) | (val << offset);
    }

    /// Population count: total number of 1-bits in the vector.
    pub fn popcount(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }
}

// ─── Deterministic vector generation ─────────────────────────────────────────

fn fnv1a_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

impl HdcVector {
    /// Generate a deterministic pseudorandom vector from `seed`.
    /// Uses ChaCha20 — never OsRng or thread_rng.
    pub fn random(seed: u64) -> Self {
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let mut words = [0u64; WORDS];
        for w in words.iter_mut() {
            *w = rng.next_u64();
        }
        Self(words)
    }

    /// Generate a deterministic vector for a symbol name via FNV-1a → random.
    pub fn symbol(name: &str) -> Self {
        let seed = fnv1a_hash(name.as_bytes());
        Self::random(seed)
    }
}

// ─── Core operations (free functions) ────────────────────────────────────────

/// Association: word-by-word XOR. Self-inverse: bind(bind(a,b), b) == a.
pub fn bind(a: &HdcVector, b: &HdcVector) -> HdcVector {
    let mut out = [0u64; WORDS];
    for ((o, aw), bw) in out.iter_mut().zip(a.0.iter()).zip(b.0.iter()) {
        *o = aw ^ bw;
    }
    HdcVector(out)
}

/// Integer Hamming distance (XOR + popcount). Consensus-safe.
///
/// Delegates to the SIMD-dispatched implementation in [`crate::search::simd`]
/// for optimal performance on all platforms.
pub fn hamming_distance(a: &HdcVector, b: &HdcVector) -> u32 {
    crate::search::simd::hamming_distance(a, b)
}

/// Normalized similarity in [0.0, 1.0]. **OFF-CHAIN ONLY** — uses f64.
pub fn similarity(a: &HdcVector, b: &HdcVector) -> f64 {
    let hamming = hamming_distance(a, b) as f64;
    1.0 - hamming / D as f64
}

/// Cyclic LEFT rotation of the full 10,240-bit vector by `n` positions.
pub fn permute(v: &HdcVector, n: usize) -> HdcVector {
    let n = n % D;
    if n == 0 {
        return v.clone();
    }
    let word_shift = n / 64;
    let bit_shift = n % 64;

    let mut out = [0u64; WORDS];
    if bit_shift == 0 {
        for (i, o) in out.iter_mut().enumerate() {
            *o = v.0[(i + word_shift) % WORDS];
        }
    } else {
        for (i, o) in out.iter_mut().enumerate() {
            let src_hi = (i + word_shift) % WORDS;
            let src_lo = (i + word_shift + 1) % WORDS;
            *o = (v.0[src_hi] >> bit_shift) | (v.0[src_lo] << (64 - bit_shift));
        }
    }
    HdcVector(out)
}

// ─── Serialization (free functions) ──────────────────────────────────────────

/// Serialize to 1,280 bytes in little-endian word order.
pub fn serialize(vector: &HdcVector) -> [u8; BYTES] {
    let mut bytes = [0u8; BYTES];
    for (i, &word) in vector.0.iter().enumerate() {
        let chunk = word.to_le_bytes();
        bytes[i * 8..(i + 1) * 8].copy_from_slice(&chunk);
    }
    bytes
}

/// Deserialize from 1,280 little-endian bytes.
pub fn deserialize(bytes: &[u8; BYTES]) -> HdcVector {
    let mut words = [0u64; WORDS];
    for i in 0..WORDS {
        let chunk: [u8; 8] = bytes[i * 8..(i + 1) * 8].try_into().unwrap();
        words[i] = u64::from_le_bytes(chunk);
    }
    HdcVector(words)
}

/// Keccak-256 content address of the serialized vector.
pub fn vector_id(vector: &HdcVector) -> [u8; 32] {
    let bytes = serialize(vector);
    let mut hasher = Keccak::v256();
    hasher.update(&bytes);
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    output
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_is_deterministic() {
        assert_eq!(HdcVector::random(42), HdcVector::random(42));
    }

    #[test]
    fn random_different_seeds_differ() {
        assert_ne!(HdcVector::random(1), HdcVector::random(2));
    }

    #[test]
    fn symbol_is_deterministic() {
        assert_eq!(HdcVector::symbol("hello"), HdcVector::symbol("hello"));
    }

    #[test]
    fn random_vector_has_roughly_half_bits_set() {
        let v = HdcVector::random(99);
        let pc = v.popcount();
        assert!((4800..5440).contains(&pc), "popcount {pc} out of expected range 4800..5440");
    }

    #[test]
    fn random_vectors_are_quasi_orthogonal() {
        let a = HdcVector::random(1);
        let b = HdcVector::random(2);
        let d = hamming_distance(&a, &b);
        assert!((4900..5350).contains(&d), "hamming distance {d} out of expected range 4900..5350");
    }

    #[test]
    fn bind_self_inverse() {
        let a = HdcVector::random(10);
        let b = HdcVector::random(11);
        assert_eq!(bind(&bind(&a, &b), &b), a);
    }

    #[test]
    fn bind_with_zero_is_identity() {
        let a = HdcVector::random(12);
        let zero = HdcVector::default();
        assert_eq!(bind(&a, &zero), a);
    }

    #[test]
    fn bind_is_commutative() {
        let a = HdcVector::random(13);
        let b = HdcVector::random(14);
        assert_eq!(bind(&a, &b), bind(&b, &a));
    }

    #[test]
    fn permute_zero_is_identity() {
        let v = HdcVector::random(20);
        assert_eq!(permute(&v, 0), v);
    }

    #[test]
    fn permute_full_rotation_is_identity() {
        let v = HdcVector::random(21);
        assert_eq!(permute(&v, D), v);
    }

    #[test]
    fn permute_inverse() {
        let v = HdcVector::random(22);
        assert_eq!(permute(&permute(&v, 37), D - 37), v);
    }

    #[test]
    fn permute_composition() {
        let v = HdcVector::random(23);
        assert_eq!(permute(&permute(&v, 10), 20), permute(&v, 30));
    }

    #[test]
    fn serialize_deserialize_roundtrip() {
        let v = HdcVector::random(30);
        assert_eq!(deserialize(&serialize(&v)), v);
    }

    #[test]
    fn vector_id_is_deterministic() {
        let v = HdcVector::random(40);
        assert_eq!(vector_id(&v), vector_id(&v));
    }

    #[test]
    fn vector_id_differs_for_different_vectors() {
        let a = HdcVector::random(50);
        let b = HdcVector::random(51);
        assert_ne!(vector_id(&a), vector_id(&b));
    }

    #[test]
    fn symbol_different_names_differ() {
        assert_ne!(HdcVector::symbol("foo"), HdcVector::symbol("bar"));
    }

    #[test]
    fn bind_result_is_dissimilar_to_inputs() {
        let a = HdcVector::random(60);
        let b = HdcVector::random(61);
        let bound = bind(&a, &b);
        // bind produces quasi-orthogonal result to both inputs
        let dist_a = hamming_distance(&bound, &a);
        let dist_b = hamming_distance(&bound, &b);
        assert!(dist_a > 4500, "bound should be dissimilar to a, got {dist_a}");
        assert!(dist_b > 4500, "bound should be dissimilar to b, got {dist_b}");
    }

    #[test]
    fn serialize_length() {
        let v = HdcVector::random(70);
        let bytes = serialize(&v);
        assert_eq!(bytes.len(), crate::constants::BYTES);
    }

    #[test]
    fn permute_result_is_quasi_orthogonal() {
        let v = HdcVector::random(80);
        let p = permute(&v, 37);
        let dist = hamming_distance(&v, &p);
        // Permuted vector should be quasi-orthogonal to original
        assert!(dist > 4500, "permuted should be quasi-orthogonal, got {dist}");
    }
}
