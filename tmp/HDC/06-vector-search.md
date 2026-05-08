# Vector Search Architecture

How to search over binary hypervectors efficiently at every scale.

---

## First Principles: The Search Problem

Hyperdimensional computing produces **binary vectors** — long bitstrings of fixed width (10,240 bits in our system). Every concept, memory, and relationship an agent encodes becomes one of these vectors. The fundamental operation on those vectors is *similarity search*: given a query vector, find the stored vectors most like it.

Concretely:

> **Given** N binary vectors, each 10,240 bits (1,280 bytes, 160 `u64` words),
> **and** a query vector of the same dimension,
> **find** the K vectors with the smallest Hamming distance to the query.

**Hamming distance** counts the number of bit positions where two vectors differ. For Binary Spatter Code (BSC) vectors — where each bit is drawn i.i.d. from Bernoulli(0.5) — Hamming distance is the natural metric. Two unrelated vectors have an expected distance of 5,120 (half the bits differ). Two vectors encoding related concepts will share significantly more bits, producing a distance well below 5,120. The search problem is to find those low-distance needles in a haystack.

### Why Binary Search is Fast

Hamming distance decomposes into two operations that modern hardware executes in a single cycle each:

1. **XOR** — produces a bitvector with 1s wherever the two inputs differ
2. **POPCOUNT** — counts the number of set bits in a word

```
  query:    1 0 1 1 0 0 1 0 1 1 ...
  stored:   1 1 1 0 0 0 1 1 1 0 ...
  XOR:      0 1 0 1 0 0 0 1 0 1 ...  =>  POPCOUNT = 4 differing bits
```

No floating-point arithmetic. No square roots. No dot products. Just integer XOR and popcount, repeated 160 times (once per `u64` word), then summed. This is why binary HDC vectors are uniquely suited to high-throughput search — the distance function is an order of magnitude cheaper than the cosine similarity or L2 distance used in float-vector systems.

### Performance Targets

| Context | Latency Target | Scale | Requires |
|---------|---------------|-------|----------|
| Local cognitive loop | < 1us per comparison | 1K - 100K vectors | Scalar (any) |
| Local search (top-10) | < 1ms total | Up to 25K vectors | AVX2 or NEON (brute-force) |
| Local search (top-10) | < 5ms total | Up to 100K vectors | AVX2 or NEON (brute-force) |
| Local search (top-10) | < 1ms total | Up to 1M vectors | HNSW index required |
| On-chain precompile | < 5ms total | Up to 100K vectors | AVX2 or better (brute-force) |
| On-chain precompile | < 50ms total | Up to 1M vectors | HNSW precompile |

---

## Hamming Distance -- The Hot Path

Everything rests on fast Hamming distance. This section covers every level of optimization, from scalar baseline to platform-specific SIMD, with actual Rust code for each.

### Scalar Baseline

The simplest correct implementation:

```rust
// CONSENSUS-SAFE: Pure integer arithmetic (XOR + popcount). Bit-exact on
// all platforms. This is the ONLY distance function used on-chain.
fn hamming_distance(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let mut dist = 0u32;
    for i in 0..160 {
        dist += (a[i] ^ b[i]).count_ones();
    }
    dist
}
```

On x86_64 with `-C target-cpu=native`, `count_ones()` compiles to the hardware `POPCNT` instruction, available since Intel Nehalem (2008) and AMD Barcelona (2007). Each loop iteration is three instructions: `XOR r64, r64` + `POPCNT r64, r64` + `ADD r32, r32`.

**Performance varies dramatically by microarchitecture:**

- **Intel Skylake and earlier**: `POPCNT` has a throughput of 1 per cycle but a 3-cycle latency, and a false dependency on the destination register (a microarchitectural quirk Intel did not fix until Ice Lake). LLVM works around this with `xor`-zeroing the destination, breaking the false dependency and allowing the loop to run at throughput-limited rate. Each iteration is a 3-instruction chain (XOR + POPCNT + ADD); at 1 POPCNT/cycle throughput, the loop takes ~480 cycles. At 4 GHz that is **~120ns**. In practice, LLVM may auto-vectorize the scalar `count_ones()` loop into SSE4.2 POPCNT over wider chunks, yielding **~50-80ns** — but this depends on compiler version and flags. The 480-cycle / 120ns figure is the conservative baseline; the 50-80ns figure assumes favorable auto-vectorization. (Estimated, not measured.)
- **AMD Zen 2 and later**: `POPCNT` has a reciprocal throughput of 0.25 cycles (i.e., 4 per cycle across multiple execution ports) with no false dependency and 1-cycle latency. LLVM can schedule 4 independent POPCNT operations per cycle. Effective throughput: ~160 cycles for the 160-iteration loop (4 iterations retire per cycle), or **~40ns at 4 GHz**. The 160-cycle lower bound assumes perfect scheduling; real-world overhead (loop control, ADD accumulation) pushes this to **~40-55ns**. (Estimated, not measured.)
- **Intel Ice Lake and later**: The false-dependency bug is fixed. POPCNT throughput is 1 per cycle with 3-cycle latency; the OoO engine can overlap iterations more effectively without the workaround penalty. ~480 cycles at throughput limit, but pipelining reduces effective cost to ~300-400 cycles, yielding **~75-100ns at 4 GHz**. (Estimated, not measured.)

The scalar version is already fast and is the easiest to audit for correctness. For many applications it is sufficient. But when search throughput is critical — scanning 100K+ vectors per query — explicit SIMD provides a further 2-3x speedup (AVX2 on the same hardware) or up to ~17x (AVX-512 vs. scalar Skylake).

### AVX2: Harley-Seal Accumulation

**Reference**: Mula, Kurz, Lemire (2017). "Faster Population Counts Using AVX2 Instructions." *Computer Journal*, 61(1), 111-120.

AVX2 provides 256-bit SIMD registers but **no native SIMD popcount instruction**. The standard workaround combines two techniques:

1. **The `vpshufb` popcount trick**: Use `_mm256_shuffle_epi8` as a 16-entry lookup table. Split each byte into two nybbles, look up each nybble's popcount, and add. This counts set bits within each byte of a 256-bit register.

2. **Harley-Seal carry-save accumulation**: Instead of summing popcount results after every XOR (which requires expensive horizontal adds), accumulate partial popcounts across three registers — `ones`, `twos`, and `fours` — using a carry-save adder pattern. This reduces the number of additions by a factor of 4.

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Population count of a 256-bit register using the vpshufb lookup trick.
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

/// Carry-save adder: produces (a XOR b XOR c, majority(a,b,c))
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn csa_256(
    a: __m256i, b: __m256i, c: __m256i
) -> (__m256i, __m256i) {
    let u = _mm256_xor_si256(a, b);
    let sum = _mm256_xor_si256(u, c);
    let carry = _mm256_or_si256(
        _mm256_and_si256(a, b),
        _mm256_and_si256(u, c),
    );
    (sum, carry)
}

/// Hamming distance between two 10,240-bit vectors using AVX2.
///
/// Processes 256 bits per iteration. Uses Harley-Seal accumulation
/// to batch popcount across groups of 4 XOR results, reducing the
/// number of horizontal additions.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn hamming_avx2(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let a_ptr = a.as_ptr() as *const __m256i;
    let b_ptr = b.as_ptr() as *const __m256i;

    // 160 u64s = 1280 bytes = 40 x 256-bit chunks
    let mut total = _mm256_setzero_si256();
    let mut ones = _mm256_setzero_si256();
    let mut twos = _mm256_setzero_si256();

    // Process in groups of 4 chunks (Harley-Seal needs 4 inputs)
    // 40 chunks / 4 per group = 10 outer iterations
    for i in 0..10 {
        let base = i * 4;

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

        // Harley-Seal carry-save: accumulate d0..d3 into ones/twos
        let (new_ones, carry0) = csa_256(ones, d0, d1);
        let (new_ones2, carry1) = csa_256(new_ones, d2, d3);
        ones = new_ones2;
        let (new_twos, carry2) = csa_256(twos, carry0, carry1);
        twos = new_twos;

        // Flush fours into total (carry2 represents groups of 4 bits)
        total = _mm256_add_epi64(
            total,
            _mm256_sad_epu8(popcount_mm256(carry2), _mm256_setzero_si256()),
        );
    }

    // Flush remaining accumulators into total with correct weights.
    // carry2 represents positions where 4 inputs had set bits,
    // so each set bit in carry2 contributes 4 to the Hamming distance.
    // twos represents positions where 2 inputs had set bits (weight 2).
    // ones represents positions where 1 input had set bits (weight 1).
    //
    // The loop above already added popcount(carry2) to total without
    // the weight, so we need to scale: total currently holds
    // sum(popcount(carry2_i)). We multiply by 4, then add 2*popcount(twos)
    // and 1*popcount(ones).
    total = _mm256_slli_epi64(total, 2); // total *= 4 (carry2 weight)
    total = _mm256_add_epi64(
        total,
        _mm256_slli_epi64(
            _mm256_sad_epu8(popcount_mm256(twos), _mm256_setzero_si256()),
            1, // twos count as 2 each
        ),
    );
    total = _mm256_add_epi64(
        total,
        _mm256_sad_epu8(popcount_mm256(ones), _mm256_setzero_si256()),
    );

    // Horizontal sum of 4 x u64 lanes
    let lo = _mm256_castsi256_si128(total);
    let hi = _mm256_extracti128_si256(total, 1);
    let sum128 = _mm_add_epi64(lo, hi);
    let upper = _mm_srli_si128(sum128, 8);
    let final_sum = _mm_add_epi64(sum128, upper);
    _mm_cvtsi128_si64(final_sum) as u32
}
```

> **Implementation note**: The code above illustrates the structure. A production implementation should be validated against the scalar baseline with property-based tests (see the consensus determinism section below). The Mula-Kurz-Lemire paper provides a thoroughly benchmarked reference.

**Performance (estimated)**: ~40-50ns for a full 10,240-bit comparison on Skylake at 4 GHz. The 10 outer iterations of the Harley-Seal loop execute ~30 SIMD instructions each (loads, XORs, CSA logic, vpshufb popcount, sad accumulation), totaling ~300 instructions. Skylake's superscalar backend can sustain ~3-4 SIMD uops/cycle on this workload (the operations are spread across multiple execution ports: loads on p2/p3, shuffles on p5, integer arithmetic on p0/p1/p5), yielding ~80-100 cycles. Adding memory latency overhead, the effective cost is ~160-200 cycles = ~40-50ns. Effective speedup is roughly **2.5-3x** over the 120ns scalar baseline on the same hardware. The Mula-Kurz-Lemire paper reports ~2.5x speedup for AVX2 Harley-Seal over scalar POPCNT, consistent with this estimate.

### AVX-512 VPOPCNTDQ

Available since Intel Ice Lake (2019) and AMD Zen 4 (2022), the `VPOPCNTDQ` extension provides what AVX2 lacks: a hardware SIMD popcount instruction. `_mm512_popcnt_epi64` computes the popcount of each of 8 x 64-bit integers in a 512-bit register simultaneously.

This eliminates the entire `vpshufb` lookup table and Harley-Seal machinery. The inner loop becomes trivially simple:

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Hamming distance using AVX-512 VPOPCNTDQ.
///
/// Processes 512 bits (8 u64s) per iteration.
/// 160 u64s / 8 per iteration = 20 iterations total.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512vpopcntdq")]
pub unsafe fn hamming_avx512(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let a_ptr = a.as_ptr() as *const __m512i;
    let b_ptr = b.as_ptr() as *const __m512i;

    let mut acc = _mm512_setzero_si512();

    // 160 u64s = 20 x 512-bit chunks
    for i in 0..20 {
        let va = _mm512_loadu_si512(a_ptr.add(i));
        let vb = _mm512_loadu_si512(b_ptr.add(i));
        let xored = _mm512_xor_si512(va, vb);
        let popcnt = _mm512_popcnt_epi64(xored);
        acc = _mm512_add_epi64(acc, popcnt);
    }

    // Horizontal sum of 8 x u64 lanes
    _mm512_reduce_add_epi64(acc) as u32
}
```

20 iterations. Each iteration: one load, one load, one XOR, one popcount, one add. The loop body is 5 instructions operating on 512 bits at a time.

**Performance (estimated)**: ~5-10ns per full 10,240-bit comparison at 3.5 GHz. 20 iterations of 5 instructions each = 100 instructions total, but the loads pipeline ahead of compute via OoO execution. The critical path is ~20-40 cycles (VPOPCNTDQ has 3-cycle latency on Ice Lake but 1-cycle reciprocal throughput; the accumulator chain pipelines across iterations). At 3.5 GHz, 20-40 cycles = 5.7-11.4ns. This is the fastest software path on hardware that supports it.

### ARM NEON (Apple M-series, AWS Graviton)

ARM NEON provides `vcntq_u8`, which computes the popcount of each byte in a 128-bit register. The result is per-byte counts that need to be widened and accumulated:

```rust
#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

/// Hamming distance using ARM NEON.
///
/// Processes 128 bits (2 u64s) per iteration.
/// 160 u64s / 2 per iteration = 80 iterations total.
///
/// vcntq_u8:   byte-level popcount (16 bytes at once)
/// vpaddlq_u8: pairwise add u8 -> u16 (accumulate without overflow)
/// vpaddlq_u16: pairwise add u16 -> u32
/// vpaddlq_u32: pairwise add u32 -> u64
#[cfg(target_arch = "aarch64")]
pub unsafe fn hamming_neon(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let a_ptr = a.as_ptr() as *const uint8x16_t;
    let b_ptr = b.as_ptr() as *const uint8x16_t;

    // Accumulate in u16 lanes to avoid u8 overflow.
    // Max popcount per byte = 8. Over 80 iterations of 16 bytes,
    // each u8 lane accumulates at most 80 * 8 = 640, which overflows u8 (max 255).
    // So we periodically widen. Process in blocks of 31 iterations
    // (31 * 8 = 248, fits in u8), then flush to wider accumulators.

    let mut total: u64 = 0;
    let mut i = 0;

    while i < 80 {
        let block_end = std::cmp::min(i + 31, 80);
        let mut acc8 = vdupq_n_u8(0);

        for j in i..block_end {
            let va = vld1q_u8(a_ptr.add(j) as *const u8);
            let vb = vld1q_u8(b_ptr.add(j) as *const u8);
            let xored = veorq_u8(va, vb);
            let popcnt = vcntq_u8(xored);
            acc8 = vaddq_u8(acc8, popcnt);
        }

        // Widen u8 -> u16 -> u32 -> u64 and accumulate
        let acc16 = vpaddlq_u8(acc8);
        let acc32 = vpaddlq_u16(acc16);
        let acc64 = vpaddlq_u32(acc32);
        total += vgetq_lane_u64(acc64, 0) + vgetq_lane_u64(acc64, 1);

        i = block_end;
    }

    total as u32
}
```

**Performance (estimated)**: ~20-35ns on Apple M2 at 3.5 GHz. The 80-iteration inner loop (with periodic u8-to-u64 widening every 31 iterations) totals ~400 NEON instructions. The M2's 8-wide decode and deep OoO engine can sustain ~3-4 NEON ops/cycle on this workload, yielding ~100-130 cycles = ~28-37ns. Earlier estimates of 8-12ns assumed throughput closer to the theoretical 8-wide peak, which is unlikely to be sustained on a dependent accumulation chain. Graviton 3 (Arm Neoverse V1) achieves comparable latency due to similar pipeline width and NEON throughput.

> **Note**: The NEON path processes 128 bits per iteration (vs. 512 for AVX-512), requiring 4x more iterations. Despite the M2's wide pipeline, this puts NEON roughly on par with AVX2 Harley-Seal rather than approaching AVX-512 VPOPCNTDQ.

### Multi-Version Dispatch

For a system where validators run on heterogeneous hardware, all paths must produce identical results. This is straightforward since every path computes the same pure-integer arithmetic — no floating-point rounding differences, no approximation, no non-determinism.

```rust
#[repr(align(64))]  // Cache-line aligned for SIMD loads
pub struct HdcVector(pub [u64; 160]);

impl HdcVector {
    /// Create a zero vector (all bits 0).
    pub fn zero() -> Self {
        HdcVector([0u64; 160])
    }

    /// Create a vector from a deterministic seed via ChaCha20.
    /// Consensus-safe: same seed always produces the same vector.
    pub fn from_seed(seed: u64) -> Self {
        use rand::SeedableRng;
        use rand_chacha::ChaCha20Rng;
        use rand::Rng;
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let mut v = [0u64; 160];
        for w in &mut v { *w = rng.gen(); }
        HdcVector(v)
    }

    /// Bitwise complement (NOT). Returns a vector where every bit is flipped.
    pub fn complement(&self) -> Self {
        let mut v = [0u64; 160];
        for i in 0..160 { v[i] = !self.0[i]; }
        HdcVector(v)
    }

    /// Bind (XOR) two vectors. Self-inverse: bind(bind(A,B), B) = A.
    pub fn bind(&self, other: &Self) -> Self {
        let mut v = [0u64; 160];
        for i in 0..160 { v[i] = self.0[i] ^ other.0[i]; }
        HdcVector(v)
    }

    /// Cyclic permutation by n positions (word-level shift + bit-level rotation).
    /// Used for position encoding in sequences.
    pub fn permute(&self, n: usize) -> Self {
        let word_shift = n % 160;
        let mut v = [0u64; 160];
        for i in 0..160 {
            v[i] = self.0[(i + word_shift) % 160];
        }
        // Additionally rotate each word by n bits for sub-word permutation
        let bit_shift = (n % 64) as u32;
        if bit_shift > 0 {
            for w in &mut v {
                *w = w.rotate_left(bit_shift);
            }
        }
        HdcVector(v)
    }

    /// Normalized similarity: 1.0 - hamming_distance / 10240.
    /// Returns f64 in [0.0, 1.0]. Use only off-chain (f64 is not
    /// consensus-safe; use raw hamming_distance for on-chain logic).
    pub fn similarity(&self, other: &Self) -> f64 {
        1.0 - (self.hamming_distance(other) as f64 / 10_240.0)
    }

    /// Get the value of bit at index b (0 <= b < 10,240).
    pub fn bit(&self, b: usize) -> u8 {
        let word = b / 64;
        let bit = b % 64;
        ((self.0[word] >> bit) & 1) as u8
    }

    /// Set the value of bit at index b.
    pub fn set_bit(&mut self, b: usize, val: u8) {
        let word = b / 64;
        let bit = b % 64;
        if val == 1 {
            self.0[word] |= 1u64 << bit;
        } else {
            self.0[word] &= !(1u64 << bit);
        }
    }

    /// Compute Hamming distance, dispatching to the fastest available
    /// SIMD implementation at runtime.
    ///
    /// All paths produce identical, bit-exact results. This is safe
    /// for consensus: two validators with different hardware will
    /// compute the same distance for the same pair of vectors.
    pub fn hamming_distance(&self, other: &Self) -> u32 {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx512vpopcntdq") {
                return unsafe { hamming_avx512(&self.0, &other.0) };
            }
            if is_x86_feature_detected!("avx2") {
                return unsafe { hamming_avx2(&self.0, &other.0) };
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // NEON is baseline on aarch64; always available.
            return unsafe { hamming_neon(&self.0, &other.0) };
        }

        // Scalar fallback: works everywhere, always correct.
        hamming_scalar(&self.0, &other.0)
    }
}

/// Scalar fallback. Also serves as the reference implementation
/// that all SIMD paths are tested against.
fn hamming_scalar(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let mut dist = 0u32;
    for i in 0..160 {
        dist += (a[i] ^ b[i]).count_ones();
    }
    dist
}
```

The `is_x86_feature_detected!` macro performs a one-time `CPUID` check (cached after the first call). There is no per-call overhead after initialization.

**Critical for consensus safety**: all paths are pure integer arithmetic. `XOR`, `POPCOUNT`, and `ADD` have no rounding modes, no denormals, no platform-dependent behavior. A distance of 4,217 on an AVX-512 machine is 4,217 on a NEON machine. This is fundamentally different from float-vector search (cosine similarity, L2 distance), where floating-point non-associativity can produce different results depending on SIMD width and reduction order.

---

## Brute-Force Search

### When to Use

Brute-force linear scan is **optimal** for N < ~100K vectors:
- Zero index overhead (no auxiliary data structure)
- Perfect recall (exact results, no approximation)
- Trivially parallelizable (partition the array across threads)
- Zero insertion cost (append to the end)
- Simple, correct, auditable (critical for consensus-critical code)

| N (vectors) | Top-10 latency (AVX2, estimated) | Memory |
|-------------|----------------------------------|--------|
| 1,000 | ~40us | 1.22 MiB |
| 10,000 | ~400us | 12.2 MiB |
| 100,000 | ~4ms | 122 MiB |

### Implementation

```rust
// CONSENSUS-SAFE: Uses u32 Hamming distance (integer only). Sort is by
// (distance, index) — deterministic composite key with canonical tiebreaker.
// Vectors stored in Vec (contiguous, ordered) not HashMap.
use std::collections::BinaryHeap;

pub struct BruteForceIndex {
    keys: Vec<H256>,
    vectors: Vec<HdcVector>,  // Contiguous memory for cache locality
}

impl BruteForceIndex {
    pub fn search(&self, query: &HdcVector, top_k: usize) -> Vec<(H256, u32)> {
        // Max-heap bounded at size top_k. When the heap overflows,
        // pop() removes the element with the LARGEST (dist, index) —
        // i.e., the worst match — keeping the K best (smallest distance)
        // candidates.
        //
        // BinaryHeap is a max-heap by default. (dist, index) pairs
        // compare lexicographically: largest distance first, then
        // largest index as tie-breaker. This means when two candidates
        // have equal distance, the one with the higher index is evicted
        // first, deterministically preferring the lower-indexed vector.
        let mut heap: BinaryHeap<(u32, usize)> = BinaryHeap::with_capacity(top_k + 1);

        for (i, vector) in self.vectors.iter().enumerate() {
            let dist = query.hamming_distance(vector);

            heap.push((dist, i));
            if heap.len() > top_k {
                heap.pop(); // removes largest (dist, i) — the worst match
            }
        }

        // Extract results sorted by (distance, index) ascending.
        let mut results: Vec<_> = heap
            .into_vec()
            .into_iter()
            .map(|(dist, i)| (self.keys[i], dist))
            .collect();
        results.sort_by_key(|&(_, dist)| dist);
        results
    }
}
```

Key optimization: store vectors contiguously (not in a HashMap) for cache locality. Sequential scan of contiguous memory is ~10x faster than random HashMap lookups because the hardware prefetcher detects the linear access pattern and speculatively loads upcoming cache lines.

### Error Handling for Search Operations

All search and index operations use the following error type hierarchy:

```rust
/// Errors that can occur during HDC index operations.
#[derive(Debug)]
enum HdcIndexError {
    /// The index is empty -- no vectors to search against.
    EmptyIndex,
    /// Requested top_k exceeds the number of stored vectors.
    /// Returns results for all stored vectors instead of erroring,
    /// but logs a warning.
    InsufficientVectors { requested: usize, available: usize },
    /// Vector deserialization failed (wrong length or corrupt data).
    InvalidVector(VectorError),
    /// Duplicate key -- a vector with this ID already exists in the index.
    DuplicateKey(H256),
    /// Index capacity exceeded (for fixed-size index implementations).
    CapacityExceeded { capacity: usize },
    /// HNSW graph corruption detected (orphan node, broken edge).
    GraphCorruption(String),
    /// Storage I/O error during persistence operations.
    StorageError(std::io::Error),
}

impl std::fmt::Display for HdcIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HdcIndexError::EmptyIndex =>
                write!(f, "cannot search an empty index"),
            HdcIndexError::InsufficientVectors { requested, available } =>
                write!(f, "requested top-{} but only {} vectors in index",
                       requested, available),
            HdcIndexError::InvalidVector(e) =>
                write!(f, "invalid vector: {}", e),
            HdcIndexError::DuplicateKey(k) =>
                write!(f, "duplicate key: {:?}", k),
            HdcIndexError::CapacityExceeded { capacity } =>
                write!(f, "index capacity {} exceeded", capacity),
            HdcIndexError::GraphCorruption(msg) =>
                write!(f, "HNSW graph corruption: {}", msg),
            HdcIndexError::StorageError(e) =>
                write!(f, "storage error: {}", e),
        }
    }
}

impl std::error::Error for HdcIndexError {}

impl From<std::io::Error> for HdcIndexError {
    fn from(e: std::io::Error) -> Self { HdcIndexError::StorageError(e) }
}

/// Error-aware search that handles edge cases.
impl BruteForceIndex {
    pub fn search_checked(
        &self,
        query: &HdcVector,
        top_k: usize,
    ) -> Result<Vec<(H256, u32)>, HdcIndexError> {
        if self.vectors.is_empty() {
            return Err(HdcIndexError::EmptyIndex);
        }
        let effective_k = if top_k > self.vectors.len() {
            // Log warning but continue with available vectors
            self.vectors.len()
        } else {
            top_k
        };
        Ok(self.search(query, effective_k))
    }

    pub fn insert_checked(
        &mut self,
        key: H256,
        vector: HdcVector,
    ) -> Result<(), HdcIndexError> {
        if self.keys.contains(&key) {
            return Err(HdcIndexError::DuplicateKey(key));
        }
        self.keys.push(key);
        self.vectors.push(vector);
        Ok(())
    }
}
```

---

## HNSW (Hierarchical Navigable Small World)

**Reference**: Malkov & Yashunin (2020). "Efficient and Robust Approximate Nearest Neighbor Using Hierarchical Navigable Small World Graphs." *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 42(4), 824-836. doi:10.1109/TPAMI.2018.2889473

### When to Use

For N > ~100K vectors, brute-force linear scan becomes too slow for real-time queries. HNSW provides:
- **O(log N) query time** with practical constants
- **High recall** (>99% at appropriate `ef_search` settings)
- **Reasonable memory overhead** (~2x raw vector storage for the graph structure)

### Algorithm: Step by Step

HNSW is a multi-layer graph inspired by skip lists. The key insight is that a navigable small-world graph (where greedy routing finds short paths) can be constructed incrementally, and layering multiple such graphs at different densities enables logarithmic search time.

#### Structure

```
Level 3:  o ----------- o                          (very sparse: ~0.024% of nodes)
          |             |
Level 2:  o ---- o ---- o ---- o                   (sparse: ~0.39% of nodes)
          |      |      |      |
Level 1:  o -- o -- o -- o -- o -- o -- o           (medium: ~6.25% of nodes)
          |    |    |    |    |    |    |
Level 0:  o-o-o-o-o-o-o-o-o-o-o-o-o-o-o-o-o-o-o   (all nodes, fully connected)

(Percentages above use the standard HNSW geometric distribution with
M=16: P(level >= k) = (1/M)^k = 16^{-k}.)
```

- **Level 0** contains every vector, with up to `M_max0 = 2*M = 32` connections each.
- **Higher levels** contain geometrically fewer vectors, each with up to `M = 16` connections.
- Connections at higher levels span longer distances in the metric space (like express lanes on a highway).

#### Insertion

When a new vector is inserted:

1. **Assign a random level** drawn from a geometric distribution: `L = floor(-ln(uniform_random) * level_multiplier)` where `level_multiplier = 1/ln(M)`. With M=16, this equals `1/ln(16) ~ 0.361`. Most vectors land at level 0; roughly 1 in M land at each successive higher level.

2. **Find the entry point**: starting from the top layer's entry node, greedily descend through each layer above L, moving to the nearest neighbor at each step.

3. **Insert at each layer from L down to 0**: at each layer, perform a beam search (with width `ef_construction`) to find the `M` nearest neighbors, then add bidirectional edges to them. If any neighbor now has too many connections (more than `M` or `M_max0`), prune its weakest connections.

#### Search

1. **Start at the top layer** from the entry point node.
2. **Greedy descent** (layers above the query's target): at each layer, greedily move to the nearest neighbor. This quickly navigates to the right "neighborhood" in the metric space.
3. **Beam search at lower layers**: at each layer from the entry layer down to level 0, maintain a candidate set of size `ef_search`. Explore neighbors of each candidate, keeping the best `ef_search` candidates seen so far.
4. **Return the top K** from the final candidate set at level 0.

The multi-layer structure ensures that the greedy phase at high layers provides a good starting point, while the beam search at lower layers provides thorough local exploration. Total work is O(log N) distance computations on average.

### Parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `M` | 16 | Max connections per node per layer. Higher values improve recall but increase memory and construction time. 16 is the standard sweet spot from the original paper. |
| `ef_construction` | 200 | Beam width during index construction. Higher values produce a better-connected graph (better recall) at the cost of slower construction. 200 is a robust default. |
| `ef_search` | 50-200 | Beam width during query. **Tunable at query time** -- the key recall/speed knob. ef=50 gives ~95% recall; ef=200 gives ~99.5% recall. |
| `M_max0` | 2*M = 32 | Max connections at level 0. Level 0 sees the most traffic during search, so it benefits from denser connectivity. |
| `level_multiplier` | 1/ln(M) ~ 0.361 | Controls the probability of a vector being inserted at higher layers. Ensures each layer has ~1/M as many nodes as the layer below. |

### Binary Vector Adaptation

Standard HNSW uses float distance (L2 or cosine). For binary vectors:
- Replace the distance function with Hamming distance.
- Everything else (graph structure, insertion, search) stays the same.
- Binary distance is ~10x faster than float L2 distance, so HNSW on binary vectors is proportionally faster at every step.

### Determinism for Consensus (Critical)

When HNSW is used in a consensus-critical context (e.g., all validators must agree on the same search results, or the graph must be identical across nodes), every source of non-determinism must be eliminated. HNSW has several:

**1. Seeded PRNG for level assignment.**
Level assignment uses randomness. Replace `rand::thread_rng()` with a deterministic PRNG seeded from the vector content:

```rust
// CONSENSUS-SAFE: Integer-only level assignment via leading_zeros.
// No f64::ln(), no unseeded RNG. Bit-exact on all platforms.
fn deterministic_level(vector_bytes: &[u8], max_level: usize) -> usize {
    // Seed from keccak256 of the vector bytes.
    // This ensures the same vector always gets the same level,
    // regardless of insertion order or which validator builds the index.
    //
    // Note: seeding from vector content alone means duplicate vectors
    // get the same level. This is acceptable because:
    // (1) the InsightBoard rejects near-duplicates at publication time,
    // (2) even if duplicates exist, same-level assignment is consistent.
    let hash = keccak256(vector_bytes);
    // Since we only need a single u64 of randomness, we can use the hash
    // bytes directly — keccak256 output is already uniformly distributed.
    // A ChaCha20Rng seeded from hash bytes would also work but adds
    // unnecessary complexity for a single draw. Use a PRNG if multiple
    // independent random values per vector are needed in the future.
    let random_bits = u64::from_le_bytes(hash[0..8].try_into().unwrap());
    // CONSENSUS SAFETY: avoid f64::ln() — it is a transcendental function
    // not covered by IEEE 754's bit-exactness guarantees. Different libm
    // implementations can produce different results for the same input.
    //
    // Integer-only geometric distribution for M=16:
    // We need P(level >= k) = (1/M)^k = (1/16)^k = 2^{-4k}.
    // Equivalently: level = floor(leading_zeros(random_bits) / 4).
    // Each "level" consumes 4 random bits (since 16 = 2^4).
    // This is bit-exact on all platforms.
    //
    // For general M that is a power of 2: divide by log2(M).
    // For non-power-of-2 M, use rejection sampling on groups of
    // ceil(log2(M)) bits.
    let level = (random_bits.leading_zeros() / 4) as usize; // log2(16) = 4
    level.min(max_level) // cap at max HNSW level
}
```

**2. Canonical insertion order.**
HNSW is order-dependent: inserting vector A then B produces a different graph than B then A. All validators must insert vectors in the same order. Use the order of on-chain event emission — the sequence in which `VectorStored` events appear in finalized blocks.

**3. Deterministic tie-breaking.**
When two candidates have the same Hamming distance during neighbor selection or search, the algorithm must break ties consistently. Use composite comparison keys:

```rust
/// Comparison key for HNSW candidates.
/// When distances are equal, the lower element_id wins.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct CandidateKey {
    distance: u32,
    element_id: u64,
}
```

**4. Single-threaded construction.**
Parallel graph construction introduces non-determinism from thread scheduling. For consensus-critical indexes, construction must be single-threaded. (Search can still be parallelized, since it is read-only.)

**5. Deterministic neighbor selection.**
When pruning a node's connection list (because it exceeds `M` connections), and multiple candidates tie on distance, always select by lowest `element_id`. This applies to both the simple selection algorithm and the heuristic selection algorithm from the original paper.

### Existing Rust Implementations

| Crate | Notes |
|-------|-------|
| `hnsw_rs` | Pure Rust, actively maintained, generic over distance functions. Good starting point but lacks consensus-determinism guarantees. |
| `instant-distance` | Pure Rust, lightweight, clean API. No built-in binary distance support. |
| `hnswlib-sys` | FFI bindings to the C++ `hnswlib` reference implementation. Fastest option but FFI complexity, harder to audit, and the C++ code uses `std::mt19937` which would need to be replaced for consensus determinism. |

**Recommendation**: custom implementation for full control over determinism, tie-breaking, gas metering, and consensus safety. The algorithm is well-specified in the original paper and not overly complex to implement correctly (~500-800 lines of Rust for the core). The existing crates are useful as references and for testing (compare results against `instant-distance` to validate correctness).

### Vector Deletion from HNSW

When knowledge is purged from the shared substrate (see
[07-shared-substrate.md](./07-shared-substrate.md), PURGED state), the
corresponding vector must be removed from the search index. Deletion from
HNSW is more complex than deletion from a brute-force array (where it is a
simple swap-and-pop) because the HNSW graph has bidirectional edges that must
be maintained.

**Deletion strategy: tombstone + lazy cleanup.**

HNSW does not support efficient in-place deletion without graph corruption.
The standard approach (used in the reference C++ `hnswlib` implementation)
is **tombstone marking**:

1. **Mark the node as deleted.** Set a `deleted` flag on the node. The node
   remains in the graph structure but is excluded from search results. When
   a search traverses a deleted node's neighbors, it skips the deleted node
   but still follows its edges to reach live neighbors.

2. **Do not remove edges immediately.** Removing a deleted node's edges could
   disconnect the graph, breaking the navigability property that HNSW depends
   on. The deleted node's edges serve as "bridge" connections until the graph
   is rebuilt.

3. **Periodic graph compaction.** When the fraction of tombstoned nodes
   exceeds a threshold (default: 20% of total nodes), trigger a full index
   rebuild. The rebuild inserts only live nodes in canonical order (by block
   number of the `VectorStored` event), producing a clean graph with no
   tombstones. This is expensive (O(N log N) for N live nodes) but
   infrequent.

```rust
impl HnswIndex {
    /// Mark a vector as deleted. O(1) operation.
    /// The vector remains in the graph but is excluded from search results.
    pub fn delete(&mut self, id: H256) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.deleted = true;
            self.tombstone_count += 1;
        }

        // Trigger compaction if tombstone ratio exceeds threshold
        if self.tombstone_count > self.nodes.len() / 5 {
            self.compact();
        }
    }

    /// Rebuild the index from scratch, excluding tombstoned nodes.
    /// Called when tombstone_count > 20% of total nodes.
    fn compact(&mut self) {
        let live_nodes: Vec<(H256, HdcVector)> = self.nodes.iter()
            .filter(|(_, node)| !node.deleted)
            .map(|(id, node)| (*id, node.vector.clone()))
            .collect();

        // Sort by canonical insertion order (block number of VectorStored event)
        // to ensure deterministic rebuild across all validators.
        let mut sorted = live_nodes;
        sorted.sort_by_key(|(id, _)| self.insertion_order[id]);

        // Rebuild from scratch
        *self = HnswIndex::new(self.params.clone());
        for (id, vector) in sorted {
            self.insert(id, vector);
        }
    }
}
```

**Consensus implications:** Deletion and compaction must be deterministic
across validators. The `delete()` call is triggered by `VectorDeleted` events
(emitted when someone calls `purge()` on the InsightBoard contract). The
compaction trigger (tombstone ratio > 20%) is deterministic because all
validators process the same sequence of `VectorStored` and `VectorDeleted`
events. The rebuild uses canonical insertion order, so all validators produce
identical post-compaction graphs.

**Note on `hnswlib` (C++ reference):** The `hnswlib` library does support
element deletion via `markDelete()`, which implements the tombstone pattern
described above. However, it does not provide automatic compaction -- the
caller must manage rebuild scheduling. The custom Rust implementation should
include automatic compaction to prevent unbounded tombstone accumulation.

### Brute-Force to HNSW Auto-Switch

The precompile maintains two index implementations and switches between them
based on the number of live (non-tombstoned) vectors:

| Vector Count | Index Type | Rationale |
|-------------|-----------|-----------|
| 0 - 100,000 | Brute-force (linear scan) | Exact results, zero index overhead, simpler code path, ~4ms search at 100K (AVX2) |
| 100,001+ | HNSW (hierarchical navigable small world) | O(log N) search, ~0.3ms at 1M vectors, necessary for gas feasibility |

**Switch-up (brute-force -> HNSW) at 100,001 vectors:**

When the 100,001st vector is inserted, the precompile triggers an index
rebuild:

1. A new HNSW index is constructed from all 100,001 vectors, inserted in
   canonical event-emission order.
2. The brute-force array is retained as a read-only backup until the HNSW
   build completes (the build is single-threaded and synchronous for
   consensus determinism, taking ~2-5 seconds for 100K vectors).
3. Once the HNSW build completes, all subsequent `searchSimilar` calls are
   routed to the HNSW index.
4. The brute-force array is deallocated.

**There is no backfill.** The HNSW index is built from scratch using all
existing vectors at switch time. This is a one-time cost of ~2-5 seconds,
which is acceptable because it occurs at most once in the index's lifetime
(the index only grows from that point). If vectors are deleted and the count
drops below 100,001, the index does NOT switch back to brute-force -- it
remains HNSW. This prevents oscillation at the boundary. Once HNSW, always
HNSW.

**Switch-down is not supported.** This is a deliberate simplification.
Supporting switch-down would require maintaining both index types
simultaneously and handling the brute-force -> HNSW -> brute-force
transition, adding complexity for a scenario that is unlikely in practice
(the vector count dropping by 1 after crossing 100K would require mass
purging). If the system needs to recover from a corrupted HNSW index, the
correct operation is a full index rebuild from the event log, which
reconstructs the appropriate index type based on the final vector count.

---

## Tiered Search Pipeline

### Motivation: Gas Optimization for On-Chain Search

On-chain search must operate within a block's gas limit. Full brute-force Hamming distance at scale is infeasible:

- 1 full comparison = 160 XOR + 160 POPCOUNT + bookkeeping ~ **5,000 gas** (EVM bytecode cost without precompile; with the HDC precompile, `hdc_hamming` costs ~1,500 gas — see doc 07)
- At 100K vectors, brute-force = 100,000 x 5,000 = **500M gas** in EVM bytecode (far exceeds any reasonable block gas limit)

The tiered pipeline exploits a statistical property of high-dimensional binary vectors: most vectors are obviously dissimilar (distance near 5,120), and a cheap partial comparison is sufficient to reject them. Only the small fraction of plausibly-similar vectors need a full comparison.

### Pipeline Architecture

```
                        N candidates
                             |
                             v
                   +-------------------+
                   |     Tier 1        |
                   |  Prefix Distance  |
                   |  Filter (1 word)  |
                   |     ~100 gas      |
                   +-------------------+
                             |
                      ~10% survive (~90% rejected)
                             |
                             v
                   +-------------------+
                   |     Tier 2        |
                   |   Approximate     |
                   |   Hamming         |
                   |   (16/160 words)  |
                   |     ~500 gas      |
                   +-------------------+
                             |
                      ~1% survive (~90% of remainder rejected)
                             |
                             v
                   +-------------------+
                   |     Tier 3        |
                   |   Exact Hamming   |
                   |   (160/160 words) |
                   |     ~5,000 gas    |
                   +-------------------+
                             |
                      Final results (top-K)
```

### Tier 1: Prefix Distance Filter (~100 gas)

> **Note**: This tier is sometimes informally called a "Bloom filter" but it is not a Bloom filter in the standard sense (no hash functions, no bit-array membership test). It is a prefix distance filter: a single-word Hamming distance check that exploits concentration of measure to reject obviously dissimilar vectors cheaply.

Compare only the first 64 bits (1 word) of each vector as a rough filter:

```rust
struct Tier1Filter {
    first_words: Vec<u64>,  // first word of each stored vector
}

impl Tier1Filter {
    /// Reject candidates whose first-word Hamming distance exceeds
    /// a threshold. Based on concentration inequality:
    ///
    /// For i.i.d. bits, the Hamming distance of a 64-bit prefix is
    /// Binomial(64, p), where p is the true bit-error rate across
    /// the full 10,240-bit vector. If the prefix distance exceeds
    /// threshold, the full distance is very likely above the
    /// search threshold too.
    ///
    /// With threshold calibrated for the target distance, achieves
    /// ~90% rejection rate with <1% false negatives.
    fn passes(&self, index: usize, query_first_word: u64, threshold: u32) -> bool {
        let dist = (self.first_words[index] ^ query_first_word).count_ones();
        dist <= threshold
    }
}
```

The key insight is a concentration inequality argument. Each 64-bit word is an i.i.d. sample of the bit-error process. If the bit-error rate is p, the expected distance in 64 bits is 64p, and by Hoeffding's inequality, the observed distance is tightly concentrated around this expectation. A first-word distance of 48 (out of 64) strongly implies a full-vector distance near 10,240 * (48/64) = 7,680, which is far from any reasonable search threshold. Setting the tier-1 threshold appropriately rejects ~90% of candidates at only ~100 gas each.

### Tier 2: Approximate Hamming (~500 gas)

Compare 16 of 160 words (10% of the vector), evenly spaced:

```rust
/// Indices of the 16 sample words: 0, 10, 20, ..., 150.
/// Evenly spaced to avoid correlation between samples.
const SAMPLE_INDICES: [usize; 16] = [
    0, 10, 20, 30, 40, 50, 60, 70,
    80, 90, 100, 110, 120, 130, 140, 150,
];

fn approximate_hamming(a: &[u64; 160], b: &[u64; 160]) -> u32 {
    let mut dist = 0u32;
    for &idx in &SAMPLE_INDICES {
        dist += (a[idx] ^ b[idx]).count_ones();
    }
    dist * 10  // Scale up to estimate full distance
}
```

**Accuracy analysis**: each word is an i.i.d. sample of 64 bits from the full 10,240. The sample distance (across 16 * 64 = 1,024 bits) estimates the full distance scaled by 10. By Hoeffding's inequality applied to the bit-error rate p = d/D (where d is the true Hamming distance and D = 10,240), the probability that the estimated rate p_hat deviates from p by more than epsilon is:

```
P(|p_hat - p| > epsilon) <= 2 * exp(-2 * n * epsilon^2)
```

where n = 1,024 (sampled bits). For epsilon = 0.05 (a 0.05 deviation in the bit-error rate):

```
P < 2 * exp(-2 * 1024 * 0.0025) = 2 * exp(-5.12) < 0.012
```

This means the estimated bit-error rate is within +/-0.05 of the true rate with >98.8% probability. In absolute terms, the distance estimate is within +/-512 bits of the true distance (0.05 * 10,240 = 512). For a typical search threshold around d = 4,000 (bit-error rate ~0.39), this is **+/-12.8% relative error**. For very dissimilar vectors near d = 5,120 (random baseline), it is +/-10% relative error. The error is large enough that Tier 2 should be treated purely as a screening filter -- never as a substitute for exact Tier 3 comparison for final ranking. Its purpose is to reject the ~90% of Tier-1 survivors that are still obviously too distant, and it accomplishes this reliably because the rejection threshold can be set conservatively.

Rejects ~90% of the candidates that survive Tier 1, at ~500 gas each.

### Tier 3: Exact Hamming (~5,000 gas)

Full 160-word comparison. Used only on the ~1% of vectors that pass both Tier 1 and Tier 2. At this stage, candidates are genuine near-neighbors and we need exact distances for correct top-K ranking.

### Gas Savings Analysis

> These figures assume EVM bytecode execution costs (5,000 gas per full comparison). With the HDC precompile (~1,500 gas per `hdc_hamming` call), all values would be ~3.3x lower but the relative savings percentages remain similar.

| Index Size | Brute Force (all Tier 3) | Tiered Pipeline | Gas Saved | Feasible in 1 Block? |
|-----------|------------------------|-----------------|-----------|---------------------|
| 1K | 5M gas | ~600K gas | 88% | Both feasible |
| 10K | 50M gas | ~5.6M gas | 89% | Both feasible |
| 100K | 500M gas | ~51M gas | 90% | Only tiered |
| 1M | 5B gas | ~510M gas | 90% | Neither (need HNSW precompile) |

Breakdown for 100K vectors:
- Tier 1: 100,000 candidates x 100 gas = 10M gas. ~90,000 rejected.
- Tier 2: 10,000 candidates x 500 gas = 5M gas. ~9,000 rejected.
- Tier 3: 1,000 candidates x 5,000 gas = 5M gas. Top-K returned.
- Overhead (heap management, indexing): ~1M gas.
- **Total: ~21M gas**.

> The detailed breakdown above uses 90%/90% rejection rates and arrives at 21M gas, which is the typical case for well-distributed (near-independent) binary vectors. The table conservatively shows ~51M for 100K as a pessimistic bound, corresponding to rejection rates of ~75-80% per tier — expected under non-ideal conditions such as clustered vector distributions where more candidates pass early filtering. Use 21M as the typical case and 51M as the pessimistic bound.

The tiered pipeline makes 100K-vector on-chain search feasible within a single block's gas limit. Beyond 100K, the HNSW precompile is needed (which provides O(log N) search at a fixed gas cost).

---

## Memory Layout for Cache Performance

### Contiguous Array (Best for Brute-Force)

```rust
/// Stores N vectors in contiguous memory for optimal sequential scan.
///
/// Memory layout: [v0_word0, v0_word1, ..., v0_word159, v1_word0, ...]
/// Total size: N * 160 * 8 = N * 1,280 bytes.
struct VectorStore {
    data: Vec<u64>,  // N x 160 words, contiguous
    count: usize,
}

impl VectorStore {
    fn get(&self, index: usize) -> &[u64; 160] {
        let offset = index * 160;
        // SAFETY: bounds checked by construction; alignment guaranteed
        // because u64 is 8-byte aligned and 160 * 8 = 1280 is a
        // multiple of 8.
        unsafe { &*(self.data[offset..offset + 160].as_ptr() as *const [u64; 160]) }
    }
}
```

**Cache analysis**: each vector is 1,280 bytes, which spans 20 cache lines (at 64 bytes per line). For sequential scan, the hardware prefetcher detects the linear pattern and prefetches upcoming lines. Effective cache miss rate: ~1 per vector (the first line; remaining 19 arrive via prefetch before they are needed). At L3 latency of ~40 cycles per miss, the prefetcher transforms what would be 20 * 40 = 800 wasted cycles into ~40 cycles of stall per vector.

### Structure of Arrays (Best for Tiered Search)

```rust
/// Stores vectors in a structure-of-arrays layout optimized for
/// the tiered search pipeline. Each tier accesses only the data
/// it needs, minimizing cache pollution.
#[repr(align(64))]
struct TieredStore {
    /// First word of each vector (for Tier 1).
    /// N * 8 bytes. For N=100K, this is 800 KB -- fits in L2 cache.
    first_words: Vec<u64>,

    /// 16 evenly-spaced words from each vector (for Tier 2).
    /// N * 128 bytes. For N=100K, this is 12.8 MB -- fits in L3 cache.
    sample_words: Vec<[u64; 16]>,

    /// Full vectors (for Tier 3). N * 1,280 bytes.
    /// Only accessed for the ~1% of candidates that survive Tiers 1 and 2.
    full_vectors: Vec<HdcVector>,
}
```

The SoA layout exploits the tiered pipeline's access pattern:
- **Tier 1** scans only `first_words` -- 8 bytes per vector. For N=100K, the entire tier-1 data is 800 KB and fits entirely in L2 cache. The scan touches every cache line exactly once in order.
- **Tier 2** accesses `sample_words` for the ~10% of candidates that survive -- random access, but the working set (10K * 128 bytes = 1.28 MB) still fits in L2/L3.
- **Tier 3** accesses `full_vectors` for the ~1% that survive -- only ~1,000 random accesses into the full vector array, amortized over the entire search.

### Alignment

`#[repr(align(64))]` ensures each vector starts on a cache-line boundary. This has two benefits:
1. **No split loads**: SIMD loads (which are 32 or 64 bytes wide) never straddle two cache lines.
2. **Prefetcher friendliness**: the hardware prefetcher works at cache-line granularity; aligned data produces clean prefetch patterns.

For the `HdcVector` struct specifically:

```rust
/// A 10,240-bit binary hypervector.
///
/// 160 u64 words = 1,280 bytes = 20 cache lines.
/// Cache-line aligned for SIMD access.
#[repr(C, align(64))]
pub struct HdcVector(pub [u64; 160]);
```

The `repr(C)` ensures the layout is exactly what we expect (no field reordering), and `align(64)` pins the start to a cache-line boundary.

---

## Benchmark Data

Realistic latency estimates for top-10 search across different scales and SIMD tiers. All measurements assume single-threaded search on vectors with typical Hamming distance distributions (independent random bits).

### Per-Comparison Latency

> All figures are **estimates** based on instruction throughput analysis, not empirical benchmarks. Real-world numbers will vary with memory subsystem state, compiler version, and surrounding code.

| SIMD Tier | Per-Comparison Latency | Throughput (comparisons/sec) | Hardware |
|-----------|----------------------|------------------------------|----------|
| Scalar (Skylake) | ~120 ns | ~8M/s | Intel Skylake, 4 GHz |
| Scalar (Zen 3) | ~45 ns | ~22M/s | AMD Zen 3, 4 GHz |
| AVX2 (Harley-Seal) | ~40 ns | ~25M/s | Intel Skylake, 4 GHz |
| AVX-512 VPOPCNTDQ | ~7 ns | ~143M/s | Intel Ice Lake, 3.5 GHz |
| NEON (M2) | ~28 ns | ~36M/s | Apple M2, 3.5 GHz |

### Brute-Force Top-10 Search Latency

> Latencies are N * per-comparison time from the table above, plus ~5-10% overhead for heap management. Memory is N * 1,280 bytes (vectors only, not counting key storage).

| N (vectors) | Scalar (Skylake) | AVX2 (Skylake) | AVX-512 (Ice Lake) | NEON (M2) | Memory |
|-------------|-----------------|----------------|-------------------|-----------|--------|
| 1,000 | 120 us | 40 us | 7 us | 28 us | 1.22 MiB |
| 10,000 | 1.2 ms | 400 us | 70 us | 280 us | 12.2 MiB |
| 100,000 | 12 ms | 4 ms | 700 us | 2.8 ms | 122 MiB |
| 1,000,000 | 120 ms | 40 ms | 7 ms | 28 ms | 1.19 GiB |
| 10,000,000 | 1.2 s | 400 ms | 70 ms | 280 ms | 11.9 GiB |

### HNSW Search Latency (ef_search=100, M=16)

> Memory estimates assume ~2x raw vector storage for the graph structure (link lists, metadata, level assignments). Actual overhead depends on implementation; typical HNSW libraries achieve 1.3-2x. Latency estimates assume AVX2-class SIMD for distance computation and warm caches for the graph traversal path. All figures are estimates, not benchmarks.

| N (vectors) | Search Latency | Recall@10 | Memory (vectors + graph) |
|-------------|---------------|-----------|--------------------------|
| 100,000 | ~200 us | ~99% | ~250 MiB |
| 1,000,000 | ~300 us | ~99% | ~2.5 GiB |
| 10,000,000 | ~400 us | ~98% | ~25 GiB |
| 100,000,000 | ~500 us | ~97% | ~250 GiB |

### Crossover Point: Brute-Force vs. HNSW

The crossover depends on hardware and recall requirements. HNSW search at ef_search=100 with binary Hamming distance takes ~200us (from the HNSW table above). The crossover N is where brute-force latency equals ~200us:

| Hardware | Crossover N (approx) | Derivation |
|----------|---------------------|------------|
| Scalar (Skylake) | ~1.5K | 200us / 120ns per comparison |
| AVX2 (Skylake) | ~5K | 200us / 40ns per comparison |
| AVX-512 (Ice Lake) | ~28K | 200us / 7ns per comparison |
| NEON (M2) | ~7K | 200us / 28ns per comparison |

> **Caveat**: HNSW has significant fixed overhead (graph traversal, random memory access patterns, cache misses on neighbor lookups) that makes the real crossover higher than the arithmetic suggests. A practical rule of thumb is **2-5x the numbers above** for the actual crossover where HNSW starts winning consistently.

**Rule of thumb**: HNSW becomes faster than brute-force at approximately **10K-50K vectors** on modern SIMD hardware. Below that, brute-force wins on simplicity, perfect recall, and comparable speed. Above that, HNSW's O(log N) scaling dominates.

For consensus-critical code, prefer brute-force up to the highest feasible N (since it is exact, deterministic by construction, and trivial to audit). Switch to HNSW only when brute-force cannot meet latency targets.

### Benchmark Methodology Specification

All performance claims in this document are instruction-throughput estimates.
Before production deployment, empirical benchmarks must be run using the
following methodology:

```rust
/// Benchmark configuration. All benchmarks use Criterion.rs with
/// the following fixed parameters to ensure reproducibility.
struct BenchConfig {
    /// Number of vectors in the index for each scale tier.
    scales: &'static [usize],         // [1_000, 10_000, 100_000, 1_000_000]
    /// Number of query iterations per measurement.
    query_iterations: usize,          // 10_000
    /// Top-K for search benchmarks.
    top_k: usize,                     // 10
    /// Vector dimension (must match production: 10,240 bits = 160 u64s).
    dimension: usize,                 // 10_240
    /// RNG seed for deterministic vector generation.
    rng_seed: u64,                    // 0xBEEF_CAFE_DEAD_BABE
    /// Warm-up iterations before measurement.
    warmup_iterations: usize,         // 1_000
    /// Measurement time per benchmark group.
    measurement_time_secs: u64,       // 10
}

/// Benchmark groups to run:
///
/// 1. `bench_hamming_scalar` -- single pair Hamming distance, scalar path.
/// 2. `bench_hamming_avx2`   -- single pair, AVX2 Harley-Seal.
/// 3. `bench_hamming_avx512` -- single pair, AVX-512 VPOPCNTDQ.
/// 4. `bench_hamming_neon`   -- single pair, NEON.
/// 5. `bench_brute_top10`    -- brute-force top-10 at each scale.
/// 6. `bench_hnsw_top10`     -- HNSW top-10 at each scale (ef_search=100).
/// 7. `bench_insert`         -- single vector insertion for each index type.
/// 8. `bench_bundle`         -- BundleAccumulator with N={10, 50, 100, 300}.
/// 9. `bench_bind`           -- XOR bind, single pair.
/// 10. `bench_serialize`     -- serialize + deserialize round-trip.
///
/// For each group, report:
///   - Median latency (p50)
///   - p99 latency
///   - Throughput (operations/sec)
///   - Memory RSS delta (via jemalloc stats)
///
/// Hardware specification must be recorded alongside results:
///   - CPU model and base/boost clock
///   - SIMD extensions available
///   - L1/L2/L3 cache sizes
///   - RAM speed and channels
///   - OS and kernel version
///   - Rust compiler version and optimization level (must be --release)
```

**Acceptance criteria for production deployment:**
- Per-comparison Hamming distance must be < 50ns on AVX2 hardware.
- Brute-force top-10 at N=10,000 must be < 500us.
- HNSW top-10 at N=1,000,000 must be < 500us with recall >= 0.98.
- Serialization round-trip must be < 200ns.
- Bundle of 100 vectors must be < 200us.

---

## Scaling Beyond 10M Vectors

For very large indexes (mature agents with years of knowledge):

### Partitioned HNSW

Split the index by knowledge kind or time period:

```rust
struct PartitionedIndex {
    /// CONSENSUS SAFETY: Use BTreeMap, not HashMap, if this index is
    /// used on-chain. HashMap iteration order is non-deterministic in Rust
    /// (randomized per-process). Any operation that iterates partitions
    /// (cross-partition search, serialization, GC) would produce different
    /// orderings on different validators, breaking consensus.
    partitions: BTreeMap<PartitionKey, HnswIndex>,
}

enum PartitionKey {
    ByKind(KnowledgeKind),
    ByEpoch(u64),  // Every N blocks
    ByTier(KnowledgeTier),
}
```

Search scans relevant partitions only. A query about causal links does not need to search episodes.

### Hierarchical Search

1. Cluster vectors using HDC k-medoids (roko-learn already has this)
2. Store cluster centroids as a "meta-index"
3. Search: find nearest clusters, then search within clusters

Two-level hierarchy: O(sqrt(N)) query time at >95% recall.

---

## Hardware Acceleration Landscape

The software SIMD paths described above (AVX2, AVX-512, NEON) are not the only
option. A growing body of HDC hardware accelerator research — spanning FPGA,
ASIC, in-memory computing, and RISC-V ISA extensions — shows that the same
XOR/POPCOUNT/bundle/bind primitives can be pushed into dedicated silicon for
order-of-magnitude gains in throughput and energy efficiency. This section
surveys the landscape and its implications for daeji's precompile design and
dimension selection.

**Reference**: Yu, T. et al., "Hyperdimensional Computing Hardware: Progress,
Trends and Prospects," *Integrated Circuits and Embedded Systems*, 25(8), 2025.

### FPGA Implementations

FPGA-based HDC accelerators exploit the embarrassingly parallel nature of
hypervector arithmetic — every element-wise operation across a D-dimensional
vector can execute simultaneously given sufficient fabric. Key results:

- **HD2FPGA** (2023) provides an automated framework that generates
  deeply-pipelined streaming accelerators from HDC algorithm descriptions.
  It achieves **36.6x speedup** over prior FPGA-based accelerators and
  **2.2x speedup** over GPU-based accelerators for HD clustering tasks.
  The key insight: once the pipeline fills, throughput is near-constant,
  whereas GPU throughput at small batch sizes is constrained by launch latency
  and SIMD underutilization.

- **F5-HD** provides a flexible FPGA framework for refreshing (retraining)
  HDC models, demonstrating that not just inference but the entire
  encode-train-query loop can be accelerated in hardware.

- **FACH** (ASP-DAC 2019) reduces computational complexity on FPGAs by
  exploiting redundancy in HDC operations, achieving further area and
  latency reductions.

- **NysX** (Arockiaraj et al., ACM FPGA 2025, arXiv:2512.08089): the first
  end-to-end FPGA accelerator for Nystrom-based HDC graph classification at
  the edge. Implemented on an AMD Zynq UltraScale+ (ZCU104), NysX achieves
  **6.85x speedup and 169x energy efficiency** over optimized CPU baselines
  (and 4.32x speedup, 314x energy efficiency over GPU baselines), while
  *simultaneously improving accuracy by 3.4%* on TUDataset graph
  classification benchmarks. The accuracy improvement comes from a **hybrid
  uniform + DPP (Determinantal Point Process) landmark sampling** scheme for
  the Nystrom kernel approximation. Standard uniform sampling selects
  landmark nodes randomly, which often produces redundant landmarks that
  oversample dense regions and undersample sparse ones. DPPs are probability
  distributions over subsets that assign higher probability to diverse
  subsets — items with parallel feature vectors are selected together with
  probability zero, while items with orthogonal feature vectors are favored.
  By combining uniform sampling (for coverage) with DPP sampling (for
  diversity), NysX reduces landmark redundancy, which has two effects: (1)
  the Nystrom kernel approximation is more accurate with fewer landmarks,
  improving classification accuracy, and (2) fewer landmarks means fewer
  memory transfers and faster histogram updates, reducing per-inference
  latency by 25-40%. Three additional hardware optimizations complement the
  sampling: a streaming architecture for the Nystrom projection matrix that
  maximizes external memory bandwidth utilization, a minimal-perfect-hash
  lookup engine providing O(1) key-to-index mapping for the codebook with
  minimal on-chip memory overhead, and sparsity-aware SpMV engines with
  static load balancing for irregular graph structures.

  The NysX result is significant because it demonstrates that better
  algorithmic design (DPP sampling) and hardware acceleration are not
  tradeoffs — they compose multiplicatively. The FPGA accelerator does not
  sacrifice accuracy for speed; it achieves both simultaneously. This
  validates the thesis that HDC's simple bitwise primitives map cleanly to
  hardware acceleration without the accuracy compromises typical of
  neural-network quantization.

Compared to the software SIMD paths in our implementation:

| Implementation | Per-comparison (10K-bit) | Throughput | Energy |
|---|---|---|---|
| Scalar x86 (Skylake) | ~120 ns (estimated) | ~8M/s | Baseline |
| AVX-512 VPOPCNTDQ | ~7 ns (estimated) | ~143M/s | ~0.3x baseline |
| FPGA (HD2FPGA) | ~1-2 ns (estimated) | ~500M+/s | ~0.01-0.05x baseline |

FPGA accelerators achieve their advantage through massive parallelism (processing
all 10,240 bits simultaneously in a single clock cycle) and elimination of
instruction fetch/decode overhead. The tradeoff is flexibility — changing the
algorithm requires resynthesizing the bitstream, which takes minutes to hours.

### The AXI BSC Accelerator — Precompile Substrate Candidate

**Reference**: Martino, R. et al., "A General-Purpose AXI Plug-and-Play
Hyperdimensional Computing Accelerator," *MDPI Electronics*, 15(2):489, 2026.

The most directly relevant hardware design for daeji's on-chain precompile is
the AXI BSC accelerator — an open-source, general-purpose HDC accelerator IP
that implements the complete Binary Spatter Code framework as a standalone,
host-agnostic AXI4 peripheral. Key architectural features:

- **Full BSC primitive set.** The accelerator implements bind (XOR), bundle
  (majority vote), permute (circular shift), and Hamming distance as
  hardware operations — the exact four primitives our precompile exposes.

- **AXI4 integration.** AMBA AXI4-compliant with an AXI4-Lite control plane
  for configuration and DMA-driven AXI4-Stream datapaths for bulk vector
  transfer. This is the standard SoC interconnect — it plugs directly into
  any ARM, RISC-V, or custom processor system without bespoke interface logic.

- **Banked scratchpad memory.** Multiple independent SPM banks allow
  simultaneous load, store, and BSC operations targeting different banks.
  The number of banks is parameterized by the SIMD width, matching the
  AXI-Stream interface width to maximize bandwidth.

- **Synthesis-time configurability.** The SIMD parallelism parameter scales
  from narrow (resource-constrained edge devices) to wide (up to SIMD=32),
  enabling deployment on platforms ranging from small Zynq-7020 FPGAs to
  larger devices.

- **Runtime programmability.** Encoding strategies and training procedures
  can be changed by recompiling the host application without regenerating
  the FPGA bitstream. This is critical for a precompile: the accelerator
  hardware stays fixed, while the operations it executes are determined by
  the smart contract calling the precompile.

- **Open source.** The complete RTL and software stack are released at
  https://github.com/RoMartino/AXI-HDC-Accelerator — enabling direct
  evaluation for precompile integration.

**Precompile relevance.** A daeji validator running on an FPGA-equipped node
(e.g., AWS F1 instances, or purpose-built validator hardware) could offload
the HDC precompile's hot path — `searchSimilar` over large indexes — to an
AXI BSC accelerator instance. The gas cost model would remain the same (gas
is a measure of computational work, not wall-clock time), but validators with
hardware acceleration would have more headroom to process complex HDC
transactions within block time limits. This is analogous to how Ethereum
validators benefit from hardware AES-NI for keccak256 hashing without
changing the gas schedule.

### RISC-V ISA Extensions

A parallel approach avoids a separate accelerator entirely by extending the
processor's instruction set. Two notable designs:

- **RISC-V ISA Extension for HDC** (RISC-V Summit Europe, 2025): the first
  dedicated ISA extension for BSC arithmetic, integrated into the
  Klessydra-T03 RISC-V core. Custom instructions for bind, bundle, permute,
  and Hamming distance execute in the ALU pipeline alongside standard
  instructions, acting as a tightly-coupled coprocessor. The architecture is
  configurable at synthesis time (hardware parallelism, memory size, supported
  operations) and at runtime (hypervector dimension and operation count via
  CSRs).

- **RISC-HD**: a lightweight RISC-V processor (extended RI5CY core) optimized
  specifically for HDC inference workloads.

- **Domain-Specific HDC RISC-V (Wasif et al., IEEE TCAS-I, 2025)**: a
  fabricated 22nm RISC-V chip extended with custom HDC instructions and a
  vector processing unit for edge-AI *training* (not just inference). The
  chip implements **FixedHD**, a 16-bit fixed-point HDC model that achieves
  accuracy comparable to floating-point while dramatically reducing
  computational complexity. Custom instructions accelerate the core HDC
  training pipeline — encoding (random-projection matrix multiplication from
  n-feature space to d-dimensional hypervector space), bundling (hypervector
  accumulation for class model construction), and cosine similarity
  (retraining via iterative cosine-similarity-based model refinement,
  converging in only 10 iterations). The extended RISC-V achieves **4x
  speedup** over the baseline processor, operates at up to 120 MHz, and
  consumes only **24.65 uJ per training sample** — a record value that is
  10-100x more efficient than comparable chips. The chip contains ~10 million
  transistors on 1 mm^2 of silicon (vs. ~200 billion for an NVIDIA GPU die),
  demonstrating that HDC's computational simplicity translates directly into
  radical hardware efficiency.

For daeji, RISC-V extensions are relevant to validator hardware roadmaps.
As RISC-V gains traction in blockchain validator nodes (driven by open-source
hardware and supply-chain sovereignty considerations), HDC ISA extensions
could make the precompile's hot path a first-class citizen of the processor
itself — no separate accelerator, no DMA transfers, just native instructions.

### In-Memory Computing

The most radical approach eliminates data movement entirely by performing HDC
operations where the data already resides — in memory:

- **FeFET-based content-addressable memory** has demonstrated multi-bit,
  array-level CAM operations achieving **826x energy improvement** and
  **30x latency reduction** compared to GPU implementations.

- **Memristor-based architectures** perform approximate associative search
  natively in the memory array, where the analog properties of resistive
  elements compute Hamming distance as a side-effect of the read operation.

These approaches are 3-5 years from production readiness, but they represent
the theoretical end-state for HDC hardware: when the memory *is* the
accelerator, the von Neumann bottleneck disappears entirely, and search
throughput scales linearly with memory capacity.

### Heim: Dimension Optimization — Is D=10,240 Justified?

**Reference**: Yi, P. & Achour, S., "Hardware-Aware Static Optimization of
Hyperdimensional Computations," *Proc. ACM on Programming Languages*
(OOPSLA), Vol. 7, Article 222, 2023. DOI: 10.1145/3622797.

Heim is a static analysis framework that analytically derives the *minimum*
hypervector dimension needed to achieve a target accuracy for a given HDC
computation, eliminating the need for expensive dynamic dimension tuning.

**Method.** Heim constructs analytical distance distribution models
parameterized by dimension D, hardware error rates, and the algebraic
structure of the HDC computation (how many binds, bundles, and permutes are
composed). From these distributions, it computes the expected
match/non-match separation and uses binary search to find the smallest D
where the expected accuracy meets the target (e.g., 99%).

**Key result.** Across 25 benchmarks with 99% accuracy targets, Heim achieves:
- **1.15x-7.14x dimension reductions** compared to dynamically-tuned
  iso-accuracy executions
- **57.8x reduction** from unoptimized 10,000-bit baselines for simple tasks
  (e.g., knowledge graph queries optimized to 173 bits)
- Median accuracy of 99.2%-100.0% after optimization

**What this means for D=10,240.** The answer depends on the algebraic
complexity of the HDC computation:

- **For simple tasks** (single-level bind-and-query, knowledge graph lookups):
  10,240 is dramatically oversized. Heim shows that 100-500 bits suffice for
  99% accuracy. This is expected — simple computations have clean
  match/non-match separation that does not require high dimensionality to
  resolve.

- **For complex tasks** (multi-level encoding with nested binds and bundles,
  language classification, EMG signal processing): the standard literature
  uses D=10,000 and achieves competitive accuracy with SVMs and lightweight
  neural networks. Here, 10,240 is in the right range — empirically validated
  across dozens of published benchmarks on language, gesture, and sensor
  classification.

- **For our specific use case** (knowledge encoding with trigram text hashing,
  role-filler binding, bundle accumulation, multi-kind type tagging, and
  on-chain search over a shared substrate): the encoding pipeline has moderate
  algebraic depth. Each knowledge vector passes through trigram hashing
  (3 nested permute-binds per trigram, bundled across all trigrams), then
  role-filler binding (one bind per field), then type tagging (one more bind).
  This is more complex than a simple knowledge graph query but less complex
  than the deepest multi-modal encodings in the literature.

**Pragmatic assessment.** D=10,240 is a defensible choice for a production
system that must handle diverse encoding patterns without per-task dimension
tuning. It provides comfortable margin for the most complex encodings while
remaining efficient for simpler ones (the overhead of unused dimensions is
negligible — the extra bits are just more XOR/POPCOUNT work, which is already
sub-microsecond). However, Heim's analysis suggests that a future optimization
pass could introduce **adaptive dimensionality** — using shorter vectors for
simple encodings and full-width vectors only for complex ones — potentially
reducing storage and gas costs by 2-4x for common operations without
sacrificing accuracy.

The dimension of 10,240 (= 10 * 1,024 = 160 x 64-bit words) also has
practical advantages: it aligns cleanly with 64-bit word boundaries, AVX-512
register widths (512 bits = 8 words, 160/8 = 20 iterations), and cache-line
sizes (64 bytes = 8 words, 160 words = 1,280 bytes = 20 cache lines).
These alignment properties matter more for gas-metered on-chain execution
than raw dimension optimality.

### Implications for the Roko System

The convergence of FPGA accelerators (NysX: 169x energy efficiency, 6.85x
speedup), fabricated RISC-V HDC chips (Wasif et al.: 24.65 uJ/sample, 4x
speedup), open-source AXI BSC accelerator IP (Martino et al.), and RISC-V
ISA extensions for HDC primitives (Klessydra-T03) paints a clear picture:
HDC operations can be accelerated in hardware at extremely low energy cost,
across multiple hardware paradigms, without sacrificing accuracy.

This has three concrete implications for daeji:

1. **EVM precompiles are software-defined hardware acceleration.** The HDC
   precompile at 0x09 exposes exactly the same primitives (bind, bundle,
   permute, Hamming distance) that these hardware accelerators implement in
   silicon. A precompile is the EVM's mechanism for making an operation
   cheaper than its Solidity implementation — just as the ECRECOVER
   precompile (0x01) makes signature verification affordable by executing
   native code instead of EVM bytecode. The hardware accelerator results
   validate that these primitives are worth promoting to first-class
   operations: they are the same primitives that silicon designers chose to
   put on chip, confirming their status as fundamental operations rather
   than arbitrary API choices.

2. **Validator hardware roadmap.** As RISC-V gains adoption in blockchain
   infrastructure (driven by open-source hardware and supply-chain
   sovereignty), validators could run on processors with native HDC
   instructions. A validator on a RISC-V chip with the Klessydra HDC
   extension would execute the precompile's hot path — `searchSimilar`
   over 100K+ vectors — as native processor instructions rather than
   library calls. The gas schedule would not change (gas measures
   computational work, not wall-clock time), but validators would have
   substantial headroom within block time limits. The Wasif et al. chip
   demonstrates that this is not speculative: a fabricated 22nm chip with
   10M transistors already achieves 4x speedup on HDC workloads at
   24.65 uJ per training sample.

3. **Edge agent deployment.** Agents running on resource-constrained
   devices (IoT sensors, drones, smartwatches) could perform local HDC
   operations — encoding, similarity search, knowledge retrieval — at
   microjoule-level energy cost. The NysX result shows that even complex
   HDC workloads like graph classification can run on edge FPGAs with 169x
   better energy efficiency than CPU baselines. For an agent architecture
   where the local cognitive loop (perceive -> encode -> search -> assemble
   context) runs continuously, the difference between millijoule and
   microjoule operations determines whether the agent can run on battery
   power for hours or for weeks.
