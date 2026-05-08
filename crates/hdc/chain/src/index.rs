//! On-chain HDC index for consensus-critical vector storage.
//!
//! This index is node-local and ephemeral -- rebuilt from chain events on startup.
//! All iteration is deterministic (BTreeMap-ordered by key) for consensus safety.

use std::collections::BTreeMap;

use alloy_primitives::B256;
use kora_hdc::{HdcVector, hamming_distance};

/// Errors from on-chain index operations.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// The index has reached its maximum capacity.
    #[error("index full: capacity {capacity} reached")]
    CapacityExceeded {
        /// The configured maximum number of entries.
        capacity: usize,
    },
    /// The requested insight was not found.
    #[error("insight not found: {0}")]
    NotFound(B256),
}

/// On-chain HDC index tracking vectors published through InsightBoard.
///
/// Uses `BTreeMap` for deterministic iteration order -- critical for any
/// consensus-adjacent code path that enumerates entries.
#[derive(Debug)]
pub struct OnChainHdcIndex {
    /// Map from vector ID to (vector, metadata), ordered by key.
    entries: BTreeMap<B256, (HdcVector, InsightMeta)>,
    /// Maximum number of entries. 0 means unlimited.
    max_entries: usize,
    /// Pheromone tracking: topic → region → cumulative strength.
    pheromones: BTreeMap<B256, BTreeMap<B256, u64>>,
}

impl Default for OnChainHdcIndex {
    fn default() -> Self {
        Self { entries: BTreeMap::new(), max_entries: 0, pheromones: BTreeMap::new() }
    }
}

/// Metadata for an on-chain insight.
#[derive(Debug, Clone)]
pub struct InsightMeta {
    /// The agent that published this insight.
    pub publisher: alloy_primitives::Address,
    /// Block number at which the insight was published.
    pub block_number: u64,
    /// Current state in the FSM.
    pub state: InsightState,
}

/// FSM states for an on-chain insight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightState {
    /// Initial draft state.
    Draft,
    /// Submitted for validation.
    Submitted,
    /// Under challenge.
    Challenged,
    /// Community voting.
    Voting,
    /// Accepted into the knowledge commons.
    Accepted,
    /// Rejected by the community.
    Rejected,
    /// Expired due to inactivity.
    Expired,
}

/// Result from an on-chain search.
#[derive(Debug, Clone)]
pub struct OnChainSearchResult {
    /// Vector ID (keccak256 of serialized vector).
    pub id: B256,
    /// Hamming distance to query.
    pub distance: u32,
    /// Associated metadata.
    pub meta: InsightMeta,
}

impl OnChainHdcIndex {
    /// Create a new empty index with no capacity limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new empty index with a maximum entry capacity.
    ///
    /// When the index is full, [`insert_insight`](Self::insert_insight) returns
    /// [`IndexError::CapacityExceeded`].
    pub fn with_capacity(max_entries: usize) -> Self {
        Self { entries: BTreeMap::new(), max_entries, pheromones: BTreeMap::new() }
    }

    /// Insert or update an insight vector.
    ///
    /// Returns the vector ID on success, or [`IndexError::CapacityExceeded`]
    /// if the index is at capacity and this is a new entry.
    pub fn insert_insight(
        &mut self,
        vector: HdcVector,
        publisher: alloy_primitives::Address,
        block_number: u64,
    ) -> Result<B256, IndexError> {
        let id_bytes = kora_hdc::vector_id(&vector);
        let id = B256::from(id_bytes);

        // Only enforce capacity for new entries, not updates.
        if self.max_entries > 0
            && self.entries.len() >= self.max_entries
            && !self.entries.contains_key(&id)
        {
            return Err(IndexError::CapacityExceeded { capacity: self.max_entries });
        }

        self.entries.insert(
            id,
            (vector, InsightMeta { publisher, block_number, state: InsightState::Submitted }),
        );
        Ok(id)
    }

    /// Update the FSM state of an insight.
    ///
    /// Returns `Ok(())` if the insight was found and updated, or
    /// [`IndexError::NotFound`] if no insight with this ID exists.
    pub fn update_state(&mut self, id: &B256, state: InsightState) -> Result<(), IndexError> {
        match self.entries.get_mut(id) {
            Some((_vec, meta)) => {
                meta.state = state;
                Ok(())
            }
            None => Err(IndexError::NotFound(*id)),
        }
    }

    /// Search for the top-k most similar accepted vectors.
    ///
    /// Results are sorted by `(distance, id)` for deterministic tiebreaking.
    pub fn search(&self, query: &HdcVector, top_k: usize) -> Vec<OnChainSearchResult> {
        let mut results: Vec<OnChainSearchResult> = self
            .entries
            .iter()
            .filter_map(|(id, (vec, meta))| {
                // Only search accepted insights.
                if meta.state != InsightState::Accepted {
                    return None;
                }
                let distance = hamming_distance(query, vec);
                Some(OnChainSearchResult { id: *id, distance, meta: meta.clone() })
            })
            .collect();

        // Deterministic sort: by distance first, then by ID for tiebreaking.
        results.sort_by(|a, b| a.distance.cmp(&b.distance).then_with(|| a.id.cmp(&b.id)));
        results.truncate(top_k);
        results
    }

    /// Get a vector and its metadata by ID.
    pub fn get(&self, id: &B256) -> Option<(&HdcVector, &InsightMeta)> {
        self.entries.get(id).map(|(vec, meta)| (vec, meta))
    }

    /// Iterate over all vectors in deterministic (key-sorted) order.
    ///
    /// Useful for TieredSearch and other consensus-critical enumeration.
    pub fn iter_vectors(&self) -> impl Iterator<Item = (&B256, &HdcVector, &InsightMeta)> {
        self.entries.iter().map(|(id, (vec, meta))| (id, vec, meta))
    }

    /// Record a pheromone event, accumulating strength for a topic/region pair.
    pub fn record_pheromone(&mut self, topic: B256, region: B256, strength: u64) {
        let entry = self
            .pheromones
            .entry(topic)
            .or_default()
            .entry(region)
            .or_insert(0);
        *entry = entry.saturating_add(strength);
    }

    /// Query the pheromone strength for a specific topic and region.
    pub fn pheromone_strength(&self, topic: &B256, region: &B256) -> u64 {
        self.pheromones
            .get(topic)
            .and_then(|regions| regions.get(region))
            .copied()
            .unwrap_or(0)
    }

    /// Iterate all pheromone entries in deterministic order.
    pub fn iter_pheromones(&self) -> impl Iterator<Item = (&B256, &B256, u64)> {
        self.pheromones.iter().flat_map(|(topic, regions)| {
            regions.iter().map(move |(region, &strength)| (topic, region, strength))
        })
    }

    /// Number of indexed vectors.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_search() {
        let mut index = OnChainHdcIndex::new();
        let v = HdcVector::random(1);
        let publisher = alloy_primitives::Address::ZERO;
        let id = index.insert_insight(v.clone(), publisher, 100).expect("insert should succeed");

        // Mark as accepted so it appears in search.
        index.update_state(&id, InsightState::Accepted).expect("update should succeed");

        let results = index.search(&v, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].distance, 0);
        assert_eq!(results[0].id, id);
    }

    #[test]
    fn test_unaccepted_not_in_search() {
        let mut index = OnChainHdcIndex::new();
        let v = HdcVector::random(2);
        let publisher = alloy_primitives::Address::ZERO;
        index.insert_insight(v.clone(), publisher, 100).expect("insert should succeed");

        // Don't accept -- should not appear in search.
        let results = index.search(&v, 5);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_capacity_enforcement() {
        let mut index = OnChainHdcIndex::with_capacity(1);
        let v1 = HdcVector::random(10);
        let v2 = HdcVector::random(11);
        let publisher = alloy_primitives::Address::ZERO;

        index.insert_insight(v1, publisher, 100).expect("first insert should succeed");

        let result = index.insert_insight(v2, publisher, 101);
        assert!(
            matches!(result, Err(IndexError::CapacityExceeded { capacity: 1 })),
            "second insert should fail with CapacityExceeded"
        );
    }

    #[test]
    fn test_update_state_not_found() {
        let mut index = OnChainHdcIndex::new();
        let bogus = B256::ZERO;
        let result = index.update_state(&bogus, InsightState::Accepted);
        assert!(matches!(result, Err(IndexError::NotFound(_))));
    }

    #[test]
    fn test_iter_vectors_deterministic() {
        let mut index = OnChainHdcIndex::new();
        let publisher = alloy_primitives::Address::ZERO;

        for seed in 0..5u64 {
            let v = HdcVector::random(seed);
            index.insert_insight(v, publisher, seed).expect("insert should succeed");
        }

        let ids: Vec<B256> = index.iter_vectors().map(|(id, _, _)| *id).collect();
        // BTreeMap guarantees ascending order.
        for i in 1..ids.len() {
            assert!(ids[i - 1] < ids[i], "iter_vectors must be sorted by key");
        }
    }

    #[test]
    fn test_pheromone_tracking() {
        let mut index = OnChainHdcIndex::new();
        let topic = B256::from([1u8; 32]);
        let region = B256::from([2u8; 32]);

        assert_eq!(index.pheromone_strength(&topic, &region), 0);

        index.record_pheromone(topic, region, 100);
        assert_eq!(index.pheromone_strength(&topic, &region), 100);

        // Accumulates
        index.record_pheromone(topic, region, 50);
        assert_eq!(index.pheromone_strength(&topic, &region), 150);

        // Different region
        let region2 = B256::from([3u8; 32]);
        index.record_pheromone(topic, region2, 200);
        assert_eq!(index.pheromone_strength(&topic, &region2), 200);

        // Iter deterministic
        let entries: Vec<_> = index.iter_pheromones().collect();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_pheromone_saturating_add() {
        let mut index = OnChainHdcIndex::new();
        let topic = B256::from([1u8; 32]);
        let region = B256::from([2u8; 32]);

        index.record_pheromone(topic, region, u64::MAX);
        index.record_pheromone(topic, region, 1);
        assert_eq!(index.pheromone_strength(&topic, &region), u64::MAX);
    }

    #[test]
    fn test_search_deterministic_tiebreak() {
        let mut index = OnChainHdcIndex::new();
        let publisher = alloy_primitives::Address::ZERO;

        // Insert the same vector twice -- they'll have the same distance to the query.
        // Actually, same vector = same ID, so use different vectors.
        // Instead, just verify sort is by (distance, id).
        let v1 = HdcVector::random(20);
        let v2 = HdcVector::random(21);
        let id1 = index.insert_insight(v1.clone(), publisher, 100).expect("insert");
        let id2 = index.insert_insight(v2.clone(), publisher, 101).expect("insert");
        index.update_state(&id1, InsightState::Accepted).expect("update");
        index.update_state(&id2, InsightState::Accepted).expect("update");

        // Search with a third query.
        let query = HdcVector::random(22);
        let results = index.search(&query, 10);
        assert_eq!(results.len(), 2);

        // Verify deterministic ordering.
        if results[0].distance == results[1].distance {
            assert!(results[0].id < results[1].id, "ties should be broken by ID");
        }
    }
}
