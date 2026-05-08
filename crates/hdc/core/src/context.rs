//! Context assembly pipeline for HDC-based knowledge retrieval.
//!
//! Implements the gather -> rank -> compress -> assemble pipeline that selects
//! the best knowledge entries for a given query within a token budget.
//!
//! **OFF-CHAIN ONLY** — uses f64 arithmetic throughout. Never touches
//! consensus-critical state.

use crate::{
    constants::DUPLICATE_THRESHOLD,
    vector::{HdcVector, hamming_distance, similarity},
};

// ─── Constants ──────────────────────────────────────────────────────────────

/// Exponential decay constant for recency scoring.
/// Age of 100 ticks -> recency score ~0.37 (1/e).
const RECENCY_DECAY: f64 = 100.0;

/// Maximum expected raw importance value (for normalization).
const IMPORTANCE_NORMALIZER: f64 = 7.0;

/// DP granularity for the VCG knapsack solver: 10 tokens per unit.
const VCG_TOKEN_UNIT: usize = 10;

// ─── Scoring weights ────────────────────────────────────────────────────────

const W_RELEVANCE: f64 = 0.35;
const W_IMPORTANCE: f64 = 0.25;
const W_RECENCY: f64 = 0.20;
const W_FRESHNESS: f64 = 0.20;

// ─── Types ──────────────────────────────────────────────────────────────────

/// A candidate knowledge entry for context assembly.
#[derive(Clone, Debug)]
pub struct ContextCandidate {
    /// Content-address identifying the knowledge entry.
    pub entry_id: [u8; 32],
    /// HDC vector of the entry.
    pub vector: HdcVector,
    /// Composite score (set during ranking).
    pub score: f64,
    /// Estimated token cost to include this entry.
    pub token_cost: usize,
    /// Value for VCG auction (how much this entry improves context).
    pub value: f64,

    // ── Scoring inputs ──
    /// Normalized HDC similarity to query (0.0-1.0).
    pub relevance: f64,
    /// Recency: tick when this entry was last reinforced.
    pub last_reinforced: u64,
    /// Confidence of the entry (0.0-1.0).
    pub confidence: f64,
    /// Tier weight (Transient=0.2, Working=0.4, Consolidated=0.7, Persistent=1.0).
    pub tier_weight: f64,
    /// Number of times this entry has been confirmed.
    pub confirmation_count: u32,
    /// Trust score (Local=1.0, Shared=computed, Contrarian=0.8).
    pub trust: f64,
}

/// Result of VCG allocation.
#[derive(Clone, Debug)]
pub struct VcgResult {
    /// Indices of selected items from the input candidate list.
    pub selected: Vec<usize>,
    /// VCG externality payment for each selected item (parallel to `selected`).
    pub payments: Vec<f64>,
}

/// The assembled context output.
#[derive(Clone, Debug)]
pub struct AssembledContext {
    /// Selected candidates that fit within the token budget.
    pub entries: Vec<ContextCandidate>,
    /// VCG payment for each selected entry (parallel to `entries`).
    pub payments: Vec<f64>,
    /// Token budget remaining after assembly.
    pub budget_remaining: usize,
}

// ─── Token estimation ───────────────────────────────────────────────────────

/// Approximate token count for a content string.
/// Uses cl100k_base heuristic: ~3.5 chars/token for mixed code/prose.
/// Conservative (overestimates) to avoid exceeding budget.
pub fn estimate_tokens(content_len: usize) -> usize {
    if content_len == 0 {
        return 0;
    }
    ((content_len as f64 / 3.5).ceil() as usize).max(1)
}

// ─── Phase 1: Gather ────────────────────────────────────────────────────────

/// Collect candidate knowledge entries by querying a store.
///
/// The `query_fn` closure performs the actual search, returning candidates
/// with their vectors and metadata. This allows decoupling from the concrete
/// knowledge store implementation.
///
/// Results are capped at `max_candidates`.
pub fn gather<F>(query: &HdcVector, max_candidates: usize, query_fn: F) -> Vec<ContextCandidate>
where
    F: FnOnce(&HdcVector, usize) -> Vec<ContextCandidate>,
{
    let mut candidates = query_fn(query, max_candidates);

    // Compute relevance (HDC similarity to query) for each candidate.
    for c in &mut candidates {
        c.relevance = similarity(query, &c.vector);
    }

    // Cap to max_candidates, keeping highest relevance.
    candidates.sort_by(|a, b| b.relevance.total_cmp(&a.relevance));
    candidates.truncate(max_candidates);
    candidates
}

// ─── Phase 2: Rank ──────────────────────────────────────────────────────────

/// Compute composite scores and sort candidates.
///
/// Four-factor scoring:
/// - relevance (0.35): HDC similarity to query
/// - importance (0.25): confidence * tier_weight * ln(confirmations + 1)
/// - recency (0.20): exponential decay based on tick age
/// - freshness/trust (0.20): trust score weighted by recency
///
/// Candidates are sorted by composite score descending, with ties broken
/// by entry_id for deterministic ordering.
pub fn rank(candidates: &mut [ContextCandidate], current_tick: u64) {
    for c in candidates.iter_mut() {
        let age = current_tick.saturating_sub(c.last_reinforced) as f64;
        let recency = (-age / RECENCY_DECAY).exp();

        let raw_importance =
            c.confidence * c.tier_weight * (c.confirmation_count as f64 + 1.0).ln();
        let importance = (raw_importance / IMPORTANCE_NORMALIZER).min(1.0);

        let relevance = c.relevance;
        let freshness = c.trust * recency;

        c.score = W_RELEVANCE * relevance
            + W_IMPORTANCE * importance
            + W_RECENCY * recency
            + W_FRESHNESS * freshness;

        // Value for VCG = score (can be refined later with emotional weighting).
        c.value = c.score;
    }

    // Sort descending by score, break ties by entry_id.
    candidates
        .sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.entry_id.cmp(&b.entry_id)));
}

// ─── Phase 3: Compress ──────────────────────────────────────────────────────

/// Remove near-duplicate entries based on Hamming distance.
///
/// Two candidates are duplicates if their vectors have Hamming distance
/// <= `duplicate_threshold`. The higher-scored duplicate is kept.
///
/// Candidates must be sorted by score descending (from `rank`).
pub fn compress(candidates: &mut Vec<ContextCandidate>, duplicate_threshold: u32) {
    let threshold =
        if duplicate_threshold == 0 { DUPLICATE_THRESHOLD } else { duplicate_threshold };

    // Already sorted by score desc from rank phase.
    let mut keep = vec![true; candidates.len()];
    for i in 0..candidates.len() {
        if !keep[i] {
            continue;
        }
        for j in (i + 1)..candidates.len() {
            if !keep[j] {
                continue;
            }
            let dist = hamming_distance(&candidates[i].vector, &candidates[j].vector);
            if dist <= threshold {
                keep[j] = false; // j has lower score, drop it
            }
        }
    }

    let mut idx = 0;
    candidates.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

// ─── VCG Auction ────────────────────────────────────────────────────────────

/// Standard 0/1 knapsack via dynamic programming.
///
/// Arguments:
/// - `items`: each item is (weight/cost, value)
/// - `capacity`: total budget in weight units
///
/// Returns indices of selected items.
///
/// Complexity: O(n * capacity).
pub fn knapsack_01(items: &[(usize, f64)], capacity: usize) -> Vec<usize> {
    let n = items.len();
    if n == 0 || capacity == 0 {
        return Vec::new();
    }

    // dp[w] = best value achievable with capacity w.
    let mut dp = vec![0.0_f64; capacity + 1];
    // Track choices for backtracking.
    let mut choice = vec![vec![false; capacity + 1]; n];

    for i in 0..n {
        let (w_i, v_i) = items[i];
        if w_i == 0 || w_i > capacity {
            continue;
        }
        // Iterate in reverse to enforce 0/1 constraint.
        for w in (w_i..=capacity).rev() {
            let value_with = dp[w - w_i] + v_i;
            if value_with > dp[w] {
                dp[w] = value_with;
                choice[i][w] = true;
            }
        }
    }

    // Backtrack to find selected items.
    let mut selected = Vec::new();
    let mut w = capacity;
    for i in (0..n).rev() {
        let (w_i, _) = items[i];
        if w_i <= w && choice[i][w] {
            selected.push(i);
            w -= w_i;
        }
    }
    selected.reverse(); // Return in ascending order.
    selected
}

/// Run VCG allocation over candidates.
///
/// 1. Solve 0/1 knapsack with all candidates to find optimal allocation.
/// 2. For each winner, re-solve excluding that winner.
/// 3. Compute VCG payment: externality cost each winner imposes on others.
///
/// VCG payment formula:
///   payment_i = value_without_i - (total_value_with_all - value_of_i)
///
/// Payments are non-negative (clamped to >= 0.0).
pub fn vcg_allocate(items: &[(usize, f64)], capacity: usize) -> VcgResult {
    if items.is_empty() || capacity == 0 {
        return VcgResult { selected: Vec::new(), payments: Vec::new() };
    }

    // Step 1: Optimal allocation with all candidates.
    let winners = knapsack_01(items, capacity);
    let optimal_value: f64 = winners.iter().map(|&i| items[i].1).sum();

    // Step 2: For each winner, compute VCG payment.
    let mut payments = Vec::with_capacity(winners.len());
    for &winner_idx in &winners {
        // Build item list excluding this winner.
        let others: Vec<(usize, f64)> = items
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != winner_idx)
            .map(|(_, item)| *item)
            .collect();

        let others_winners = knapsack_01(&others, capacity);
        let value_without: f64 = others_winners.iter().map(|&i| others[i].1).sum();

        // VCG payment = value others would get without me
        //             - value others actually get with me.
        let my_value = items[winner_idx].1;
        let others_value_with_me = optimal_value - my_value;
        let payment = (value_without - others_value_with_me).max(0.0);

        payments.push(payment);
    }

    VcgResult { selected: winners, payments }
}

// ─── Phase 4: Assemble ─────────────────────────────────────────────────────

/// Select the best candidates that fit within the token budget using VCG auction.
///
/// Converts candidates into knapsack items (weight = token_cost in VCG units,
/// value = score), runs VCG allocation, and returns the selected entries
/// with their payments.
pub fn assemble(candidates: &[ContextCandidate], token_budget: usize) -> AssembledContext {
    if candidates.is_empty() || token_budget == 0 {
        return AssembledContext {
            entries: Vec::new(),
            payments: Vec::new(),
            budget_remaining: token_budget,
        };
    }

    // Convert to knapsack items: (weight in VCG units, value).
    let items: Vec<(usize, f64)> = candidates
        .iter()
        .map(|c| {
            let weight = c.token_cost.div_ceil(VCG_TOKEN_UNIT);
            (weight.max(1), c.value) // min weight of 1 unit
        })
        .collect();

    let capacity = token_budget / VCG_TOKEN_UNIT;
    let vcg = vcg_allocate(&items, capacity);

    let mut entries = Vec::with_capacity(vcg.selected.len());
    let mut payments = Vec::with_capacity(vcg.selected.len());
    let mut tokens_used = 0;

    for (i, &idx) in vcg.selected.iter().enumerate() {
        let candidate = &candidates[idx];
        if tokens_used + candidate.token_cost <= token_budget {
            entries.push(candidate.clone());
            payments.push(vcg.payments[i]);
            tokens_used += candidate.token_cost;
        }
    }

    AssembledContext {
        entries,
        payments,
        budget_remaining: token_budget.saturating_sub(tokens_used),
    }
}

// ─── Full Pipeline ──────────────────────────────────────────────────────────

/// Run the full context assembly pipeline: gather -> rank -> compress -> assemble.
///
/// Parameters:
/// - `query`: HDC query vector
/// - `token_budget`: maximum tokens for the assembled context
/// - `current_tick`: current simulation tick (for recency scoring)
/// - `max_candidates`: maximum candidates to gather
/// - `query_fn`: closure that searches the knowledge store
///
/// Returns the assembled context with selected entries and VCG payments.
pub fn assemble_context<F>(
    query: &HdcVector,
    token_budget: usize,
    current_tick: u64,
    max_candidates: usize,
    query_fn: F,
) -> AssembledContext
where
    F: FnOnce(&HdcVector, usize) -> Vec<ContextCandidate>,
{
    // Phase 1: Gather candidates from knowledge store.
    let mut candidates = gather(query, max_candidates, query_fn);

    // Phase 2: Rank by composite score.
    rank(&mut candidates, current_tick);

    // Phase 3: Compress (remove near-duplicates).
    compress(&mut candidates, DUPLICATE_THRESHOLD);

    // Phase 4: Assemble within token budget via VCG auction.
    assemble(&candidates, token_budget)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::HdcVector;

    /// Helper: create a candidate with given parameters.
    fn make_candidate(
        seed: u64,
        token_cost: usize,
        confidence: f64,
        tier_weight: f64,
        confirmations: u32,
        last_reinforced: u64,
        trust: f64,
    ) -> ContextCandidate {
        let vector = HdcVector::random(seed);
        let entry_id = crate::vector::vector_id(&vector);
        ContextCandidate {
            entry_id,
            vector,
            score: 0.0,
            token_cost,
            value: 0.0,
            relevance: 0.0,
            last_reinforced,
            confidence,
            tier_weight,
            confirmation_count: confirmations,
            trust,
        }
    }

    // ── gather tests ────────────────────────────────────────────────────

    #[test]
    fn gather_returns_relevant_candidates() {
        let query = HdcVector::random(1);
        let candidates = gather(&query, 5, |_q, max| {
            // Return some candidates with different seeds.
            (0..10).map(|i| make_candidate(100 + i, 50, 0.8, 1.0, 5, 90, 1.0)).take(max).collect()
        });

        assert_eq!(candidates.len(), 5);
        // All should have relevance computed.
        for c in &candidates {
            assert!(c.relevance >= 0.0 && c.relevance <= 1.0);
        }
    }

    #[test]
    fn gather_caps_at_max_candidates() {
        let query = HdcVector::random(2);
        let candidates = gather(&query, 3, |_q, _max| {
            (0..20).map(|i| make_candidate(200 + i, 30, 0.9, 0.7, 3, 80, 1.0)).collect()
        });
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn gather_returns_empty_for_empty_store() {
        let query = HdcVector::random(3);
        let candidates = gather(&query, 10, |_q, _max| Vec::new());
        assert!(candidates.is_empty());
    }

    // ── rank tests ──────────────────────────────────────────────────────

    #[test]
    fn rank_sorts_correctly() {
        let mut candidates = vec![
            make_candidate(100, 50, 0.3, 0.2, 1, 0, 0.5), // low score
            make_candidate(101, 50, 1.0, 1.0, 100, 99, 1.0), // high score
            make_candidate(102, 50, 0.5, 0.5, 5, 50, 0.8), // medium score
        ];

        // Set relevance manually for predictable results.
        candidates[0].relevance = 0.2;
        candidates[1].relevance = 0.9;
        candidates[2].relevance = 0.5;

        rank(&mut candidates, 100);

        // Should be sorted descending by score.
        assert!(candidates[0].score >= candidates[1].score);
        assert!(candidates[1].score >= candidates[2].score);

        // Highest relevance + confidence + recency candidate should be first.
        assert!(candidates[0].relevance > 0.5);
    }

    #[test]
    fn rank_recency_decays_with_age() {
        let mut recent = make_candidate(200, 50, 0.8, 1.0, 5, 99, 1.0);
        recent.relevance = 0.5;

        let mut old = make_candidate(201, 50, 0.8, 1.0, 5, 0, 1.0);
        old.relevance = 0.5;

        let mut candidates = vec![old, recent];
        rank(&mut candidates, 100);

        // Recent entry should score higher due to recency factor.
        assert!(candidates[0].last_reinforced > candidates[1].last_reinforced);
    }

    #[test]
    fn rank_scores_are_positive() {
        let mut candidates = vec![
            make_candidate(300, 50, 0.0, 0.2, 0, 0, 0.1),
            make_candidate(301, 50, 1.0, 1.0, 100, 100, 1.0),
        ];
        candidates[0].relevance = 0.1;
        candidates[1].relevance = 0.9;

        rank(&mut candidates, 100);

        for c in &candidates {
            assert!(c.score >= 0.0, "Score should be non-negative: {}", c.score);
        }
    }

    // ── compress tests ──────────────────────────────────────────────────

    #[test]
    fn compress_removes_duplicates() {
        let base = HdcVector::random(400);
        let entry_id = crate::vector::vector_id(&base);

        // Create two candidates with the same vector (distance = 0).
        let mut c1 = make_candidate(400, 50, 0.8, 1.0, 5, 90, 1.0);
        c1.score = 0.9;
        c1.vector = base.clone();
        c1.entry_id = entry_id;

        let mut c2 = make_candidate(400, 50, 0.5, 0.5, 2, 80, 0.8);
        c2.score = 0.5;
        c2.vector = base.clone();
        c2.entry_id = entry_id;

        let mut candidates = vec![c1, c2];
        compress(&mut candidates, DUPLICATE_THRESHOLD);

        // Should keep only one (the higher-scored one).
        assert_eq!(candidates.len(), 1);
        assert!((candidates[0].score - 0.9).abs() < 1e-10);
    }

    #[test]
    fn compress_keeps_dissimilar_entries() {
        // Two random vectors are quasi-orthogonal (~5120 Hamming distance),
        // well above DUPLICATE_THRESHOLD (512).
        let mut c1 = make_candidate(500, 50, 0.8, 1.0, 5, 90, 1.0);
        c1.score = 0.9;

        let mut c2 = make_candidate(501, 50, 0.5, 0.5, 2, 80, 0.8);
        c2.score = 0.5;

        let mut candidates = vec![c1, c2];
        compress(&mut candidates, DUPLICATE_THRESHOLD);

        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn compress_empty_is_noop() {
        let mut candidates: Vec<ContextCandidate> = Vec::new();
        compress(&mut candidates, DUPLICATE_THRESHOLD);
        assert!(candidates.is_empty());
    }

    // ── knapsack_01 tests ───────────────────────────────────────────────

    #[test]
    fn knapsack_01_basic_correctness() {
        // Classic knapsack: capacity 50.
        // Items: (weight, value)
        let items = vec![
            (10, 60.0),  // item 0
            (20, 100.0), // item 1
            (30, 120.0), // item 2
        ];

        let selected = knapsack_01(&items, 50);
        let total_weight: usize = selected.iter().map(|&i| items[i].0).sum();
        let total_value: f64 = selected.iter().map(|&i| items[i].1).sum();

        assert!(total_weight <= 50);
        // Optimal: items 1+2 = weight 50, value 220.
        assert!((total_value - 220.0).abs() < 1e-10);
        assert!(selected.contains(&1));
        assert!(selected.contains(&2));
    }

    #[test]
    fn knapsack_01_single_item_fits() {
        let items = vec![(5, 10.0)];
        let selected = knapsack_01(&items, 5);
        assert_eq!(selected, vec![0]);
    }

    #[test]
    fn knapsack_01_single_item_too_heavy() {
        let items = vec![(10, 100.0)];
        let selected = knapsack_01(&items, 5);
        assert!(selected.is_empty());
    }

    #[test]
    fn knapsack_01_empty_items() {
        let items: Vec<(usize, f64)> = Vec::new();
        let selected = knapsack_01(&items, 100);
        assert!(selected.is_empty());
    }

    #[test]
    fn knapsack_01_zero_capacity() {
        let items = vec![(5, 10.0), (3, 7.0)];
        let selected = knapsack_01(&items, 0);
        assert!(selected.is_empty());
    }

    #[test]
    fn knapsack_01_all_items_fit() {
        let items = vec![(2, 5.0), (3, 8.0), (4, 10.0)];
        let selected = knapsack_01(&items, 100);
        // All items should be selected when capacity is large.
        assert_eq!(selected.len(), 3);
    }

    // ── VCG pricing tests ───────────────────────────────────────────────

    #[test]
    fn vcg_payments_are_non_negative() {
        let items = vec![(10, 60.0), (20, 100.0), (30, 120.0)];

        let result = vcg_allocate(&items, 50);

        for payment in &result.payments {
            assert!(*payment >= 0.0, "VCG payment should be non-negative, got {}", payment);
        }
    }

    #[test]
    fn vcg_truthful_pricing() {
        // With capacity 50 and items (10, 60), (20, 100), (30, 120):
        // Optimal: items 1+2, total value 220.
        //
        // Payment for item 1 (weight 20, value 100):
        //   Without item 1: knapsack over {0, 2} with cap 50 -> items 0+2, value 180.
        //   Others' value with item 1: 220 - 100 = 120.
        //   Payment = 180 - 120 = 60.
        //
        // Payment for item 2 (weight 30, value 120):
        //   Without item 2: knapsack over {0, 1} with cap 50 -> items 0+1, value 160.
        //   Others' value with item 2: 220 - 120 = 100.
        //   Payment = 160 - 100 = 60.
        let items = vec![(10, 60.0), (20, 100.0), (30, 120.0)];

        let result = vcg_allocate(&items, 50);

        assert_eq!(result.selected.len(), 2);
        assert!(result.selected.contains(&1));
        assert!(result.selected.contains(&2));

        // Both payments should be 60.0.
        for (i, &idx) in result.selected.iter().enumerate() {
            if idx == 1 {
                assert!(
                    (result.payments[i] - 60.0).abs() < 1e-10,
                    "Payment for item 1 should be 60.0, got {}",
                    result.payments[i]
                );
            }
            if idx == 2 {
                assert!(
                    (result.payments[i] - 60.0).abs() < 1e-10,
                    "Payment for item 2 should be 60.0, got {}",
                    result.payments[i]
                );
            }
        }
    }

    #[test]
    fn vcg_empty_input() {
        let result = vcg_allocate(&[], 100);
        assert!(result.selected.is_empty());
        assert!(result.payments.is_empty());
    }

    #[test]
    fn vcg_zero_capacity() {
        let items = vec![(10, 60.0)];
        let result = vcg_allocate(&items, 0);
        assert!(result.selected.is_empty());
    }

    #[test]
    fn vcg_single_item_no_competition() {
        // Single item, no competition -> payment should be 0.
        let items = vec![(5, 10.0)];
        let result = vcg_allocate(&items, 10);
        assert_eq!(result.selected, vec![0]);
        assert_eq!(result.payments.len(), 1);
        assert!(
            result.payments[0].abs() < 1e-10,
            "Single item with no competition should have zero payment"
        );
    }

    // ── assemble tests ──────────────────────────────────────────────────

    #[test]
    fn assemble_respects_token_budget() {
        let candidates: Vec<ContextCandidate> = (0..5)
            .map(|i| {
                let mut c = make_candidate(600 + i, 100, 0.8, 1.0, 5, 90, 1.0);
                c.value = 1.0 - (i as f64 * 0.1);
                c
            })
            .collect();

        // Budget for ~3 entries (300 tokens, each costs 100).
        let result = assemble(&candidates, 300);

        let total_cost: usize = result.entries.iter().map(|e| e.token_cost).sum();
        assert!(total_cost <= 300, "Total token cost {} exceeds budget 300", total_cost);
    }

    #[test]
    fn assemble_empty_candidates() {
        let result = assemble(&[], 1000);
        assert!(result.entries.is_empty());
        assert_eq!(result.budget_remaining, 1000);
    }

    #[test]
    fn assemble_zero_budget() {
        let candidates = vec![make_candidate(700, 50, 0.8, 1.0, 5, 90, 1.0)];
        let result = assemble(&candidates, 0);
        assert!(result.entries.is_empty());
    }

    // ── Full pipeline E2E test ──────────────────────────────────────────

    #[test]
    fn full_pipeline_end_to_end() {
        let query = HdcVector::random(1000);

        let result = assemble_context(
            &query,
            500, // token budget
            100, // current tick
            20,  // max candidates
            |_q, _max| {
                // Simulate a knowledge store returning varied candidates.
                (0..15)
                    .map(|i| {
                        make_candidate(
                            1000 + i,
                            30 + (i as usize * 5), // varying token costs
                            0.5 + (i as f64 * 0.03),
                            if i < 5 {
                                0.2
                            } else if i < 10 {
                                0.7
                            } else {
                                1.0
                            },
                            i as u32,
                            80 + i,
                            if i % 3 == 0 { 0.8 } else { 1.0 },
                        )
                    })
                    .collect()
            },
        );

        // Should have selected some entries.
        assert!(!result.entries.is_empty(), "Pipeline should select entries");

        // Total token cost should not exceed budget.
        let total_cost: usize = result.entries.iter().map(|e| e.token_cost).sum();
        assert!(total_cost <= 500, "Total cost {} exceeds budget 500", total_cost);

        // Budget remaining should be correct.
        assert_eq!(result.budget_remaining, 500 - total_cost);

        // Payments should be parallel to entries.
        assert_eq!(result.entries.len(), result.payments.len());

        // All payments should be non-negative.
        for p in &result.payments {
            assert!(*p >= 0.0, "Payment should be non-negative: {}", p);
        }
    }

    #[test]
    fn full_pipeline_with_duplicates() {
        let query = HdcVector::random(2000);
        let dup_vector = HdcVector::random(2001);

        let result = assemble_context(&query, 1000, 100, 20, |_q, _max| {
            let mut candidates = Vec::new();
            // Add 5 unique candidates.
            for i in 0..5 {
                candidates.push(make_candidate(2010 + i, 50, 0.8, 1.0, 5, 90, 1.0));
            }
            // Add 3 duplicates (same vector).
            for _i in 0..3 {
                let mut c = make_candidate(2001, 50, 0.7, 0.8, 3, 85, 1.0);
                c.vector = dup_vector.clone();
                c.entry_id = crate::vector::vector_id(&dup_vector);
                candidates.push(c);
            }
            candidates
        });

        // Duplicates should have been compressed away - at most 1 of the 3 dups remains.
        // Total unique entries: 5 + 1 = 6 max.
        assert!(result.entries.len() <= 6);
    }

    // ── estimate_tokens test ────────────────────────────────────────────

    #[test]
    fn estimate_tokens_correctness() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(1), 1);
        assert_eq!(estimate_tokens(7), 2); // 7/3.5 = 2.0
        assert_eq!(estimate_tokens(35), 10); // 35/3.5 = 10.0
        assert_eq!(estimate_tokens(36), 11); // ceil(36/3.5) = 11
    }
}
