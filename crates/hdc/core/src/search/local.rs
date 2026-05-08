//! LocalIndex -- auto-switching between brute-force and HNSW.
//!
//! Starts as brute-force, converts to HNSW when vector count exceeds threshold.
//! Once switched to HNSW, never switches back (prevents oscillation).

use super::{
    H256, HdcIndexError, SearchIndex, SearchResult, brute::BruteForceIndex, hnsw::HnswIndex,
};
use crate::HdcVector;

/// Threshold at which to switch from brute-force to HNSW.
const SWITCH_THRESHOLD: usize = 10_000;

/// Auto-switching index. Starts as brute-force, converts to HNSW when
/// vector count exceeds SWITCH_THRESHOLD.
pub enum LocalIndex {
    /// Brute-force linear scan (used when entry count <= SWITCH_THRESHOLD).
    BruteForce(BruteForceIndex),
    /// HNSW approximate nearest neighbor (used after threshold is crossed).
    Hnsw(HnswIndex),
}

impl std::fmt::Debug for LocalIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BruteForce(bf) => f.debug_tuple("LocalIndex::BruteForce").field(bf).finish(),
            Self::Hnsw(hnsw) => f.debug_tuple("LocalIndex::Hnsw").field(hnsw).finish(),
        }
    }
}

impl Default for LocalIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalIndex {
    /// Create a new auto-switching index (starts as brute-force).
    pub const fn new() -> Self {
        Self::BruteForce(BruteForceIndex::new())
    }

    /// Upgrade from brute-force to HNSW. Drains all vectors and re-inserts
    /// them in original insertion order (canonical for determinism).
    fn upgrade(&mut self) {
        if let Self::BruteForce(bf) = self {
            let mut hnsw = HnswIndex::new();
            let (keys, vectors) = bf.drain();

            for (key, vector) in keys.into_iter().zip(vectors) {
                let _ = hnsw.insert(key, vector);
            }

            *self = Self::Hnsw(hnsw);
        }
    }
}

impl SearchIndex for LocalIndex {
    fn insert(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
        match self {
            Self::BruteForce(bf) => {
                bf.insert(key, vector)?;
                if bf.len() > SWITCH_THRESHOLD {
                    self.upgrade();
                }
                Ok(())
            }
            Self::Hnsw(hnsw) => hnsw.insert(key, vector),
        }
    }

    fn delete(&mut self, key: &H256) -> bool {
        match self {
            Self::BruteForce(bf) => bf.delete(key),
            Self::Hnsw(hnsw) => hnsw.delete(key),
        }
    }

    fn search(&self, query: &HdcVector, top_k: usize) -> Result<SearchResult, HdcIndexError> {
        match self {
            Self::BruteForce(bf) => bf.search(query, top_k),
            Self::Hnsw(hnsw) => hnsw.search(query, top_k),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::BruteForce(bf) => bf.len(),
            Self::Hnsw(hnsw) => hnsw.len(),
        }
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
    fn starts_as_brute_force() {
        let index = LocalIndex::new();
        assert!(matches!(index, LocalIndex::BruteForce(_)));
    }

    #[test]
    fn basic_insert_and_search() {
        let mut index = LocalIndex::new();
        let v = HdcVector::random(1);
        let k = make_key(1);
        index.insert(k, v.clone()).unwrap();

        let results = index.search(&v, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, k);
        assert_eq!(results[0].1, 0);
    }

    #[test]
    fn delete_works() {
        let mut index = LocalIndex::new();
        let k = make_key(1);
        index.insert(k, HdcVector::random(1)).unwrap();
        assert_eq!(index.len(), 1);
        assert!(index.delete(&k));
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn search_multiple() {
        let mut index = LocalIndex::new();
        for i in 0..50u64 {
            index.insert(make_key(i), HdcVector::random(i)).unwrap();
        }

        let query = HdcVector::random(0);
        let results = index.search(&query, 5).unwrap();
        assert_eq!(results.len(), 5);
        // First result is exact match
        assert_eq!(results[0].1, 0);
        // Sorted by distance
        for w in results.windows(2) {
            assert!(w[0].1 <= w[1].1);
        }
    }
}
