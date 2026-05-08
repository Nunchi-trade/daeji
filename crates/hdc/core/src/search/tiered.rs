//! TieredSearchPipeline -- gas-optimized on-chain search.
//!
//! Three tiers of progressively more expensive filtering, each rejecting
//! ~90% of remaining candidates. Saves ~96% gas vs brute-force at 100K vectors.

use std::collections::BinaryHeap;

use super::{H256, simd::hamming_distance};
use crate::HdcVector;

/// Evenly-spaced sample indices for Tier 2.
/// 16 of 160 words (10%), spaced 10 apart.
const SAMPLE_INDICES: [usize; 16] =
    [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150];

/// Structure-of-arrays layout for tiered access patterns.
/// Each tier touches only the data it needs, minimizing cache pollution.
#[derive(Debug)]
pub struct TieredSearchPipeline {
    keys: Vec<H256>,
    /// Tier 1: first u64 word of each vector. N * 8 bytes.
    first_words: Vec<u64>,
    /// Tier 2: 16 evenly-spaced words from each vector. N * 128 bytes.
    sample_words: Vec<[u64; 16]>,
    /// Tier 3: full vectors. N * 1280 bytes.
    full_vectors: Vec<HdcVector>,
}

impl Default for TieredSearchPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl TieredSearchPipeline {
    /// Create a new empty tiered search pipeline.
    pub const fn new() -> Self {
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
    #[inline]
    fn tier1_passes(&self, index: usize, query_first_word: u64, threshold: u32) -> bool {
        let dist = (self.first_words[index] ^ query_first_word).count_ones();
        dist <= threshold
    }

    /// Tier 2: Approximate Hamming distance using 16 sampled words.
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
    #[inline]
    fn tier3_distance(&self, index: usize, query: &HdcVector) -> u32 {
        hamming_distance(&self.full_vectors[index], query)
    }

    /// Number of vectors in the pipeline.
    pub const fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the pipeline is empty.
    pub const fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Run the full tiered pipeline. Returns top_k results sorted by
    /// (distance ASC, key ASC).
    ///
    /// tier1_threshold: max first-word distance to pass Tier 1.
    /// tier2_threshold: max estimated full distance to pass Tier 2.
    pub fn search(
        &self,
        query: &HdcVector,
        top_k: usize,
        tier1_threshold: u32,
        tier2_threshold: u32,
    ) -> Vec<(H256, u32)> {
        let query_first_word = query.0[0];

        let mut query_samples = [0u64; 16];
        for (s, &idx) in SAMPLE_INDICES.iter().enumerate() {
            query_samples[s] = query.0[idx];
        }

        // Max-heap for top-k tracking
        let mut heap: BinaryHeap<(u32, usize)> = BinaryHeap::with_capacity(top_k + 1);

        for i in 0..self.keys.len() {
            // Tier 1: ~100 gas per candidate
            if !self.tier1_passes(i, query_first_word, tier1_threshold) {
                continue;
            }

            // Tier 2: ~500 gas per candidate
            let approx_dist = self.tier2_distance(i, &query_samples);
            if approx_dist > tier2_threshold {
                continue;
            }

            // Tier 3: ~5,000 gas per candidate (only ~1% reach here)
            let exact_dist = self.tier3_distance(i, query);

            heap.push((exact_dist, i));
            if heap.len() > top_k {
                heap.pop();
            }
        }

        let mut results: Vec<(H256, u32)> =
            heap.into_vec().into_iter().map(|(dist, i)| (self.keys[i], dist)).collect();
        results.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::{SearchIndex, brute::BruteForceIndex},
        *,
    };

    fn make_key(i: u64) -> H256 {
        let mut k = [0u8; 32];
        k[0..8].copy_from_slice(&i.to_le_bytes());
        k
    }

    #[test]
    fn basic_insert_and_search() {
        let mut pipeline = TieredSearchPipeline::new();
        let v = HdcVector::random(1);
        let k = make_key(1);
        pipeline.insert(k, v.clone());

        // Use very permissive thresholds to find everything
        let results = pipeline.search(&v, 1, 64, 10_240);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, k);
        assert_eq!(results[0].1, 0);
    }

    #[test]
    fn tiered_matches_brute_force_with_permissive_thresholds() {
        // With high enough thresholds (no filtering), tiered should return
        // the same results as brute-force.
        let mut bf = BruteForceIndex::new();
        let mut tiered = TieredSearchPipeline::new();

        for i in 0..100u64 {
            let k = make_key(i);
            let v = HdcVector::random(i);
            bf.insert(k, v.clone()).unwrap();
            tiered.insert(k, v);
        }

        let query = HdcVector::random(0);
        let bf_results = bf.search(&query, 5).unwrap();
        // tier1=64 (max possible for a single u64 word), tier2=10240 (max possible)
        let tiered_results = tiered.search(&query, 5, 64, 10_240);

        assert_eq!(
            bf_results, tiered_results,
            "Tiered with permissive thresholds should match brute-force exactly"
        );
    }

    #[test]
    fn tiered_filters_correctly() {
        let mut pipeline = TieredSearchPipeline::new();

        // Insert vectors with known distances
        for i in 0..50u64 {
            pipeline.insert(make_key(i), HdcVector::random(i));
        }

        let query = HdcVector::random(0);

        // Search with strict thresholds -- should find fewer results
        let strict_results = pipeline.search(&query, 50, 10, 2000);
        // Search with permissive thresholds -- should find more
        let permissive_results = pipeline.search(&query, 50, 64, 10_240);

        // Strict should find <= permissive (may filter some)
        assert!(strict_results.len() <= permissive_results.len());
    }

    #[test]
    fn tiered_results_are_sorted() {
        let mut pipeline = TieredSearchPipeline::new();
        for i in 0..100u64 {
            pipeline.insert(make_key(i), HdcVector::random(i));
        }

        let query = HdcVector::random(999);
        let results = pipeline.search(&query, 10, 64, 10_240);

        for w in results.windows(2) {
            assert!(w[0].1 <= w[1].1, "Results not sorted by distance");
        }
    }

    #[test]
    fn tiered_deterministic() {
        let mut pipeline = TieredSearchPipeline::new();
        for i in 0..50u64 {
            pipeline.insert(make_key(i), HdcVector::random(i));
        }

        let query = HdcVector::random(999);
        let r1 = pipeline.search(&query, 5, 64, 10_240);
        let r2 = pipeline.search(&query, 5, 64, 10_240);
        assert_eq!(r1, r2);
    }

    #[test]
    fn empty_pipeline() {
        let pipeline = TieredSearchPipeline::new();
        let results = pipeline.search(&HdcVector::default(), 10, 64, 10_240);
        assert!(results.is_empty());
    }
}
