//! In-memory seed tracker implementation.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use alloy_primitives::B256;
use parking_lot::RwLock;

use crate::traits::{Digest, SeedTracker};

/// Maximum number of seeds retained in the tracker.
///
/// Only the most recent parent block's seed is ever queried (for prevrandao
/// computation).  Seeds for blocks older than a few blocks are never accessed
/// again.  Capping the tracker at 256 entries prevents unbounded memory
/// growth (~200 MB/day at 33 blocks/s) while retaining far more seeds than
/// will ever be needed.
const MAX_SEEDS: usize = 256;

/// In-memory seed tracker with bounded capacity.
///
/// Seeds older than [`MAX_SEEDS`] entries are automatically evicted on
/// insertion to prevent unbounded memory growth.
#[derive(Debug, Clone)]
pub struct InMemorySeedTracker {
    inner: Arc<RwLock<BTreeMap<Digest, B256>>>,
    /// Insertion-ordered queue of digests for oldest-first eviction.
    order: Arc<RwLock<VecDeque<Digest>>>,
}

impl InMemorySeedTracker {
    /// Create a new seed tracker with genesis seed.
    #[must_use]
    pub fn new(genesis_digest: Digest) -> Self {
        let mut seeds = BTreeMap::new();
        seeds.insert(genesis_digest, B256::ZERO);
        let mut order = VecDeque::new();
        order.push_back(genesis_digest);
        Self { inner: Arc::new(RwLock::new(seeds)), order: Arc::new(RwLock::new(order)) }
    }

    /// Create an empty seed tracker.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(BTreeMap::new())),
            order: Arc::new(RwLock::new(VecDeque::new())),
        }
    }
}

impl Default for InMemorySeedTracker {
    fn default() -> Self {
        Self::empty()
    }
}

impl SeedTracker for InMemorySeedTracker {
    fn get(&self, digest: &Digest) -> Option<B256> {
        self.inner.read().get(digest).copied()
    }

    fn insert(&self, digest: Digest, seed: B256) {
        let mut inner = self.inner.write();
        let mut order = self.order.write();

        if inner.insert(digest, seed).is_none() {
            order.push_back(digest);
        }

        // Evict oldest entries beyond the capacity limit.
        while order.len() > MAX_SEEDS {
            if let Some(oldest) = order.pop_front() {
                inner.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_tracker_insert_and_get() {
        let tracker = InMemorySeedTracker::empty();

        let digest = Digest::from([0x01u8; 32]);
        let seed = B256::repeat_byte(0x02);

        assert!(tracker.get(&digest).is_none());

        tracker.insert(digest, seed);
        assert_eq!(tracker.get(&digest), Some(seed));
    }

    #[test]
    fn seed_tracker_genesis() {
        let genesis = Digest::from([0xABu8; 32]);
        let tracker = InMemorySeedTracker::new(genesis);

        assert_eq!(tracker.get(&genesis), Some(B256::ZERO));
    }

    #[test]
    fn seed_tracker_evicts_oldest() {
        let tracker = InMemorySeedTracker::empty();

        // Insert MAX_SEEDS + 1 entries; the first should be evicted.
        for i in 0..=(MAX_SEEDS as u16) {
            let mut bytes = [0u8; 32];
            bytes[0] = (i & 0xFF) as u8;
            bytes[1] = (i >> 8) as u8;
            let digest = Digest::from(bytes);
            let seed = B256::repeat_byte((i & 0xFF) as u8);
            tracker.insert(digest, seed);
        }

        // The first entry (byte 0) should have been evicted.
        let first = Digest::from([0u8; 32]);
        assert!(tracker.get(&first).is_none(), "oldest entry should be evicted");

        // The most recent entry should still be present.
        let mut last_bytes = [0u8; 32];
        last_bytes[0] = (MAX_SEEDS & 0xFF) as u8;
        last_bytes[1] = (MAX_SEEDS >> 8) as u8;
        let last = Digest::from(last_bytes);
        assert!(tracker.get(&last).is_some(), "newest entry should be retained");

        // Total size should be exactly MAX_SEEDS.
        assert_eq!(tracker.inner.read().len(), MAX_SEEDS);
    }
}
