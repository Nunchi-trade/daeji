//! HnswIndex -- Hierarchical Navigable Small World graph.
//!
//! Deterministic, consensus-safe HNSW implementation.
//! Uses integer-only arithmetic and BTreeMap for deterministic iteration.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BinaryHeap},
};

use tiny_keccak::{Hasher, Keccak};

use super::{H256, HdcIndexError, SearchIndex, SearchResult, simd::hamming_distance};
use crate::HdcVector;

/// Max connections per node per layer (layers >= 1).
const M: usize = 16;

/// Max connections at level 0 (denser for better recall at the base layer).
const M_MAX0: usize = 2 * M; // 32

/// Beam width during index construction.
const EF_CONSTRUCTION: usize = 200;

/// Default beam width during search.
const EF_SEARCH_DEFAULT: usize = 100;

/// Maximum HNSW level. With M=16, P(level >= 5) = 16^{-5} ~ 10^{-6}.
const MAX_LEVEL: usize = 16;

/// Tombstone compaction threshold: rebuild when >20% of nodes are deleted.
const COMPACTION_THRESHOLD_PERCENT: usize = 20;

/// Composite comparison key for deterministic tie-breaking.
/// Lower distance wins. On tie, lower element_id wins.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct CandidateKey {
    distance: u32,
    element_id: u64,
}

impl Ord for CandidateKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance.cmp(&other.distance).then_with(|| self.element_id.cmp(&other.element_id))
    }
}

impl PartialOrd for CandidateKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A node in the HNSW graph.
#[allow(dead_code)]
struct HnswNode {
    key: H256,
    vector: HdcVector,
    /// Internal numeric ID, assigned sequentially at insertion time.
    element_id: u64,
    /// The level this node was assigned to (present on levels 0..=level).
    level: usize,
    /// Adjacency lists per level. BTreeMap for deterministic iteration.
    /// neighbors[lev] maps neighbor_element_id -> distance_to_neighbor.
    neighbors: Vec<BTreeMap<u64, u32>>,
    /// Tombstone flag.
    deleted: bool,
}

/// Hierarchical Navigable Small World graph index.
///
/// Deterministic, consensus-safe HNSW for approximate nearest neighbor search.
/// Uses BTreeMap for deterministic iteration and integer-only level assignment.
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
    insertion_order: BTreeMap<u64, u64>,
    /// Search beam width (tunable at query time).
    ef_search: usize,
}

/// Deterministic level assignment using keccak256 of the vector bytes.
///
/// CONSENSUS SAFETY: Integer-only arithmetic. No f64::ln().
///
/// The geometric distribution P(level >= k) = (1/M)^k = (1/16)^k = 2^{-4k}
/// is computed as: level = leading_zeros(hash_u64) / 4.
fn deterministic_level(vector: &HdcVector) -> usize {
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(vector.0.as_ptr() as *const u8, 160 * 8) };
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    // Safety: hash[0..8] is always exactly 8 bytes, so try_into is infallible.
    let hash_bytes: [u8; 8] =
        [hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7]];
    let random_bits = u64::from_le_bytes(hash_bytes);

    let level = (random_bits.leading_zeros() / 4) as usize;
    level.min(MAX_LEVEL)
}

impl std::fmt::Debug for HnswIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HnswIndex")
            .field("len", &self.len())
            .field("max_level", &self.max_level)
            .finish()
    }
}

impl Default for HnswIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl HnswIndex {
    /// Create an empty HNSW index with default parameters.
    pub const fn new() -> Self {
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

    /// Set the search beam width. Higher values increase recall.
    pub fn set_ef_search(&mut self, ef: usize) {
        self.ef_search = ef.clamp(50, 200);
    }

    /// Greedy search at a single layer. Returns the element_id of the
    /// nearest node to `query` reachable from `entry_id` at `level`.
    fn greedy_search_layer(&self, query: &HdcVector, entry_id: u64, level: usize) -> u64 {
        let mut current = entry_id;
        let mut current_dist = hamming_distance(query, &self.nodes[&current].vector);

        loop {
            let mut changed = false;
            let node = &self.nodes[&current];

            if let Some(neighbors) = node.neighbors.get(level) {
                for &neighbor_id in neighbors.keys() {
                    let neighbor = &self.nodes[&neighbor_id];
                    if neighbor.deleted {
                        continue;
                    }
                    let d = hamming_distance(query, &neighbor.vector);
                    if d < current_dist || (d == current_dist && neighbor_id < current) {
                        current = neighbor_id;
                        current_dist = d;
                        changed = true;
                    }
                }
            }

            if !changed {
                break;
            }
        }
        current
    }

    /// Beam search at a single layer. Returns up to `ef` nearest candidates.
    fn search_layer(
        &self,
        query: &HdcVector,
        entry_ids: &[u64],
        ef: usize,
        level: usize,
    ) -> Vec<CandidateKey> {
        use std::{cmp::Reverse, collections::HashSet};

        let mut visited: HashSet<u64> = HashSet::new();

        // candidates: min-heap (closest first)
        let mut candidates: BinaryHeap<Reverse<CandidateKey>> = BinaryHeap::new();
        // result: max-heap (farthest first)
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
            let Some(farthest) = result.peek() else { break };
            if candidate.distance > farthest.distance
                || (candidate.distance == farthest.distance
                    && candidate.element_id > farthest.element_id)
            {
                break;
            }

            let node = &self.nodes[&candidate.element_id];
            if let Some(neighbors) = node.neighbors.get(level) {
                for &neighbor_id in neighbors.keys() {
                    if !visited.insert(neighbor_id) {
                        continue;
                    }
                    let neighbor = &self.nodes[&neighbor_id];
                    let d = hamming_distance(query, &neighbor.vector);
                    let key = CandidateKey { distance: d, element_id: neighbor_id };

                    let Some(farthest) = result.peek() else { break };
                    if result.len() < ef
                        || key.distance < farthest.distance
                        || (key.distance == farthest.distance
                            && key.element_id < farthest.element_id)
                    {
                        candidates.push(Reverse(key));
                        result.push(key);
                        if result.len() > ef {
                            result.pop();
                        }
                    }
                }
            }
        }

        result.into_vec()
    }

    /// Select the best `m` neighbors from candidates.
    fn select_neighbors(candidates: &mut Vec<CandidateKey>, m: usize) -> Vec<CandidateKey> {
        candidates.sort();
        candidates.truncate(m);
        candidates.clone()
    }

    /// Add a bidirectional edge between two nodes at `level`.
    fn connect(&mut self, id_a: u64, id_b: u64, dist: u32, level: usize) {
        let max_conn = if level == 0 { M_MAX0 } else { M };

        // a -> b
        if let Some(node_a) = self.nodes.get_mut(&id_a) {
            node_a.neighbors[level].insert(id_b, dist);
        }
        self.prune_connections(id_a, level, max_conn);

        // b -> a
        if let Some(node_b) = self.nodes.get_mut(&id_b) {
            node_b.neighbors[level].insert(id_a, dist);
        }
        self.prune_connections(id_b, level, max_conn);
    }

    /// If node has more than `max_conn` neighbors at `level`, prune farthest.
    fn prune_connections(&mut self, node_id: u64, level: usize, max_conn: usize) {
        let Some(node) = self.nodes.get_mut(&node_id) else { return };
        let neighbors = &mut node.neighbors[level];

        if neighbors.len() <= max_conn {
            return;
        }

        let mut entries: Vec<CandidateKey> = neighbors
            .iter()
            .map(|(&eid, &dist)| CandidateKey { distance: dist, element_id: eid })
            .collect();
        entries.sort();
        entries.truncate(max_conn);

        let new_neighbors: BTreeMap<u64, u32> =
            entries.iter().map(|ck| (ck.element_id, ck.distance)).collect();
        *neighbors = new_neighbors;
    }

    /// Mark a node as deleted (tombstone).
    fn mark_deleted(&mut self, element_id: u64) {
        if let Some(node) = self.nodes.get_mut(&element_id)
            && !node.deleted
        {
            node.deleted = true;
            self.tombstone_count += 1;
        }

        let total = self.nodes.len();
        if total > 0 && self.tombstone_count * 100 / total > COMPACTION_THRESHOLD_PERCENT {
            self.compact();
        }
    }

    /// Rebuild the index from scratch, excluding tombstoned nodes.
    fn compact(&mut self) {
        let mut live: Vec<(u64, H256, HdcVector)> = Vec::new();
        for (&eid, node) in &self.nodes {
            if !node.deleted {
                let seq = self.insertion_order[&eid];
                live.push((seq, node.key, node.vector.clone()));
            }
        }
        live.sort_by_key(|(seq, _, _)| *seq);

        let ef = self.ef_search;
        *self = Self::new();
        self.ef_search = ef;
        for (_, key, vector) in live {
            let _ = self.insert_impl(key, vector);
        }
    }

    /// Internal insertion (called by both SearchIndex::insert and compact).
    fn insert_impl(&mut self, key: H256, vector: HdcVector) -> Result<(), HdcIndexError> {
        let level = deterministic_level(&vector);
        let element_id = self.next_id;
        self.next_id += 1;

        let seq = element_id;
        self.insertion_order.insert(element_id, seq);

        let mut neighbors = Vec::with_capacity(level + 1);
        for _ in 0..=level {
            neighbors.push(BTreeMap::new());
        }

        let node =
            HnswNode { key, vector: vector.clone(), element_id, level, neighbors, deleted: false };
        self.nodes.insert(element_id, node);
        self.key_to_id.insert(key, element_id);

        if self.entry_point.is_none() {
            self.entry_point = Some(element_id);
            self.max_level = level;
            return Ok(());
        }

        // entry_point is guaranteed Some here since we checked and returned above.
        let Some(entry) = self.entry_point else {
            return Ok(());
        };

        // Phase 1: Greedy descent from top level down to (level + 1)
        let mut current_entry = entry;
        for lev in (level + 1..=self.max_level).rev() {
            current_entry = self.greedy_search_layer(&vector, current_entry, lev);
        }

        // Phase 2: Insert at each layer from min(level, max_level) down to 0
        let insert_top = level.min(self.max_level);
        let mut entry_ids = vec![current_entry];

        for lev in (0..=insert_top).rev() {
            let mut candidates = self.search_layer(&vector, &entry_ids, EF_CONSTRUCTION, lev);

            let max_conn = if lev == 0 { M_MAX0 } else { M };
            let selected = Self::select_neighbors(&mut candidates, max_conn);

            for ck in &selected {
                let dist = hamming_distance(&vector, &self.nodes[&ck.element_id].vector);
                self.connect(element_id, ck.element_id, dist, lev);
            }

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

        // Phase 2: Beam search at level 0
        let candidates = self.search_layer(query, &[current], self.ef_search.max(effective_k), 0);

        // Filter out tombstoned nodes and take top_k
        let mut results: Vec<(H256, u32)> = candidates
            .into_iter()
            .filter(|ck| self.nodes.get(&ck.element_id).map(|n| !n.deleted).unwrap_or(false))
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
        let mut index = HnswIndex::new();
        let v1 = HdcVector::random(1);
        let k1 = make_key(1);
        index.insert(k1, v1.clone()).unwrap();

        let results = index.search(&v1, 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, k1);
        assert_eq!(results[0].1, 0);
    }

    #[test]
    fn duplicate_key_rejected() {
        let mut index = HnswIndex::new();
        let k = make_key(1);
        index.insert(k, HdcVector::random(1)).unwrap();
        assert!(index.insert(k, HdcVector::random(2)).is_err());
    }

    #[test]
    fn empty_index_error() {
        let index = HnswIndex::new();
        assert!(index.search(&HdcVector::default(), 10).is_err());
    }

    #[test]
    fn delete_works() {
        let mut index = HnswIndex::new();
        let k = make_key(1);
        index.insert(k, HdcVector::random(1)).unwrap();
        assert_eq!(index.len(), 1);
        assert!(index.delete(&k));
        assert_eq!(index.len(), 0);
        assert!(!index.delete(&k));
    }

    #[test]
    fn hnsw_matches_brute_force() {
        use super::super::brute::BruteForceIndex;

        let mut bf = BruteForceIndex::new();
        let mut hnsw = HnswIndex::new();

        for i in 0..200u64 {
            let k = make_key(i);
            let v = HdcVector::random(i);
            bf.insert(k, v.clone()).unwrap();
            hnsw.insert(k, v).unwrap();
        }

        for q_seed in 1000..1010u64 {
            let query = HdcVector::random(q_seed);
            let bf_results = bf.search(&query, 5).unwrap();
            let hnsw_results = hnsw.search(&query, 5).unwrap();

            // HNSW should find the exact same top-1 (nearest neighbor)
            assert_eq!(
                bf_results[0], hnsw_results[0],
                "HNSW nearest neighbor differs from brute-force for query seed {q_seed}"
            );

            // Verify HNSW results are in the same distance range
            let bf_worst_dist = bf_results.last().unwrap().1;
            for (_, dist) in &hnsw_results {
                assert!(
                    *dist <= bf_worst_dist + 100,
                    "HNSW result distance {dist} much worse than brute-force worst {bf_worst_dist}"
                );
            }
        }
    }

    #[test]
    fn deterministic_level_assignment() {
        let v = HdcVector::random(42);
        let l1 = deterministic_level(&v);
        let l2 = deterministic_level(&v);
        assert_eq!(l1, l2);

        let levels: Vec<usize> =
            (0..100u64).map(|i| deterministic_level(&HdcVector::random(i))).collect();
        let level_0_count = levels.iter().filter(|&&l| l == 0).count();
        assert!(level_0_count > 50, "expected most vectors at level 0, got {level_0_count}");
    }

    #[test]
    fn deterministic_index_builds() {
        let mut idx1 = HnswIndex::new();
        let mut idx2 = HnswIndex::new();

        for i in 0..100u64 {
            let k = make_key(i);
            let v = HdcVector::random(i);
            idx1.insert(k, v.clone()).unwrap();
            idx2.insert(k, v).unwrap();
        }

        let query = HdcVector::random(999);
        let r1 = idx1.search(&query, 10).unwrap();
        let r2 = idx2.search(&query, 10).unwrap();
        assert_eq!(r1, r2, "Two identically-built HNSW indexes produced different results");
    }

    #[test]
    fn top_k_ordering() {
        let mut index = HnswIndex::new();
        for i in 0..100u64 {
            index.insert(make_key(i), HdcVector::random(i)).unwrap();
        }

        let query = HdcVector::random(0);
        let results = index.search(&query, 5).unwrap();
        assert_eq!(results.len(), 5);
        for w in results.windows(2) {
            assert!(w[0].1 <= w[1].1);
        }
        assert_eq!(results[0].1, 0);
    }

    #[test]
    fn delete_excludes_from_search() {
        let mut index = HnswIndex::new();
        let k1 = make_key(1);
        let v1 = HdcVector::random(1);
        let k2 = make_key(2);
        let v2 = HdcVector::random(2);

        index.insert(k1, v1.clone()).unwrap();
        index.insert(k2, v2).unwrap();

        index.delete(&k1);

        let results = index.search(&v1, 10).unwrap();
        assert!(!results.iter().any(|(k, _)| *k == k1));
    }
}
