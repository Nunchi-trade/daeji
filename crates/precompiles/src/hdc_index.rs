//! Brute-force HDC similarity index for the chain substrate.
//!
//! Maintains a flat list of `(id, vector, weight)` tuples. Queries compute
//! Hamming similarity against every entry and return the top-K by combined
//! score (`similarity × weight`).
//!
//! At the 100K-entry scale described in doc 04, a full scan runs in ~170μs
//! with SIMD popcount. We do not ship SIMD-specific code paths in this POC
//! (we use [`HdcVector::similarity`], which compiles to scalar XOR+popcnt on
//! x86-64 and ARM without any intrinsics). Benchmarks at 10K entries: ~2ms
//! per query on a modern laptop. That's the sub-millisecond target for the
//! chain's 400ms block budget.
//!
//! Vendored from `roko/apps/mirage-rs/src/chain/hdc_index.rs` (MIT OR
//! Apache-2.0). Imports re-pointed to local crate modules.

use crate::{hdc_vector::HdcVector, insight_id::InsightId};

/// One entry stored in the flat index.
#[derive(Clone, Debug)]
pub struct IndexedVector {
    /// Entry id that identifies the source insight.
    pub id: InsightId,
    /// The HDC vector.
    pub vector: HdcVector,
    /// Weight multiplier applied to similarity for ranking.
    pub weight: f32,
    /// Block timestamp at which this insight was posted (Unix seconds).
    /// Used by `evict_expired` to drop entries past their effective lifespan.
    pub posted_at: u64,
    /// Effective half-life in seconds (`baseHalfLife × tierMultiplierBps / 1000`).
    /// Mirrors `InsightBoard._weight()` so on-chain query results and
    /// off-chain computation agree.
    pub effective_half_life_seconds: u64,
}

/// A ranked hit returned from a top-K query.
#[derive(Clone, Debug)]
pub struct Hit {
    /// Matching entry id.
    pub id: InsightId,
    /// Raw Hamming similarity in `[0, 1]`.
    pub similarity: f32,
    /// Weight of the matched entry at query time.
    pub weight: f32,
    /// Combined score (`similarity × weight`), used for ranking.
    pub score: f32,
}

/// In-memory flat HDC index with brute-force top-K Hamming similarity search.
#[derive(Default, Clone, Debug)]
pub struct HdcIndex {
    entries: Vec<IndexedVector>,
}

impl HdcIndex {
    /// Constructs an empty index.
    #[must_use]
    pub const fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Number of vectors in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Inserts an entry. Replaces any existing entry with the same id.
    /// `posted_at` defaults to 0 and `effective_half_life_seconds` to
    /// `u64::MAX` (never expires) — call sites that have decay metadata
    /// should use `insert_with_decay` instead.
    pub fn insert(&mut self, id: InsightId, vector: HdcVector, weight: f32) {
        self.insert_with_decay(id, vector, weight, 0, u64::MAX);
    }

    /// Insert an entry with full decay metadata (`posted_at` Unix seconds,
    /// effective half-life in seconds). Used by `HDCState::on_finalize_extend`
    /// to mirror the on-chain `InsightBoard` decay model.
    pub fn insert_with_decay(
        &mut self,
        id: InsightId,
        vector: HdcVector,
        weight: f32,
        posted_at: u64,
        effective_half_life_seconds: u64,
    ) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.id == id) {
            existing.vector = vector;
            existing.weight = weight;
            existing.posted_at = posted_at;
            existing.effective_half_life_seconds = effective_half_life_seconds;
        } else {
            self.entries.push(IndexedVector {
                id,
                vector,
                weight,
                posted_at,
                effective_half_life_seconds,
            });
        }
    }

    /// Drop entries past `DEAD_QUANTA × effective_half_life_seconds`
    /// (the "considered dead" threshold from spec line 431). Mirrors
    /// `InsightBoard.currentWeight()` returning 0 once `quanta >= 7`.
    /// Entries with `effective_half_life_seconds == u64::MAX` (the default
    /// sentinel) are never evicted.
    pub fn evict_expired(&mut self, now_ts: u64) {
        const DEAD_QUANTA: u64 = 7;
        self.entries.retain(|e| {
            if e.effective_half_life_seconds == u64::MAX {
                return true;
            }
            // saturating_mul guards against overflow at extreme half-lives.
            let dead_at = e
                .posted_at
                .saturating_add(DEAD_QUANTA.saturating_mul(e.effective_half_life_seconds));
            now_ts < dead_at
        });
    }

    /// Updates just the weight for an id. Returns true if it existed.
    pub fn set_weight(&mut self, id: InsightId, weight: f32) -> bool {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.id == id) {
            existing.weight = weight;
            true
        } else {
            false
        }
    }

    /// Removes an entry by id. Returns true if it existed.
    pub fn remove(&mut self, id: InsightId) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            self.entries.swap_remove(pos);
            true
        } else {
            false
        }
    }

    /// Returns top-K hits by combined score, brute-force scan.
    ///
    /// Ranking: `score = similarity(query, entry.vector) × entry.weight`.
    /// Entries with `weight <= 0.0` are skipped. Ties broken by insertion order.
    #[must_use]
    pub fn top_k(&self, query: &HdcVector, k: usize) -> Vec<Hit> {
        if k == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<Hit> = self
            .entries
            .iter()
            .filter(|e| e.weight > 0.0)
            .map(|e| {
                let sim = query.similarity(&e.vector);
                Hit { id: e.id, similarity: sim, weight: e.weight, score: sim * e.weight }
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(k);
        hits
    }

    /// Returns the current weight for an id, if present.
    #[must_use]
    pub fn weight_of(&self, id: InsightId) -> Option<f32> {
        self.entries.iter().find(|e| e.id == id).map(|e| e.weight)
    }

    /// Iterator over index entries (read-only).
    pub fn iter(&self) -> impl Iterator<Item = &IndexedVector> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight_id::KnowledgeKind;

    fn mk_id(s: &str) -> InsightId {
        InsightId::derive(b"a", s.as_bytes(), KnowledgeKind::Insight)
    }

    #[test]
    fn insert_and_query_returns_exact_match() {
        let mut idx = HdcIndex::new();
        let v = HdcVector::from_seed(b"proxy pattern");
        let id = mk_id("proxy pattern");
        idx.insert(id, v, 1.0);

        let hits = idx.top_k(&v, 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id);
        assert!((hits[0].similarity - 1.0).abs() < 1e-6);
        assert!((hits[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn top_k_respects_k() {
        let mut idx = HdcIndex::new();
        for i in 0..10 {
            let key = format!("entry {i}");
            idx.insert(mk_id(&key), HdcVector::from_seed(key.as_bytes()), 1.0);
        }
        let query = HdcVector::from_seed(b"entry 3");
        let hits = idx.top_k(&query, 3);
        assert_eq!(hits.len(), 3);
        // The exact match should be first.
        assert_eq!(hits[0].id, mk_id("entry 3"));
    }

    #[test]
    fn weight_biases_ranking() {
        let mut idx = HdcIndex::new();
        let close = HdcVector::from_seed(b"apple");
        let other = HdcVector::from_seed(b"apricot");
        idx.insert(mk_id("apple"), close, 0.2); // low weight
        idx.insert(mk_id("apricot"), other, 5.0); // high weight
        let hits = idx.top_k(&close, 2);
        // Even though "apple" is an exact match, its weight is so low that
        // "apricot" (lower similarity but 25x weight) wins.
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, mk_id("apricot"));
    }

    #[test]
    fn remove_drops_entry() {
        let mut idx = HdcIndex::new();
        let id = mk_id("drop me");
        idx.insert(id, HdcVector::from_seed(b"drop me"), 1.0);
        assert_eq!(idx.len(), 1);
        assert!(idx.remove(id));
        assert_eq!(idx.len(), 0);
        assert!(!idx.remove(id));
    }

    #[test]
    fn top_k_skips_zero_weight_entries() {
        let mut idx = HdcIndex::new();
        idx.insert(mk_id("zero"), HdcVector::from_seed(b"zero"), 0.0);
        idx.insert(mk_id("one"), HdcVector::from_seed(b"one"), 1.0);
        let hits = idx.top_k(&HdcVector::from_seed(b"one"), 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, mk_id("one"));
    }

    #[test]
    fn set_weight_updates_existing() {
        let mut idx = HdcIndex::new();
        let id = mk_id("mutate");
        idx.insert(id, HdcVector::from_seed(b"mutate"), 1.0);
        assert!(idx.set_weight(id, 0.5));
        assert_eq!(idx.weight_of(id), Some(0.5));
        assert!(!idx.set_weight(mk_id("missing"), 0.5));
    }

    #[test]
    fn insert_with_same_id_replaces() {
        let mut idx = HdcIndex::new();
        let id = mk_id("same");
        idx.insert(id, HdcVector::from_seed(b"first"), 1.0);
        idx.insert(id, HdcVector::from_seed(b"second"), 2.0);
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.weight_of(id), Some(2.0));
    }
}
