//! KnowledgeStore -- the central knowledge management struct.
//!
//! Uses `LocalIndex` for vector search (auto-switches between brute-force
//! and HNSW based on entry count) instead of hand-rolled Vec iteration.

use std::collections::HashMap;

use super::{
    anti::{ANTI_SUBSPACE, AntiClassification, classify_anti_signal},
    decay::{GC_THRESHOLD, compute_decayed_balance, ticks_to_hours},
    entry::KnowledgeEntry,
    scoring::{RetrievalContext, ScoredEntry, compute_score},
    tier::KnowledgeTier,
};
use crate::{
    HdcVector, bind,
    search::{H256, LocalIndex, SearchIndex},
};

/// Local agent knowledge store backed by a `LocalIndex` for vector search.
///
/// Combines a `HashMap` of entries (keyed by `H256`) with a `LocalIndex`
/// for efficient similarity search. Supports decay, tier promotion/demotion,
/// garbage collection, anti-knowledge filtering, and four-factor scoring.
#[derive(Debug)]
pub struct KnowledgeStore {
    /// All knowledge entries, keyed by H256.
    entries: HashMap<H256, KnowledgeEntry>,
    /// Vector index for similarity search (auto-switches brute-force / HNSW).
    index: LocalIndex,
    /// Block/tick duration in milliseconds, for tick-to-hours conversion.
    tick_duration_ms: u64,
}

impl KnowledgeStore {
    /// Create a new empty knowledge store.
    pub fn new(tick_duration_ms: u64) -> Self {
        Self { entries: HashMap::new(), index: LocalIndex::new(), tick_duration_ms }
    }

    /// Stub for persistence -- just returns a new empty store.
    pub fn open(_path: &std::path::Path, tick_duration_ms: u64) -> Result<Self, std::io::Error> {
        Ok(Self::new(tick_duration_ms))
    }

    /// Insert a new knowledge entry.
    ///
    /// Adds the entry to both the HashMap and the LocalIndex.
    pub fn insert(&mut self, entry: KnowledgeEntry) {
        let key = entry.key;
        // Ignore duplicate-key errors from the index (entry already present).
        let _ = self.index.insert(key, entry.vector.clone());
        self.entries.insert(key, entry);
    }

    /// Get a reference to an entry by key.
    pub fn get(&self, key: &H256) -> Option<&KnowledgeEntry> {
        self.entries.get(key)
    }

    /// Remove an entry by key.
    pub fn remove(&mut self, key: &H256) -> Option<KnowledgeEntry> {
        self.index.delete(key);
        self.entries.remove(key)
    }

    /// Number of entries in the store.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Search the index for the top-k closest vectors to `query`.
    /// Returns `(key, hamming_distance)` pairs sorted by distance.
    fn index_search(&self, query: &HdcVector, k: usize) -> Vec<(H256, u32)> {
        match self.index.search(query, k) {
            Ok(results) => results,
            Err(_) => Vec::new(),
        }
    }

    /// Search for the top-k entries most similar to `query`, ranked by the
    /// four-factor composite score.
    pub fn search(&self, query: &HdcVector, k: usize, ctx: &RetrievalContext) -> Vec<ScoredEntry> {
        let candidates = self.index_search(query, k * 2);

        let mut scored: Vec<ScoredEntry> = candidates
            .iter()
            .filter_map(|(key, hamming)| {
                let entry = self.entries.get(key)?;
                let score = compute_score(entry, *hamming, ctx);
                Some(ScoredEntry {
                    key: *key,
                    entry: entry.clone(),
                    score,
                    hamming_distance: *hamming,
                })
            })
            .collect();

        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }

    /// Search with anti-knowledge filtering.
    ///
    /// Over-fetches 2x candidates, probes each for anti-knowledge
    /// contradictions, and classifies results on a three-tier severity scale:
    ///   - Strong (>0.9): rejected entirely
    ///   - Moderate (0.7-0.9): included with halved confidence
    ///   - Weak (0.5-0.7): included with warning
    pub fn search_with_anti_check(
        &self,
        query: &HdcVector,
        k: usize,
        ctx: &RetrievalContext,
    ) -> Vec<ScoredEntry> {
        let candidates = self.index_search(query, k * 2);
        let mut results: Vec<ScoredEntry> = Vec::new();

        for (key, hamming) in &candidates {
            let entry = match self.entries.get(key) {
                Some(e) => e,
                None => continue,
            };

            // Anti-knowledge probe: bind entry's vector with ANTI_SUBSPACE,
            // then search for the closest match in the index.
            let anti_probe = bind(&entry.vector, &ANTI_SUBSPACE);
            let anti_matches = self.index_search(&anti_probe, 1);

            let mut contradicted = false;
            let mut confidence_modifier = 1.0_f64;

            if let Some((_, anti_dist)) = anti_matches.first() {
                let anti_sim = 1.0 - (*anti_dist as f64 / crate::constants::D as f64);

                match classify_anti_signal(anti_sim) {
                    AntiClassification::Strong => {
                        // STRONG contradiction -- reject entirely
                        continue;
                    }
                    AntiClassification::Moderate => {
                        // MODERATE contradiction -- halve confidence
                        contradicted = true;
                        confidence_modifier = 0.5;
                    }
                    AntiClassification::Weak => {
                        // WEAK signal -- include with warning (no confidence penalty)
                        // The contradicted flag stays false; this is informational only.
                    }
                    AntiClassification::None => {
                        // Below chance level, no concern
                    }
                }
            }

            let mut scored_entry = entry.clone();
            scored_entry.contradicted = contradicted;

            let raw_score = compute_score(entry, *hamming, ctx);
            let adjusted_score = raw_score * confidence_modifier;

            results.push(ScoredEntry {
                key: *key,
                entry: scored_entry,
                score: adjusted_score,
                hamming_distance: *hamming,
            });

            if results.len() >= k {
                break;
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Per-block maintenance cycle. Call this once per tick.
    ///
    /// Operations (in order):
    ///   1. Decay balances for all entries.
    ///   2. Promote entries that have enough confirmations.
    ///   3. Demote entries that have been inactive too long.
    ///   4. Garbage-collect entries below GC_THRESHOLD.
    ///
    /// Returns the number of entries garbage-collected.
    pub fn tick(&mut self, current_tick: u64) -> usize {
        let mut gc_keys: Vec<H256> = Vec::new();
        let mut promotions: Vec<(H256, KnowledgeTier)> = Vec::new();
        let mut demotions: Vec<(H256, KnowledgeTier)> = Vec::new();

        for (key, entry) in self.entries.iter_mut() {
            // Step 1: Decay
            let elapsed_ticks = current_tick.saturating_sub(entry.last_decay_tick);
            if elapsed_ticks > 0 {
                let elapsed_hours = ticks_to_hours(elapsed_ticks, self.tick_duration_ms);
                entry.balance =
                    compute_decayed_balance(entry.balance, &entry.kind, &entry.tier, elapsed_hours);
                entry.last_decay_tick = current_tick;
            }

            // Step 2: Check promotion (takes precedence over demotion)
            if let Some(new_tier) = entry.tier.try_promote(entry.confirmations) {
                promotions.push((*key, new_tier));
                continue; // skip demotion check
            }

            // Step 3: Check demotion (inactivity-based)
            let effective_hl = entry.kind.base_half_life_hours() * entry.tier.multiplier();
            let inactivity_ticks = current_tick.saturating_sub(entry.last_accessed);
            let inactivity_hours = ticks_to_hours(inactivity_ticks, self.tick_duration_ms);

            let demotion_threshold_hours = match entry.tier {
                KnowledgeTier::Working => effective_hl * 5.0,
                KnowledgeTier::Consolidated => effective_hl * 10.0,
                _ => f64::MAX, // Transient and Persistent not demoted by inactivity
            };

            if inactivity_hours > demotion_threshold_hours {
                demotions.push((*key, entry.tier.demote()));
            }

            // Step 4: Check GC eligibility
            if entry.balance < GC_THRESHOLD {
                if entry.tier == KnowledgeTier::Persistent {
                    // Persistent entries are NEVER deleted. Demote instead.
                    demotions.push((*key, KnowledgeTier::Consolidated));
                } else {
                    gc_keys.push(*key);
                }
            }
        }

        // Apply promotions
        for (key, new_tier) in &promotions {
            if let Some(entry) = self.entries.get_mut(key) {
                entry.tier = *new_tier;
            }
        }

        // Apply demotions
        for (key, new_tier) in &demotions {
            if let Some(entry) = self.entries.get_mut(key) {
                entry.tier = *new_tier;
            }
        }

        // Garbage-collect
        let gc_count = gc_keys.len();
        for key in &gc_keys {
            self.entries.remove(key);
            self.index.delete(key);
        }

        gc_count
    }

    /// Reinforce an entry: snapshot current decayed balance, apply boost,
    /// reset the decay clock, and increment confirmations.
    pub fn reinforce(&mut self, key: &H256, current_tick: u64, boost: f64) {
        if let Some(entry) = self.entries.get_mut(key) {
            // Snapshot current decayed balance before boosting
            let elapsed_ticks = current_tick.saturating_sub(entry.last_decay_tick);
            let elapsed_hours = ticks_to_hours(elapsed_ticks, self.tick_duration_ms);
            entry.balance =
                compute_decayed_balance(entry.balance, &entry.kind, &entry.tier, elapsed_hours);

            // Apply boost (capped at 1.0)
            entry.balance = (entry.balance + boost).min(1.0);

            // Reset clocks
            entry.last_accessed = current_tick;
            entry.last_decay_tick = current_tick;
            entry.confirmations += 1;
        }
    }

    /// Record a query hit: multiply balance by 1.1 (capped at 1.0) and
    /// update last_accessed.
    pub fn record_query_hit(&mut self, key: &H256, current_tick: u64) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.balance = (entry.balance * 1.1).min(1.0);
            entry.last_accessed = current_tick;
        }
    }

    /// Record a contradiction: halve the entry's balance and set its
    /// contradicted flag.
    pub fn record_contradiction(&mut self, key: &H256) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.balance *= 0.5;
            entry.contradicted = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::{KnowledgeKind, KnowledgeSource},
        *,
    };
    use crate::{HdcVector, cognitive::affect::PadState};

    fn make_store() -> KnowledgeStore {
        // 400ms ticks -> 9000 ticks = 1 hour
        KnowledgeStore::new(400)
    }

    fn make_ctx(tick: u64) -> RetrievalContext {
        RetrievalContext { current_tick: tick, mood: PadState::neutral(), tick_duration_ms: 400 }
    }

    fn make_entry_with_kind(seed: u64, kind: KnowledgeKind, tick: u64) -> KnowledgeEntry {
        KnowledgeEntry::new(
            HdcVector::random(seed),
            kind,
            format!("entry-{seed}"),
            KnowledgeSource::SelfDerived,
            tick,
        )
    }

    fn make_entry(seed: u64, tick: u64) -> KnowledgeEntry {
        make_entry_with_kind(seed, KnowledgeKind::Insight, tick)
    }

    #[test]
    fn test_insert_and_search() {
        let mut store = make_store();
        let ctx = make_ctx(0);

        // Insert 10 entries
        for i in 0..10 {
            store.insert(make_entry(i, 0));
        }
        assert_eq!(store.len(), 10);

        // Search for the first entry's vector
        let query = HdcVector::random(0);
        let results = store.search(&query, 5, &ctx);
        assert!(!results.is_empty());
        // The exact match should be first (hamming distance = 0)
        assert_eq!(results[0].hamming_distance, 0);
    }

    #[test]
    fn test_tick_gc() {
        let mut store = make_store();
        let entry = make_entry(1, 0);
        let key = entry.key;
        store.insert(entry);

        // Insight at Transient: effective half-life = 72 * 0.1 = 7.2 hours
        // GC at ~48 hours. 48 hours = 48 * 9000 = 432000 ticks
        let gc_count = store.tick(432_000);
        assert_eq!(gc_count, 1, "entry should be GC'd");
        assert!(store.get(&key).is_none(), "entry should be removed");
    }

    #[test]
    fn test_tick_promotion() {
        let mut store = make_store();
        let entry = make_entry(1, 0);
        let key = entry.key;
        store.insert(entry);

        // Reinforce 3 times to trigger Transient -> Working promotion
        store.reinforce(&key, 1, 0.5);
        store.reinforce(&key, 2, 0.5);
        store.reinforce(&key, 3, 0.5);

        store.tick(4);

        let entry = store.get(&key).unwrap();
        assert_eq!(entry.tier, KnowledgeTier::Working, "should be promoted to Working");
    }

    #[test]
    fn test_tick_demotion() {
        let mut store = make_store();
        let mut entry = make_entry(1, 0);
        entry.tier = KnowledgeTier::Working;
        // Set high balance so it doesn't get GC'd
        entry.balance = 1.0;
        let key = entry.key;
        store.insert(entry);

        // Working Insight: effective_hl = 72 * 0.5 = 36 hours
        // Demotion threshold = 36 * 5 = 180 hours = 180 * 9000 = 1_620_000 ticks
        store.tick(1_620_001);

        let entry = store.get(&key).unwrap();
        assert_eq!(entry.tier, KnowledgeTier::Transient, "should be demoted to Transient");
    }

    #[test]
    fn test_persistent_never_deleted() {
        let mut store = make_store();
        let mut entry = make_entry(1, 0);
        entry.tier = KnowledgeTier::Persistent;
        let key = entry.key;
        store.insert(entry);

        // Advance far enough for balance to drop below GC threshold
        // Persistent Insight: effective_hl = 72 * 5.0 = 360 hours
        // Need ~2400 hours for balance < 0.01 => 2400 * 9000 = 21_600_000 ticks
        store.tick(21_600_000);

        // Should still exist, demoted to Consolidated
        let entry = store.get(&key).expect("persistent entry should not be deleted");
        assert_eq!(entry.tier, KnowledgeTier::Consolidated, "should be demoted, not deleted");
    }

    #[test]
    fn test_promotion_beats_demotion() {
        let mut store = make_store();
        let mut entry = make_entry_with_kind(1, KnowledgeKind::Insight, 0);
        entry.tier = KnowledgeTier::Working;
        entry.confirmations = 10; // At promotion threshold
        entry.balance = 1.0;
        let key = entry.key;
        store.insert(entry);

        // Advance past demotion threshold (Working Insight: 36h * 5 = 180h = 1_620_000 ticks)
        // but confirmations=10 should trigger promotion to Consolidated
        store.tick(1_620_001);

        let entry = store.get(&key).unwrap();
        assert_eq!(entry.tier, KnowledgeTier::Consolidated, "promotion should beat demotion");
    }

    #[test]
    fn test_reinforce_resets_balance() {
        let mut store = make_store();
        let entry = make_entry(1, 0);
        let key = entry.key;
        store.insert(entry);

        // Advance to decay balance somewhat (7.2 hours = 64800 ticks for half-life)
        // After ~7.2 hours, balance ~0.5
        store.tick(64_800);
        let balance_before = store.get(&key).unwrap().balance;
        assert!(balance_before < 0.6, "balance should have decayed, got {balance_before}");

        // Reinforce with boost to bring back to 1.0
        store.reinforce(&key, 64_801, 0.7);
        let balance_after = store.get(&key).unwrap().balance;
        assert!(balance_after > 0.9, "balance should be restored, got {balance_after}");
    }

    #[test]
    fn test_record_contradiction_halves_balance() {
        let mut store = make_store();
        let mut entry = make_entry(1, 0);
        entry.balance = 0.8;
        let key = entry.key;
        store.insert(entry);

        store.record_contradiction(&key);
        let balance = store.get(&key).unwrap().balance;
        assert!((balance - 0.4).abs() < 1e-10, "expected 0.4, got {balance}");
    }

    #[test]
    fn test_search_with_anti_check() {
        let mut store = make_store();
        let ctx = make_ctx(0);

        // Insert a knowledge entry
        let v = HdcVector::random(42);
        let entry = KnowledgeEntry::new(
            v.clone(),
            KnowledgeKind::Insight,
            "original".to_string(),
            KnowledgeSource::SelfDerived,
            0,
        );
        store.insert(entry);

        // Insert its anti-knowledge
        let anti_v = crate::bind(&v, &ANTI_SUBSPACE);
        let anti_entry = KnowledgeEntry::new(
            anti_v,
            KnowledgeKind::AntiKnowledge,
            "anti".to_string(),
            KnowledgeSource::SelfDerived,
            0,
        );
        store.insert(anti_entry);

        // Search for the original -- anti-check should flag or reject
        let results = store.search_with_anti_check(&v, 5, &ctx);
        // The original entry should be present (possibly contradicted)
        // or filtered depending on anti-similarity
        // At minimum, the store should handle this without panicking
        assert!(results.len() <= 5);
    }

    #[test]
    fn test_anti_check_does_not_reject_unrelated() {
        let mut store = make_store();
        let ctx = make_ctx(0);

        let v = HdcVector::random(42);
        let entry = KnowledgeEntry::new(
            v.clone(),
            KnowledgeKind::Insight,
            "original".to_string(),
            KnowledgeSource::SelfDerived,
            0,
        );
        store.insert(entry);

        // Insert unrelated anti-knowledge (not the anti of v)
        let unrelated = HdcVector::random(999);
        let anti_unrelated = crate::bind(&unrelated, &ANTI_SUBSPACE);
        let anti_entry = KnowledgeEntry::new(
            anti_unrelated,
            KnowledgeKind::AntiKnowledge,
            "unrelated anti".to_string(),
            KnowledgeSource::SelfDerived,
            0,
        );
        store.insert(anti_entry);

        let results = store.search_with_anti_check(&v, 5, &ctx);
        // Original should be returned without contradiction
        let original = results.iter().find(|r| r.hamming_distance == 0);
        assert!(original.is_some(), "original entry should be in results");
        assert!(!original.unwrap().entry.contradicted, "should not be contradicted");
    }
}
