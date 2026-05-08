# 02 — `kora-hdc` Core Algebra Crate

> **STATUS: FULLY IMPLEMENTED** (all core algebra complete and tested)
>
> | Component | Status | Source File |
> |-----------|--------|------------|
> | Constants (D, WORDS, BYTES, thresholds) | DONE | `crates/hdc/core/src/constants.rs` |
> | HdcVector type + bit access + popcount | DONE | `crates/hdc/core/src/vector.rs` |
> | Deterministic generation (random, symbol) | DONE | `crates/hdc/core/src/vector.rs` |
> | bind (XOR) | DONE | `crates/hdc/core/src/vector.rs` |
> | hamming_distance (delegates to SIMD) | DONE | `crates/hdc/core/src/vector.rs` -> `search/simd.rs` |
> | similarity (f64, off-chain) | DONE | `crates/hdc/core/src/vector.rs` |
> | permute (cyclic left rotation) | DONE | `crates/hdc/core/src/vector.rs` |
> | serialize / deserialize / vector_id | DONE | `crates/hdc/core/src/vector.rs` |
> | BundleAccumulator + bundle() | DONE | `crates/hdc/core/src/bundle.rs` |
> | TrigramEncoder | DONE | `crates/hdc/core/src/encode.rs` |
> | ProjectionEncoder | DONE | `crates/hdc/core/src/encode.rs` |
> | StructuredEncoder | DONE | `crates/hdc/core/src/encode.rs` |
> | Search module (SIMD, BruteForce, HNSW, Local, Tiered) | DONE | `crates/hdc/core/src/search/` |
> | Knowledge module (store, decay, anti, scoring, trust) | DONE | `crates/hdc/core/src/knowledge/` |
> | Trust module | DONE | `crates/hdc/core/src/trust.rs` |
> | Context assembly | DONE | `crates/hdc/core/src/context.rs` |
> | Cognitive modules (affect, dream, replay, somatic, state_machine) | DONE | `crates/hdc/core/src/cognitive/` |

---

## Verification Commands

```bash
# Build the crate in isolation:
cargo check -p kora-hdc

# Run all tests (vector, bundle, encode, search, knowledge):
cargo test -p kora-hdc

# Clippy:
cargo clippy -p kora-hdc

# Specific test names in vector.rs:
#   random_is_deterministic, random_different_seeds_differ, symbol_is_deterministic,
#   random_vector_has_roughly_half_bits_set, random_vectors_are_quasi_orthogonal,
#   bind_self_inverse, bind_with_zero_is_identity, bind_is_commutative,
#   permute_zero_is_identity, permute_full_rotation_is_identity, permute_inverse,
#   permute_composition, serialize_deserialize_roundtrip, vector_id_is_deterministic,
#   vector_id_differs_for_different_vectors, symbol_different_names_differ,
#   bind_result_is_dissimilar_to_inputs, serialize_length, permute_result_is_quasi_orthogonal
```

---

## Deviations From Spec

1. **`hamming_distance` in `vector.rs` delegates to `search::simd::hamming_distance`** rather than implementing inline. This is better -- avoids code duplication and ensures the SIMD-dispatched path is always used.

2. **`HdcVector` uses `rng.next_u64()` instead of `rng.gen()`** for random generation. Both produce identical results (same ChaCha20 output); `next_u64()` is the `RngCore` method while `gen()` is the `Rng` extension method.

3. **`set_bit()` uses a branchless bit manipulation pattern** (`(word & !(1 << offset)) | (val << offset)`) instead of the if-else pattern in the spec. Functionally identical.

4. **The crate additionally exports `knowledge`, `trust`, `context`, and `cognitive` modules** beyond what this spec covers. Those are from docs 04-07.

5. **The spec says `search.rs` should be a re-export placeholder** (section 7). In reality, `search/` is a full module directory with SIMD, brute-force, HNSW, local, and tiered implementations (covered by doc 03).

---

> **Purpose:** This document contains everything an implementing agent needs to
> build the `kora-hdc` crate from scratch. No prior context about the project is
> assumed. Read top to bottom, implement in order, check every box at the end.

---

## 1. What This Crate Is

`kora-hdc` is a pure Rust library implementing Binary Spatter Code (BSC)
Hyperdimensional Computing. It provides:

- A 10,240-bit binary vector type (`HdcVector`)
- Three algebraic operations: **bind** (XOR), **bundle** (majority vote),
  **permute** (cyclic rotation)
- Hamming distance and similarity measurement
- Deterministic vector generation (seeded PRNG)
- Text and structured-data encoders
- Serialization and content-addressing (keccak256)

The crate has **zero blockchain dependencies**. It is a standalone algebra
library consumed by higher-level crates (`kora-hdc-chain`, knowledge store,
context assembly). It must compile and pass tests in isolation.

---

## 2. Crate Location and Workspace Registration

### 2.1 Directory

```
crates/hdc/core/
  Cargo.toml
  src/
    lib.rs
    constants.rs
    vector.rs
    bundle.rs
    encode.rs
    search.rs        # Re-exports only. Search impl is doc 03.
```

### 2.2 Workspace `Cargo.toml` Changes

In the root `/Cargo.toml`, add `"crates/hdc/*"` to the workspace members glob
and register the crate in `[workspace.dependencies]`:

```toml
[workspace]
members = [
    "bin/*",
    "crates/e2e",
    "crates/hdc/*",          # <-- ADD THIS LINE
    "crates/network/*",
    "crates/node/*",
    "crates/storage/*",
    "crates/utilities/*",
]

[workspace.dependencies]
# ... existing entries ...

# HDC
kora-hdc = { path = "crates/hdc/core" }

# Add these if not already present:
rand_chacha = "0.3"
bytemuck = { version = "1", features = ["derive"] }
tiny-keccak = { version = "2", features = ["keccak"] }
```

> **Note:** `rand = "0.8"` and `sha3 = "0.10"` are already in workspace deps.
> `rand_chacha` and `bytemuck` are new. For keccak256, use `tiny-keccak` (lighter
> than pulling in `sha3` for a single hash). If you prefer `sha3` (already in the
> workspace), that works too -- just use `sha3::Keccak256` instead of
> `tiny_keccak::Keccak`.

### 2.3 Crate `Cargo.toml`

Create `crates/hdc/core/Cargo.toml`:

```toml
[package]
name = "kora-hdc"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Hyperdimensional Computing core algebra for Kora"

[lints]
workspace = true

[dependencies]
# PRNG (deterministic vector generation)
rand = { workspace = true }
rand_chacha = { workspace = true }

# Zero-copy casting for serialization
bytemuck = { workspace = true }

# Keccak256 for vector content-addressing
tiny-keccak = { workspace = true }

[dev-dependencies]
# Property testing
rand.workspace = true
```

---

## 3. Constants (`src/constants.rs`)

```rust
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
/// `(1.0 - 0.526) * 10_240 = 4,853.76`, rounded up to 4,854.
///
/// **ON-CHAIN ONLY.** All on-chain threshold checks compare
/// `hamming_distance < THRESHOLD_HAMMING` (integer comparison).
/// Never use the float 0.526 on-chain.
pub const THRESHOLD_HAMMING: u32 = 4_854;

/// Hamming distance below which two vectors are considered near-duplicates.
/// Corresponds to similarity > 0.95.
pub const DUPLICATE_THRESHOLD: u32 = 512;

/// Hamming distance below which two vectors are considered to "resonate"
/// (strong meaningful similarity). Corresponds to similarity > 0.90.
pub const RESONANCE_THRESHOLD_HAMMING: u32 = 1_024;
```

All three thresholds are integer Hamming distances. On-chain code compares
`hamming_distance(a, b) < THRESHOLD` -- never converts to float.

---

## 4. The `HdcVector` Type (`src/vector.rs`)

### 4.1 Type Definition

```rust
use crate::constants::{BYTES, D, WORDS};

/// A 10,240-bit binary hypervector.
///
/// Stored as 160 little-endian `u64` words. Aligned to 64 bytes for
/// cache-line alignment and SIMD compatibility.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[repr(C, align(64))]
pub struct HdcVector(pub [u64; WORDS]);

// SAFETY: HdcVector is a plain array of u64. No interior mutability,
// no pointers, no non-Send/Sync fields.
unsafe impl Send for HdcVector {}
unsafe impl Sync for HdcVector {}
```

`#[repr(C, align(64))]` is required. The 64-byte alignment matches a cache
line on x86-64 and ARM, and enables future AVX-512 SIMD paths. `repr(C)`
ensures a predictable memory layout for zero-copy serialization.

### 4.2 `Default` -- Zero Vector

```rust
impl Default for HdcVector {
    fn default() -> Self {
        Self([0u64; WORDS])
    }
}
```

The zero vector is the identity element for XOR bind: `bind(a, zero) = a`.

### 4.3 Bit Access Helpers

```rust
impl HdcVector {
    /// Returns the value of bit `i` (0 or 1). Panics if `i >= D`.
    #[inline]
    pub fn bit(&self, i: usize) -> u64 {
        assert!(i < D, "bit index {i} out of range [0, {D})");
        let word = i / 64;
        let bit = i % 64;
        (self.0[word] >> bit) & 1
    }

    /// Sets bit `i` to `val` (0 or 1). Panics if `i >= D` or `val > 1`.
    #[inline]
    pub fn set_bit(&mut self, i: usize, val: u64) {
        assert!(i < D, "bit index {i} out of range [0, {D})");
        assert!(val <= 1, "bit value must be 0 or 1, got {val}");
        let word = i / 64;
        let bit = i % 64;
        if val == 1 {
            self.0[word] |= 1u64 << bit;
        } else {
            self.0[word] &= !(1u64 << bit);
        }
    }

    /// Returns the number of bits set to 1 (population count).
    pub fn popcount(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }
}
```

### 4.4 Deterministic Vector Generation

```rust
use rand::Rng;
use rand_chacha::ChaCha20Rng;
use rand::SeedableRng;

impl HdcVector {
    /// Generate a deterministic random vector from a u64 seed.
    ///
    /// Uses ChaCha20Rng for cryptographic-quality randomness.
    /// The same seed ALWAYS produces the same vector on every platform.
    ///
    /// CONSENSUS-SAFE: Deterministic. Integer-only PRNG. No floats.
    pub fn random(seed: u64) -> Self {
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let mut v = [0u64; WORDS];
        for w in &mut v {
            *w = rng.gen();
        }
        Self(v)
    }

    /// Generate a deterministic vector for a named symbol.
    ///
    /// Computes FNV-1a hash of `name` to get a u64 seed, then calls
    /// `random(seed)`. The same name ALWAYS produces the same vector.
    ///
    /// CONSENSUS-SAFE: Deterministic. Same name -> same seed -> same vector.
    pub fn symbol(name: &str) -> Self {
        let seed = fnv1a_hash(name.as_bytes());
        Self::random(seed)
    }
}

/// FNV-1a hash producing a u64. Deterministic across all platforms.
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
```

**Critical invariant:** `HdcVector::random(42)` must return the exact same
160 words on every platform, every OS, every Rust version. ChaCha20Rng
guarantees this. **Never** use `thread_rng()`, `OsRng`, or any
non-deterministic source.

### 4.5 Core Operations

All core operations are free functions, not methods. This is deliberate --
they mirror the mathematical notation `bind(a, b)` rather than `a.bind(b)`.

#### Bind (XOR)

```rust
/// Bind two vectors via XOR. Produces a vector dissimilar to both inputs.
///
/// Self-inverse: `bind(bind(a, b), b) == a`.
///
/// CONSENSUS-SAFE: Integer-only. Bitwise XOR.
pub fn bind(a: &HdcVector, b: &HdcVector) -> HdcVector {
    let mut out = [0u64; WORDS];
    for i in 0..WORDS {
        out[i] = a.0[i] ^ b.0[i];
    }
    HdcVector(out)
}
```

#### Hamming Distance

```rust
/// Count the number of differing bits between two vectors.
///
/// CONSENSUS-SAFE: Integer-only. XOR + popcount.
pub fn hamming_distance(a: &HdcVector, b: &HdcVector) -> u32 {
    let mut dist: u32 = 0;
    for i in 0..WORDS {
        dist += (a.0[i] ^ b.0[i]).count_ones();
    }
    dist
}
```

#### Similarity

```rust
/// Normalized similarity: `1.0 - hamming_distance(a, b) / D`.
///
/// Returns a value in [0.0, 1.0]:
///   1.0 = identical, 0.5 = quasi-orthogonal (random), 0.0 = complementary.
///
/// OFF-CHAIN ONLY. Uses f64 division. Never use this for on-chain consensus
/// decisions. Use `hamming_distance()` with integer thresholds instead.
pub fn similarity(a: &HdcVector, b: &HdcVector) -> f64 {
    1.0 - hamming_distance(a, b) as f64 / D as f64
}
```

#### Permute (Cyclic Left Rotation)

```rust
/// Cyclic left rotation of the entire 10,240-bit vector by `n` bit positions.
///
/// Wraps around: bits shifted out of the MSB of word 159 re-enter at the
/// LSB of word 0. This is a rotation across the FULL bit vector, not per-word.
///
/// CONSENSUS-SAFE: Integer-only. Bit shifts and masks.
pub fn permute(v: &HdcVector, n: usize) -> HdcVector {
    let n = n % D; // Normalize rotation amount
    if n == 0 {
        return v.clone();
    }

    let word_shift = n / 64;    // Number of full words to shift
    let bit_shift = n % 64;     // Remaining bits within a word

    let mut out = [0u64; WORDS];

    if bit_shift == 0 {
        // Exact word-boundary rotation -- no bit splitting needed
        for i in 0..WORDS {
            out[i] = v.0[(i + word_shift) % WORDS];
        }
    } else {
        // Cross-word rotation: each output word is assembled from two
        // input words
        let complement = 64 - bit_shift;
        for i in 0..WORDS {
            let src_hi = (i + word_shift) % WORDS;
            let src_lo = (i + word_shift + 1) % WORDS;
            out[i] = (v.0[src_hi] >> bit_shift) | (v.0[src_lo] << complement);
        }
    }

    HdcVector(out)
}
```

> **IMPORTANT:** This rotates left across the full 10,240-bit vector, not
> per-word. Word 0 bit 0 is the least-significant bit of the entire vector.
> After rotating left by 1, what was bit 1 is now bit 0, and what was bit 0
> (of word 0) wraps to bit 10,239 (MSB of word 159).

**Wait -- verify the direction.** The specification says "cyclic left
rotation by n bits." In the implementation above, "left" means toward lower
bit indices (toward bit 0), which is the standard convention in the BSC
literature. The key property to verify in tests: `permute(v, 1)` produces a
vector quasi-orthogonal to `v`, and `permute(permute(v, n), D - n)` recovers
`v` (inverse rotation).

### 4.6 Serialization

```rust
/// Serialize a vector to 1,280 bytes (little-endian u64 words).
///
/// CONSENSUS-SAFE: Deterministic byte layout.
pub fn serialize(vector: &HdcVector) -> [u8; BYTES] {
    let mut buf = [0u8; BYTES];
    for (i, word) in vector.0.iter().enumerate() {
        let start = i * 8;
        buf[start..start + 8].copy_from_slice(&word.to_le_bytes());
    }
    buf
}

/// Deserialize 1,280 bytes (little-endian u64 words) into a vector.
///
/// CONSENSUS-SAFE: Deterministic. Inverse of `serialize`.
pub fn deserialize(bytes: &[u8; BYTES]) -> HdcVector {
    let mut words = [0u64; WORDS];
    for (i, word) in words.iter_mut().enumerate() {
        let start = i * 8;
        *word = u64::from_le_bytes(
            bytes[start..start + 8].try_into().expect("slice is 8 bytes")
        );
    }
    HdcVector(words)
}

/// Compute the keccak256 hash of a serialized vector.
///
/// This is the vector's content address -- a 32-byte identifier that
/// uniquely identifies the vector's content. Two vectors with the same
/// bits produce the same ID. Used for deduplication and on-chain references.
///
/// CONSENSUS-SAFE: Deterministic hash of deterministic serialization.
pub fn vector_id(vector: &HdcVector) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};

    let serialized = serialize(vector);
    let mut output = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(&serialized);
    hasher.finalize(&mut output);
    output
}
```

---

## 5. `BundleAccumulator` (`src/bundle.rs`)

```rust
use crate::constants::{D, WORDS};
use crate::vector::HdcVector;

/// Streaming majority-vote accumulator for bundling hypervectors.
///
/// Maintains an `i32` counter per bit position. Each `add()` call
/// increments (bit=1) or decrements (bit=0) each counter. The final
/// vector is produced by thresholding at 0.
///
/// CONSENSUS-SAFE: All arithmetic is integer. Tie-breaking is deterministic
/// (ties -> 0). No floats.
pub struct BundleAccumulator {
    counts: Vec<i32>,
}

impl BundleAccumulator {
    /// Create a new accumulator with all counts at zero.
    pub fn new() -> Self {
        Self {
            counts: vec![0i32; D],
        }
    }

    /// Add a vector to the accumulator.
    ///
    /// For each bit position: if the bit is 1, increment the count;
    /// if the bit is 0, decrement the count.
    pub fn add(&mut self, vector: &HdcVector) {
        for i in 0..D {
            if vector.bit(i) == 1 {
                self.counts[i] += 1;
            } else {
                self.counts[i] -= 1;
            }
        }
    }

    /// Add a vector with integer weight.
    ///
    /// Equivalent to calling `add()` `weight` times, but O(D) instead of
    /// O(weight * D).
    pub fn add_weighted(&mut self, vector: &HdcVector, weight: u32) {
        let w = weight as i32;
        for i in 0..D {
            if vector.bit(i) == 1 {
                self.counts[i] += w;
            } else {
                self.counts[i] -= w;
            }
        }
    }

    /// Produce the bundled vector via majority vote.
    ///
    /// Each bit is 1 if its count > 0, else 0.
    /// **Ties (count == 0) break to 0.** This is deterministic and
    /// consensus-safe.
    pub fn to_vector(&self) -> HdcVector {
        let mut result = HdcVector::default(); // all zeros
        for i in 0..D {
            if self.counts[i] > 0 {
                result.set_bit(i, 1);
            }
            // count == 0 -> bit stays 0 (deterministic tie-break)
            // count < 0  -> bit stays 0
        }
        result
    }
}

impl Default for BundleAccumulator {
    fn default() -> Self {
        Self::new()
    }
}
```

### Convenience: `bundle` free function

```rust
/// Bundle a slice of vectors via majority vote.
///
/// Convenience wrapper around `BundleAccumulator`. For streaming use cases
/// or weighted bundling, use `BundleAccumulator` directly.
///
/// CONSENSUS-SAFE: Delegates to `BundleAccumulator`.
pub fn bundle(vectors: &[&HdcVector]) -> HdcVector {
    let mut acc = BundleAccumulator::new();
    for v in vectors {
        acc.add(v);
    }
    acc.to_vector()
}
```

### Performance Note

The bit-by-bit loop in `add()` is O(D) with per-bit overhead. A faster
implementation processes word-by-word (iterating over 160 u64 words and
extracting bits in bulk). The naive per-bit version is correct and should be
the initial implementation. Optimize later if profiling shows this is a
bottleneck. A word-level implementation would look like:

```rust
// Optimized add -- process 64 bits at a time
pub fn add_fast(&mut self, vector: &HdcVector) {
    for word_idx in 0..WORDS {
        let w = vector.0[word_idx];
        let base = word_idx * 64;
        for bit in 0..64 {
            if (w >> bit) & 1 == 1 {
                self.counts[base + bit] += 1;
            } else {
                self.counts[base + bit] -= 1;
            }
        }
    }
}
```

This is still O(D) but avoids the division and modulo in `bit()` for each
access. Either approach is acceptable for the initial implementation.

---

## 6. Encoders (`src/encode.rs`)

### 6.1 `TrigramEncoder`

```rust
use crate::vector::HdcVector;
use crate::bundle::BundleAccumulator;
use crate::vector::{bind, permute};

/// Encodes text into a hypervector using character-level trigrams.
///
/// Algorithm:
/// 1. Slide a 3-character window across the input.
/// 2. For each trigram (c0, c1, c2):
///    - Look up each character's symbol vector: `HdcVector::symbol(&c.to_string())`
///    - Permute by position:  `permute(sym_c0, 2)`, `permute(sym_c1, 1)`, `sym_c2`
///    - Bind the three:  `bind(permute(sym_c0, 2), bind(permute(sym_c1, 1), sym_c2))`
/// 3. Bundle all trigram vectors via majority vote.
///
/// For inputs shorter than 3 characters:
///   - 0 chars: returns the zero vector.
///   - 1 char:  returns `HdcVector::symbol(&c.to_string())`.
///   - 2 chars: returns `bind(permute(sym_c0, 1), sym_c1)`.
///
/// CONSENSUS-SAFE: All operations are integer-only (bind, permute, bundle).
pub struct TrigramEncoder;

impl TrigramEncoder {
    pub fn encode(text: &str) -> HdcVector {
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
                    let trigram = bind(
                        &permute(&s0, 2),
                        &bind(&permute(&s1, 1), &s2),
                    );
                    acc.add(&trigram);
                }
                acc.to_vector()
            }
        }
    }
}
```

### 6.2 `ProjectionEncoder`

```rust
/// Projects a dense f32 embedding into a binary hypervector via random
/// projection with a quantized (i8) projection matrix.
///
/// OFF-CHAIN ONLY. Uses floating-point arithmetic (f32 dot products).
/// The projection matrix is deterministic from seed.
///
/// Algorithm:
/// 1. Generate a D x input_dim matrix of i8 values from ChaCha20Rng.
/// 2. For each of the D output bits: compute dot(row_i, embedding).
/// 3. Output bit = 1 if dot > 0, else 0.
pub struct ProjectionEncoder {
    /// Projection matrix, row-major: `matrix[row * input_dim + col]`.
    matrix: Vec<i8>,
    input_dim: usize,
}

impl ProjectionEncoder {
    /// Create a new encoder for embeddings of `input_dim` dimensions.
    ///
    /// The projection matrix is deterministic from `seed`.
    ///
    /// OFF-CHAIN ONLY.
    pub fn new(input_dim: usize, seed: u64) -> Self {
        use rand::SeedableRng;
        use rand::Rng;
        use rand_chacha::ChaCha20Rng;

        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let total = D * input_dim;
        let mut matrix = Vec::with_capacity(total);
        for _ in 0..total {
            // Binary {-1, +1} stored as i8
            let val: i8 = if rng.gen::<bool>() { 1 } else { -1 };
            matrix.push(val);
        }
        Self { matrix, input_dim }
    }

    /// Project an f32 embedding into a binary hypervector.
    ///
    /// OFF-CHAIN ONLY.
    pub fn encode(&self, embedding: &[f32]) -> HdcVector {
        assert_eq!(
            embedding.len(),
            self.input_dim,
            "embedding dimension mismatch: expected {}, got {}",
            self.input_dim,
            embedding.len()
        );

        let mut result = HdcVector::default();
        for row in 0..D {
            let offset = row * self.input_dim;
            let mut dot: f32 = 0.0;
            for col in 0..self.input_dim {
                dot += self.matrix[offset + col] as f32 * embedding[col];
            }
            if dot > 0.0 {
                result.set_bit(row, 1);
            }
        }
        result
    }
}
```

### 6.3 `StructuredEncoder`

```rust
/// Encodes structured data (role-filler pairs) into a single hypervector.
///
/// Usage:
/// ```
/// let mut enc = StructuredEncoder::new();
/// enc.add_field("subject", "Alice");
/// enc.add_field("action", "transfer");
/// enc.add_field("amount", "100");
/// let vector = enc.encode();
/// ```
///
/// Each field becomes `bind(HdcVector::symbol(role), HdcVector::symbol(filler))`.
/// All fields are bundled via majority vote.
///
/// CONSENSUS-SAFE: All operations are integer-only.
pub struct StructuredEncoder {
    pairs: Vec<HdcVector>,
}

impl StructuredEncoder {
    pub fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    /// Add a role-filler pair. Both role and filler are converted to symbol
    /// vectors via `HdcVector::symbol()`.
    pub fn add_field(&mut self, role: &str, filler: &str) {
        let role_vec = HdcVector::symbol(role);
        let filler_vec = HdcVector::symbol(filler);
        self.pairs.push(bind(&role_vec, &filler_vec));
    }

    /// Add a role-filler pair where the filler is a pre-computed vector.
    pub fn add_field_vec(&mut self, role: &str, filler: &HdcVector) {
        let role_vec = HdcVector::symbol(role);
        self.pairs.push(bind(&role_vec, filler));
    }

    /// Bundle all role-filler pairs into a single vector.
    pub fn encode(&self) -> HdcVector {
        let refs: Vec<&HdcVector> = self.pairs.iter().collect();
        bundle(&refs)
    }
}

impl Default for StructuredEncoder {
    fn default() -> Self {
        Self::new()
    }
}
```

---

## 7. Search Re-exports (`src/search.rs`)

This file is a placeholder. The actual search implementation (brute-force,
HNSW, tiered pipeline) is specified in document `03-vector-search.md`.

```rust
//! Vector search algorithms.
//!
//! This module will contain brute-force and HNSW search implementations.
//! See implementation plan `03-vector-search.md`.

// Re-export core functions needed by search consumers.
pub use crate::vector::{hamming_distance, similarity};
pub use crate::constants::{THRESHOLD_HAMMING, DUPLICATE_THRESHOLD, RESONANCE_THRESHOLD_HAMMING};
```

---

## 8. Library Root (`src/lib.rs`)

```rust
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
```

---

## 9. Consensus Safety Annotations -- Summary

Every public function must be annotated. Here is the complete classification:

| Function / Type | Consensus | Why |
|---|---|---|
| `HdcVector::random(seed)` | SAFE | Deterministic ChaCha20, integer-only |
| `HdcVector::symbol(name)` | SAFE | FNV-1a + `random()` |
| `HdcVector::bit(i)` | SAFE | Pure bit access |
| `HdcVector::set_bit(i, v)` | SAFE | Pure bit access |
| `HdcVector::popcount()` | SAFE | Integer popcount |
| `bind(a, b)` | SAFE | XOR |
| `bundle(vecs)` | SAFE | Integer accumulation + threshold |
| `BundleAccumulator::add()` | SAFE | Integer increment/decrement |
| `BundleAccumulator::add_weighted()` | SAFE | Integer multiply + add |
| `BundleAccumulator::to_vector()` | SAFE | Integer threshold at 0 |
| `permute(v, n)` | SAFE | Bit shift + mask |
| `hamming_distance(a, b)` | SAFE | XOR + popcount |
| `serialize(v)` | SAFE | Little-endian copy |
| `deserialize(bytes)` | SAFE | Little-endian copy |
| `vector_id(v)` | SAFE | keccak256 of deterministic bytes |
| `similarity(a, b)` | **OFF-CHAIN** | Uses f64 division |
| `ProjectionEncoder::new()` | **OFF-CHAIN** | Generates f32-adjacent matrix |
| `ProjectionEncoder::encode()` | **OFF-CHAIN** | f32 dot products |
| `TrigramEncoder::encode()` | SAFE | Delegates to bind/permute/bundle |
| `StructuredEncoder::encode()` | SAFE | Delegates to bind/bundle |

---

## 10. Unit Tests

All tests go in `src/vector.rs`, `src/bundle.rs`, and `src/encode.rs` using
`#[cfg(test)] mod tests { ... }` blocks. You may also add integration tests
in a `tests/` directory.

### 10.1 Vector Generation Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_is_deterministic() {
        let v1 = HdcVector::random(42);
        let v2 = HdcVector::random(42);
        assert_eq!(v1, v2, "same seed must produce identical vectors");
    }

    #[test]
    fn random_different_seeds_differ() {
        let v1 = HdcVector::random(1);
        let v2 = HdcVector::random(2);
        assert_ne!(v1, v2);
    }

    #[test]
    fn symbol_is_deterministic() {
        let v1 = HdcVector::symbol("hello");
        let v2 = HdcVector::symbol("hello");
        assert_eq!(v1, v2);
    }

    #[test]
    fn symbol_different_names_differ() {
        let v1 = HdcVector::symbol("hello");
        let v2 = HdcVector::symbol("world");
        assert_ne!(v1, v2);
    }

    #[test]
    fn random_vector_has_roughly_half_bits_set() {
        let v = HdcVector::random(123);
        let ones = v.popcount();
        // Expected: 5120. Allow a generous range.
        assert!(ones > 4800 && ones < 5440,
            "expected ~5120 bits set, got {ones}");
    }

    #[test]
    fn random_vectors_are_quasi_orthogonal() {
        let a = HdcVector::random(100);
        let b = HdcVector::random(200);
        let dist = hamming_distance(&a, &b);
        // Expected: ~5120 +/- 200
        assert!(dist > 4900 && dist < 5350,
            "random vectors should be quasi-orthogonal, hamming = {dist}");
    }
}
```

### 10.2 Bind Tests

```rust
#[test]
fn bind_self_inverse() {
    let a = HdcVector::random(1);
    let b = HdcVector::random(2);
    let c = bind(&a, &b);
    let recovered = bind(&c, &b);
    assert_eq!(recovered, a, "bind must be self-inverse: bind(bind(a,b), b) = a");
}

#[test]
fn bind_with_zero_is_identity() {
    let a = HdcVector::random(1);
    let zero = HdcVector::default();
    assert_eq!(bind(&a, &zero), a);
}

#[test]
fn bind_is_commutative() {
    let a = HdcVector::random(1);
    let b = HdcVector::random(2);
    assert_eq!(bind(&a, &b), bind(&b, &a));
}

#[test]
fn bind_result_is_dissimilar_to_inputs() {
    let a = HdcVector::random(10);
    let b = HdcVector::random(20);
    let c = bind(&a, &b);
    let dist_a = hamming_distance(&c, &a);
    let dist_b = hamming_distance(&c, &b);
    // Result should be quasi-orthogonal to both inputs
    assert!(dist_a > 4900 && dist_a < 5350,
        "bind result should be quasi-orthogonal to a, hamming = {dist_a}");
    assert!(dist_b > 4900 && dist_b < 5350,
        "bind result should be quasi-orthogonal to b, hamming = {dist_b}");
}
```

### 10.3 Bundle Tests

```rust
#[test]
fn bundle_single_vector_returns_same() {
    let a = HdcVector::random(1);
    let result = bundle(&[&a]);
    assert_eq!(result, a);
}

#[test]
fn bundle_result_is_similar_to_all_inputs() {
    let a = HdcVector::random(1);
    let b = HdcVector::random(2);
    let c = HdcVector::random(3);
    let result = bundle(&[&a, &b, &c]);

    // Bundle of 3 vectors: expected similarity to each ~0.631
    // Hamming distance should be well below 5120 (the random baseline)
    for (label, v) in [("a", &a), ("b", &b), ("c", &c)] {
        let dist = hamming_distance(&result, v);
        assert!(dist < THRESHOLD_HAMMING,
            "bundle should be similar to constituent {label}, hamming = {dist}");
    }
}

#[test]
fn bundle_tie_breaks_to_zero() {
    // Two complementary vectors: every bit position is a tie
    let a = HdcVector::random(1);
    let mut b = HdcVector::default();
    for i in 0..WORDS {
        b.0[i] = !a.0[i]; // complement
    }
    let result = bundle(&[&a, &b]);
    // All ties -> all zeros
    assert_eq!(result, HdcVector::default(),
        "bundling a vector with its complement should produce all zeros (tie -> 0)");
}

#[test]
fn bundle_accumulator_weighted() {
    let a = HdcVector::random(1);
    let b = HdcVector::random(2);
    let mut acc = BundleAccumulator::new();
    acc.add_weighted(&a, 10);
    acc.add(&b);
    let result = acc.to_vector();

    // a has 10x the weight, so result should be very similar to a
    let dist_a = hamming_distance(&result, &a);
    let dist_b = hamming_distance(&result, &b);
    assert!(dist_a < dist_b,
        "weighted bundle should be closer to heavily-weighted vector: dist_a={dist_a}, dist_b={dist_b}");
    assert!(dist_a < 1000,
        "heavily weighted vector should dominate: dist_a={dist_a}");
}
```

### 10.4 Permute Tests

```rust
#[test]
fn permute_zero_is_identity() {
    let v = HdcVector::random(1);
    assert_eq!(permute(&v, 0), v);
}

#[test]
fn permute_full_rotation_is_identity() {
    let v = HdcVector::random(1);
    assert_eq!(permute(&v, D), v, "rotating by D should be identity");
}

#[test]
fn permute_inverse() {
    let v = HdcVector::random(1);
    let rotated = permute(&v, 37);
    let recovered = permute(&rotated, D - 37);
    assert_eq!(recovered, v, "permute(permute(v, n), D-n) should recover v");
}

#[test]
fn permute_result_is_quasi_orthogonal() {
    let v = HdcVector::random(1);
    let rotated = permute(&v, 1);
    let dist = hamming_distance(&v, &rotated);
    assert!(dist > 4900 && dist < 5350,
        "permute by 1 should produce quasi-orthogonal vector, hamming = {dist}");
}

#[test]
fn permute_composition() {
    let v = HdcVector::random(1);
    let a = permute(&permute(&v, 10), 20);
    let b = permute(&v, 30);
    assert_eq!(a, b, "rho^10(rho^20(v)) should equal rho^30(v)");
}
```

### 10.5 Serialization Tests

```rust
#[test]
fn serialize_deserialize_roundtrip() {
    let v = HdcVector::random(999);
    let bytes = serialize(&v);
    let recovered = deserialize(&bytes);
    assert_eq!(recovered, v, "serialize -> deserialize must be lossless");
}

#[test]
fn serialize_length() {
    let v = HdcVector::random(1);
    let bytes = serialize(&v);
    assert_eq!(bytes.len(), BYTES, "serialized vector must be {BYTES} bytes");
}

#[test]
fn vector_id_is_deterministic() {
    let v = HdcVector::random(42);
    let id1 = vector_id(&v);
    let id2 = vector_id(&v);
    assert_eq!(id1, id2);
}

#[test]
fn vector_id_differs_for_different_vectors() {
    let a = HdcVector::random(1);
    let b = HdcVector::random(2);
    assert_ne!(vector_id(&a), vector_id(&b));
}
```

### 10.6 Encoder Tests

```rust
#[test]
fn trigram_encoder_deterministic() {
    let v1 = TrigramEncoder::encode("hello world");
    let v2 = TrigramEncoder::encode("hello world");
    assert_eq!(v1, v2);
}

#[test]
fn trigram_encoder_similar_strings_are_similar() {
    let v1 = TrigramEncoder::encode("hello world");
    let v2 = TrigramEncoder::encode("hello world!");
    let dist = hamming_distance(&v1, &v2);
    // Very similar strings should produce similar vectors
    assert!(dist < 4000,
        "similar strings should produce similar vectors, hamming = {dist}");
}

#[test]
fn trigram_encoder_different_strings_are_dissimilar() {
    let v1 = TrigramEncoder::encode("the quick brown fox");
    let v2 = TrigramEncoder::encode("42 is the answer");
    let dist = hamming_distance(&v1, &v2);
    // Different strings should be quasi-orthogonal
    assert!(dist > 4500,
        "different strings should be quasi-orthogonal, hamming = {dist}");
}

#[test]
fn trigram_encoder_empty_returns_zero() {
    let v = TrigramEncoder::encode("");
    assert_eq!(v, HdcVector::default());
}

#[test]
fn structured_encoder_role_filler_retrieval() {
    // Encode a 2-field record
    let mut enc = StructuredEncoder::new();
    enc.add_field("name", "alice");
    enc.add_field("role", "engineer");
    let record = enc.encode();

    // Unbind with "name" role -> should be similar to "alice"
    let role_name = HdcVector::symbol("name");
    let query = bind(&record, &role_name);
    let alice = HdcVector::symbol("alice");

    let dist_correct = hamming_distance(&query, &alice);
    let dist_wrong = hamming_distance(&query, &HdcVector::symbol("bob"));

    assert!(dist_correct < dist_wrong,
        "unbinding with the correct role should yield a closer match: \
         dist_alice={dist_correct}, dist_bob={dist_wrong}");
    assert!(dist_correct < THRESHOLD_HAMMING,
        "unbinding result should be similar to the correct filler: dist={dist_correct}");
}
```

---

## 11. Anti-Patterns

These are mistakes that will break consensus or cause subtle bugs. The
implementing agent must avoid every one.

### 11.1 Never Use Floats On-Chain

```rust
// WRONG: float comparison in consensus path
if similarity(a, b) > 0.526 { ... }

// RIGHT: integer comparison in consensus path
if hamming_distance(a, b) < THRESHOLD_HAMMING { ... }
```

IEEE 754 permits different rounding modes and intermediate precision across
platforms. Two validators computing the same float expression can get
different results. Integer arithmetic is identical everywhere.

### 11.2 Never Use Non-Deterministic RNG

```rust
// WRONG: non-deterministic -- different result every call
use rand::thread_rng;
let v = HdcVector::random_from_rng(&mut thread_rng());

// WRONG: OS entropy -- non-deterministic
use rand::rngs::OsRng;

// RIGHT: seeded, deterministic PRNG
let v = HdcVector::random(42);
let v = HdcVector::symbol("token_name");
```

Every vector generation path must go through `ChaCha20Rng::seed_from_u64()`.
If you need a random vector for testing, use a fixed seed.

### 11.3 Never Use `HashMap` for Iteration-Order-Sensitive Code

```rust
// WRONG: HashMap iteration order is non-deterministic across runs
let map: HashMap<String, HdcVector> = ...;
for (name, vec) in &map {  // order varies!
    acc.add(vec);
}

// RIGHT: use BTreeMap or sort keys first
let map: BTreeMap<String, HdcVector> = ...;
for (name, vec) in &map {  // deterministic order
    acc.add(vec);
}
```

`BundleAccumulator::add()` is commutative in theory (majority vote does not
depend on order), but `BundleAccumulator` with an odd number of items and
ties can exhibit order-dependent edge cases if the accumulator is finalized
mid-stream. Use deterministic iteration to eliminate this class of bugs.

### 11.4 Never Break the `#[repr(C, align(64))]` Layout

```rust
// WRONG: wrapping HdcVector in a struct that changes alignment
struct Wrapper {
    tag: u8,
    vector: HdcVector, // alignment may be broken by preceding u8
}

// RIGHT: if wrapping, ensure alignment is preserved
#[repr(C, align(64))]
struct Wrapper {
    vector: HdcVector,
    tag: u8,
}
```

### 11.5 Never Assume Endianness Without Explicit Conversion

The `serialize` and `deserialize` functions use explicit `to_le_bytes()` and
`from_le_bytes()`. Never use `bytemuck::cast_slice` or pointer casts to
reinterpret `[u64; 160]` as `[u8; 1280]` without endian conversion -- this
produces different bytes on big-endian machines.

### 11.6 Never Use `f32` Where `f64` Is Specified (or Vice Versa)

The `similarity` function returns `f64`. The `ProjectionEncoder` uses `f32`
for dot products. Do not mix them. The `ProjectionEncoder` is off-chain only
precisely because it uses floats.

---

## 12. Implementation Checklist

Complete each item in order. Do not skip ahead.

### Phase 1: Scaffolding
- [x] Create directory `crates/hdc/core/src/`
- [x] Create `Cargo.toml` with correct workspace inheritance and dependencies
- [x] Add `"crates/hdc/*"` to workspace members in root `Cargo.toml`
- [x] Add `kora-hdc`, `rand_chacha`, `bytemuck`, `tiny-keccak` to `[workspace.dependencies]`
- [x] Create `src/lib.rs` with module declarations (can be empty stubs initially)
- [x] Verify: `cargo check -p kora-hdc` compiles with no errors

### Phase 2: Constants and Vector Type
- [x] Implement `src/constants.rs` with all six constants
- [x] Implement `HdcVector` struct with `repr(C, align(64))`
- [x] Derive `Clone, Debug, PartialEq, Eq, Hash`
- [x] Implement `Default` (zero vector)
- [x] Implement `bit()`, `set_bit()`, `popcount()`
- [x] Implement `Send` and `Sync` -- NOTE: not explicitly `unsafe impl`; the type is naturally Send+Sync since `[u64; 160]` is Send+Sync
- [x] Verify: `cargo test -p kora-hdc` passes (no tests yet, but compiles)

### Phase 3: Vector Generation
- [x] Implement `fnv1a_hash()`
- [x] Implement `HdcVector::random(seed)` using `ChaCha20Rng`
- [x] Implement `HdcVector::symbol(name)`
- [x] Add tests: determinism, different seeds differ, ~50% bit density, quasi-orthogonality
- [x] Verify: `cargo test -p kora-hdc` -- all generation tests pass

### Phase 4: Core Operations
- [x] Implement `bind(a, b)` (XOR)
- [x] Implement `hamming_distance(a, b)` -- delegates to `search::simd::hamming_distance`
- [x] Implement `similarity(a, b)` -- mark OFF-CHAIN ONLY in doc comment
- [x] Implement `permute(v, n)` -- full-vector cyclic left rotation
- [x] Add tests: self-inverse, identity, commutativity, dissimilarity, permute inverse, permute composition
- [x] Verify: `cargo test -p kora-hdc` -- all operation tests pass

### Phase 5: Bundle
- [x] Implement `BundleAccumulator` with `new()`, `add()`, `add_weighted()`, `to_vector()`
- [x] Implement `bundle()` convenience function
- [x] Add tests: single-vector bundle, similarity to inputs, tie-breaking, weighted bundle
- [x] Verify: `cargo test -p kora-hdc` -- all bundle tests pass

### Phase 6: Serialization
- [x] Implement `serialize()`
- [x] Implement `deserialize()`
- [x] Implement `vector_id()` (keccak256)
- [x] Add tests: round-trip, length, determinism, different vectors get different IDs
- [x] Verify: `cargo test -p kora-hdc` -- all serialization tests pass

### Phase 7: Encoders
- [x] Implement `TrigramEncoder::encode()`
- [x] Implement `ProjectionEncoder::new()` and `encode()` -- mark OFF-CHAIN ONLY
- [x] Implement `StructuredEncoder` with `add_field()`, `add_field_vec()`, `encode()`
- [x] Add tests: determinism, similar/different strings, role-filler retrieval
- [x] Verify: `cargo test -p kora-hdc` -- all encoder tests pass

### Phase 8: Cleanup and Stub
- [x] Create `src/search/` as full module directory (NOT a single `search.rs` placeholder -- this exceeded spec scope positively)
- [x] Wire all modules into `src/lib.rs` with public re-exports
- [x] Run `cargo clippy -p kora-hdc` -- fix all warnings
- [ ] Run `cargo doc -p kora-hdc --no-deps` -- verify documentation builds
- [x] Verify: `cargo test -p kora-hdc` -- all tests pass, no warnings

### Final Verification
- [x] `HdcVector::random(42)` returns the same 160 words every time -- test: `random_is_deterministic`
- [x] `HdcVector::symbol("test")` returns the same vector every time -- test: `symbol_is_deterministic`
- [x] `bind(bind(a, b), b) == a` for all tested pairs -- test: `bind_self_inverse`
- [x] `permute(permute(v, n), D - n) == v` for all tested values -- test: `permute_inverse`
- [x] `serialize -> deserialize` round-trips perfectly -- test: `serialize_deserialize_roundtrip`
- [x] No `f32` or `f64` in any CONSENSUS-SAFE function
- [x] No `thread_rng()`, `OsRng`, or `StdRng` anywhere in the crate
- [x] No `HashMap` used in iteration-order-sensitive paths (HNSW uses BTreeMap)
- [x] `cargo test -p kora-hdc` passes with zero failures

---

## Audit Findings

> Audit date: 2026-05-08
>
> Files audited:
> - `crates/kora-hdc/` (6 files): `Cargo.toml`, `src/lib.rs`, `src/constants.rs`, `src/encode.rs`, `src/vector.rs`, `src/bundle.rs`, `src/search.rs`
> - `crates/hdc/core/` (28 files): same core 6 files plus `search/`, `knowledge/`, `trust.rs`, `context.rs`, `cognitive/`
> - Spec: `tmp/HDC/impl/02-kora-hdc-core.md`

### AF-01: Two crates with identical `package.name = "kora-hdc"` (Critical)

Both `crates/kora-hdc/Cargo.toml` (line 2) and `crates/hdc/core/Cargo.toml` (line 2) declare `name = "kora-hdc"`. The workspace `Cargo.toml` (line 77) points `kora-hdc` at `crates/hdc/core`, and the workspace members glob `crates/hdc/*` matches `crates/hdc/core`. The `crates/kora-hdc/` directory is **not** included in the workspace members and appears to be an orphaned earlier attempt. Cargo will refuse to compile both simultaneously. The workspace currently builds against `crates/hdc/core/` only.

**Verdict:** `crates/kora-hdc/` is dead code. It should be deleted or, if intended as the canonical location, the workspace must be redirected.

### AF-02: `ProjectionEncoder::new()` uses `rng.next_u32() >> 31` instead of `rng.gen::<bool>()` (Minor spec deviation)

Spec section 6.2 specifies:
```rust
let val: i8 = if rng.gen::<bool>() { 1 } else { -1 };
```

Both implementations (`crates/kora-hdc/src/encode.rs` line 77, `crates/hdc/core/src/encode.rs` line 77) use:
```rust
let val = if (rng.next_u32() >> 31) == 0 { 1i8 } else { -1i8 };
```

These are **not** equivalent. `rng.gen::<bool>()` calls `rng.gen_range(0..2)` which consumes different PRNG state than `rng.next_u32()`. The bit extraction `>> 31` is functionally correct (produces uniform {0,1}) but generates **different projection matrices** from the same seed compared to the spec.

Both implementations use `rand::RngCore` instead of `rand::Rng`. The spec uses `rng.gen::<bool>()` which is from the `Rng` trait. The implementations avoid importing `Rng` and use `RngCore::next_u64()` and `RngCore::next_u32()` directly. This is internally consistent but means the implementations deviate from the spec's PRNG consumption sequence.

**Impact:** Since `ProjectionEncoder` is OFF-CHAIN ONLY, this does not affect consensus. But if vectors generated by the spec-reference implementation are ever compared against vectors from this implementation (same seed), they will differ.

### AF-03: `HdcVector::random()` uses `rng.next_u64()` instead of `rng.gen()` (Functionally equivalent but spec deviation)

Spec section 4.4 specifies:
```rust
*w = rng.gen();
```

Both implementations (`crates/kora-hdc/src/vector.rs` line 72, `crates/hdc/core/src/vector.rs` line 72) use:
```rust
*w = rng.next_u64();
```

For `u64`, `rng.gen::<u64>()` delegates to `rng.next_u64()`, so this is **functionally identical**. However, both files import `rand::RngCore` instead of `rand::Rng`. The code works but if a future change needs `rng.gen::<T>()` for other types, it will fail without also importing `Rng`.

**Impact:** None. Semantically equivalent.

### AF-04: `StructuredEncoder::encode()` does not use the `bundle()` free function (Minor deviation)

Spec section 6.3 specifies:
```rust
pub fn encode(&self) -> HdcVector {
    let refs: Vec<&HdcVector> = self.pairs.iter().collect();
    bundle(&refs)
}
```

Both implementations (`crates/kora-hdc/src/encode.rs` lines 139-148, `crates/hdc/core/src/encode.rs` lines 139-148) instead do:
```rust
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
```

This manually inlines the accumulator loop instead of delegating to `bundle()`. The empty check returning `HdcVector::default()` is an **improvement** over the spec (the spec's `bundle(&[])` would also work since `BundleAccumulator::new()` starts at zero and `to_vector()` on all-zeros produces the zero vector, but the early return is clearer).

However, the field is named `fields` instead of `pairs` per spec. This is cosmetic.

**Impact:** Functionally equivalent. The explicit empty check is arguably better.

### AF-05: Missing `bytemuck` dependency (Spec deviation)

Spec section 2.3 lists `bytemuck` as a dependency. Neither `crates/kora-hdc/Cargo.toml` nor `crates/hdc/core/Cargo.toml` includes it. Neither implementation uses `bytemuck` anywhere.

**Impact:** None currently. The spec mentions it for "zero-copy casting for serialization" but the implementations use manual `to_le_bytes`/`from_le_bytes` loops, which is correct and avoids the endianness pitfall the spec warns about in section 11.5.

### AF-06: `hdc/core` has `serde` dependency not in spec

`crates/hdc/core/Cargo.toml` (line 17) includes `serde = { workspace = true }`. This is not mentioned in the spec and is likely pulled in by the extra modules (`knowledge/`, `trust.rs`, `context.rs`, `cognitive/`). The `kora-hdc` orphan crate does not have this dependency.

### AF-07: `hdc/core` has `criterion` benchmark configuration not in spec

`crates/hdc/core/Cargo.toml` (lines 21-25) includes `criterion` in dev-dependencies and a `[[bench]]` section. This is extra infrastructure not in the spec, but is reasonable.

### AF-08: `hdc/core` has extra modules beyond the spec (Not a problem per se)

`crates/hdc/core/src/lib.rs` declares modules `knowledge`, `trust`, `context`, `cognitive` beyond what the spec calls for. The spec (section 2.1) says `search.rs` should be "re-exports only" for this doc, with the actual search impl in doc 03. The `hdc/core` version has a fully fleshed-out `search/` module with `brute.rs`, `hnsw.rs`, `local.rs`, `tiered.rs`, `simd.rs`.

This is fine -- the spec is for the core algebra, and these are additive. But the crate is no longer a "standalone algebra library" as described in section 1.

### AF-09: `set_bit()` implementation differs stylistically from spec

Spec section 4.3:
```rust
if val == 1 {
    self.0[word] |= 1u64 << bit;
} else {
    self.0[word] &= !(1u64 << bit);
}
```

Both implementations (line 43):
```rust
self.0[word] = (self.0[word] & !(1u64 << offset)) | (val << offset);
```

The implementation uses a branchless single-expression form. **Functionally equivalent** but subtly different: the spec's version only modifies the word if setting to 1 or clearing to 0. The implementation always clears and re-sets. Both correct for valid inputs (val in {0,1}).

---

## Code Duplication Analysis

### CD-01: `crates/kora-hdc/` is a 100% duplicate of `crates/hdc/core/` (core files)

The following file pairs are **byte-for-byte identical** or differ only in trivially (unused import tidiness):

| kora-hdc file | hdc/core file | Identical? |
|---|---|---|
| `src/constants.rs` (23 lines) | `src/constants.rs` (23 lines) | **Identical** |
| `src/vector.rs` (305 lines) | `src/vector.rs` (305 lines) | **Identical** |
| `src/bundle.rs` (127 lines) | `src/bundle.rs` (127 lines) | **Identical** |
| `src/encode.rs` (252 lines) | `src/encode.rs` (252 lines) | **Identical** |
| `src/search.rs` (9 lines) | `src/search/mod.rs` (99 lines) | **Diverged** -- kora-hdc has stub; hdc/core has full implementation |
| `src/lib.rs` (36 lines) | `src/lib.rs` (43 lines) | **Diverged** -- hdc/core has extra module declarations and re-exports |

**Total duplicated code:** ~707 lines across 4 files are exact copies. These will inevitably drift if both are maintained.

### CD-02: The duplication is a maintenance hazard

Both crates declare `name = "kora-hdc"` so they cannot coexist in the workspace. The `crates/kora-hdc/` directory is not referenced by any workspace member glob. Any consumer (`kora-e2e`, `kora-rpc`, `kora-hdc-chain`, `kora-runner`, `kora-executor`) that depends on `kora-hdc.workspace = true` gets `crates/hdc/core/`, not `crates/kora-hdc/`.

The orphaned `crates/kora-hdc/` crate is dead weight that will confuse contributors.

---

## Anti-Patterns & Duct Tape

### AP-01: Orphan crate pattern

Having `crates/kora-hdc/` sitting outside the workspace with the same package name as `crates/hdc/core/` is a classic "forgot to delete the old one" anti-pattern. It means:
- `cargo check` in `crates/kora-hdc/` may pass but produces a binary/library that nothing links against
- IDE navigation may jump to the wrong file
- Search results show duplicate hits for every function

### AP-02: `BundleAccumulator::add()` per-bit loop is O(D) with per-bit overhead

Both implementations use the naive per-bit `vector.bit(i)` loop in `BundleAccumulator::add()` (bundle.rs lines 23-30) and `add_weighted()` (lines 34-43). Each `bit(i)` call performs a division and modulo. The spec acknowledges this (section 5, "Performance Note") and provides a word-level optimization. Neither implementation adopted it.

For 10,240-bit vectors, this means 10,240 divisions and modulos per `add()` call. With the word-level approach, this drops to 160 iterations with simple shifts.

**Severity:** Performance anti-pattern, not a correctness issue. The `BundleAccumulator` is used in `TrigramEncoder::encode()` on every trigram, so long strings will feel this.

### AP-03: `TrigramEncoder::encode()` regenerates symbol vectors on every call

In `encode.rs` lines 37-49, for each trigram window, three `HdcVector::symbol()` calls are made. Each call runs FNV-1a hash + ChaCha20Rng seeding + 160 `next_u64()` calls. For a string of length N, this is `3*(N-2)` full vector generations.

Since the same character appears in multiple trigrams (e.g., in "hello", 'l' appears in trigrams "hel", "ell", "llo"), the same symbol vector is regenerated repeatedly. A cache (`HashMap<char, HdcVector>` or simple array for ASCII) would avoid redundant PRNG work.

**Severity:** Performance anti-pattern. Not a correctness issue.

### AP-04: `StructuredEncoder` stores intermediate bound vectors unnecessarily

`StructuredEncoder.fields: Vec<HdcVector>` (encode.rs line 116) stores a full 1,280-byte `HdcVector` per field. Since these are only used once in `encode()`, the accumulator could be run inline in `add_field()` to avoid the allocation:

```rust
pub struct StructuredEncoder {
    acc: BundleAccumulator,
}
impl StructuredEncoder {
    pub fn add_field(&mut self, role: &str, filler: &str) {
        let bound = bind(&HdcVector::symbol(role), &HdcVector::symbol(filler));
        self.acc.add(&bound);
    }
    pub fn encode(&self) -> HdcVector { self.acc.to_vector() }
}
```

This would save `N * 1,280` bytes of heap allocation for N fields.

**Severity:** Minor. Unlikely to matter for small field counts.

### AP-05: No `#[inline]` annotations on hot-path functions

The spec marks `bit()` and `set_bit()` with `#[inline]`. Both implementations omit `#[inline]` on these methods, as well as on `bind()`, `hamming_distance()`, and the inner loop of `permute()`. Since these are free functions in a library crate, the compiler may not inline them across crate boundaries without LTO.

**Severity:** Performance consideration for consumers. The `profile.release` in the workspace has `lto = "thin"` which partially mitigates this.

### AP-06: `unwrap()` in `deserialize()` instead of `expect()`

`vector.rs` line 150: `bytes[i * 8..(i + 1) * 8].try_into().unwrap()`. The spec uses `.expect("slice is 8 bytes")`. The `unwrap()` is fine since the slice is guaranteed to be 8 bytes (input is `&[u8; BYTES]`), but `expect()` is better practice for documenting the invariant.

### AP-07: Unused import `rand::RngCore` in `encode.rs`

`crates/kora-hdc/src/encode.rs` line 3 and `crates/hdc/core/src/encode.rs` line 3 both import `rand::RngCore`. This trait is only used inside `ProjectionEncoder::new()` (via `rng.next_u32()`). The import at file scope is fine but slightly unusual -- the spec puts the import inside the method. With workspace lint `clippy::all = "warn"`, this may or may not trigger depending on whether the import is considered "used."

---

## Recommended Changes Checklist

### P0 (Critical -- do first)

- [ ] **Delete `crates/kora-hdc/` entirely.** It is an orphaned duplicate. All consumers resolve `kora-hdc` to `crates/hdc/core/`. Search the repo for any stray references to the `crates/kora-hdc/` path and remove them.
  - Files to delete: `crates/kora-hdc/Cargo.toml`, `crates/kora-hdc/src/lib.rs`, `crates/kora-hdc/src/constants.rs`, `crates/kora-hdc/src/vector.rs`, `crates/kora-hdc/src/bundle.rs`, `crates/kora-hdc/src/encode.rs`, `crates/kora-hdc/src/search.rs`

### P1 (Important -- do soon)

- [ ] **Add `#[inline]` to hot-path functions** in `crates/hdc/core/src/vector.rs`: `bit()` (line 30), `set_bit()` (line 38), `popcount()` (line 47), `bind()` (line 87), `hamming_distance()` (line 96)
- [ ] **Adopt word-level loop in `BundleAccumulator::add()` and `add_weighted()`** in `crates/hdc/core/src/bundle.rs` (lines 23-43). Use the `add_fast` pattern from spec section 5 to avoid per-bit division/modulo overhead.
- [ ] **Change `unwrap()` to `expect("slice is 8 bytes")` in `deserialize()`** at `crates/hdc/core/src/vector.rs` line 150.
- [ ] **Document the `next_u32() >> 31` vs `gen::<bool>()` divergence** from the spec in `crates/hdc/core/src/encode.rs` line 77. Add a comment explaining the choice and noting it produces different matrices than the spec's reference code.

### P2 (Nice to have -- do when touching these files)

- [ ] **Cache symbol vectors in `TrigramEncoder::encode()`** (`crates/hdc/core/src/encode.rs` lines 37-49). Use a local `HashMap<char, HdcVector>` or `[Option<HdcVector>; 128]` for ASCII to avoid regenerating the same symbol vector multiple times per encode call.
- [ ] **Refactor `StructuredEncoder` to accumulate inline** instead of storing intermediate vectors (`crates/hdc/core/src/encode.rs` lines 115-149). Replace `fields: Vec<HdcVector>` with `acc: BundleAccumulator` and add in `add_field()` directly. Note: this changes the API slightly (you can no longer call `encode()` multiple times with different accumulated states, though the current API does not support removal either, so this is safe).
- [ ] **Scope the `rand::RngCore` import** inside `ProjectionEncoder::new()` rather than at file level in `crates/hdc/core/src/encode.rs` line 3. Similarly for `rand::SeedableRng` (line 4) and `rand_chacha::ChaCha20Rng` (line 5) -- these are only used in `ProjectionEncoder::new()`.
- [ ] **Add `TrigramEncoder::new()` doc-test** or usage example in `crates/hdc/core/src/encode.rs`. The `new()` method (line 21) exists but is never tested directly (all tests call `TrigramEncoder::encode()` as a static method).
- [ ] **Consider whether `HdcVector(pub [u64; WORDS])` should remain `pub`**. The inner array is public, allowing callers to bypass `set_bit()` and directly mutate words. This is intentional per spec (bundle.rs uses `a.0[i]`), but means the `assert!(val <= 1)` invariant in `set_bit()` can be circumvented.

### P3 (Informational -- no action needed)

- [ ] The `hdc/core` crate has grown beyond the "standalone algebra library" scope described in section 1 of this spec. The `knowledge/`, `trust.rs`, `context.rs`, and `cognitive/` modules introduce domain concepts that the spec says belong in higher-level crates. Consider whether these should be factored out into separate crates (e.g., `kora-hdc-knowledge`, `kora-hdc-trust`) or if the spec should be updated to reflect the expanded scope.
- [ ] The `search/` module in `hdc/core` has a full HNSW + brute-force + tiered pipeline implementation, which the spec says belongs in doc 03. This is fine as long as doc 03 is considered implemented. Verify that `crates/hdc/core/src/search/` matches the doc 03 spec.
- [ ] Both implementations use `rand::RngCore` trait methods directly instead of the `rand::Rng` blanket. This is a deliberate choice (avoids importing `Rng` trait) but means the PRNG consumption sequence for `ProjectionEncoder` differs from spec. Since `ProjectionEncoder` is OFF-CHAIN ONLY, this is acceptable.

---

## Second-Pass Remediation Detail

This section turns the audit findings above into a concrete implementation plan.
It is intentionally scoped to future code changes in `crates/hdc/core/` plus
deletion of the orphan crate. The canonical crate is `crates/hdc/core`; do not
try to keep both crate trees alive.

### Canonical Crate and Duplicate Deletion

**Decision:** keep `crates/hdc/core/` as the only `package.name = "kora-hdc"`
implementation and delete `crates/kora-hdc/` completely.

Rationale:

- Root `Cargo.toml` already has `members = [..., "crates/hdc/*", ...]`.
- Root `[workspace.dependencies]` already resolves `kora-hdc` to
  `path = "crates/hdc/core"`.
- `crates/kora-hdc/` is outside the workspace and duplicates the same package
  name, which makes editor navigation and search results ambiguous.

Implementation steps:

1. Delete the entire `crates/kora-hdc/` directory:
   - `crates/kora-hdc/Cargo.toml`
   - `crates/kora-hdc/src/lib.rs`
   - `crates/kora-hdc/src/constants.rs`
   - `crates/kora-hdc/src/vector.rs`
   - `crates/kora-hdc/src/bundle.rs`
   - `crates/kora-hdc/src/encode.rs`
   - `crates/kora-hdc/src/search.rs`
2. Do not change the root workspace dependency unless it points away from
   `crates/hdc/core`.
3. Verify no path references remain:
   - `rg -n "crates/kora-hdc|path = \"crates/kora-hdc\"" .`
   - Expected: no matches.
4. Verify only one `kora-hdc` package is visible to Cargo:
   - `cargo metadata --no-deps --format-version 1`
   - Expected: exactly one package named `kora-hdc`, with manifest path
     `crates/hdc/core/Cargo.toml`.

### Vector Layout Safety

**Target public API:** keep these signatures and names stable:

```rust
#[repr(C, align(64))]
pub struct HdcVector(pub [u64; WORDS]);

pub fn serialize(vector: &HdcVector) -> [u8; BYTES];
pub fn deserialize(bytes: &[u8; BYTES]) -> HdcVector;
pub fn vector_id(vector: &HdcVector) -> [u8; 32];
```

The layout guarantee is for in-memory alignment and predictable field order
only. It is not the serialization format. The serialization format remains
explicit little-endian words, written with `to_le_bytes()` and read with
`from_le_bytes()`.

Required checks in `crates/hdc/core/src/vector.rs`:

- Add `#[inline]` to hot-path helpers:
  - `HdcVector::bit`
  - `HdcVector::set_bit`
  - `HdcVector::popcount`
  - `bind`
  - `hamming_distance`
  - `permute`
- Change `deserialize()` from `.unwrap()` to
  `.expect("slice is 8 bytes")`.
- Keep `HdcVector(pub [u64; WORDS])` public unless all downstream direct word
  access is first audited. The search, tests, and complement helpers currently
  rely on direct word-level access.

Required tests in `crates/hdc/core/src/vector.rs`:

```rust
#[test]
fn vector_layout_size_and_alignment() {
    assert_eq!(std::mem::size_of::<HdcVector>(), BYTES);
    assert_eq!(std::mem::align_of::<HdcVector>(), 64);
}

#[test]
fn serialize_uses_little_endian_words() {
    let mut words = [0u64; WORDS];
    words[0] = 0x0102_0304_0506_0708;
    words[1] = 0x1112_1314_1516_1718;
    let bytes = serialize(&HdcVector(words));
    assert_eq!(&bytes[0..8], &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    assert_eq!(&bytes[8..16], &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]);
}
```

### Serde and Bytemuck Decision

**Decision:** do not add `bytemuck` for consensus serialization. Manual
little-endian conversion is the canonical encoding.

Reasoning:

- `bytemuck::cast_slice()` exposes native-endian `u64` layout, not canonical
  little-endian bytes.
- It also makes future layout changes more dangerous because consumers may
  start treating in-memory layout as wire format.
- `HdcVector` currently has no padding because `160 * 8 = 1,280` and `1,280`
  is divisible by the 64-byte alignment, but that should be proven by tests,
  not used as a shortcut for serialization.

**Decision:** remove `serde = { workspace = true }` from
`crates/hdc/core/Cargo.toml` unless a follow-up patch actually adds serde
derives or manual serde impls. The current audited source does not use serde.

If serde support is needed later, add it deliberately and serialize
`HdcVector` through the canonical byte functions:

- `Serialize` should emit `serialize(self)` as bytes.
- `Deserialize` should require exactly `BYTES` bytes and call `deserialize()`.
- Do not derive serde directly for `HdcVector(pub [u64; WORDS])` if the result
  may be used as a portable or consensus encoding.

### Bundle Optimization

**Target public API:** keep the current API stable:

```rust
impl BundleAccumulator {
    pub fn new() -> Self;
    pub fn add(&mut self, vector: &HdcVector);
    pub fn add_weighted(&mut self, vector: &HdcVector, weight: u32);
    pub fn to_vector(&self) -> HdcVector;
}

pub fn bundle(vectors: &[&HdcVector]) -> HdcVector;
```

Implementation target in `crates/hdc/core/src/bundle.rs`:

- Import `WORDS` alongside `D`.
- Replace the per-bit `for i in 0..D { vector.bit(i) ... }` loops in
  `BundleAccumulator::add()` and `add_weighted()` with word-level loops.
- Keep the accumulator layout as `counts: Vec<i32>` for now to avoid an API or
  invariant change. This optimization is purely about avoiding division and
  modulo in the inner loop.
- Preserve deterministic tie behavior: `to_vector()` sets a bit only when
  `count > 0`; ties stay zero.
- Preserve `weight == 0` as a no-op.
- Keep the existing `weight.try_into().expect("weight exceeds i32::MAX")`
  guard unless the accumulator is redesigned to use wider counters.

Suggested private helper:

```rust
fn add_word_counts(counts: &mut [i32], word_idx: usize, word: u64, delta: i32) {
    let base = word_idx * 64;
    for bit in 0..64 {
        let sign = if ((word >> bit) & 1) == 1 { delta } else { -delta };
        counts[base + bit] += sign;
    }
}
```

Required tests in `crates/hdc/core/src/bundle.rs`:

- Existing bundle tests must remain unchanged and pass.
- Add `bundle_add_weighted_zero_is_noop`.
- Add `bundle_word_level_matches_repeated_add`:
  - Build one random vector.
  - Compare `add_weighted(&v, 3)` with three calls to `add(&v)`.
  - Assert `to_vector()` equality.
- Add a hand-constructed word-boundary test:
  - Set bit 63 and bit 64 on one vector.
  - Bundle it once.
  - Assert those exact bits are set in the result.

Benchmark follow-up:

- Keep or update `crates/hdc/core/benches/hdc_bench.rs`.
- Compare `bundle_50` before and after the word-level change.
- The benchmark is supporting evidence only; correctness tests are mandatory.

### Encoder Caching and PRNG Clarification

**Target public API:** keep the encoder APIs stable:

```rust
impl TrigramEncoder {
    pub const fn new() -> Self;
    pub fn encode(text: &str) -> HdcVector;
}

impl ProjectionEncoder {
    pub fn new(input_dim: usize, seed: u64) -> Self;
    pub fn encode(&self, embedding: &[f32]) -> HdcVector;
}

impl StructuredEncoder {
    pub const fn new() -> Self;
    pub fn add_field(&mut self, role: &str, filler: &str);
    pub fn add_field_vec(&mut self, role: &str, filler: &HdcVector);
    pub fn encode(&self) -> HdcVector;
}
```

`TrigramEncoder::encode()` should cache character symbol vectors inside a
single encode call. This preserves determinism and avoids repeatedly running
FNV-1a plus ChaCha20 vector generation for common characters.

Implementation target in `crates/hdc/core/src/encode.rs`:

- Use a local `BTreeMap<char, HdcVector>` cache or a local lookup helper.
  `HashMap` would be safe if never iterated, but `BTreeMap` avoids future
  iteration-order concerns and keeps the implementation obviously
  deterministic.
- Do not introduce a global cache unless it is protected by a clear concurrency
  plan and a bounded memory policy. A local cache is enough for this pass.
- Keep Unicode behavior based on `text.chars()`. Do not switch to bytes.
- For 0, 1, and 2 character inputs, preserve exact current output.
- For 3+ character inputs, preserve the current trigram formula:
  `bind(permute(s0, 2), bind(permute(s1, 1), s2))`.

Suggested private helper:

```rust
fn cached_symbol(cache: &mut BTreeMap<char, HdcVector>, ch: char) -> HdcVector {
    cache
        .entry(ch)
        .or_insert_with(|| HdcVector::symbol(&ch.to_string()))
        .clone()
}
```

Required tests in `crates/hdc/core/src/encode.rs`:

- Add a test-only reference implementation of the current uncached algorithm
  and assert the cached encoder matches it for:
  - empty string
  - one character
  - two characters
  - repeated ASCII text such as `"banana bandana"`
  - Unicode text such as `"naive cafe"` with the intended source spelling
    used consistently in the test fixture
- Keep `trigram_encoder_similar_strings_are_similar` and
  `trigram_encoder_different_strings_are_dissimilar`.

`ProjectionEncoder::new()` currently uses `rng.next_u32() >> 31` rather than
`rng.gen::<bool>()`. Since `ProjectionEncoder` is off-chain only, either choice
is acceptable, but the crate must pick one as the canonical local behavior.

Second-pass decision:

- Keep `rng.next_u32() >> 31` if preserving current output compatibility is
  more important than matching this document's earlier pseudocode.
- Add a doc comment stating that the projection matrix is deterministic for
  this implementation but does not match the earlier `gen::<bool>()`
  pseudocode bit-for-bit.
- Add a regression test for the first few matrix signs indirectly through a
  fixed seed and fixed embedding output, or expose no internals and keep the
  existing deterministic encode test.

### StructuredEncoder Storage Decision

`StructuredEncoder` currently stores `fields: Vec<HdcVector>` and bundles on
`encode()`. This is memory-heavy but API-friendly: callers can call
`encode()` multiple times while adding fields between calls.

Second-pass decision:

- Do not refactor `StructuredEncoder` to store only a `BundleAccumulator` in
  the same patch as trigram caching. That would change observable behavior if
  future callers rely on field inspection, cloning semantics, or repeated
  encode calls after intermediate additions.
- If memory pressure becomes real, introduce a separate streaming type instead:

```rust
pub struct StreamingStructuredEncoder {
    acc: BundleAccumulator,
}
```

That keeps `StructuredEncoder` stable while giving high-volume callers a
lower-allocation path.

### File and Function-Level Checklist

Use this checklist for the remediation implementation pass.

Duplicate crate:

- [ ] Delete `crates/kora-hdc/`.
- [ ] Root `Cargo.toml`: keep `kora-hdc = { path = "crates/hdc/core" }`.
- [ ] Root `Cargo.toml`: keep `"crates/hdc/*"` in `workspace.members`.
- [ ] Verify `rg -n "crates/kora-hdc|path = \"crates/kora-hdc\"" .` has no
      matches.
- [ ] Verify `cargo metadata --no-deps --format-version 1` reports one
      `kora-hdc` package.

`crates/hdc/core/Cargo.toml`:

- [ ] Remove `serde = { workspace = true }` if still unused.
- [ ] Do not add `bytemuck` for vector serialization.
- [ ] Keep `tiny-keccak`, `rand`, and `rand_chacha`.
- [ ] Keep `criterion` only if `benches/hdc_bench.rs` remains in use.

`crates/hdc/core/src/vector.rs`:

- [ ] Preserve `#[repr(C, align(64))] pub struct HdcVector(pub [u64; WORDS]);`.
- [ ] Add `#[inline]` to `bit`, `set_bit`, `popcount`, `bind`,
      `hamming_distance`, and `permute`.
- [ ] Keep `HdcVector::random(seed)` deterministic through
      `ChaCha20Rng::seed_from_u64(seed)`.
- [ ] Keep `HdcVector::symbol(name)` as FNV-1a seed plus `random(seed)`.
- [ ] Keep `serialize()` and `deserialize()` manual little-endian conversions.
- [ ] Change `deserialize()` `.unwrap()` to `.expect("slice is 8 bytes")`.
- [ ] Add layout and little-endian serialization tests.

`crates/hdc/core/src/bundle.rs`:

- [ ] Import `WORDS`.
- [ ] Replace per-bit `vector.bit(i)` loops in `add()` and `add_weighted()`
      with word-level loops.
- [ ] Preserve `BundleAccumulator` public methods and `bundle()`.
- [ ] Preserve tie-to-zero behavior in `to_vector()`.
- [ ] Add weighted-zero, repeated-add equivalence, and word-boundary tests.

`crates/hdc/core/src/encode.rs`:

- [ ] Add local symbol caching inside `TrigramEncoder::encode()`.
- [ ] Use `BTreeMap<char, HdcVector>` or an equivalent deterministic local
      cache.
- [ ] Preserve char-based Unicode semantics.
- [ ] Preserve the exact current trigram binding/permutation formula.
- [ ] Document the `ProjectionEncoder::new()` PRNG choice.
- [ ] Do not change `StructuredEncoder` storage in this same patch.
- [ ] Add cached-vs-reference encoder tests.

`crates/hdc/core/src/lib.rs`:

- [ ] Keep crate-root re-exports for `HdcVector`, `bind`, `hamming_distance`,
      `similarity`, `permute`, `serialize`, `deserialize`, `vector_id`,
      `BundleAccumulator`, `bundle`, `TrigramEncoder`, `ProjectionEncoder`,
      and `StructuredEncoder`.
- [ ] Do not remove the extra `knowledge`, `trust`, `context`, `cognitive`, or
      `search` modules as part of this core cleanup unless a separate migration
      plan owns that scope.

Verification commands:

- [ ] `cargo test -p kora-hdc`
- [ ] `cargo clippy -p kora-hdc --all-targets --all-features -- -D warnings`
- [ ] `cargo doc -p kora-hdc --no-deps`
- [ ] Optional benchmark check: `cargo bench -p kora-hdc --bench hdc_bench`
