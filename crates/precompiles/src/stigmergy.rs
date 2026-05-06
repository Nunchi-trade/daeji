#![allow(missing_docs)]
#![allow(clippy::missing_const_for_fn)]
//! Stigmergy precompile at address `0xA0D`.
//!
//! Owns the on-chain stigmergic state per Will's 2026-05-06 standup:
//!
//! > "Those don't need to go into any contracts necessarily... an agent
//! > would post things like this storage layer that's kind of shared
//! > between everyone... earn rewards based on how many other people
//! > maybe like query this or back this with conviction."
//!
//! What lives here (chain-native, deterministic via event-replay):
//! - **Pheromone counter** per insight (permanent, monotonic).
//! - **Tier** per insight (`Transient` → `Working` → `Consolidated` → `Persistent`).
//! - **Distinct context tags** seen per insight (gates `Consolidated` promotion).
//! - **Effective half-life** computation (`baseHalfLife × tierMultiplierBps / 1000`).
//! - **`currentWeight`** decay formula per spec L417-425:
//!   `weight(t) = INITIAL × 2^(-elapsed / effectiveHalfLife)`,
//!   `boost(p) = (p × 500) / (p + 1000)`,
//!   `effectiveWeight = weight(t) × (1000 + boost) / 1000`.
//!
//! What stays in `InsightBoard.sol` (token-economic surface):
//! - Reward token bookkeeping (`earningsOf`, `claim`).
//! - Confirmation dedupe (one address can only confirm once per insight).
//! - Stake-backed challenges (DAEJI escrow + manager resolution).
//! - Rate limiting on `post()`.
//! - Provenance fields on the `Insight` struct (poster, contentHash,
//!   postedAt, kind, hdcVector, uri, revealAt).
//!
//! Mutation is exclusively via event-replay on `BlockExecutor::on_finalize`:
//! - `InsightPosted` → record `posted_at`, `kind`, `tier=Transient`.
//! - `InsightConfirmed` → increment pheromone, add `contextTag` to set,
//!   bump `distinct_contexts_count` if new.
//! - `InsightPromoted` → update tier.
//!
//! External callers cannot mutate state — same consensus-correctness
//! discipline as `HDCState`.
//!
//! # Selectors (`0xA0D`)
//!
//! - `currentWeight(uint256 id) -> uint256` — 1e6 fixed-point. 0 if unknown / dead.
//! - `tierOf(uint256 id) -> uint8` — 0=Transient, 1=Working, 2=Consolidated, 3=Persistent.
//! - `pheromoneOf(uint256 id) -> uint64`.
//! - `distinctContextsCount(uint256 id) -> uint64`.
//! - `earnedTier(uint256 id) -> uint8` — highest non-Persistent tier criteria are met for.
//!
//! # Gas
//!
//! Flat 5,000 gas per call (cheap reads of in-memory state).

use std::sync::Arc;

#[allow(unused_imports)]
use alloy_primitives::Bytes as _;
use alloy_primitives::{Address, B256, Bytes, U256, address};
use parking_lot::RwLock;
use revm::{
    context::Cfg,
    context_interface::ContextTr,
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{CallInput, CallInputs, Gas, InstructionResult, InterpreterResult},
    primitives::hardfork::SpecId,
};

use crate::insight_id::InsightId;

/// Canonical precompile address — reserved `0xA0D` slot in the Nunchi `0xA00–0xA0F` range.
pub const STIGMERGY_PRECOMPILE_ADDRESS: Address =
    address!("0x0000000000000000000000000000000000000A0D");

/// Flat gas cost per stigmergy precompile call. Reads of in-memory state.
const FLAT_GAS_COST: u64 = 5_000;

/// Initial weight in 1e6 fixed-point (1.0).
const INITIAL_WEIGHT_E6: u64 = 1_000_000;

/// After this many half-life quanta, weight is < 1% of initial. Spec L431.
const DEAD_QUANTA: u64 = 7;

// ---- Selectors (keccak256(sig)[..4]) ----
const SELECTOR_CURRENT_WEIGHT: [u8; 4] = [0x82, 0x05, 0x68, 0xc4]; // currentWeight(uint256)
const SELECTOR_TIER_OF: [u8; 4] = [0x9e, 0x67, 0xf2, 0x09]; // tierOf(uint256)
const SELECTOR_PHEROMONE_OF: [u8; 4] = [0x4f, 0x82, 0xfb, 0x68]; // pheromoneOf(uint256)
const SELECTOR_DISTINCT_CONTEXTS: [u8; 4] = [0x88, 0xfe, 0xfa, 0x12]; // distinctContextsCount(uint256)
const SELECTOR_EARNED_TIER: [u8; 4] = [0xa3, 0xb1, 0x16, 0xc1]; // earnedTier(uint256)

/// Knowledge kind code mirror — matches `InsightBoard.Kind` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum KnowledgeKindCode {
    Insight = 0,
    Heuristic = 1,
    Warning = 2,
    AntiKnowledge = 3,
    CausalLink = 4,
    StrategyFragment = 5,
}

impl KnowledgeKindCode {
    /// Spec on-chain half-life in seconds.
    pub const fn half_life_seconds(self) -> u64 {
        match self {
            Self::Warning => 3 * 60,
            Self::Insight => 7 * 24 * 60 * 60,
            Self::Heuristic => 15 * 24 * 60 * 60,
            Self::AntiKnowledge => 15 * 24 * 60 * 60,
            Self::CausalLink => 15 * 24 * 60 * 60,
            Self::StrategyFragment => 15 * 24 * 60 * 60,
        }
    }

    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Insight),
            1 => Some(Self::Heuristic),
            2 => Some(Self::Warning),
            3 => Some(Self::AntiKnowledge),
            4 => Some(Self::CausalLink),
            5 => Some(Self::StrategyFragment),
            _ => None,
        }
    }
}

/// Retention tier mirror — matches `InsightBoard.Tier` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TierCode {
    Transient = 0,
    Working = 1,
    Consolidated = 2,
    Persistent = 3,
}

impl TierCode {
    /// Multiplier in basis points where 1000 = 1.0× per spec L402-405.
    pub const fn multiplier_bps(self) -> u64 {
        match self {
            Self::Transient => 100,     // 0.1×
            Self::Working => 500,       // 0.5×
            Self::Consolidated => 1000, // 1.0×
            Self::Persistent => 5000,   // 5.0×
        }
    }

    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Transient),
            1 => Some(Self::Working),
            2 => Some(Self::Consolidated),
            3 => Some(Self::Persistent),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Per-insight stigmergic record. Plain struct — no nested locks.
#[derive(Debug, Clone)]
struct InsightRecord {
    posted_at: u64,
    kind: KnowledgeKindCode,
    tier: TierCode,
    pheromone: u64,
    distinct_contexts_count: u64,
    /// Set of context tags seen (membership only — content-addressed via hash).
    /// Uses `Vec<B256>` for determinism across replays. Keeps a flat list
    /// because `HashSet` iteration order is not deterministic and we
    /// occasionally serialize (e.g. snapshots) — even if not today.
    context_tags_seen: Vec<B256>,
}

/// Persistent stigmergy state held across EVM invocations. Sibling to `HDCState`.
#[derive(Debug, Default)]
pub struct StigmergyState {
    insights: RwLock<std::collections::HashMap<InsightId, InsightRecord>>,
    /// Authoritative `InsightBoard` contract address. Events from any other
    /// emitter are filtered out before being applied.
    insight_board: RwLock<Option<Address>>,
}

impl StigmergyState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Authoritative emitter address. Set during chain bootstrap (or on
    /// first finalize); subsequent events from foreign contracts ignored.
    pub fn set_insight_board(&self, address: Address) {
        *self.insight_board.write() = Some(address);
    }

    fn matches_emitter(&self, emitter: Address) -> bool {
        self.insight_board.read().is_some_and(|canonical| canonical == emitter)
    }

    /// Apply a decoded `InsightPosted` event. Records `posted_at`, `kind`,
    /// initial `tier=Transient`. Idempotent on duplicate ids (last-write-wins
    /// — should not happen because contract id is monotonic, but guards
    /// against malformed replay).
    pub fn apply_posted(
        &self,
        emitter: Address,
        id: InsightId,
        posted_at: u64,
        kind: KnowledgeKindCode,
    ) {
        if !self.matches_emitter(emitter) {
            return;
        }
        let mut insights = self.insights.write();
        insights.insert(
            id,
            InsightRecord {
                posted_at,
                kind,
                tier: TierCode::Transient,
                pheromone: 0,
                distinct_contexts_count: 0,
                context_tags_seen: Vec::new(),
            },
        );
    }

    /// Apply a decoded `InsightConfirmed` event. Increments pheromone; if
    /// `context_tag` has not been seen for this insight, increments distinct
    /// count. Caller is responsible for deduping confirmer addresses (the
    /// contract enforces that via `confirmed[id][addr]`).
    pub fn apply_confirmed(&self, emitter: Address, id: InsightId, context_tag: B256) {
        if !self.matches_emitter(emitter) {
            return;
        }
        let mut insights = self.insights.write();
        let Some(record) = insights.get_mut(&id) else {
            // Confirmation arrived before the post replay — should not
            // happen because a transaction ordering rules these into the
            // same block. Defensive no-op.
            return;
        };
        record.pheromone = record.pheromone.saturating_add(1);
        if !record.context_tags_seen.contains(&context_tag) {
            record.context_tags_seen.push(context_tag);
            record.distinct_contexts_count = record.distinct_contexts_count.saturating_add(1);
        }
    }

    /// Apply a decoded `InsightPromoted` event. Last-write-wins on tier.
    pub fn apply_promoted(&self, emitter: Address, id: InsightId, new_tier: TierCode) {
        if !self.matches_emitter(emitter) {
            return;
        }
        let mut insights = self.insights.write();
        if let Some(record) = insights.get_mut(&id) {
            record.tier = new_tier;
        }
    }

    /// Drop entries past `DEAD_QUANTA × effective_half_life_seconds`.
    /// Mirrors `HdcIndex::evict_expired` policy.
    pub fn evict_expired(&self, now_ts: u64) {
        let mut insights = self.insights.write();
        insights.retain(|_, record| {
            let eff_hl = effective_half_life_seconds(record.kind, record.tier);
            if eff_hl == u64::MAX {
                return true;
            }
            let dead_at = record.posted_at.saturating_add(DEAD_QUANTA.saturating_mul(eff_hl));
            now_ts < dead_at
        });
    }

    // ---- Read helpers (also exposed via precompile selectors) ----

    pub fn current_weight_e6(&self, id: InsightId, now_ts: u64) -> u64 {
        let insights = self.insights.read();
        let Some(record) = insights.get(&id) else {
            return 0;
        };
        compute_weight_e6(record, now_ts)
    }

    pub fn tier_of(&self, id: InsightId) -> Option<TierCode> {
        self.insights.read().get(&id).map(|r| r.tier)
    }

    pub fn pheromone_of(&self, id: InsightId) -> u64 {
        self.insights.read().get(&id).map_or(0, |r| r.pheromone)
    }

    pub fn distinct_contexts_count(&self, id: InsightId) -> u64 {
        self.insights.read().get(&id).map_or(0, |r| r.distinct_contexts_count)
    }

    /// Highest tier the insight currently qualifies for, ignoring its
    /// stored tier. Persistent is always manually promoted (returns
    /// `Consolidated` at most).
    pub fn earned_tier(&self, id: InsightId) -> TierCode {
        let insights = self.insights.read();
        let Some(record) = insights.get(&id) else {
            return TierCode::Transient;
        };
        if record.distinct_contexts_count >= 3 {
            return TierCode::Consolidated;
        }
        if record.pheromone >= 2 {
            return TierCode::Working;
        }
        TierCode::Transient
    }
}

/// `effective_half_life = baseHalfLife × tierMultiplierBps / 1000`.
pub const fn effective_half_life_seconds(kind: KnowledgeKindCode, tier: TierCode) -> u64 {
    let base = kind.half_life_seconds();
    base.saturating_mul(tier.multiplier_bps()) / 1000
}

fn compute_weight_e6(record: &InsightRecord, now_ts: u64) -> u64 {
    let elapsed = now_ts.saturating_sub(record.posted_at);
    let eff_hl = effective_half_life_seconds(record.kind, record.tier);
    if eff_hl == 0 {
        return INITIAL_WEIGHT_E6;
    }
    let quanta = elapsed / eff_hl;
    if quanta >= DEAD_QUANTA {
        return 0;
    }
    let base_weight = INITIAL_WEIGHT_E6 >> quanta;
    let p = record.pheromone;
    let boost = (p.saturating_mul(500)) / (p.saturating_add(1000));
    (base_weight.saturating_mul(1000 + boost)) / 1000
}

/// Custom `PrecompileProvider` extending `EthPrecompiles` with `0xA0D` (Stigmergy).
#[derive(Debug)]
pub struct StigmergyPrecompile {
    eth: EthPrecompiles,
    state: Arc<StigmergyState>,
    /// Block timestamp the precompile uses for `currentWeight`'s elapsed
    /// computation. Updated on each finalize via `set_block_timestamp`.
    /// Defaults to 0 — `currentWeight` returns `INITIAL_WEIGHT_E6` for
    /// genesis-clock reads.
    block_timestamp: RwLock<u64>,
}

impl StigmergyPrecompile {
    pub fn new(spec: SpecId, state: Arc<StigmergyState>) -> Self {
        Self { eth: EthPrecompiles::new(spec), state, block_timestamp: RwLock::new(0) }
    }

    pub fn set_block_timestamp(&self, ts: u64) {
        *self.block_timestamp.write() = ts;
    }

    fn run_stigmergy(&self, input: &[u8], gas_limit: u64) -> InterpreterResult {
        let mut gas = Gas::new(gas_limit);
        if !gas.record_regular_cost(FLAT_GAS_COST) {
            return InterpreterResult {
                result: InstructionResult::PrecompileOOG,
                gas,
                output: Bytes::new(),
            };
        }
        if input.len() < 36 {
            return revert(gas, b"stig: calldata too short");
        }
        let selector: [u8; 4] = input[..4].try_into().expect("len>=4 checked");
        let id_bytes: [u8; 32] = input[4..36].try_into().expect("len>=36 checked");
        let id = insight_id_from_word(id_bytes);
        let now = *self.block_timestamp.read();

        match selector {
            SELECTOR_CURRENT_WEIGHT => {
                let weight = self.state.current_weight_e6(id, now);
                ok_uint(gas, U256::from(weight))
            }
            SELECTOR_TIER_OF => {
                let tier = self.state.tier_of(id).map_or(0u8, |t| t.as_u8());
                ok_uint(gas, U256::from(tier))
            }
            SELECTOR_PHEROMONE_OF => {
                let p = self.state.pheromone_of(id);
                ok_uint(gas, U256::from(p))
            }
            SELECTOR_DISTINCT_CONTEXTS => {
                let n = self.state.distinct_contexts_count(id);
                ok_uint(gas, U256::from(n))
            }
            SELECTOR_EARNED_TIER => {
                let t = self.state.earned_tier(id).as_u8();
                ok_uint(gas, U256::from(t))
            }
            _ => revert(gas, b"stig: unknown selector"),
        }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for StigmergyPrecompile {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        <EthPrecompiles as PrecompileProvider<CTX>>::set_spec(&mut self.eth, spec)
    }

    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<Self::Output>, String> {
        if inputs.bytecode_address == STIGMERGY_PRECOMPILE_ADDRESS {
            let input_bytes = match &inputs.input {
                CallInput::Bytes(b) => b.0.as_ref(),
                CallInput::SharedBuffer(_) => {
                    return Ok(Some(revert(
                        Gas::new(inputs.gas_limit),
                        b"stig: shared-buffer calldata unsupported",
                    )));
                }
            };
            let out = self.run_stigmergy(input_bytes, inputs.gas_limit);
            return Ok(Some(out));
        }
        <EthPrecompiles as PrecompileProvider<CTX>>::run(&mut self.eth, context, inputs)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        let mut all: Vec<Address> = self.eth.warm_addresses().collect();
        all.push(STIGMERGY_PRECOMPILE_ADDRESS);
        Box::new(all.into_iter())
    }

    fn contains(&self, address: &Address) -> bool {
        *address == STIGMERGY_PRECOMPILE_ADDRESS || self.eth.contains(address)
    }
}

fn ok_uint(gas: Gas, v: U256) -> InterpreterResult {
    InterpreterResult {
        result: InstructionResult::Return,
        gas,
        output: Bytes::from(v.to_be_bytes::<32>().to_vec()),
    }
}

fn revert(gas: Gas, msg: &[u8]) -> InterpreterResult {
    InterpreterResult {
        result: InstructionResult::Revert,
        gas,
        output: Bytes::copy_from_slice(msg),
    }
}

/// Solidity emits `uint256 id`; only the bottom 128 bits are meaningful
/// (sequential ids from `nextInsightId++`). Truncate to `bytes16` InsightId,
/// big-endian.
fn insight_id_from_word(word: [u8; 32]) -> InsightId {
    let mut bytes16 = [0u8; 16];
    bytes16.copy_from_slice(&word[16..]);
    InsightId(bytes16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: Address = address!("0x000000000000000000000000000000000000000a");

    fn fresh_state() -> Arc<StigmergyState> {
        let s = StigmergyState::new();
        s.set_insight_board(BOARD);
        s
    }

    #[test]
    fn apply_posted_records_initial_state() {
        let state = fresh_state();
        let id = InsightId([0x01; 16]);
        state.apply_posted(BOARD, id, 1_000_000, KnowledgeKindCode::Insight);

        assert_eq!(state.tier_of(id), Some(TierCode::Transient));
        assert_eq!(state.pheromone_of(id), 0);
        assert_eq!(state.distinct_contexts_count(id), 0);
    }

    #[test]
    fn apply_posted_filters_foreign_emitter() {
        let state = fresh_state();
        let other = address!("0x000000000000000000000000000000000000bbbb");
        let id = InsightId([0x02; 16]);
        state.apply_posted(other, id, 1, KnowledgeKindCode::Insight);
        assert!(state.tier_of(id).is_none());
    }

    #[test]
    fn apply_confirmed_increments_pheromone_and_distinct_contexts() {
        let state = fresh_state();
        let id = InsightId([0x03; 16]);
        state.apply_posted(BOARD, id, 1, KnowledgeKindCode::Insight);

        let tag_a = B256::repeat_byte(0xAA);
        let tag_b = B256::repeat_byte(0xBB);

        state.apply_confirmed(BOARD, id, tag_a);
        state.apply_confirmed(BOARD, id, tag_a); // same tag — distinct count stays 1
        state.apply_confirmed(BOARD, id, tag_b);

        assert_eq!(state.pheromone_of(id), 3);
        assert_eq!(state.distinct_contexts_count(id), 2);
    }

    #[test]
    fn earned_tier_progression() {
        let state = fresh_state();
        let id = InsightId([0x04; 16]);
        state.apply_posted(BOARD, id, 1, KnowledgeKindCode::Insight);

        assert_eq!(state.earned_tier(id), TierCode::Transient);

        state.apply_confirmed(BOARD, id, B256::repeat_byte(1));
        state.apply_confirmed(BOARD, id, B256::repeat_byte(1));
        // 2 confirmations, 1 distinct ctx → Working
        assert_eq!(state.earned_tier(id), TierCode::Working);

        state.apply_confirmed(BOARD, id, B256::repeat_byte(2));
        state.apply_confirmed(BOARD, id, B256::repeat_byte(3));
        // 4 confirmations, 3 distinct ctx → Consolidated
        assert_eq!(state.earned_tier(id), TierCode::Consolidated);
    }

    #[test]
    fn apply_promoted_changes_tier() {
        let state = fresh_state();
        let id = InsightId([0x05; 16]);
        state.apply_posted(BOARD, id, 1, KnowledgeKindCode::Insight);
        state.apply_promoted(BOARD, id, TierCode::Persistent);
        assert_eq!(state.tier_of(id), Some(TierCode::Persistent));
    }

    #[test]
    fn current_weight_at_genesis_returns_initial() {
        let state = fresh_state();
        let id = InsightId([0x06; 16]);
        state.apply_posted(BOARD, id, 1_000_000, KnowledgeKindCode::Insight);

        assert_eq!(state.current_weight_e6(id, 1_000_000), INITIAL_WEIGHT_E6);
    }

    #[test]
    fn current_weight_decays_per_effective_half_life() {
        let state = fresh_state();
        let id = InsightId([0x07; 16]);
        state.apply_posted(BOARD, id, 0, KnowledgeKindCode::Insight);
        // Insight × Transient (0.1×) = 7d / 10 = 60480s effective HL.
        let eff_hl = effective_half_life_seconds(KnowledgeKindCode::Insight, TierCode::Transient);
        assert_eq!(eff_hl, 60_480);

        // After 1 effective HL: weight halves.
        assert_eq!(state.current_weight_e6(id, eff_hl), INITIAL_WEIGHT_E6 / 2);
        assert_eq!(state.current_weight_e6(id, 2 * eff_hl), INITIAL_WEIGHT_E6 / 4);
    }

    #[test]
    fn current_weight_zero_past_seven_half_lives() {
        let state = fresh_state();
        let id = InsightId([0x08; 16]);
        state.apply_posted(BOARD, id, 0, KnowledgeKindCode::Warning);
        let eff_hl = effective_half_life_seconds(KnowledgeKindCode::Warning, TierCode::Transient);
        // Warning Transient: 180s × 0.1 = 18s.
        assert_eq!(eff_hl, 18);

        assert_eq!(state.current_weight_e6(id, 7 * eff_hl), 0);
    }

    #[test]
    fn pheromone_boost_caps_near_1_5x_at_genesis() {
        let state = fresh_state();
        let id = InsightId([0x09; 16]);
        state.apply_posted(BOARD, id, 0, KnowledgeKindCode::Insight);
        // Pump pheromone via many confirmations — distinct tags don't matter for boost.
        for i in 0..10_000u32 {
            let mut bytes = [0u8; 32];
            bytes[0..4].copy_from_slice(&i.to_be_bytes());
            state.apply_confirmed(BOARD, id, B256::from_slice(&bytes));
        }
        let weight = state.current_weight_e6(id, 0); // no time elapsed
        // boost = 10000 × 500 / 11000 ≈ 454; factor = 1.454.
        let expected_boost = (10_000u64 * 500) / 11_000;
        let expected = (INITIAL_WEIGHT_E6 * (1000 + expected_boost)) / 1000;
        assert_eq!(weight, expected);
    }

    #[test]
    fn evict_expired_drops_dead_warning() {
        let state = fresh_state();
        let id = InsightId([0x0A; 16]);
        state.apply_posted(BOARD, id, 0, KnowledgeKindCode::Warning);
        assert_eq!(state.tier_of(id), Some(TierCode::Transient));

        // 22 minutes well past 7 × 18s = 126s.
        state.evict_expired(22 * 60);
        assert!(state.tier_of(id).is_none());
    }

    #[test]
    fn deterministic_across_replays() {
        let s1 = fresh_state();
        let s2 = fresh_state();
        let id = InsightId([0x0B; 16]);
        let now = 1_000_000;

        for state in [&s1, &s2] {
            state.apply_posted(BOARD, id, now, KnowledgeKindCode::Insight);
            state.apply_confirmed(BOARD, id, B256::repeat_byte(1));
            state.apply_confirmed(BOARD, id, B256::repeat_byte(2));
            state.apply_promoted(BOARD, id, TierCode::Working);
        }
        assert_eq!(s1.pheromone_of(id), s2.pheromone_of(id));
        assert_eq!(s1.tier_of(id), s2.tier_of(id));
        assert_eq!(s1.distinct_contexts_count(id), s2.distinct_contexts_count(id));
    }
}
