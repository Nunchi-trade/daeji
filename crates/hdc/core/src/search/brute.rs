//! BruteForceIndex -- linear scan with top-K heap.
//!
//! Optimal for N < ~100K. O(N log K) search complexity.

use std::collections::BinaryHeap;

use super::{H256, HdcIndexError, SearchIndex, SearchResult, simd::hamming_distance};
use crate::HdcVector;

/// Linear-scan index with top-K max-heap. Optimal for N < ~100K.
pub struct BruteForceIndex {
    /// Keys in insertion order. keys[i] corresponds to vectors[i].
    keys: Vec<H256>,
    /// Contiguous vector storage for cache locality.
    vectors: Vec<HdcVector>,
}

impl std::fmt::Debug for BruteForceIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BruteForceIndex").field("len", &self.keys.len()).finish()
    }
}

impl Default for BruteForceIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl BruteForceIndex {
    /// Create an empty brute-force index.
    pub const fn new() -> Self {
        Self { keys: Vec::new(), vectors: Vec::new() }
    }

    /// Create a brute-force index with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self { keys: Vec::with_capacity(cap), vectors: Vec::with_capacity(cap) }
    }

    /// Drain all entries. Returns (keys, vectors) in insertion order.
    pub(super) fn drain(&mut self) -> (Vec<H256>, Vec<HdcVector>) {
        let keys = std::mem::take(&mut self.keys);
        let vectors = std::mem::take(&mut self.vectors);
        (keys, vectors)
    }
}

impl SearchIndex for BruteForceIndex {
    fn insert(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
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
        // pop() removes the element with the LARGEST (dist, idx) -- the worst
        // match -- keeping the K best.
        let mut heap: BinaryHeap<(u32, usize)> = BinaryHeap::with_capacity(effective_k + 1);

        for (i, vector) in self.vectors.iter().enumerate() {
            let dist = hamming_distance(query, vector);
            heap.push((dist, i));
            if heap.len() > effective_k {
                heap.pop();
            }
        }

        let mut results: Vec<(H256, u32)> =
            heap.into_vec().into_iter().map(|(dist, i)| (self.keys[i], dist)).collect();
        results.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        Ok(results)
    }

    fn len(&self) -> usize {
        self.keys.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(i: u64) -> H256 {
        let mut k = [0u8; 32];
        k[0..8].copy_from_slice(&i.to_le_bytes());
        k
    }

    #[test]
    fn insert_search_roundtrip() {
        let mut index = BruteForceIndex::new();
        let v1 = HdcVector::random(1);
        let k1 = make_key(1);
        index.insert(k1, v1.clone()).unwrap();

        let results = index.search(&v1, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, k1);
        assert_eq!(results[0].1, 0); // self-distance = 0
    }

    #[test]
    fn duplicate_key_rejected() {
        let mut index = BruteForceIndex::new();
        let k = make_key(1);
        index.insert(k, HdcVector::random(1)).unwrap();
        assert!(index.insert(k, HdcVector::random(2)).is_err());
    }

    #[test]
    fn empty_index_error() {
        let index = BruteForceIndex::new();
        assert!(index.search(&HdcVector::default(), 10).is_err());
    }

    #[test]
    fn delete_works() {
        let mut index = BruteForceIndex::new();
        let k = make_key(1);
        index.insert(k, HdcVector::random(1)).unwrap();
        assert_eq!(index.len(), 1);
        assert!(index.delete(&k));
        assert_eq!(index.len(), 0);
        assert!(!index.delete(&k)); // already deleted
    }

    #[test]
    fn top_k_ordering() {
        let mut index = BruteForceIndex::new();
        for i in 0..100u64 {
            index.insert(make_key(i), HdcVector::random(i)).unwrap();
        }

        let query = HdcVector::random(0);
        let results = index.search(&query, 5).unwrap();
        assert_eq!(results.len(), 5);
        // Verify sorted by distance ascending
        for w in results.windows(2) {
            assert!(w[0].1 <= w[1].1);
        }
        // First result should be the query itself (distance 0)
        assert_eq!(results[0].1, 0);
    }

    #[test]
    fn top_k_larger_than_index() {
        let mut index = BruteForceIndex::new();
        for i in 0..3u64 {
            index.insert(make_key(i), HdcVector::random(i)).unwrap();
        }
        let results = index.search(&HdcVector::random(0), 10).unwrap();
        assert_eq!(results.len(), 3); // only 3 available
    }

    #[test]
    fn deterministic_results() {
        // Same index, same query -> same results every time
        let mut index = BruteForceIndex::new();
        for i in 0..50u64 {
            index.insert(make_key(i), HdcVector::random(i)).unwrap();
        }

        let query = HdcVector::random(999);
        let r1 = index.search(&query, 10).unwrap();
        let r2 = index.search(&query, 10).unwrap();
        assert_eq!(r1, r2);
    }
}
