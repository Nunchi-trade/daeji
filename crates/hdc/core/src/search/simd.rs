//! SIMD-accelerated Hamming distance computation.
//!
//! All paths produce identical, bit-exact results (pure integer arithmetic:
//! XOR + popcount + add). Safe for consensus.

use crate::{HdcVector, constants::WORDS};

/// Reference scalar implementation. Pure integer arithmetic. Bit-exact on all
/// platforms. Also used as the oracle for testing SIMD paths.
#[inline]
pub(super) fn hamming_scalar(a: &[u64; WORDS], b: &[u64; WORDS]) -> u32 {
    let mut dist = 0u32;
    for i in 0..WORDS {
        dist += (a[i] ^ b[i]).count_ones();
    }
    dist
}

// ─── AVX2 Harley-Seal ────────────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Byte-level popcount of a 256-bit register via the vpshufb lookup trick.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn popcount_mm256(v: __m256i) -> __m256i {
    unsafe {
        let lookup = _mm256_setr_epi8(
            0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, 0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2,
            3, 3, 4,
        );
        let low_mask = _mm256_set1_epi8(0x0f);
        let lo = _mm256_and_si256(v, low_mask);
        let hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), low_mask);
        let popcnt_lo = _mm256_shuffle_epi8(lookup, lo);
        let popcnt_hi = _mm256_shuffle_epi8(lookup, hi);
        _mm256_add_epi8(popcnt_lo, popcnt_hi)
    }
}

/// Carry-save adder: returns (sum, carry) = (a XOR b XOR c, majority(a,b,c)).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn csa_256(a: __m256i, b: __m256i, c: __m256i) -> (__m256i, __m256i) {
    unsafe {
        let u = _mm256_xor_si256(a, b);
        let sum = _mm256_xor_si256(u, c);
        let carry = _mm256_or_si256(_mm256_and_si256(a, b), _mm256_and_si256(u, c));
        (sum, carry)
    }
}

/// Hamming distance using AVX2 Harley-Seal.
/// 160 u64s = 1280 bytes = 40 x 256-bit chunks, processed in groups of 4.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn hamming_avx2(a: &[u64; WORDS], b: &[u64; WORDS]) -> u32 {
    unsafe {
        let a_ptr = a.as_ptr() as *const __m256i;
        let b_ptr = b.as_ptr() as *const __m256i;

        let mut total = _mm256_setzero_si256();
        let mut ones = _mm256_setzero_si256();
        let mut twos = _mm256_setzero_si256();

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

            let (new_ones, carry0) = csa_256(ones, d0, d1);
            let (new_ones2, carry1) = csa_256(new_ones, d2, d3);
            ones = new_ones2;
            let (new_twos, carry2) = csa_256(twos, carry0, carry1);
            twos = new_twos;

            total = _mm256_add_epi64(
                total,
                _mm256_sad_epu8(popcount_mm256(carry2), _mm256_setzero_si256()),
            );
        }

        // Apply weights
        total = _mm256_slli_epi64(total, 2); // total *= 4

        total = _mm256_add_epi64(
            total,
            _mm256_slli_epi64(_mm256_sad_epu8(popcount_mm256(twos), _mm256_setzero_si256()), 1),
        );

        total =
            _mm256_add_epi64(total, _mm256_sad_epu8(popcount_mm256(ones), _mm256_setzero_si256()));

        // Horizontal sum of 4 x u64 lanes -> single u32
        let lo = _mm256_castsi256_si128(total);
        let hi = _mm256_extracti128_si256(total, 1);
        let sum128 = _mm_add_epi64(lo, hi);
        let upper = _mm_srli_si128(sum128, 8);
        let final_sum = _mm_add_epi64(sum128, upper);
        _mm_cvtsi128_si64(final_sum) as u32
    }
}

// ─── AVX-512 VPOPCNTDQ ──────────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512vpopcntdq")]
pub unsafe fn hamming_avx512(a: &[u64; WORDS], b: &[u64; WORDS]) -> u32 {
    unsafe {
        let a_ptr = a.as_ptr() as *const __m512i;
        let b_ptr = b.as_ptr() as *const __m512i;

        let mut acc = _mm512_setzero_si512();

        for i in 0..20 {
            let va = _mm512_loadu_si512(a_ptr.add(i) as *const __m512i);
            let vb = _mm512_loadu_si512(b_ptr.add(i) as *const __m512i);
            let xored = _mm512_xor_si512(va, vb);
            let popcnt = _mm512_popcnt_epi64(xored);
            acc = _mm512_add_epi64(acc, popcnt);
        }

        _mm512_reduce_add_epi64(acc) as u32
    }
}

// ─── ARM NEON ────────────────────────────────────────────────────────────────

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

#[cfg(target_arch = "aarch64")]
pub(super) unsafe fn hamming_neon(a: &[u64; WORDS], b: &[u64; WORDS]) -> u32 {
    unsafe {
        let a_ptr = a.as_ptr() as *const u8;
        let b_ptr = b.as_ptr() as *const u8;

        let mut total: u64 = 0;
        let mut i = 0usize;

        // 1280 bytes / 16 bytes per 128-bit register = 80 iterations
        while i < 80 {
            // Process blocks of at most 31 iterations to avoid u8 overflow
            // (31 * 8 = 248 max per byte, fits in u8 max 255)
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
}

// ─── Runtime Dispatch ────────────────────────────────────────────────────────

/// Compute Hamming distance, dispatching to the fastest available SIMD path.
///
/// All paths produce identical, bit-exact results. Safe for consensus.
///
/// Dispatch order (x86_64): AVX-512 VPOPCNTDQ > AVX2 > scalar.
/// Dispatch order (aarch64): NEON (always available).
/// Fallback: scalar.
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

    #[allow(unreachable_code)]
    hamming_scalar(&a.0, &b.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hamming_zero_vectors() {
        let a = HdcVector::default();
        let b = HdcVector::default();
        assert_eq!(hamming_distance(&a, &b), 0);
    }

    #[test]
    fn hamming_complementary_vectors() {
        let a = HdcVector::default(); // all zeros
        let mut b_words = [0u64; WORDS];
        for w in b_words.iter_mut() {
            *w = !0u64;
        }
        let b = HdcVector(b_words);
        assert_eq!(hamming_distance(&a, &b), 10_240);
    }

    #[test]
    fn hamming_self_distance() {
        let a = HdcVector::random(42);
        assert_eq!(hamming_distance(&a, &a), 0);
    }

    #[test]
    fn hamming_symmetry() {
        let a = HdcVector::random(1);
        let b = HdcVector::random(2);
        assert_eq!(hamming_distance(&a, &b), hamming_distance(&b, &a));
    }

    #[test]
    fn hamming_simd_matches_scalar() {
        for seed in 0..100u64 {
            let a = HdcVector::random(seed * 2);
            let b = HdcVector::random(seed * 2 + 1);
            let scalar = hamming_scalar(&a.0, &b.0);
            let dispatched = hamming_distance(&a, &b);
            assert_eq!(scalar, dispatched, "mismatch at seed {seed}");
        }
    }
}
