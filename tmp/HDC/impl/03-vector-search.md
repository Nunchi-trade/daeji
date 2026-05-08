# 03 -- Vector Search Module

> **STATUS: FULLY IMPLEMENTED** (all search infrastructure complete and tested)
>
> | Component | Status | Source File | Test Count |
> |-----------|--------|------------|------------|
> | Scalar Hamming distance | DONE | `crates/hdc/core/src/search/simd.rs:12-18` | 5 tests |
> | AVX2 Harley-Seal | DONE | `crates/hdc/core/src/search/simd.rs:64-130` | shared |
> | AVX-512 VPOPCNTDQ | DONE | `crates/hdc/core/src/search/simd.rs:136-153` | shared |
> | ARM NEON | DONE | `crates/hdc/core/src/search/simd.rs:161-196` | shared |
> | Runtime dispatch | DONE | `crates/hdc/core/src/search/simd.rs:207-225` | shared |
> | SearchIndex trait + HdcIndexError | DONE | `crates/hdc/core/src/search/mod.rs` | -- |
> | BruteForceIndex | DONE | `crates/hdc/core/src/search/brute.rs` | 7 tests |
> | HnswIndex | DONE | `crates/hdc/core/src/search/hnsw.rs` | 8 tests |
> | LocalIndex (auto-switching) | DONE | `crates/hdc/core/src/search/local.rs` | 4 tests |
> | TieredSearchPipeline | DONE | `crates/hdc/core/src/search/tiered.rs` | 5 tests |
> | Criterion benchmarks | TODO | -- | -- |

---

## Verification Commands

```bash
# Run all search module tests:
cargo test -p kora-hdc search::

# Specific test names:
#   simd: hamming_zero_vectors, hamming_complementary_vectors, hamming_self_distance,
#         hamming_symmetry, hamming_simd_matches_scalar
#   brute: insert_search_roundtrip, duplicate_key_rejected, empty_index_error,
#          delete_works, top_k_ordering, top_k_larger_than_index, deterministic_results
#   hnsw: insert_search_roundtrip, duplicate_key_rejected, empty_index_error,
#          delete_works, hnsw_matches_brute_force, deterministic_level_assignment,
#          deterministic_index_builds, top_k_ordering, delete_excludes_from_search
#   local: starts_as_brute_force, basic_insert_and_search, delete_works, search_multiple
#   tiered: basic_insert_and_search, tiered_matches_brute_force_with_permissive_thresholds,
#           tiered_filters_correctly, tiered_results_are_sorted, tiered_deterministic, empty_pipeline

# Clippy:
cargo clippy -p kora-hdc
```

---

## HNSW Parameters (as implemented)

| Parameter | Constant Name | Value | Notes |
|-----------|--------------|-------|-------|
| Max connections per layer (L>=1) | `M` | 16 | Standard HNSW parameter |
| Max connections at layer 0 | `M_MAX0` | 32 | `2 * M`, denser base layer |
| Construction beam width | `EF_CONSTRUCTION` | 200 | Higher = better graph quality |
| Default search beam width | `EF_SEARCH_DEFAULT` | 100 | Tunable via `set_ef_search(50..=200)` |
| Maximum level | `MAX_LEVEL` | 16 | Caps structure for extreme hash values |
| Tombstone compaction threshold | `COMPACTION_THRESHOLD_PERCENT` | 20% | Rebuild when >20% nodes deleted |
| Auto-switch threshold (LocalIndex) | `SWITCH_THRESHOLD` | 10,000 | BruteForce -> HNSW at this count |

---

## SIMD Performance Targets

| SIMD path | Target latency (D=10,240) | Hardware |
|---|---|---|
| Scalar | ~120 ns | Intel Skylake 4 GHz |
| AVX2 Harley-Seal | ~40 ns | Intel Skylake 4 GHz |
| AVX-512 VPOPCNTDQ | ~7 ns | Intel Ice Lake 3.5 GHz |
| NEON | ~28 ns | Apple M2 3.5 GHz |

**Note:** No criterion benchmarks have been added yet. These targets are from the design spec.
To benchmark: `cargo bench -p kora-hdc` (after adding `benches/` with criterion).

---

## On-Chain vs Off-Chain Search

PR #42 changed the HDC precompile to address `0x0A0C` (not `0x09` as originally planned)
and uses 4-byte selectors with a 50k gas budget. The on-chain search is **brute-force only**:

- The `TieredSearchPipeline` is designed for on-chain gas optimization but is currently
  used only in the off-chain knowledge store.
- HNSW is **not used on-chain** because its non-deterministic graph traversal order
  (due to floating-point level assignment in the original HNSW paper) would break consensus.
  The implementation uses a deterministic `keccak256 + leading_zeros` level assignment,
  so it IS consensus-safe, but the precompile currently only exposes brute-force search.
- The `BruteForceIndex` is the only search method exposed via the precompile because
  it is simplest to audit and has zero false negatives.
- Off-chain (agent-local) search uses `LocalIndex` which auto-switches to HNSW above 10K vectors.

---

## Remaining Work

- [ ] Add criterion benchmarks (`benches/hamming.rs`, `benches/search.rs`)
- [ ] Increase SIMD-vs-scalar test to 1000 pairs (currently 100)
- [ ] Add `LocalIndex` upgrade threshold test (insert >10K, verify HNSW switch)
- [ ] Add `TieredSearchPipeline` delete capability (currently append-only)
- [ ] Add threshold calibration helpers to `TieredSearchPipeline`
- [ ] Add `top_k = 0` edge case tests for all index types
- [ ] Consider `Arc<HdcVector>` or arena allocation for HNSW to avoid 1,280-byte clones

---

> **Crate:** `kora-hdc` (`crates/hdc/core/`)
> **Module path:** `crates/hdc/core/src/search/`
> **Depends on:** `02-kora-hdc-core.md` (the `HdcVector` type, `H256` key type)
> **Design spec:** `/Users/will/dev/nunchi/daeji/tmp/HDC/06-vector-search.md`

---

## 0. What This Module Does

Given N binary hypervectors (10,240 bits each, stored as `[u64; 160]`) and a
query vector, find the K vectors with the smallest Hamming distance to the
query. This module provides:

1. SIMD-accelerated Hamming distance (the hot path)
2. Brute-force linear scan with top-K heap
3. HNSW approximate nearest neighbor index
4. Auto-switching LocalIndex (brute-force below threshold, HNSW above)
5. Tiered search pipeline for on-chain gas optimization

---

## 1. Module Structure

Create the following files under `crates/hdc/core/src/search/`:

```
search/
  mod.rs      -- re-exports, SearchIndex trait, HdcIndexError
  simd.rs     -- Hamming distance: scalar, AVX2, AVX-512, NEON, runtime dispatch
  brute.rs    -- BruteForceIndex
  hnsw.rs     -- HnswIndex (deterministic, consensus-safe)
  local.rs    -- LocalIndex (auto-switching enum)
  tiered.rs   -- TieredSearchPipeline (gas-optimized on-chain search)
```

Register the module in `crates/hdc/core/src/lib.rs`:

```rust
pub mod search;
```

---

## 2. `search/mod.rs` -- Trait, Error, Re-exports

```rust
use crate::HdcVector; // from the core algebra module (doc 02)

/// Opaque key identifying a stored vector.
/// Use the H256 type from the core crate (32-byte hash).
/// If H256 is not yet defined, use: pub type H256 = [u8; 32];
use crate::H256;

mod simd;
mod brute;
mod hnsw;
mod local;
mod tiered;

pub use simd::hamming_distance;
pub use brute::BruteForceIndex;
pub use hnsw::HnswIndex;
pub use local::LocalIndex;
pub use tiered::TieredSearchPipeline;

/// Result type alias for search operations.
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
    fn is_empty(&self) -> bool { self.len() == 0 }
}

/// Errors from index operations.
#[derive(Debug)]
pub enum HdcIndexError {
    /// No vectors in the index.
    EmptyIndex,
    /// Requested top_k > stored vector count. The search will
    /// still return all available vectors (not an abort).
    InsufficientVectors { requested: usize, available: usize },
    /// Vector data is corrupt or wrong length.
    InvalidVector(String),
    /// A vector with this key already exists.
    DuplicateKey(H256),
    /// Fixed-capacity index is full.
    CapacityExceeded { capacity: usize },
    /// HNSW graph invariant violated.
    GraphCorruption(String),
    /// I/O error during persistence.
    StorageError(std::io::Error),
}

impl std::fmt::Display for HdcIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyIndex => write!(f, "cannot search an empty index"),
            Self::InsufficientVectors { requested, available } =>
                write!(f, "requested top-{requested} but only {available} vectors in index"),
            Self::InvalidVector(e) => write!(f, "invalid vector: {e}"),
            Self::DuplicateKey(k) => write!(f, "duplicate key: {k:?}"),
            Self::CapacityExceeded { capacity } =>
                write!(f, "index capacity {capacity} exceeded"),
            Self::GraphCorruption(msg) => write!(f, "HNSW graph corruption: {msg}"),
            Self::StorageError(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for HdcIndexError {}

impl From<std::io::Error> for HdcIndexError {
    fn from(e: std::io::Error) -> Self { Self::StorageError(e) }
}
```

---

## 3. `search/simd.rs` -- Hamming Distance

This is the single hottest function in the entire system. Every search
operation calls it N times (brute-force) or hundreds of times (HNSW
traversal). Get this right first.

### 3.1 Scalar Fallback

```rust
/// Reference implementation. Pure integer arithmetic. Bit-exact on all
/// platforms. Also used as the oracle for testing SIMD paths.
#[inline]
pub fn hamming_scalar(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let mut dist = 0u32;
    for i in 0..160 {
        dist += (a[i] ^ b[i]).count_ones();
    }
    dist
}
```

Performance target: ~120ns at D=10,240 on Intel Skylake 4 GHz.

### 3.2 AVX2 Harley-Seal

Reference: Mula, Kurz, Lemire (2017), "Faster Population Counts Using AVX2
Instructions."

Two building blocks, then the main function:

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Byte-level popcount of a 256-bit register via the vpshufb lookup trick.
/// Each byte of the result contains the popcount of the corresponding input byte.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn popcount_mm256(v: __m256i) -> __m256i {
    let lookup = _mm256_setr_epi8(
        0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4,
        0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4,
    );
    let low_mask = _mm256_set1_epi8(0x0f);
    let lo = _mm256_and_si256(v, low_mask);
    let hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), low_mask);
    let popcnt_lo = _mm256_shuffle_epi8(lookup, lo);
    let popcnt_hi = _mm256_shuffle_epi8(lookup, hi);
    _mm256_add_epi8(popcnt_lo, popcnt_hi)
}

/// Carry-save adder: returns (sum, carry) = (a XOR b XOR c, majority(a,b,c)).
/// Each output bit position independently computes the full-adder logic.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn csa_256(a: __m256i, b: __m256i, c: __m256i) -> (__m256i, __m256i) {
    let u = _mm256_xor_si256(a, b);
    let sum = _mm256_xor_si256(u, c);
    let carry = _mm256_or_si256(
        _mm256_and_si256(a, b),
        _mm256_and_si256(u, c),
    );
    (sum, carry)
}

/// Hamming distance between two 10,240-bit vectors using AVX2 Harley-Seal.
///
/// Processes 256 bits per load. 160 u64s = 1280 bytes = 40 x 256-bit chunks.
/// Groups of 4 chunks are accumulated via carry-save adders into ones/twos/fours,
/// reducing the number of horizontal additions by 4x.
///
/// Loop structure:
///   - 10 outer iterations (40 chunks / 4 per group)
///   - Each iteration: 4 loads+XORs, 3 CSA calls, 1 popcount+sad accumulation
///   - After the loop: flush ones (weight 1) and twos (weight 2) into total,
///     then apply weight 4 to the accumulated fours
///
/// Performance target: ~40ns at D=10,240 on Skylake 4 GHz.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn hamming_avx2(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let a_ptr = a.as_ptr() as *const __m256i;
    let b_ptr = b.as_ptr() as *const __m256i;

    let mut total = _mm256_setzero_si256(); // accumulates popcount(carry2) per group
    let mut ones = _mm256_setzero_si256();  // bit positions seen an odd number of times
    let mut twos = _mm256_setzero_si256();  // bit positions seen 2 (mod 4) times

    // 40 chunks / 4 per group = 10 outer iterations
    for i in 0..10 {
        let base = i * 4;

        // Load and XOR 4 chunk pairs
        let d0 = _mm256_xor_si256(
            _mm256_loadu_si256(a_ptr.add(base)),
            _mm256_loadu_si256(b_ptr.add(base)),
        );
        let d1 = _mm256_xor_si256(
            _mm256_loadu_si256(a_ptr.add(base + 1)),
            _mm256_loadu_si256(b_ptr.add(base + 1)),
        );
        let d2 = _mm256_xor_si256(
            _mm256_loadu_si256(a_ptr.add(base + 2)),
            _mm256_loadu_si256(b_ptr.add(base + 2)),
        );
        let d3 = _mm256_xor_si256(
            _mm256_loadu_si256(a_ptr.add(base + 3)),
            _mm256_loadu_si256(b_ptr.add(base + 3)),
        );

        // Carry-save: fold d0..d3 into ones and twos
        let (new_ones, carry0) = csa_256(ones, d0, d1);
        let (new_ones2, carry1) = csa_256(new_ones, d2, d3);
        ones = new_ones2;
        let (new_twos, carry2) = csa_256(twos, carry0, carry1);
        twos = new_twos;

        // carry2 = "fours": each set bit here represents 4 set bits in
        // the original XOR differences. Popcount it and accumulate.
        total = _mm256_add_epi64(
            total,
            _mm256_sad_epu8(popcount_mm256(carry2), _mm256_setzero_si256()),
        );
    }

    // Apply weights: total currently holds sum(popcount(carry2_i)) unweighted.
    // carry2 bits each represent 4 original bits, so multiply by 4.
    total = _mm256_slli_epi64(total, 2); // total *= 4

    // twos: each set bit represents 2 original bits.
    total = _mm256_add_epi64(
        total,
        _mm256_slli_epi64(
            _mm256_sad_epu8(popcount_mm256(twos), _mm256_setzero_si256()),
            1, // weight = 2
        ),
    );

    // ones: each set bit represents 1 original bit.
    total = _mm256_add_epi64(
        total,
        _mm256_sad_epu8(popcount_mm256(ones), _mm256_setzero_si256()),
    );

    // Horizontal sum of 4 x u64 lanes -> single u32
    let lo = _mm256_castsi256_si128(total);           // lower 128 bits
    let hi = _mm256_extracti128_si256(total, 1);      // upper 128 bits
    let sum128 = _mm_add_epi64(lo, hi);               // 2 x u64
    let upper = _mm_srli_si128(sum128, 8);             // shift high u64 down
    let final_sum = _mm_add_epi64(sum128, upper);      // 1 x u64 (low lane)
    _mm_cvtsi128_si64(final_sum) as u32                // extract as i64, cast to u32
}
```

**Why `_mm_cvtsi128_si64` and not `_mm256_extract_epi64`**: `_mm_cvtsi128_si64`
extracts the low 64 bits of a 128-bit register with a single `movq` instruction.
Using `_mm256_extract_epi64` on a 256-bit register would require an additional
`vextracti128` first. Since we have already reduced to a 128-bit register via
`_mm256_castsi256_si128` + `_mm256_extracti128_si256` + `_mm_add_epi64`, the
`_mm_cvtsi128_si64` path is the natural final step.

### 3.3 AVX-512 VPOPCNTDQ

Available since Ice Lake (2019) / Zen 4 (2022). Hardware SIMD popcount
eliminates the entire vpshufb + Harley-Seal machinery.

```rust
/// Hamming distance using AVX-512 VPOPCNTDQ.
/// 160 u64s / 8 per 512-bit register = 20 iterations. Each iteration:
/// load, load, XOR, VPOPCNTDQ, add. 5 instructions total.
///
/// Performance target: ~7ns at D=10,240 on Ice Lake 3.5 GHz.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512vpopcntdq")]
pub unsafe fn hamming_avx512(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let a_ptr = a.as_ptr() as *const __m512i;
    let b_ptr = b.as_ptr() as *const __m512i;

    let mut acc = _mm512_setzero_si512();

    for i in 0..20 {
        let va = _mm512_loadu_si512(a_ptr.add(i));
        let vb = _mm512_loadu_si512(b_ptr.add(i));
        let xored = _mm512_xor_si512(va, vb);
        let popcnt = _mm512_popcnt_epi64(xored);
        acc = _mm512_add_epi64(acc, popcnt);
    }

    _mm512_reduce_add_epi64(acc) as u32
}
```

### 3.4 ARM NEON

NEON is baseline on aarch64 -- always available, no runtime detection needed.
`vcntq_u8` gives per-byte popcount on a 128-bit register.

```rust
#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

/// Hamming distance using ARM NEON.
/// 160 u64s = 1280 bytes. Each iteration processes 128 bits (16 bytes).
/// 1280 / 16 = 80 iterations.
///
/// vcntq_u8 returns per-byte popcount (max 8 per byte). Over 31 iterations,
/// each byte accumulates at most 31 * 8 = 248, which fits in u8 (max 255).
/// After each block of 31, widen u8 -> u16 -> u32 -> u64 and flush.
///
/// Performance target: ~28ns at D=10,240 on Apple M2 3.5 GHz.
#[cfg(target_arch = "aarch64")]
pub unsafe fn hamming_neon(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let a_ptr = a.as_ptr() as *const u8;
    let b_ptr = b.as_ptr() as *const u8;

    let mut total: u64 = 0;
    let mut i = 0usize;

    while i < 80 {
        let block_end = std::cmp::min(i + 31, 80);
        let mut acc8 = vdupq_n_u8(0);

        for j in i..block_end {
            let offset = j * 16;
            let va = vld1q_u8(a_ptr.add(offset));
            let vb = vld1q_u8(b_ptr.add(offset));
            let xored = veorq_u8(va, vb);
            let popcnt = vcntq_u8(xored);
            acc8 = vaddq_u8(acc8, popcnt);
        }

        // Widen: u8 -> u16 -> u32 -> u64
        let acc16 = vpaddlq_u8(acc8);
        let acc32 = vpaddlq_u16(acc16);
        let acc64 = vpaddlq_u32(acc32);
        total += vgetq_lane_u64(acc64, 0) + vgetq_lane_u64(acc64, 1);

        i = block_end;
    }

    total as u32
}
```

### 3.5 Runtime Dispatch

```rust
/// Compute Hamming distance, dispatching to the fastest available SIMD path.
///
/// All paths produce identical, bit-exact results (pure integer arithmetic:
/// XOR + popcount + add). Safe for consensus: two validators with different
/// hardware compute the same distance for the same inputs.
///
/// Dispatch order (x86_64): AVX-512 VPOPCNTDQ > AVX2 > scalar.
/// Dispatch order (aarch64): NEON (always available on aarch64).
/// Fallback: scalar.
///
/// The `is_x86_feature_detected!` macro does a one-time CPUID check, cached
/// after first call. No per-call overhead.
pub fn hamming_distance(a: &HdcVector, b: &HdcVector) -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512vpopcntdq") {
            return unsafe { hamming_avx512(&a.0, &b.0) };
        }
        if is_x86_feature_detected!("avx2") {
            return unsafe { hamming_avx2(&a.0, &b.0) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        return unsafe { hamming_neon(&a.0, &b.0) };
    }

    hamming_scalar(&a.0, &b.0)
}
```

### 3.6 Performance Targets

| SIMD path | Target latency (D=10,240) | Hardware |
|---|---|---|
| Scalar | ~120 ns | Intel Skylake 4 GHz |
| AVX2 Harley-Seal | ~40 ns | Intel Skylake 4 GHz |
| AVX-512 VPOPCNTDQ | ~7 ns | Intel Ice Lake 3.5 GHz |
| NEON | ~28 ns | Apple M2 3.5 GHz |

---

## 4. `search/brute.rs` -- BruteForceIndex

Linear scan over all vectors. Optimal for N < ~100K.

```rust
use std::collections::BinaryHeap;
use crate::HdcVector;
use crate::H256;
use super::{SearchIndex, SearchResult, HdcIndexError, simd::hamming_distance};

pub struct BruteForceIndex {
    /// Keys in insertion order. keys[i] corresponds to vectors[i].
    keys: Vec<H256>,
    /// Contiguous vector storage for cache locality.
    /// Sequential scan of contiguous memory is ~10x faster than random
    /// HashMap lookups because the hardware prefetcher detects the
    /// linear access pattern.
    vectors: Vec<HdcVector>,
}

impl BruteForceIndex {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            vectors: Vec::new(),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            keys: Vec::with_capacity(cap),
            vectors: Vec::with_capacity(cap),
        }
    }
}

impl SearchIndex for BruteForceIndex {
    fn insert(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
        // Linear scan for duplicate. For brute-force at <100K this is acceptable.
        // If profiling shows this is a bottleneck, add a BTreeSet<H256> for O(log n) lookup.
        if self.keys.contains(&key) {
            return Err(HdcIndexError::DuplicateKey(key));
        }
        self.keys.push(key);
        self.vectors.push(vector);
        Ok(())
    }

    fn delete(&mut self, key: &H256) -> bool {
        if let Some(pos) = self.keys.iter().position(|k| k == key) {
            self.keys.swap_remove(pos);
            self.vectors.swap_remove(pos);
            true
        } else {
            false
        }
    }

    fn search(&self, query: &HdcVector, top_k: usize) -> Result<SearchResult, HdcIndexError> {
        if self.vectors.is_empty() {
            return Err(HdcIndexError::EmptyIndex);
        }

        let effective_k = top_k.min(self.vectors.len());

        // Max-heap of (distance, insertion_index).
        // BinaryHeap is a max-heap by default. When heap.len() > effective_k,
        // pop() removes the element with the LARGEST (dist, idx) -- the worst
        // match -- keeping the K best.
        //
        // Tie-breaking: when two candidates have equal distance, the one with
        // the higher index is evicted first (lexicographic comparison on
        // (dist, idx)), deterministically preferring the lower-indexed vector.
        let mut heap: BinaryHeap<(u32, usize)> = BinaryHeap::with_capacity(effective_k + 1);

        for (i, vector) in self.vectors.iter().enumerate() {
            let dist = hamming_distance(query, vector);
            heap.push((dist, i));
            if heap.len() > effective_k {
                heap.pop(); // evict worst (largest distance, then largest index)
            }
        }

        // Extract and sort by (distance ASC, key ASC) for deterministic output.
        let mut results: Vec<(H256, u32)> = heap
            .into_vec()
            .into_iter()
            .map(|(dist, i)| (self.keys[i], dist))
            .collect();
        results.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        Ok(results)
    }

    fn len(&self) -> usize {
        self.keys.len()
    }
}
```

**Why the heap works this way**: `BinaryHeap<(u32, usize)>` is a max-heap.
Pushing `(dist, idx)` and popping when size exceeds K removes the entry with the
largest `(dist, idx)` -- the worst match. After the scan, the heap contains the K
smallest distances. This is O(N log K) total.

---

## 5. `search/hnsw.rs` -- HNSW Index

Reference: Malkov & Yashunin (2020), "Efficient and Robust Approximate Nearest
Neighbor Using Hierarchical Navigable Small World Graphs."

### 5.1 Constants

```rust
/// Max connections per node per layer (layers >= 1).
const M: usize = 16;

/// Max connections at level 0 (denser for better recall at the base layer).
const M_MAX0: usize = 2 * M; // 32

/// Beam width during index construction.
const EF_CONSTRUCTION: usize = 200;

/// Default beam width during search. Tunable at query time.
const EF_SEARCH_DEFAULT: usize = 100;

/// Maximum HNSW level. With M=16, P(level >= 5) = 16^{-5} ~ 10^{-6}.
/// This caps the structure even for extreme hash values.
const MAX_LEVEL: usize = 16;

/// Tombstone compaction threshold: rebuild when >20% of nodes are deleted.
const COMPACTION_THRESHOLD_PERCENT: usize = 20;
```

### 5.2 Types

```rust
use std::collections::{BinaryHeap, BTreeMap};
use std::cmp::Ordering;
use crate::{HdcVector, H256};
use super::{SearchIndex, SearchResult, HdcIndexError, simd::hamming_distance};

/// Composite comparison key for deterministic tie-breaking.
/// Lower distance wins. On tie, lower element_id wins.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CandidateKey {
    distance: u32,
    element_id: u64,
}

impl Ord for CandidateKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance.cmp(&other.distance)
            .then_with(|| self.element_id.cmp(&other.element_id))
    }
}

impl PartialOrd for CandidateKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A node in the HNSW graph.
struct HnswNode {
    key: H256,
    vector: HdcVector,
    /// Internal numeric ID, assigned sequentially at insertion time.
    element_id: u64,
    /// The level this node was assigned to (present on levels 0..=level).
    level: usize,
    /// Adjacency lists per level. BTreeMap (NOT HashMap) for deterministic
    /// iteration order. Key = level, Value = sorted neighbor list.
    /// Each neighbor is stored as (distance_to_neighbor, neighbor_element_id).
    neighbors: Vec<BTreeMap<u64, u32>>,
    /// Tombstone flag. Deleted nodes stay in the graph but are excluded
    /// from search results. Their edges remain as bridges.
    deleted: bool,
}

pub struct HnswIndex {
    /// All nodes, keyed by element_id. BTreeMap for deterministic iteration.
    nodes: BTreeMap<u64, HnswNode>,
    /// Map from H256 key to element_id for O(log n) key lookup.
    key_to_id: BTreeMap<H256, u64>,
    /// Entry point: the element_id of the node at the highest level.
    entry_point: Option<u64>,
    /// Current maximum level in the graph.
    max_level: usize,
    /// Next element_id to assign (monotonically increasing).
    next_id: u64,
    /// Number of tombstoned nodes.
    tombstone_count: usize,
    /// Insertion order record for deterministic compaction rebuild.
    /// Maps element_id -> insertion sequence number.
    insertion_order: BTreeMap<u64, u64>,
    /// Search beam width (tunable at query time).
    ef_search: usize,
}
```

### 5.3 Level Assignment (Deterministic, No f64)

```rust
/// Deterministic level assignment using keccak256 of the vector bytes.
///
/// CONSENSUS SAFETY: This uses integer-only arithmetic. No f64::ln().
///
/// The geometric distribution P(level >= k) = (1/M)^k = (1/16)^k = 2^{-4k}
/// is computed as: level = leading_zeros(hash_u64) / 4.
///
/// Since M=16 = 2^4, each "level" consumes 4 bits of randomness.
/// leading_zeros(u64) ranges from 0..=64, so max level = 64/4 = 16.
///
/// The keccak256 hash ensures the same vector always gets the same level
/// regardless of insertion order or which validator builds the index.
fn deterministic_level(vector: &HdcVector) -> usize {
    // Hash the raw vector bytes
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            vector.0.as_ptr() as *const u8,
            160 * 8, // 1280 bytes
        )
    };
    let hash = keccak256(bytes); // use tiny-keccak or sha3 crate
    let random_bits = u64::from_le_bytes(hash[0..8].try_into().unwrap());

    // Integer-only geometric distribution for M=16:
    // level = leading_zeros(random_bits) / 4
    let level = (random_bits.leading_zeros() / 4) as usize;
    level.min(MAX_LEVEL)
}
```

### 5.4 Core Methods

```rust
impl HnswIndex {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            key_to_id: BTreeMap::new(),
            entry_point: None,
            max_level: 0,
            next_id: 0,
            tombstone_count: 0,
            insertion_order: BTreeMap::new(),
            ef_search: EF_SEARCH_DEFAULT,
        }
    }

    /// Set the search beam width. Higher values increase recall at the
    /// cost of more distance computations. Range: 50..=200.
    pub fn set_ef_search(&mut self, ef: usize) {
        self.ef_search = ef.clamp(50, 200);
    }

    /// Greedy search at a single layer. Returns the element_id of the
    /// nearest node to `query` reachable from `entry_id` at `level`,
    /// following only edges at that level.
    fn greedy_search_layer(
        &self,
        query: &HdcVector,
        entry_id: u64,
        level: usize,
    ) -> u64 {
        let mut current = entry_id;
        let mut current_dist = hamming_distance(
            query,
            &self.nodes[&current].vector,
        );

        loop {
            let mut changed = false;
            let node = &self.nodes[&current];

            if let Some(neighbors) = node.neighbors.get(level) {
                for (&neighbor_id, _) in neighbors {
                    let neighbor = &self.nodes[&neighbor_id];
                    if neighbor.deleted { continue; }
                    let d = hamming_distance(query, &neighbor.vector);
                    if d < current_dist || (d == current_dist && neighbor_id < current) {
                        current = neighbor_id;
                        current_dist = d;
                        changed = true;
                    }
                }
            }

            if !changed { break; }
        }
        current
    }

    /// Beam search at a single layer. Returns up to `ef` nearest candidates.
    /// Uses a min-heap (candidates to explore) and a max-heap (result bound).
    ///
    /// All comparisons use CandidateKey = (distance, element_id) for
    /// deterministic tie-breaking.
    fn search_layer(
        &self,
        query: &HdcVector,
        entry_ids: &[u64],
        ef: usize,
        level: usize,
    ) -> Vec<CandidateKey> {
        use std::cmp::Reverse;
        use std::collections::HashSet;

        // visited set
        let mut visited: HashSet<u64> = HashSet::new();

        // candidates: min-heap (closest first) -- nodes to explore
        let mut candidates: BinaryHeap<Reverse<CandidateKey>> = BinaryHeap::new();

        // result: max-heap (farthest first) -- best ef nodes seen so far
        let mut result: BinaryHeap<CandidateKey> = BinaryHeap::new();

        for &eid in entry_ids {
            if visited.insert(eid) {
                let d = hamming_distance(query, &self.nodes[&eid].vector);
                let key = CandidateKey { distance: d, element_id: eid };
                candidates.push(Reverse(key));
                result.push(key);
            }
        }

        while let Some(Reverse(candidate)) = candidates.pop() {
            // If the closest unexplored candidate is farther than the
            // farthest result, we cannot improve -- stop.
            let farthest = result.peek().unwrap();
            if candidate.distance > farthest.distance
                || (candidate.distance == farthest.distance
                    && candidate.element_id > farthest.element_id)
            {
                break;
            }

            let node = &self.nodes[&candidate.element_id];
            if let Some(neighbors) = node.neighbors.get(level) {
                for (&neighbor_id, _) in neighbors {
                    if !visited.insert(neighbor_id) { continue; }
                    let neighbor = &self.nodes[&neighbor_id];
                    let d = hamming_distance(query, &neighbor.vector);
                    let key = CandidateKey { distance: d, element_id: neighbor_id };

                    let farthest = result.peek().unwrap();
                    if result.len() < ef
                        || key.distance < farthest.distance
                        || (key.distance == farthest.distance
                            && key.element_id < farthest.element_id)
                    {
                        candidates.push(Reverse(key));
                        result.push(key);
                        if result.len() > ef {
                            result.pop(); // evict farthest
                        }
                    }
                }
            }
        }

        result.into_vec()
    }

    /// Select the best `m` neighbors from candidates for a node.
    /// Simple selection: sort by CandidateKey, take first `m`.
    fn select_neighbors(
        candidates: &mut Vec<CandidateKey>,
        m: usize,
    ) -> Vec<CandidateKey> {
        candidates.sort(); // sort by (distance ASC, element_id ASC)
        candidates.truncate(m);
        candidates.clone()
    }

    /// Add a bidirectional edge between two nodes at `level`.
    /// If either node exceeds its max connections, prune the farthest neighbor.
    fn connect(
        &mut self,
        id_a: u64,
        id_b: u64,
        dist: u32,
        level: usize,
    ) {
        let max_conn = if level == 0 { M_MAX0 } else { M };

        // a -> b
        self.nodes.get_mut(&id_a).unwrap()
            .neighbors[level].insert(id_b, dist);
        self.prune_connections(id_a, level, max_conn);

        // b -> a
        self.nodes.get_mut(&id_b).unwrap()
            .neighbors[level].insert(id_a, dist);
        self.prune_connections(id_b, level, max_conn);
    }

    /// If node has more than `max_conn` neighbors at `level`, remove the
    /// farthest (by CandidateKey) until it fits.
    fn prune_connections(&mut self, node_id: u64, level: usize, max_conn: usize) {
        let node = self.nodes.get_mut(&node_id).unwrap();
        let neighbors = &mut node.neighbors[level];

        if neighbors.len() <= max_conn { return; }

        // Collect into CandidateKey, sort, keep best max_conn
        let mut entries: Vec<CandidateKey> = neighbors.iter()
            .map(|(&eid, &dist)| CandidateKey { distance: dist, element_id: eid })
            .collect();
        entries.sort(); // (distance ASC, element_id ASC)
        entries.truncate(max_conn);

        // Rebuild the BTreeMap with only the kept neighbors
        let new_neighbors: BTreeMap<u64, u32> = entries.iter()
            .map(|ck| (ck.element_id, ck.distance))
            .collect();
        *neighbors = new_neighbors;
    }

    /// Mark a node as deleted (tombstone). O(1).
    /// The node stays in the graph to preserve connectivity.
    /// Trigger compaction when tombstone ratio exceeds 20%.
    fn mark_deleted(&mut self, element_id: u64) {
        if let Some(node) = self.nodes.get_mut(&element_id) {
            if !node.deleted {
                node.deleted = true;
                self.tombstone_count += 1;
            }
        }

        let total = self.nodes.len();
        if total > 0 && self.tombstone_count * 100 / total > COMPACTION_THRESHOLD_PERCENT {
            self.compact();
        }
    }

    /// Rebuild the index from scratch, excluding tombstoned nodes.
    /// Inserts in canonical order (by insertion sequence number) to
    /// ensure all validators produce identical post-compaction graphs.
    fn compact(&mut self) {
        // Collect live nodes with their insertion order
        let mut live: Vec<(u64, H256, HdcVector)> = Vec::new(); // (insertion_seq, key, vector)
        for (&eid, node) in &self.nodes {
            if !node.deleted {
                let seq = self.insertion_order[&eid];
                live.push((seq, node.key, node.vector.clone()));
            }
        }
        // Sort by insertion sequence for deterministic rebuild
        live.sort_by_key(|(seq, _, _)| *seq);

        // Reset and re-insert
        let ef = self.ef_search;
        *self = HnswIndex::new();
        self.ef_search = ef;
        for (_, key, vector) in live {
            // insert() handles next_id, insertion_order, etc.
            let _ = self.insert_impl(key, vector);
        }
    }

    /// Internal insertion (called by both SearchIndex::insert and compact).
    fn insert_impl(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
        let level = deterministic_level(&vector);
        let element_id = self.next_id;
        self.next_id += 1;

        let seq = element_id; // insertion sequence = element_id (monotonic)
        self.insertion_order.insert(element_id, seq);

        // Create the node with empty neighbor lists for each level
        let mut neighbors = Vec::with_capacity(level + 1);
        for _ in 0..=level {
            neighbors.push(BTreeMap::new());
        }

        let node = HnswNode {
            key,
            vector: vector.clone(),
            element_id,
            level,
            neighbors,
            deleted: false,
        };
        self.nodes.insert(element_id, node);
        self.key_to_id.insert(key, element_id);

        if self.entry_point.is_none() {
            // First node: just set as entry point
            self.entry_point = Some(element_id);
            self.max_level = level;
            return Ok(());
        }

        let entry = self.entry_point.unwrap();

        // Phase 1: Greedy descent from top level down to (level + 1)
        let mut current_entry = entry;
        for lev in (level + 1..=self.max_level).rev() {
            current_entry = self.greedy_search_layer(&vector, current_entry, lev);
        }

        // Phase 2: Insert at each layer from min(level, max_level) down to 0
        let insert_top = level.min(self.max_level);
        let mut entry_ids = vec![current_entry];

        for lev in (0..=insert_top).rev() {
            let mut candidates = self.search_layer(
                &vector, &entry_ids, EF_CONSTRUCTION, lev,
            );

            let max_conn = if lev == 0 { M_MAX0 } else { M };
            let selected = Self::select_neighbors(&mut candidates, max_conn);

            for ck in &selected {
                let dist = hamming_distance(&vector, &self.nodes[&ck.element_id].vector);
                self.connect(element_id, ck.element_id, dist, lev);
            }

            // Use selected neighbors as entry points for the next level down
            entry_ids = selected.iter().map(|ck| ck.element_id).collect();
        }

        // Update entry point if new node is at a higher level
        if level > self.max_level {
            self.entry_point = Some(element_id);
            self.max_level = level;
        }

        Ok(())
    }
}
```

### 5.5 SearchIndex Trait Implementation

```rust
impl SearchIndex for HnswIndex {
    fn insert(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
        if self.key_to_id.contains_key(&key) {
            return Err(HdcIndexError::DuplicateKey(key));
        }
        self.insert_impl(key, vector)
    }

    fn delete(&mut self, key: &H256) -> bool {
        if let Some(&eid) = self.key_to_id.get(key) {
            self.mark_deleted(eid);
            self.key_to_id.remove(key);
            true
        } else {
            false
        }
    }

    fn search(&self, query: &HdcVector, top_k: usize) -> Result<SearchResult, HdcIndexError> {
        let live_count = self.nodes.len() - self.tombstone_count;
        if live_count == 0 {
            return Err(HdcIndexError::EmptyIndex);
        }

        let entry = self.entry_point.ok_or(HdcIndexError::EmptyIndex)?;
        let effective_k = top_k.min(live_count);

        // Phase 1: Greedy descent from top level to level 1
        let mut current = entry;
        for lev in (1..=self.max_level).rev() {
            current = self.greedy_search_layer(query, current, lev);
        }

        // Phase 2: Beam search at level 0 with ef_search width
        let candidates = self.search_layer(
            query,
            &[current],
            self.ef_search.max(effective_k),
            0,
        );

        // Filter out tombstoned nodes and take top_k
        let mut results: Vec<(H256, u32)> = candidates.into_iter()
            .filter(|ck| {
                self.nodes.get(&ck.element_id)
                    .map(|n| !n.deleted)
                    .unwrap_or(false)
            })
            .map(|ck| {
                let node = &self.nodes[&ck.element_id];
                (node.key, ck.distance)
            })
            .collect();

        results.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        results.truncate(effective_k);
        Ok(results)
    }

    fn len(&self) -> usize {
        self.nodes.len() - self.tombstone_count
    }
}
```

### 5.6 Five Determinism Requirements (Consensus Safety Checklist)

Every one of these must hold for two validators processing the same events to
produce byte-identical HNSW graphs:

| # | Requirement | How We Satisfy It |
|---|---|---|
| 1 | **Deterministic level assignment** | `deterministic_level()` uses `keccak256(vector_bytes).leading_zeros() / 4`. Integer-only. No f64. No PRNG state. Same vector always gets the same level. |
| 2 | **Canonical insertion order** | Vectors are inserted in the order their `VectorStored` events appear in finalized blocks. All validators see the same finalized block sequence. |
| 3 | **Deterministic tie-breaking** | All comparisons use `CandidateKey { distance, element_id }` with `Ord` derived as (distance ASC, element_id ASC). No ambiguity when distances are equal. |
| 4 | **Single-threaded construction** | `insert()` is `&mut self` (exclusive borrow). No parallel mutation. Search (`&self`) can be parallelized since it is read-only. |
| 5 | **Deterministic neighbor selection** | `select_neighbors()` sorts by `CandidateKey` then truncates. When candidates tie on distance, the one with the lower element_id is kept. `BTreeMap` for adjacency lists ensures deterministic iteration. |

---

## 6. `search/local.rs` -- LocalIndex (Auto-Switching)

```rust
use crate::{HdcVector, H256};
use super::{SearchIndex, SearchResult, HdcIndexError};
use super::brute::BruteForceIndex;
use super::hnsw::HnswIndex;

/// Threshold at which to switch from brute-force to HNSW.
const SWITCH_THRESHOLD: usize = 10_000;

/// Auto-switching index. Starts as brute-force, converts to HNSW when
/// vector count exceeds SWITCH_THRESHOLD.
///
/// Once switched to HNSW, never switches back (prevents oscillation at
/// the boundary). If the count drops below the threshold due to deletions,
/// the index remains HNSW.
pub enum LocalIndex {
    BruteForce(BruteForceIndex),
    Hnsw(HnswIndex),
}

impl LocalIndex {
    pub fn new() -> Self {
        LocalIndex::BruteForce(BruteForceIndex::new())
    }

    /// Upgrade from brute-force to HNSW. Drains all vectors from the
    /// brute-force index and re-inserts them into a new HNSW index
    /// in their original insertion order (canonical for determinism).
    fn upgrade(&mut self) {
        if let LocalIndex::BruteForce(bf) = self {
            let mut hnsw = HnswIndex::new();

            // Drain brute-force in insertion order (Vec preserves order)
            let keys: Vec<H256> = bf.keys.drain(..).collect();
            let vectors: Vec<HdcVector> = bf.vectors.drain(..).collect();

            for (key, vector) in keys.into_iter().zip(vectors.into_iter()) {
                let _ = hnsw.insert(key, vector);
            }

            *self = LocalIndex::Hnsw(hnsw);
        }
    }
}

impl SearchIndex for LocalIndex {
    fn insert(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
        match self {
            LocalIndex::BruteForce(bf) => {
                bf.insert(key, vector)?;
                if bf.len() > SWITCH_THRESHOLD {
                    self.upgrade();
                }
                Ok(())
            }
            LocalIndex::Hnsw(hnsw) => hnsw.insert(key, vector),
        }
    }

    fn delete(&mut self, key: &H256) -> bool {
        match self {
            LocalIndex::BruteForce(bf) => bf.delete(key),
            LocalIndex::Hnsw(hnsw) => hnsw.delete(key),
        }
    }

    fn search(&self, query: &HdcVector, top_k: usize) -> Result<SearchResult, HdcIndexError> {
        match self {
            LocalIndex::BruteForce(bf) => bf.search(query, top_k),
            LocalIndex::Hnsw(hnsw) => hnsw.search(query, top_k),
        }
    }

    fn len(&self) -> usize {
        match self {
            LocalIndex::BruteForce(bf) => bf.len(),
            LocalIndex::Hnsw(hnsw) => hnsw.len(),
        }
    }
}
```

**Note on `bf.keys` and `bf.vectors` access in `upgrade()`**: The `upgrade()`
method reaches into `BruteForceIndex` internals. Either make `keys` and `vectors`
`pub(super)`, or add a `drain` method on `BruteForceIndex`:

```rust
impl BruteForceIndex {
    /// Drain all entries. Returns (keys, vectors) in insertion order.
    pub(super) fn drain(&mut self) -> (Vec<H256>, Vec<HdcVector>) {
        let keys = std::mem::take(&mut self.keys);
        let vectors = std::mem::take(&mut self.vectors);
        (keys, vectors)
    }
}
```

---

## 7. `search/tiered.rs` -- Tiered Search Pipeline

For on-chain gas optimization. Three tiers of progressively more expensive
filtering, each rejecting ~90% of remaining candidates.

```rust
use crate::{HdcVector, H256};
use super::simd::hamming_distance;

/// Evenly-spaced sample indices for Tier 2.
/// 16 of 160 words (10%), spaced 10 apart.
const SAMPLE_INDICES: [usize; 16] = [
    0, 10, 20, 30, 40, 50, 60, 70,
    80, 90, 100, 110, 120, 130, 140, 150,
];

/// Structure-of-arrays layout for tiered access patterns.
/// Each tier touches only the data it needs, minimizing cache pollution.
pub struct TieredSearchPipeline {
    keys: Vec<H256>,

    /// Tier 1: first u64 word of each vector. N * 8 bytes.
    /// For N=100K this is 800 KB -- fits in L2 cache.
    first_words: Vec<u64>,

    /// Tier 2: 16 evenly-spaced words from each vector. N * 128 bytes.
    /// For N=100K this is 12.8 MB -- fits in L3 cache.
    sample_words: Vec<[u64; 16]>,

    /// Tier 3: full vectors. N * 1280 bytes.
    /// Only accessed for the ~1% that survives Tiers 1 and 2.
    full_vectors: Vec<HdcVector>,
}

impl TieredSearchPipeline {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            first_words: Vec::new(),
            sample_words: Vec::new(),
            full_vectors: Vec::new(),
        }
    }

    /// Insert a vector. Populates all three tier data structures.
    pub fn insert(&mut self, key: H256, vector: HdcVector) {
        self.keys.push(key);
        self.first_words.push(vector.0[0]);

        let mut samples = [0u64; 16];
        for (s, &idx) in SAMPLE_INDICES.iter().enumerate() {
            samples[s] = vector.0[idx];
        }
        self.sample_words.push(samples);
        self.full_vectors.push(vector);
    }

    /// Tier 1: First-word Hamming distance filter.
    /// Compares only the first u64 word (64 bits out of 10,240).
    /// Cost: ~100 gas (1 XOR + 1 POPCNT + 1 compare).
    ///
    /// Rejection logic: if the first-word distance exceeds the threshold,
    /// the full-vector distance is very likely above the search distance
    /// threshold too (concentration of measure on i.i.d. bits).
    #[inline]
    fn tier1_passes(&self, index: usize, query_first_word: u64, threshold: u32) -> bool {
        let dist = (self.first_words[index] ^ query_first_word).count_ones();
        dist <= threshold
    }

    /// Tier 2: Approximate Hamming distance using 16 sampled words.
    /// Compares 10% of the vector (1,024 bits out of 10,240).
    /// Cost: ~500 gas (16 XOR + 16 POPCNT + multiply + compare).
    ///
    /// Returns estimated full-vector distance (sample distance * 10).
    #[inline]
    fn tier2_distance(&self, index: usize, query_samples: &[u64; 16]) -> u32 {
        let mut dist = 0u32;
        let stored = &self.sample_words[index];
        for i in 0..16 {
            dist += (stored[i] ^ query_samples[i]).count_ones();
        }
        dist * 10 // scale to estimate full distance
    }

    /// Tier 3: Exact full-vector Hamming distance.
    /// Compares all 160 words (10,240 bits).
    /// Cost: ~5,000 gas (EVM bytecode) or ~1,500 gas (HDC precompile).
    #[inline]
    fn tier3_distance(&self, index: usize, query: &HdcVector) -> u32 {
        hamming_distance(&self.full_vectors[index], query)
    }

    /// Run the full tiered pipeline. Returns top_k results sorted by
    /// (distance ASC, key ASC).
    ///
    /// tier1_threshold: max first-word distance to pass Tier 1.
    ///   Calibrate based on target search distance. Example: if searching
    ///   for vectors within Hamming distance 2000 of query (bit-error rate
    ///   ~0.195), set tier1_threshold = ceil(64 * 0.195 * 1.5) = 19.
    ///   The 1.5x multiplier provides margin against false negatives.
    ///
    /// tier2_threshold: max estimated full distance to pass Tier 2.
    ///   Example: if searching within Hamming distance 2000, set
    ///   tier2_threshold = 2000 * 1.3 = 2600 (30% margin on the
    ///   +/-12.8% estimation error from 10% sampling).
    pub fn search(
        &self,
        query: &HdcVector,
        top_k: usize,
        tier1_threshold: u32,
        tier2_threshold: u32,
    ) -> Vec<(H256, u32)> {
        let query_first_word = query.0[0];

        // Extract query sample words for Tier 2
        let mut query_samples = [0u64; 16];
        for (s, &idx) in SAMPLE_INDICES.iter().enumerate() {
            query_samples[s] = query.0[idx];
        }

        // Max-heap for top-k tracking
        use std::collections::BinaryHeap;
        let mut heap: BinaryHeap<(u32, usize)> = BinaryHeap::with_capacity(top_k + 1);

        for i in 0..self.keys.len() {
            // Tier 1: ~100 gas per candidate
            if !self.tier1_passes(i, query_first_word, tier1_threshold) {
                continue; // ~90% rejected here
            }

            // Tier 2: ~500 gas per candidate
            let approx_dist = self.tier2_distance(i, &query_samples);
            if approx_dist > tier2_threshold {
                continue; // ~90% of remainder rejected here
            }

            // Tier 3: ~5,000 gas per candidate (only ~1% reach here)
            let exact_dist = self.tier3_distance(i, query);

            heap.push((exact_dist, i));
            if heap.len() > top_k {
                heap.pop();
            }
        }

        let mut results: Vec<(H256, u32)> = heap
            .into_vec()
            .into_iter()
            .map(|(dist, i)| (self.keys[i], dist))
            .collect();
        results.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        results
    }
}
```

### Gas Breakdown (100K vectors)

| Tier | Candidates | Gas/candidate | Total gas | Survivors |
|---|---|---|---|---|
| 1 (first word) | 100,000 | ~100 | 10M | ~10,000 (90% rejected) |
| 2 (10% sample) | 10,000 | ~500 | 5M | ~1,000 (90% rejected) |
| 3 (full Hamming) | 1,000 | ~5,000 | 5M | top-K |
| Overhead | -- | -- | ~1M | -- |
| **Total** | | | **~21M** | |

Compared to 500M gas for brute-force full Hamming on all 100K vectors, the
tiered pipeline saves ~96%.

---

## 8. Anti-Patterns

Do NOT do any of the following. Each is a specific mistake that will cause
non-determinism, incorrect results, or performance regressions.

### 8.1 Do NOT use f64 for level assignment

```rust
// WRONG -- f64::ln() is a transcendental function not covered by IEEE 754
// bit-exactness guarantees. Different libm implementations produce different
// results for the same input. Two validators may assign different levels.
let level = (-f64::ln(uniform_random) * (1.0 / f64::ln(M as f64))) as usize;
```

Use the integer-only `leading_zeros()` method shown in section 5.3.

### 8.2 Do NOT use `Reverse<>` wrapper for the top-K heap

```rust
// WRONG -- BinaryHeap<Reverse<(u32, usize)>> is a MIN-heap.
// pop() removes the SMALLEST distance -- i.e., the BEST match.
// You end up keeping the K WORST results.
let mut heap: BinaryHeap<Reverse<(u32, usize)>> = BinaryHeap::new();
```

Use a bare `BinaryHeap<(u32, usize)>` (max-heap). `pop()` removes the largest
`(dist, idx)` -- the worst match -- keeping the K best.

### 8.3 Do NOT use HashMap for HNSW node storage or adjacency lists

```rust
// WRONG -- HashMap iteration order is non-deterministic (depends on hasher
// seed, insertion order, and resize timing). Two validators iterating the
// same HashMap may process neighbors in different orders, producing different
// search results and different graphs.
let nodes: HashMap<u64, HnswNode> = HashMap::new();
```

Use `BTreeMap<u64, HnswNode>`. BTreeMap iterates in key order (ascending),
which is deterministic regardless of insertion order.

### 8.4 Do NOT use unstable sort

```rust
// WRONG -- sort_unstable does not guarantee a deterministic order for
// elements that compare equal. Two runs may produce different orderings
// of equal-distance candidates.
results.sort_unstable_by_key(|r| r.distance);
```

Use `sort_by()` with the full composite key `(distance, key)`. Stable sort
preserves the relative order of equal elements, but relying on insertion order
for tie-breaking is fragile. Always use an explicit composite key.

### 8.5 Do NOT skip the NEON u8 overflow guard

```rust
// WRONG -- vcntq_u8 produces per-byte counts (max 8 per byte).
// Over 80 iterations, each u8 lane accumulates up to 80 * 8 = 640,
// which overflows u8 (max 255). The distance will be silently wrong.
let mut acc8 = vdupq_n_u8(0);
for j in 0..80 {
    acc8 = vaddq_u8(acc8, vcntq_u8(veorq_u8(va, vb)));
}
```

Process in blocks of at most 31 iterations (31 * 8 = 248, fits in u8),
then widen to u64 before continuing. See section 3.4.

### 8.6 Do NOT use `_mm256_extract_epi64` for final reduction

```rust
// WRONG -- _mm256_extract_epi64 requires a compile-time constant index
// and still operates on a 256-bit register. You need two extractions
// and two additions. The correct pattern is to split into two 128-bit
// halves, add, then use _mm_cvtsi128_si64 on the result.
let result = _mm256_extract_epi64(total, 0)
           + _mm256_extract_epi64(total, 1)
           + _mm256_extract_epi64(total, 2)
           + _mm256_extract_epi64(total, 3);
```

Use the `_mm256_castsi256_si128` / `_mm256_extracti128_si256` / `_mm_add_epi64`
/ `_mm_cvtsi128_si64` pattern shown in section 3.2.

---

## 9. Implementation Checklist

Complete these in order. Each step is independently testable.

- [x] **simd.rs**: `hamming_scalar` -- reference implementation
- [x] **simd.rs**: test `hamming_scalar` against known vectors (zero vs zero = 0, complementary = 10240, etc.)
- [x] **simd.rs**: `hamming_avx2` with `popcount_mm256` and `csa_256` (gated on `cfg(target_arch = "x86_64")`)
- [x] **simd.rs**: `hamming_avx512` (gated on `cfg(target_arch = "x86_64")`)
- [x] **simd.rs**: `hamming_neon` (gated on `cfg(target_arch = "aarch64")`)
- [x] **simd.rs**: `hamming_distance` runtime dispatch function
- [x] **simd.rs**: property test -- SIMD vs scalar for 100 random pairs (`hamming_simd_matches_scalar`)
- [x] **mod.rs**: `SearchIndex` trait, `HdcIndexError` enum, re-exports
- [x] **brute.rs**: `BruteForceIndex` struct + `SearchIndex` impl
- [x] **brute.rs**: tests -- insert/search round-trip, duplicate key rejection, delete, empty index error, top_k_ordering, top_k_larger_than_index, deterministic_results
- [x] **hnsw.rs**: `CandidateKey`, `HnswNode`, `HnswIndex` structs
- [x] **hnsw.rs**: `deterministic_level()` using keccak256 + leading_zeros (integer-only, no f64::ln)
- [x] **hnsw.rs**: `greedy_search_layer`, `search_layer`, `select_neighbors`
- [x] **hnsw.rs**: `insert_impl`, `connect`, `prune_connections`
- [x] **hnsw.rs**: `mark_deleted`, `compact`
- [x] **hnsw.rs**: `SearchIndex` impl (insert, delete, search, len)
- [x] **hnsw.rs**: determinism test -- `deterministic_index_builds` verifies identical results
- [x] **local.rs**: `LocalIndex` enum + `SearchIndex` impl + `upgrade()`
- [x] **local.rs**: tests -- basic_insert_and_search, delete_works, search_multiple (upgrade threshold test still needed)
- [x] **tiered.rs**: `TieredSearchPipeline` struct, insert, tier methods
- [x] **tiered.rs**: `search()` with all three tiers
- [x] **tiered.rs**: test -- `tiered_matches_brute_force_with_permissive_thresholds` verifies equivalence
- [ ] **benchmarks**: criterion benchmarks for hamming_scalar, hamming_avx2/avx512/neon, brute-force top-10, hnsw top-10

---

## 10. Test Plan

### 10.1 Correctness Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // --- Hamming distance ---

    #[test]
    fn hamming_zero_vectors() {
        let a = HdcVector::zero();
        let b = HdcVector::zero();
        assert_eq!(hamming_distance(&a, &b), 0);
    }

    #[test]
    fn hamming_complementary_vectors() {
        let a = HdcVector::zero();
        let b = a.complement(); // all 1s
        assert_eq!(hamming_distance(&a, &b), 10_240);
    }

    #[test]
    fn hamming_self_distance() {
        let a = HdcVector::from_seed(42);
        assert_eq!(hamming_distance(&a, &a), 0);
    }

    #[test]
    fn hamming_symmetry() {
        let a = HdcVector::from_seed(1);
        let b = HdcVector::from_seed(2);
        assert_eq!(hamming_distance(&a, &b), hamming_distance(&b, &a));
    }

    #[test]
    fn hamming_simd_matches_scalar() {
        // Property test: generate 1000 random pairs, verify all SIMD paths
        // match scalar.
        for seed in 0..1000u64 {
            let a = HdcVector::from_seed(seed * 2);
            let b = HdcVector::from_seed(seed * 2 + 1);
            let scalar = hamming_scalar(&a.0, &b.0);
            let dispatched = hamming_distance(&a, &b);
            assert_eq!(scalar, dispatched, "mismatch at seed {seed}");
        }
    }

    // --- BruteForceIndex ---

    #[test]
    fn brute_insert_search_roundtrip() {
        let mut index = BruteForceIndex::new();
        let v1 = HdcVector::from_seed(1);
        let k1: H256 = [1u8; 32];
        index.insert(k1, v1.clone()).unwrap();

        let results = index.search(&v1, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, k1);
        assert_eq!(results[0].1, 0); // self-distance = 0
    }

    #[test]
    fn brute_duplicate_key_rejected() {
        let mut index = BruteForceIndex::new();
        let k: H256 = [1u8; 32];
        index.insert(k, HdcVector::from_seed(1)).unwrap();
        assert!(index.insert(k, HdcVector::from_seed(2)).is_err());
    }

    #[test]
    fn brute_empty_index_error() {
        let index = BruteForceIndex::new();
        assert!(index.search(&HdcVector::zero(), 10).is_err());
    }

    #[test]
    fn brute_delete() {
        let mut index = BruteForceIndex::new();
        let k: H256 = [1u8; 32];
        index.insert(k, HdcVector::from_seed(1)).unwrap();
        assert_eq!(index.len(), 1);
        assert!(index.delete(&k));
        assert_eq!(index.len(), 0);
        assert!(!index.delete(&k)); // already deleted
    }

    #[test]
    fn brute_top_k_ordering() {
        // Insert 100 vectors, search for top-5.
        // Verify results are sorted by distance ascending.
        let mut index = BruteForceIndex::new();
        for i in 0..100u64 {
            let mut k = [0u8; 32];
            k[0..8].copy_from_slice(&i.to_le_bytes());
            index.insert(k, HdcVector::from_seed(i)).unwrap();
        }

        let query = HdcVector::from_seed(0);
        let results = index.search(&query, 5).unwrap();
        assert_eq!(results.len(), 5);
        // Verify sorted by distance ascending
        for w in results.windows(2) {
            assert!(w[0].1 <= w[1].1);
        }
        // First result should be the query itself (distance 0)
        assert_eq!(results[0].1, 0);
    }

    // --- HnswIndex ---

    #[test]
    fn hnsw_insert_search_roundtrip() {
        let mut index = HnswIndex::new();
        let v1 = HdcVector::from_seed(1);
        let k1: H256 = [1u8; 32];
        index.insert(k1, v1.clone()).unwrap();

        let results = index.search(&v1, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, k1);
        assert_eq!(results[0].1, 0);
    }

    #[test]
    fn hnsw_determinism() {
        // Build two indexes with the same vectors in the same order.
        // They must produce identical search results.
        let mut index_a = HnswIndex::new();
        let mut index_b = HnswIndex::new();

        for i in 0..500u64 {
            let mut k = [0u8; 32];
            k[0..8].copy_from_slice(&i.to_le_bytes());
            let v = HdcVector::from_seed(i);
            index_a.insert(k, v.clone()).unwrap();
            index_b.insert(k, v).unwrap();
        }

        let query = HdcVector::from_seed(9999);
        let results_a = index_a.search(&query, 10).unwrap();
        let results_b = index_b.search(&query, 10).unwrap();
        assert_eq!(results_a, results_b);
    }

    #[test]
    fn hnsw_deterministic_level() {
        // Same vector always gets the same level.
        let v = HdcVector::from_seed(42);
        let l1 = deterministic_level(&v);
        let l2 = deterministic_level(&v);
        assert_eq!(l1, l2);
    }

    #[test]
    fn hnsw_delete_excludes_from_results() {
        let mut index = HnswIndex::new();
        let k1: H256 = [1u8; 32];
        let k2: H256 = [2u8; 32];
        index.insert(k1, HdcVector::from_seed(1)).unwrap();
        index.insert(k2, HdcVector::from_seed(2)).unwrap();

        index.delete(&k1);
        let results = index.search(&HdcVector::from_seed(1), 10).unwrap();
        assert!(!results.iter().any(|(k, _)| *k == k1));
    }

    // --- LocalIndex ---

    #[test]
    fn local_starts_brute_force() {
        let index = LocalIndex::new();
        assert!(matches!(index, LocalIndex::BruteForce(_)));
    }

    #[test]
    fn local_upgrades_past_threshold() {
        let mut index = LocalIndex::new();
        for i in 0..=SWITCH_THRESHOLD as u64 {
            let mut k = [0u8; 32];
            k[0..8].copy_from_slice(&i.to_le_bytes());
            index.insert(k, HdcVector::from_seed(i)).unwrap();
        }
        assert!(matches!(index, LocalIndex::Hnsw(_)));
    }

    // --- TieredSearchPipeline ---

    #[test]
    fn tiered_matches_brute_force() {
        // For well-calibrated thresholds, tiered search must return the
        // same top-K as exact brute-force search (no false negatives).
        let mut tiered = TieredSearchPipeline::new();
        let mut brute = BruteForceIndex::new();

        for i in 0..1000u64 {
            let mut k = [0u8; 32];
            k[0..8].copy_from_slice(&i.to_le_bytes());
            let v = HdcVector::from_seed(i);
            tiered.insert(k, v.clone());
            brute.insert(k, v).unwrap();
        }

        let query = HdcVector::from_seed(0);
        // Use very generous thresholds to avoid false negatives
        let tiered_results = tiered.search(&query, 10, 64, 10_240);
        let brute_results = brute.search(&query, 10).unwrap();

        // Same top-K keys (tiered with max thresholds = exact)
        let tiered_keys: Vec<H256> = tiered_results.iter().map(|r| r.0).collect();
        let brute_keys: Vec<H256> = brute_results.iter().map(|r| r.0).collect();
        assert_eq!(tiered_keys, brute_keys);
    }
}
```

### 10.2 Benchmarks

Create `crates/hdc/core/benches/search.rs` using Criterion:

```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use kora_hdc::HdcVector;
use kora_hdc::search::{hamming_distance, BruteForceIndex, HnswIndex, SearchIndex};

fn bench_hamming(c: &mut Criterion) {
    let a = HdcVector::from_seed(0);
    let b = HdcVector::from_seed(1);

    c.bench_function("hamming_distance", |bencher| {
        bencher.iter(|| hamming_distance(&a, &b))
    });
}

fn bench_brute_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("brute_force_top10");
    for &n in &[1_000, 10_000, 100_000] {
        let mut index = BruteForceIndex::new();
        for i in 0..n as u64 {
            let mut k = [0u8; 32];
            k[0..8].copy_from_slice(&i.to_le_bytes());
            index.insert(k, HdcVector::from_seed(i)).unwrap();
        }
        let query = HdcVector::from_seed(u64::MAX);

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bencher, _| {
            bencher.iter(|| index.search(&query, 10).unwrap())
        });
    }
    group.finish();
}

fn bench_hnsw_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_top10");
    for &n in &[10_000, 100_000] {
        let mut index = HnswIndex::new();
        for i in 0..n as u64 {
            let mut k = [0u8; 32];
            k[0..8].copy_from_slice(&i.to_le_bytes());
            index.insert(k, HdcVector::from_seed(i)).unwrap();
        }
        let query = HdcVector::from_seed(u64::MAX);

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bencher, _| {
            bencher.iter(|| index.search(&query, 10).unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, bench_hamming, bench_brute_search, bench_hnsw_search);
criterion_main!(benches);
```

Add to `crates/hdc/core/Cargo.toml`:

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "search"
harness = false
```

---

## 11. Dependencies

Add these to `crates/hdc/core/Cargo.toml` (some may already exist from doc 02):

```toml
[dependencies]
tiny-keccak = { version = "2", features = ["keccak"] }
# Or use sha3 crate -- either works. Just need keccak256.

# For HdcVector::from_seed in tests/benches:
[dev-dependencies]
rand = "0.8"
rand_chacha = "0.3"
criterion = { version = "0.5", features = ["html_reports"] }
```

The SIMD intrinsics come from `std::arch` -- no external crate needed.

---

## 12. Build Notes

### SIMD Feature Gates

The AVX2 and AVX-512 functions use `#[target_feature(enable = ...)]` attributes.
This means:

- They compile on any x86_64 target (the attribute tells LLVM to emit those
  instructions for that specific function only).
- They are only *called* when the runtime `is_x86_feature_detected!` check
  passes.
- No special `RUSTFLAGS` are needed for the library to compile. But for
  benchmarks, use `-C target-cpu=native` to let the compiler auto-vectorize
  other code paths too.

### Running Benchmarks

```bash
cd crates/hdc/core
cargo bench --bench search -- --output-format bencher
# Or with native CPU features for maximum speed:
RUSTFLAGS="-C target-cpu=native" cargo bench --bench search
```

### Running Tests

```bash
cd crates/hdc/core
cargo test --lib search
```

---

## Audit Findings

> Audit date: 2026-05-08
> Auditor scope: spec (sections 0-12) vs. implementation in `crates/hdc/core/src/search/`
> Secondary: `crates/kora-hdc/src/search.rs`, `crates/hdc/core/src/vector.rs`,
> `crates/hdc/core/tests/search.rs`, `crates/hdc/core/benches/hdc_bench.rs`

### F01 -- SIMD: Implementation is Correct and Faithful to Spec

All four Hamming distance paths (scalar, AVX2 Harley-Seal, AVX-512 VPOPCNTDQ,
NEON) are implemented verbatim from the spec. Specific verification:

- **Scalar** (`simd.rs:12-18`): Pure `XOR + count_ones()` loop over `WORDS`.
  Matches spec section 3.1 exactly.

- **AVX2 Harley-Seal** (`simd.rs:26-124`): `popcount_mm256` nibble-lookup,
  `csa_256` full-adder, 10-iteration outer loop processing 4 chunks per group,
  weighted reduction with `_mm256_slli_epi64(total, 2)`, horizontal sum via
  `_mm256_castsi256_si128` / `_mm256_extracti128_si256` / `_mm_add_epi64` /
  `_mm_cvtsi128_si64`. Matches spec section 3.2 line-for-line. No anti-pattern
  8.6 (`_mm256_extract_epi64`) detected.

- **AVX-512** (`simd.rs:128-145`): 20-iteration loop using
  `_mm512_popcnt_epi64` + `_mm512_reduce_add_epi64`. Matches spec section 3.3.
  Minor deviation: implementation casts load pointer as `*const i32` instead of
  the spec's bare `a_ptr.add(i)`. Functionally identical -- `_mm512_loadu_si512`
  accepts `*const i32` by signature -- but the cast is cosmetic noise.

- **NEON** (`simd.rs:149-188`): Block-of-31 overflow guard is correctly
  implemented (spec anti-pattern 8.5 avoided). Widening chain
  `vpaddlq_u8` -> `vpaddlq_u16` -> `vpaddlq_u32` is correct. The redundant
  inner `unsafe` block (`unsafe { ... }` inside an already-`unsafe fn`) at
  line 154 is harmless but unnecessary.

- **Runtime dispatch** (`simd.rs:199-217`): Dispatch order matches spec
  (AVX-512 > AVX2 > scalar on x86_64, NEON on aarch64). The
  `is_x86_feature_detected!` macro provides zero per-call overhead after
  first invocation.

- **Tests** (`simd.rs:219-264`): All five spec-mandated tests are present
  (zero vectors, complementary, self-distance, symmetry, SIMD-vs-scalar).
  The SIMD-vs-scalar test uses 100 seed pairs (spec says 1000).

### F02 -- HNSW: Algorithm is Correct, Consensus-Safe

The HNSW implementation (`hnsw.rs`) faithfully implements the spec's five
determinism requirements (section 5.6):

| Requirement | Status | Evidence |
|---|---|---|
| Deterministic level assignment | PASS | `deterministic_level()` at line 96-111 uses `keccak256` + `leading_zeros() / 4`. Integer-only. |
| Canonical insertion order | PASS | `insertion_order` BTreeMap tracks sequence numbers. `compact()` sorts by seq before rebuild. |
| Deterministic tie-breaking | PASS | `CandidateKey { distance, element_id }` with `Ord` as (distance ASC, element_id ASC). Used in all comparison paths. |
| Single-threaded construction | PASS | `insert()` takes `&mut self`. No parallel mutation. |
| Deterministic neighbor selection | PASS | `select_neighbors()` sorts by `CandidateKey`, truncates. BTreeMap adjacency lists. |

**Algorithm structure verified:**
- `greedy_search_layer` (line 144-176): Correct greedy descent with
  deterministic tie-breaking (`d == current_dist && neighbor_id < current`).
- `search_layer` (line 179-239): Correct two-heap beam search (min-heap for
  candidates via `Reverse<CandidateKey>`, max-heap for results).
- `insert_impl` (line 326-391): Correct two-phase insertion (greedy descent
  above node level, beam search + connect below).
- `compact` (line 307-323): Correct canonical-order rebuild.
- `prune_connections` (line 273-289): Correct sort-and-truncate pruning.

**No spec anti-patterns detected:**
- No `f64::ln()` for level assignment (anti-pattern 8.1).
- No `Reverse<>` for top-K heap in brute-force (anti-pattern 8.2).
- No `HashMap` for nodes or adjacency (anti-pattern 8.3).
- No `sort_unstable` (anti-pattern 8.4).

### F03 -- Tiered Search: Correct Structure, Design Concern with Accuracy

The `TieredSearchPipeline` (`tiered.rs`) implements the spec's three-tier
structure faithfully:

- Tier 1 first-word filter (line 57-59): Correct.
- Tier 2 sampled-word estimator (line 64-72): Correct `dist * 10` scaling.
- Tier 3 exact Hamming distance (line 76-78): Delegates to SIMD `hamming_distance`.
- SoA layout with separate `first_words`, `sample_words`, `full_vectors` vectors.

**Design concern:** The Tier 2 scaling factor `dist * 10` is a rough linear
extrapolation. For 10% sampling of a 10,240-bit vector, the expected
estimation error is approximately +/- 12.8% (binomial variance). This is
acknowledged in the spec (section 7) but the implementation provides no
built-in calibration. Users must manually set `tier1_threshold` and
`tier2_threshold` -- there are no helper functions to compute them from a
target search distance, as the spec comments suggest.

### F04 -- LocalIndex: Correct but Silent Upgrade Failures

The `LocalIndex` auto-switching enum (`local.rs`) correctly implements
the spec. One concern:

- **Line 47: `let _ = hnsw.insert(key, vector);`** -- Insertion errors during
  upgrade are silently discarded. If any vector fails to insert into the HNSW
  index during the brute-force-to-HNSW migration, it is permanently lost with
  no error reported. This is spec-compliant (the spec uses the same pattern at
  line 1084: `let _ = hnsw.insert(key, vector);`) but represents a data-loss
  risk in the design itself.

### F05 -- Duplicate `search` Module in `kora-hdc` Crate

Two separate `search` modules exist:

1. **`crates/hdc/core/src/search/mod.rs`** (99 lines) -- Full implementation
   with `SearchIndex` trait, `HdcIndexError`, SIMD, BruteForce, HNSW,
   LocalIndex, TieredSearch.

2. **`crates/kora-hdc/src/search.rs`** (10 lines) -- Stub that only
   re-exports `vector::hamming_distance`, `vector::similarity`, and three
   threshold constants from the `constants` module.

The integration tests in `crates/hdc/core/tests/search.rs` import from
`kora_hdc::search`, which resolves to the **stub** (file #2), not the full
implementation. This means:
- `BruteForceIndex` and `SearchIndex` in the tests come from the stub's
  re-export chain.
- The tests work because `kora_hdc` (crate at `crates/hdc/core/`) exports the
  full `search` module (both files are in the same crate, with `mod.rs` taking
  precedence when the directory exists).
- The 10-line stub at `crates/kora-hdc/src/search.rs` is dead code in a
  *different* crate and will cause confusion.

### F06 -- `unsafe impl Send + Sync` for `HdcVector` is Unnecessary

In `crates/hdc/core/src/vector.rs` (lines 23-24):

```rust
unsafe impl Send for HdcVector {}
unsafe impl Sync for HdcVector {}
```

`HdcVector` is `pub struct HdcVector(pub [u64; 160])`. Since `[u64; 160]`
is `Send + Sync` automatically (it contains no pointers, `RefCell`, `Rc`, or
other `!Send`/`!Sync` types), the manual `unsafe impl` is unnecessary. It is
harmless but gives a false impression that something special is going on.

---

## Implementation Status

| Spec Section | File | Status | Notes |
|---|---|---|---|
| 2. mod.rs -- trait, error, re-exports | `search/mod.rs` | COMPLETE | Faithful to spec. Added legacy re-exports not in spec. |
| 3.1 Scalar fallback | `search/simd.rs:12-18` | COMPLETE | Exact match. |
| 3.2 AVX2 Harley-Seal | `search/simd.rs:22-124` | COMPLETE | Exact match. Minor: `*const i32` cast in AVX-512. |
| 3.3 AVX-512 VPOPCNTDQ | `search/simd.rs:128-145` | COMPLETE | Exact match. |
| 3.4 ARM NEON | `search/simd.rs:149-188` | COMPLETE | Redundant inner `unsafe` block. |
| 3.5 Runtime dispatch | `search/simd.rs:199-217` | COMPLETE | Exact match. |
| 3.6 SIMD tests | `search/simd.rs:219-264` | PARTIAL | 100 seed pairs vs spec's 1000. |
| 4. BruteForceIndex | `search/brute.rs` | COMPLETE | Includes `drain()` helper. |
| 5.1-5.5 HnswIndex | `search/hnsw.rs` | COMPLETE | All methods match spec. |
| 5.6 Determinism checklist | `search/hnsw.rs` | PASS | All 5 requirements satisfied. |
| 6. LocalIndex | `search/local.rs` | COMPLETE | Silent error swallowing on upgrade (spec-matching). |
| 7. TieredSearchPipeline | `search/tiered.rs` | COMPLETE | No `SearchIndex` trait impl (by design). No calibration helpers. |
| 9. Checklist: benchmarks | `benches/hdc_bench.rs` | PARTIAL | Only `brute_force_search_1000`. No HNSW bench. No SIMD-specific bench (dispatch only). No `hamming_scalar` bench. |
| 10.1 Correctness tests | `search/*.rs` + `tests/search.rs` | PARTIAL | See missing tests below. |
| 10.2 Criterion benchmarks | `benches/hdc_bench.rs` | PARTIAL | Missing HNSW benchmark, missing multi-size brute force (1K/10K/100K). |

### Missing Tests

1. **SIMD scalar-vs-dispatched test uses 100 pairs, spec requires 1000.**
   File: `search/simd.rs:255-263`.

2. **No test for HNSW compaction.** The `compact()` method (triggered when
   tombstone ratio > 20%) has zero test coverage. No test deletes enough nodes
   to trigger compaction, then verifies the rebuilt index produces correct
   results.

3. **No test for LocalIndex upgrade threshold crossing.** The spec test
   `local_upgrades_past_threshold` (spec section 10.1, line 1615-1623) inserts
   `SWITCH_THRESHOLD + 1` vectors and asserts `matches!(index, LocalIndex::Hnsw(_))`.
   The implementation tests (`local.rs:100-146`) do NOT include this test --
   they only test with 50 vectors (well below the 10,000 threshold).

4. **No test for `TieredSearchPipeline` delete.** The pipeline has no `delete`
   method and no `SearchIndex` trait impl, meaning vectors cannot be removed.
   If this is intentional (append-only for on-chain gas optimization), it
   should be documented. If not, it is a missing feature.

5. **Integration tests** (`tests/search.rs`) only test `BruteForceIndex`.
   No integration tests for `HnswIndex`, `LocalIndex`, or
   `TieredSearchPipeline`.

6. **No test for HNSW `search` when entry point is tombstoned.** If the
   entry point node is deleted, `greedy_search_layer` still starts from it.
   The code does skip deleted neighbors, but the starting node itself may be
   deleted. This edge case is not tested.

7. **No test for `top_k = 0`.** All search implementations accept `top_k: usize`
   but none handle the `top_k == 0` case explicitly. BruteForce returns an empty
   vec (correct by accident). HNSW behavior for `top_k = 0` is untested.

---

## Anti-Patterns & Duct Tape

### AP01 -- HashSet in HNSW `search_layer` Breaks Determinism Guarantee

**File:** `crates/hdc/core/src/search/hnsw.rs`, line 187-189
```rust
use std::collections::HashSet;
let mut visited: HashSet<u64> = HashSet::new();
```

The spec uses `HashSet` too (section 5.4, line 737), so the implementation
is spec-compliant. However, this is a latent determinism concern:

- `HashSet` itself does NOT break determinism here because it is only used
  for membership tests (`visited.insert(eid)` returns bool), never iterated.
  The iteration order of `HashSet` is irrelevant since no code path iterates
  the visited set.
- However, the *insertion-order sensitivity* of `HashSet`'s internal layout
  can affect cache performance non-deterministically across runs. For
  consensus-critical code, replacing with `BTreeSet<u64>` would be more
  consistent with the BTreeMap discipline used everywhere else, at the cost
  of O(log n) membership tests instead of O(1) amortized.

**Verdict:** Not a correctness bug, but violates the codebase's own design
principle (spec anti-pattern 8.3 says "Do NOT use HashMap" -- `HashSet` has
the same internal structure).

### AP02 -- `unsafe` Pointer Cast in `deterministic_level`

**File:** `crates/hdc/core/src/search/hnsw.rs`, lines 97-101
```rust
let bytes: &[u8] = unsafe {
    std::slice::from_raw_parts(
        vector.0.as_ptr() as *const u8,
        160 * 8,
    )
};
```

This `unsafe` block reinterprets `[u64; 160]` as `[u8; 1280]`. It is correct
because:
- `HdcVector` has `#[repr(C, align(64))]`, so the memory layout is predictable.
- `u8` has alignment 1, so any pointer is valid for `u8`.
- The length `160 * 8 = 1280` is the exact byte size of `[u64; 160]`.

However, this could be replaced with safe code:
```rust
let bytes: &[u8] = bytemuck::cast_slice(&vector.0);
// or:
let serialized = crate::vector::serialize(vector);
```

The `serialize()` function in `vector.rs` already performs a safe conversion
to `[u8; 1280]`. The `unsafe` block is unnecessary given the existing safe
alternative.

### AP03 -- Redundant Hamming Distance Implementations

Three implementations of Hamming distance exist:

1. `crates/hdc/core/src/vector.rs:96-101` -- `pub fn hamming_distance`
   (scalar-only, used by `vector` module consumers).
2. `crates/hdc/core/src/search/simd.rs:12-18` -- `hamming_scalar`
   (scalar reference for SIMD testing).
3. `crates/hdc/core/src/search/simd.rs:199-217` -- `pub fn hamming_distance`
   (SIMD-dispatching version).

The `search/mod.rs` re-exports both:
```rust
pub use simd::hamming_distance;                           // SIMD version
pub use crate::vector::hamming_distance as vector_hamming_distance; // scalar
```

And `crate::hamming_distance` (the root re-export) points to the scalar-only
`vector::hamming_distance`. This means code that calls `kora_hdc::hamming_distance`
gets the **slow scalar path**, while code that calls `kora_hdc::search::hamming_distance`
gets the **SIMD-accelerated path**. This is confusing and likely a performance
trap for callers who use the more natural root-level import.

### AP04 -- `TieredSearchPipeline` Does Not Implement `SearchIndex`

The `TieredSearchPipeline` has its own `insert` and `search` methods but does
not implement the `SearchIndex` trait. This means:
- No `delete` method exists.
- The `search` method has a different signature (requires two threshold
  parameters).
- The pipeline cannot be used polymorphically with other index types.

This is a deliberate design decision (the extra parameters make a trait impl
awkward), but it means the pipeline is a standalone structure with no shared
interface.

### AP05 -- `BruteForceIndex::insert` Has O(N) Duplicate Check

**File:** `crates/hdc/core/src/search/brute.rs`, line 52
```rust
if self.keys.contains(&key) {
    return Err(HdcIndexError::DuplicateKey(key));
}
```

Linear scan of `keys` for every insertion. For N < 100K this is acceptable
(the spec acknowledges this at line 466), but at 100K entries this adds ~3ms
per insert for a 32-byte key comparison. The spec suggests adding a
`BTreeSet<H256>` if profiling shows it is a bottleneck. No such optimization
has been added.

---

## Performance Concerns

### PC01 -- Root-Level `hamming_distance` is Scalar-Only

As noted in AP03, `kora_hdc::hamming_distance` calls `vector::hamming_distance`
which is the scalar loop. Any caller using the crate-root re-export bypasses
all SIMD acceleration. On Apple M2, this is ~4x slower than the NEON path.
On AVX-512 hardware, it is ~17x slower.

**Impact:** Any code path outside the `search` module that computes Hamming
distance (e.g., trust scoring, duplicate detection, resonance checks) pays
the full scalar cost.

### PC02 -- HNSW Uses BTreeMap for Node Storage

`BTreeMap<u64, HnswNode>` provides deterministic iteration but has O(log n)
lookup/insert vs HashMap's O(1) amortized. For an HNSW index with 100K nodes,
each `search_layer` call performs many node lookups
(`self.nodes[&candidate.element_id]`). The BTreeMap overhead adds
~2-3x to these lookups compared to HashMap.

This is a conscious tradeoff for consensus determinism (spec section 5.6,
anti-pattern 8.3). Not a bug, but worth noting for performance profiling.

### PC03 -- HNSW `compact()` Does a Full Rebuild

The `compact()` method at `hnsw.rs:307-323` creates a brand new `HnswIndex`,
re-inserts all live nodes, and replaces `*self`. For an index with 80K live
nodes (triggered at ~100K with 20% tombstones), this is an O(N * log(N) * M)
operation that could take several seconds. During compaction, the index is
being rebuilt and the old data is dropped.

If compaction is triggered during a hot path (e.g., processing a block with
many deletions), it could cause a latency spike. There is no incremental
compaction or background compaction mechanism.

### PC04 -- No SIMD in Tiered Tier 2

The Tier 2 sampled-word distance computation (`tiered.rs:65-72`) uses a
scalar loop over 16 words. At 16 iterations this is fast, but for very
large datasets (N > 1M), SIMD-accelerating the 16-word comparison could
reduce Tier 2 cost by ~4x. This is a micro-optimization.

### PC05 -- HNSW Vector Cloning on Insert

`insert_impl` at `hnsw.rs:339` clones the vector:
```rust
vector: vector.clone(),
```

Each `HdcVector` is 1,280 bytes. For 100K insertions this copies 128 MB total.
The clone is necessary because the vector is stored in the node and also
used for distance computations during the insertion process. An `Arc<HdcVector>`
wrapper could eliminate the copy but would add indirection cost to every
distance computation.

### PC06 -- Missing Benchmarks

The benchmark file (`benches/hdc_bench.rs`) only benchmarks:
- `hamming_distance` (dispatched, single pair)
- `brute_force_search_1000`

Missing benchmarks specified in section 10.2:
- `hamming_scalar` (isolated scalar performance)
- `brute_force_top10` at 10K and 100K sizes
- `hnsw_top10` at 10K and 100K sizes
- Any NEON/AVX2-specific benchmarks

---

## Recommended Changes Checklist

### Critical (Correctness / Consensus)

- [ ] **Replace `HashSet` with `BTreeSet` in `search_layer`.**
  File: `crates/hdc/core/src/search/hnsw.rs`, line 189.
  Rationale: Consistency with the project's determinism discipline. While
  `HashSet` is not iterated here, using `BTreeSet` removes any doubt and
  aligns with anti-pattern 8.3. Cost: O(log n) vs O(1) for `visited` checks,
  negligible relative to Hamming distance computations.

- [ ] **Unify `hamming_distance` so root-level re-export uses SIMD path.**
  File: `crates/hdc/core/src/lib.rs`, line 37.
  Change `pub use vector::hamming_distance` to
  `pub use search::hamming_distance`. Keep `vector::hamming_distance` as a
  `pub(crate)` scalar reference for testing only. This ensures all callers
  get SIMD acceleration by default.

- [ ] **Add HNSW compaction test.**
  File: `crates/hdc/core/src/search/hnsw.rs`, new test.
  Insert 100 vectors, delete 25 (triggering compaction at >20%), verify
  search still returns correct results and `len()` is 75.

- [ ] **Add test for tombstoned entry point.**
  File: `crates/hdc/core/src/search/hnsw.rs`, new test.
  Insert 3 vectors where one becomes the entry point, delete the entry
  point, verify search still works on remaining vectors.

### Important (Quality / Safety)

- [ ] **Remove redundant `unsafe impl Send/Sync` on `HdcVector`.**
  File: `crates/hdc/core/src/vector.rs`, lines 23-24.
  `[u64; 160]` is automatically `Send + Sync`.

- [ ] **Replace `unsafe` pointer cast in `deterministic_level` with safe code.**
  File: `crates/hdc/core/src/search/hnsw.rs`, lines 97-101.
  Use `bytemuck::cast_slice(&vector.0)` or `crate::vector::serialize(vector)`.

- [ ] **Remove redundant inner `unsafe` block in `hamming_neon`.**
  File: `crates/hdc/core/src/search/simd.rs`, line 154.
  The function is already `unsafe fn`; the inner `unsafe { }` block is
  redundant.

- [ ] **Handle or document silent error swallowing in `LocalIndex::upgrade`.**
  File: `crates/hdc/core/src/search/local.rs`, line 47.
  Either propagate errors from `hnsw.insert()` or add a doc comment explaining
  why failures are acceptable (e.g., duplicate keys cannot occur because
  they were already validated by `BruteForceIndex`).

- [ ] **Clean up dead stub at `crates/kora-hdc/src/search.rs`.**
  This 10-line stub re-exports are shadowed by the full `search/mod.rs`
  module. If `crates/kora-hdc/` is a separate crate that should proxy to
  `crates/hdc/core/`, make that explicit. If it is dead code, remove it.

### Nice-to-Have (Testing / Performance)

- [ ] **Increase SIMD-vs-scalar test to 1000 pairs (spec requirement).**
  File: `crates/hdc/core/src/search/simd.rs`, line 256.
  Change `for seed in 0..100u64` to `for seed in 0..1000u64`.

- [ ] **Add `LocalIndex` upgrade threshold test.**
  File: `crates/hdc/core/src/search/local.rs`, new test.
  Insert `SWITCH_THRESHOLD + 1` vectors, assert `matches!(index, LocalIndex::Hnsw(_))`.

- [ ] **Add HNSW benchmark at 10K and 100K sizes.**
  File: `crates/hdc/core/benches/hdc_bench.rs`.

- [ ] **Add brute-force benchmark at 10K and 100K sizes.**
  File: `crates/hdc/core/benches/hdc_bench.rs`.

- [ ] **Add `TieredSearchPipeline` delete capability or document as append-only.**
  File: `crates/hdc/core/src/search/tiered.rs`.

- [ ] **Add threshold calibration helpers to `TieredSearchPipeline`.**
  E.g., `fn recommended_thresholds(target_distance: u32) -> (u32, u32)`
  using the bit-error-rate formula described in the spec (section 7, lines
  1243-1251).

- [ ] **Add `top_k = 0` edge case tests for all index types.**

- [ ] **Consider `Arc<HdcVector>` or arena allocation for HNSW to avoid 1,280-byte clones per insert.**

---

## Second-Pass Remediation Detail

This pass narrows the vector-search fixes to implementation-ready changes in
`crates/hdc/core/src/search/`. The priority is consensus determinism first,
then API consistency, test coverage, and benchmark coverage. These are design
notes only; implementation code should be changed in the Rust sources in a
separate patch.

### 1. HNSW Determinism: Canonical Candidate Ordering

**Files/functions:**
- `crates/hdc/core/src/search/hnsw.rs`
- `CandidateKey`
- `HnswIndex::greedy_search_layer`
- `HnswIndex::search_layer`
- `HnswIndex::select_neighbors`
- `HnswIndex::prune_connections`
- `SearchIndex for HnswIndex::search`

The current `CandidateKey` ties by `element_id`. That is deterministic for a
single identical operation log, but it makes graph shape and traversal depend
on an internal allocation sequence instead of the stable public key. Prefer the
stable key as the consensus tie-breaker and keep `element_id` only as the node
lookup handle.

Implementation sketch:

```rust
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CandidateKey {
    distance: u32,
    key: H256,
    element_id: u64,
}

impl Ord for CandidateKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .cmp(&other.distance)
            .then_with(|| self.key.cmp(&other.key))
            .then_with(|| self.element_id.cmp(&other.element_id))
    }
}
```

Then update every construction site:

```rust
let node = &self.nodes[&eid];
let key = CandidateKey {
    distance: d,
    key: node.key,
    element_id: eid,
};
```

Apply the same ordering in `greedy_search_layer`. Today the tie-breaker is
`neighbor_id < current`; change it to compare `(distance, neighbor.key,
neighbor_id)` against `(current_dist, current.key, current)`:

```rust
let current_key = self.nodes[&current].key;
let better = d < current_dist
    || (d == current_dist
        && (neighbor.key, neighbor_id) < (current_key, current));

if better {
    current = neighbor_id;
    current_dist = d;
    changed = true;
}
```

For `prune_connections`, build `CandidateKey` with the neighbor's `H256` and
truncate by `(distance, key, element_id)`, not `(distance, element_id)`:

```rust
let mut entries: Vec<CandidateKey> = neighbors
    .iter()
    .map(|(&eid, &dist)| {
        let neighbor = &self.nodes[&eid];
        CandidateKey { distance: dist, key: neighbor.key, element_id: eid }
    })
    .collect();
entries.sort();
entries.truncate(max_conn);
```

The final result sort in `HnswIndex::search` already sorts by `(distance,
key)`. Keep that as the public result contract.

### 2. HNSW `HashSet` Use: Replace With `BTreeSet`

**Files/functions:**
- `crates/hdc/core/src/search/hnsw.rs`
- `HnswIndex::search_layer`

`search_layer` currently imports and uses `std::collections::HashSet` for
`visited`. It is only used for membership, so iteration order does not currently
escape into results. Still, `HashSet` carries randomized state and conflicts
with the module's determinism posture. Replace it with `BTreeSet`.

Implementation sketch:

```rust
use std::cmp::Reverse;
use std::collections::BTreeSet;

let mut visited: BTreeSet<u64> = BTreeSet::new();
```

No algorithmic rewrite is required. The additional `O(log n)` membership cost
is small compared with `hamming_distance` calls and removes a needless source
of consensus-audit noise.

### 3. Unsafe Pointer Cast: Hash Serialized Bytes

**Files/functions:**
- `crates/hdc/core/src/search/hnsw.rs`
- `deterministic_level`
- `crates/hdc/core/src/vector.rs::serialize`

`deterministic_level` currently reinterprets `HdcVector.0` as raw bytes via
`std::slice::from_raw_parts`. The cast is likely memory-safe for `u8`, but it
hashes native-endian memory representation. Consensus code should hash the
same bytes on every architecture. Use `crate::vector::serialize`, which already
emits the vector in explicit little-endian word order.

Implementation sketch:

```rust
fn deterministic_level(vector: &HdcVector) -> usize {
    let bytes = crate::vector::serialize(vector);

    let mut hasher = Keccak::v256();
    hasher.update(&bytes);

    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);

    let random_bits = u64::from_le_bytes(hash[0..8].try_into().unwrap());
    let level = (random_bits.leading_zeros() / 4) as usize;
    level.min(MAX_LEVEL)
}
```

Do not replace this with `bytemuck::cast_slice(&vector.0)` unless the code also
normalizes each `u64` to little-endian bytes first; a raw cast still follows
host endianness.

### 4. SearchIndex and TieredSearchPipeline Design

**Files/functions:**
- `crates/hdc/core/src/search/mod.rs::SearchIndex`
- `crates/hdc/core/src/search/tiered.rs`
- `TieredSearchPipeline::insert`
- `TieredSearchPipeline::search`
- new `TieredSearchPipeline::search_with_thresholds`
- new `impl SearchIndex for TieredSearchPipeline`

`TieredSearchPipeline` currently has a separate API: infallible `insert`, no
`delete`, no duplicate-key handling, and `search` requires thresholds. This is
fine for an append-only gas experiment, but it does not match the module-level
index abstraction.

Recommended fix: make `TieredSearchPipeline` implement `SearchIndex` for
normal lifecycle semantics, and keep the gas-specific threshold API as an
explicit method.

Data-structure sketch:

```rust
pub struct TieredSearchPipeline {
    keys: Vec<H256>,
    key_to_index: BTreeMap<H256, usize>,
    deleted: Vec<bool>,
    live_len: usize,
    first_words: Vec<u64>,
    sample_words: Vec<[u64; 16]>,
    full_vectors: Vec<HdcVector>,
    default_tier1_threshold: u32,
    default_tier2_threshold: u32,
}
```

Do not use `swap_remove` for deletion because all parallel arrays must keep
stable indices and deterministic ordering. Use tombstones:

```rust
fn delete(&mut self, key: &H256) -> bool {
    let Some(&idx) = self.key_to_index.get(key) else { return false; };
    if self.deleted[idx] { return false; }

    self.deleted[idx] = true;
    self.key_to_index.remove(key);
    self.live_len -= 1;
    true
}
```

Make the public thresholded method explicit:

```rust
pub fn search_with_thresholds(
    &self,
    query: &HdcVector,
    top_k: usize,
    tier1_threshold: u32,
    tier2_threshold: u32,
) -> Result<SearchResult, HdcIndexError> {
    if self.live_len == 0 {
        return Err(HdcIndexError::EmptyIndex);
    }
    if top_k == 0 {
        return Ok(Vec::new());
    }

    // existing tier loop, with `if self.deleted[i] { continue; }`
}
```

Then implement the shared trait:

```rust
impl SearchIndex for TieredSearchPipeline {
    fn insert(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
        if self.key_to_index.contains_key(&key) {
            return Err(HdcIndexError::DuplicateKey(key));
        }

        let idx = self.keys.len();
        self.key_to_index.insert(key, idx);
        self.keys.push(key);
        self.deleted.push(false);
        self.live_len += 1;
        self.first_words.push(vector.0[0]);

        let mut samples = [0u64; 16];
        for (s, &word_idx) in SAMPLE_INDICES.iter().enumerate() {
            samples[s] = vector.0[word_idx];
        }
        self.sample_words.push(samples);
        self.full_vectors.push(vector);
        Ok(())
    }

    fn delete(&mut self, key: &H256) -> bool {
        // tombstone implementation above
    }

    fn search(&self, query: &HdcVector, top_k: usize) -> Result<SearchResult, HdcIndexError> {
        self.search_with_thresholds(
            query,
            top_k,
            self.default_tier1_threshold,
            self.default_tier2_threshold,
        )
    }

    fn len(&self) -> usize {
        self.live_len
    }
}
```

If the trait is expected to mean exact top-K, set the defaults to permissive
values `(64, 10_240)` so the trait implementation has no false negatives. Use
`search_with_thresholds` for the gas-saving approximate path. If the project
accepts approximate semantics under `SearchIndex`, update the trait docs in
`search/mod.rs` to state that implementations may be approximate and that the
result ordering is deterministic among returned candidates.

### 5. Benchmark Plan

**Files/functions:**
- `crates/hdc/core/benches/hdc_bench.rs`
- `crates/hdc/core/Cargo.toml`
- `crates/hdc/core/src/search/simd.rs`

The benchmark suite should separate scalar distance, SIMD-dispatched distance,
index build cost, and query cost.

Add Criterion groups:

```rust
fn bench_hamming_scalar(c: &mut Criterion) {
    use kora_hdc::vector::hamming_distance as scalar_hamming_distance;
    // bench scalar_hamming_distance(&a, &b)
}

fn bench_hamming_dispatched(c: &mut Criterion) {
    use kora_hdc::search::hamming_distance as dispatched_hamming_distance;
    // bench dispatched_hamming_distance(&a, &b)
}
```

For direct AVX2/AVX-512/NEON benchmarks, expose internals intentionally instead
of making `search::simd` public by accident. Add one of these designs:

```rust
// Preferred: feature-gated bench wrappers in search/simd.rs.
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub unsafe fn hamming_avx2_bench(a: &HdcVector, b: &HdcVector) -> u32 {
    hamming_avx2(&a.0, &b.0)
}
```

or keep architecture-specific direct benches out of tree and benchmark only the
dispatched public function.

Index benchmark matrix:

```text
hamming/scalar
hamming/dispatched
brute_force/search_top10/1_000
brute_force/search_top10/10_000
brute_force/search_top10/100_000
hnsw/build/10_000
hnsw/build/100_000
hnsw/search_top10/10_000
hnsw/search_top10/100_000
tiered/search_top10/permissive/100_000
tiered/search_top10/calibrated/100_000
```

Use deterministic fixture generation:

```rust
fn fixture(n: u64) -> Vec<(H256, HdcVector)> {
    (0..n)
        .map(|i| {
            let mut key = [0u8; 32];
            key[0..8].copy_from_slice(&i.to_le_bytes());
            (key, HdcVector::random(i))
        })
        .collect()
}
```

For query benchmarks, build the index outside `bencher.iter`. For build
benchmarks, construct a fresh index inside `iter_batched` so insertion cost is
measured without fixture generation.

### 6. Required Tests

**Files/functions:**
- `crates/hdc/core/src/search/hnsw.rs`
- `crates/hdc/core/src/search/local.rs`
- `crates/hdc/core/src/search/tiered.rs`
- `crates/hdc/core/src/search/simd.rs`
- `crates/hdc/core/tests/search.rs`

Add tests that assert graph determinism, tie-breakers, tombstones, trait
semantics, and edge cases.

HNSW graph signature test:

```rust
#[cfg(test)]
fn graph_signature(index: &HnswIndex) -> Vec<(H256, usize, Vec<Vec<H256>>)> {
    index.nodes.values().map(|node| {
        let layers = node.neighbors.iter().map(|neighbors| {
            let mut keys: Vec<H256> = neighbors
                .keys()
                .map(|eid| index.nodes[eid].key)
                .collect();
            keys.sort();
            keys
        }).collect();
        (node.key, node.level, layers)
    }).collect()
}

#[test]
fn deterministic_builds_have_identical_graph_signature() {
    let mut a = HnswIndex::new();
    let mut b = HnswIndex::new();

    for i in 0..500u64 {
        let key = make_key(i);
        let vector = HdcVector::random(i);
        a.insert(key, vector.clone()).unwrap();
        b.insert(key, vector).unwrap();
    }

    assert_eq!(graph_signature(&a), graph_signature(&b));
}
```

Equal-distance tie-breaker test:

```rust
#[test]
fn equal_distance_results_sort_by_key() {
    let query = HdcVector::default();
    let mut v1 = HdcVector::default();
    let mut v2 = HdcVector::default();
    v1.0[0] = 0b0011;
    v2.0[0] = 0b1100;

    let k_hi = make_key(9);
    let k_lo = make_key(1);

    let mut index = HnswIndex::new();
    index.insert(k_hi, v1).unwrap();
    index.insert(k_lo, v2).unwrap();

    let results = index.search(&query, 2).unwrap();
    assert_eq!(results, vec![(k_lo, 2), (k_hi, 2)]);
}
```

Unsafe-cast replacement regression:

```rust
#[test]
fn deterministic_level_uses_serialized_little_endian_bytes() {
    let mut vector = HdcVector::default();
    vector.0[0] = 0x0102_0304_0506_0708;

    let serialized = crate::vector::serialize(&vector);
    assert_eq!(&serialized[0..8], &[8, 7, 6, 5, 4, 3, 2, 1]);

    assert_eq!(deterministic_level(&vector), deterministic_level(&vector));
}
```

Compaction and tombstoned-entry tests:

```rust
#[test]
fn compaction_preserves_live_search_results() {
    let mut index = HnswIndex::new();
    let mut live = Vec::new();

    for i in 0..100u64 {
        let key = make_key(i);
        let vector = HdcVector::random(i);
        if i >= 25 {
            live.push((key, vector.clone()));
        }
        index.insert(key, vector).unwrap();
    }

    for i in 0..25u64 {
        assert!(index.delete(&make_key(i)));
    }

    assert_eq!(index.len(), 75);
    let results = index.search(&live[0].1, 10).unwrap();
    assert!(results.iter().all(|(key, _)| *key >= make_key(25)));
}

#[test]
fn search_survives_deleted_entry_point() {
    let mut index = HnswIndex::new();
    for i in 0..64u64 {
        index.insert(make_key(i), HdcVector::random(i)).unwrap();
    }

    let entry = index.entry_point.unwrap();
    let entry_key = index.nodes[&entry].key;
    assert!(index.delete(&entry_key));

    let results = index.search(&HdcVector::random(999), 5).unwrap();
    assert!(!results.iter().any(|(key, _)| *key == entry_key));
}
```

`LocalIndex` upgrade test:

```rust
#[test]
fn crossing_threshold_upgrades_to_hnsw() {
    let mut index = LocalIndex::new();
    for i in 0..=SWITCH_THRESHOLD as u64 {
        index.insert(make_key(i), HdcVector::random(i)).unwrap();
    }
    assert!(matches!(index, LocalIndex::Hnsw(_)));
}
```

`TieredSearchPipeline` trait and deletion tests:

```rust
#[test]
fn tiered_rejects_duplicates_and_deletes_by_key() {
    let mut index = TieredSearchPipeline::new();
    let key = make_key(1);
    let vector = HdcVector::random(1);

    index.insert(key, vector.clone()).unwrap();
    assert!(index.insert(key, vector.clone()).is_err());
    assert!(index.delete(&key));
    assert_eq!(index.len(), 0);
    assert!(index.search(&vector, 1).is_err());
}

#[test]
fn tiered_search_index_permissive_defaults_match_brute_force() {
    let mut brute = BruteForceIndex::new();
    let mut tiered = TieredSearchPipeline::with_thresholds(64, 10_240);

    for i in 0..200u64 {
        let key = make_key(i);
        let vector = HdcVector::random(i);
        brute.insert(key, vector.clone()).unwrap();
        tiered.insert(key, vector).unwrap();
    }

    let query = HdcVector::random(999);
    assert_eq!(tiered.search(&query, 10).unwrap(), brute.search(&query, 10).unwrap());
}
```

Shared edge-case tests:

```rust
#[test]
fn top_k_zero_returns_empty_for_all_indexes() {
    // BruteForceIndex, HnswIndex, LocalIndex, and TieredSearchPipeline should
    // return Ok(vec![]) when non-empty and top_k == 0.
}
```

Increase the SIMD dispatch regression in `search/simd.rs` from 100 to 1000
seed pairs:

```rust
for seed in 0..1000u64 {
    let a = HdcVector::random(seed * 2);
    let b = HdcVector::random(seed * 2 + 1);
    assert_eq!(hamming_scalar(&a.0, &b.0), hamming_distance(&a, &b));
}
```
