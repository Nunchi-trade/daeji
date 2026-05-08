# `kora-hdc-chain` -- On-Chain HDC Integration Layer

> **Status: PARTIALLY IMPLEMENTED** -- Core structures exist but many handlers
> are stubs. PR #42 introduced a parallel precompile implementation that
> partially overlaps with this crate's scope.

## Goal

Create the `kora-hdc-chain` crate at `crates/hdc/chain/`. This crate bridges
`kora-hdc` (the pure HDC algebra crate with zero chain dependencies) with the
blockchain runtime. It provides the on-chain HDC index, consensus-safe decay
arithmetic, gas-optimized tiered search, event synchronization, and the
WisdomGate filter.

The crate is consumed by:
- `kora-executor` -- registers the HDC precompile at address `0x09`
- `kora-rpc` -- exposes HDC query methods over JSON-RPC

It depends on:
- `kora-hdc` -- core algebra (`HdcVector`, `LocalIndex`, `BundleAccumulator`,
  `hamming_distance`)
- `alloy-primitives` -- `H256`, `Address`, `Bytes`
- `tokio` -- async event subscription
- `tracing` -- structured logging

---

## Repository Context

The repo root is `/Users/will/dev/nunchi/daeji/`. It is a Rust workspace using
edition 2024 and resolver 2. The workspace `Cargo.toml` is at the root. Key
conventions:

- All workspace crates live under `crates/`.
- The workspace members glob is `"crates/hdc/*"` (you will add this).
- Workspace lints: `missing-docs = "warn"`, `unused-must-use = "deny"`,
  `clippy::all = "warn"`.
- Dependencies are declared in `[workspace.dependencies]` and referenced via
  `.workspace = true` in each crate.
- Common deps already in the workspace: `alloy-primitives = "1.0"`,
  `tokio`, `tracing`, `thiserror = "2"`, `serde`, `parking_lot`.

The `crates/hdc/` directory does not exist yet. You will create both
`crates/hdc/core/` (the `kora-hdc` crate, documented in `02-kora-hdc-core.md`)
and `crates/hdc/chain/` (this crate). If `kora-hdc` is not yet implemented, stub
it with the types and traits listed below so that `kora-hdc-chain` compiles.

---

## 1. Workspace Changes

### `Cargo.toml` (workspace root)

Add the members glob and workspace dependencies:

```toml
# In [workspace] members list, add:
members = [
    # ... existing entries ...
    "crates/hdc/*",
]

# In [workspace.dependencies], add:
kora-hdc = { path = "crates/hdc/core" }
kora-hdc-chain = { path = "crates/hdc/chain" }
```

### `crates/hdc/chain/Cargo.toml`

```toml
[package]
name = "kora-hdc-chain"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "On-chain HDC integration layer for Kora"

[dependencies]
kora-hdc = { workspace = true }
alloy-primitives.workspace = true
tokio = { workspace = true, features = ["sync", "rt"] }
tracing.workspace = true
thiserror.workspace = true
serde = { workspace = true, features = ["derive"] }
parking_lot.workspace = true

[dev-dependencies]
rstest.workspace = true
tokio = { workspace = true, features = ["macros", "test-util"] }

[lints]
workspace = true
```

---

## 2. Module Structure

```
crates/hdc/chain/
  Cargo.toml
  src/
    lib.rs               # Public re-exports, crate-level docs
    index.rs             # OnChainHdcIndex
    decay.rs             # fixed_point_decay, fixed_point_exp2_neg
    search.rs            # Tiered search (gas-optimized)
    sync.rs              # NeuroChainSync (event replay)
    wisdom_gate.rs       # WisdomGate filter
    types.rs             # Shared types (InsightEvent, InsightMetadata, etc.)
    error.rs             # Error types
```

### `src/lib.rs`

```rust
//! On-chain HDC integration layer for Kora.
//!
//! This crate bridges [`kora_hdc`] (core algebra) with the blockchain
//! runtime. It provides:
//!
//! - [`OnChainHdcIndex`]: In-memory vector index rebuilt from chain events
//! - [`fixed_point_decay`]: Consensus-safe exponential decay in integer arithmetic
//! - [`TieredSearch`]: Gas-optimized 3-tier search pipeline
//! - [`NeuroChainSync`]: Event subscription and replay for index synchronization
//! - [`WisdomGate`]: Trust-aware filter for consuming shared knowledge

#![doc(issue_tracker_base_url = "https://github.com/refcell/kora/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod index;
pub use index::OnChainHdcIndex;

mod decay;
pub use decay::{fixed_point_decay, fixed_point_exp2_neg};

mod search;
pub use search::{TieredSearch, TieredSearchResult, TIER1_GAS, TIER2_GAS, TIER3_GAS};

mod sync;
pub use sync::NeuroChainSync;

mod wisdom_gate;
pub use wisdom_gate::{WisdomGate, WisdomGateConfig, WisdomRejection};

mod types;
pub use types::{
    InsightEvent, InsightKind, InsightMetadata, InsightState, InsightTier,
    PheromoneDeposit, PheromoneType,
};

mod error;
pub use error::ChainHdcError;
```

---

## 3. Types (`src/types.rs`)

```rust
use alloy_primitives::{Address, H256};
use serde::{Deserialize, Serialize};

/// Events emitted by the InsightBoard contract.
///
/// Used for index reconstruction via [`NeuroChainSync`].
#[derive(Clone, Debug)]
pub enum InsightEvent {
    /// A new insight was published on-chain.
    Published {
        /// Unique identifier (keccak256 of vectorHash + author + blockNumber).
        insight_id: H256,
        /// The full 10,240-bit HDC vector (1,280 bytes).
        vector: kora_hdc::HdcVector,
        /// Metadata extracted from the event.
        metadata: InsightMetadata,
    },
    /// An insight was confirmed by another agent.
    Confirmed {
        insight_id: H256,
        confirmer: Address,
        total_confirmations: u64,
    },
    /// An insight was deleted (purged) from the index.
    Deleted {
        insight_id: H256,
    },
}

/// Metadata stored alongside each vector in the on-chain index.
///
/// This is reconstructed from InsightPublished event data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InsightMetadata {
    /// Author address.
    pub author: Address,
    /// Block number at which the insight was published.
    pub publish_block: u64,
    /// Knowledge kind (Insight, Heuristic, Warning, etc.).
    pub kind: InsightKind,
    /// Current tier (Transient, Working, Consolidated, Persistent).
    pub tier: InsightTier,
    /// Current lifecycle state.
    pub state: InsightState,
    /// Number of independent confirmations received.
    pub confirmations: u64,
    /// Block number of the most recent confirmation.
    pub last_confirmed_block: u64,
    /// keccak256 of the content payload.
    pub content_hash: H256,
}

/// The six knowledge kinds. Values must match the Solidity enum.
///
/// CONSENSUS SAFETY: The discriminant values are protocol-defined.
/// Do not reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InsightKind {
    Insight = 0,
    Heuristic = 1,
    AntiKnowledge = 2,
    Warning = 3,
    CausalLink = 4,
    Strategy = 5,
}

/// Lifecycle state of an on-chain insight. Matches the Solidity enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InsightState {
    Submitted = 0,
    Verified = 1,
    Active = 2,
    Challenged = 3,
    Decaying = 4,
    Archived = 5,
    Purged = 6,
}

/// Promotion tier. Matches the Solidity enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InsightTier {
    Transient = 0,
    Working = 1,
    Consolidated = 2,
    Persistent = 3,
}

/// Pheromone deposit event from the PheromoneRegistry contract.
#[derive(Clone, Debug)]
pub struct PheromoneDeposit {
    pub deposit_id: H256,
    pub depositor: Address,
    pub pheromone_type: PheromoneType,
    pub target: H256,
    pub intensity_bps: u64,
    pub deposit_block: u64,
}

/// Pheromone type. Matches the Solidity enum in PheromoneRegistry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum PheromoneType {
    Threat = 0,
    Opportunity = 1,
    Wisdom = 2,
}
```

---

## 4. On-Chain Index (`src/index.rs`)

```rust
use std::collections::HashMap;

use alloy_primitives::H256;
use kora_hdc::{HdcVector, LocalIndex};

use crate::{InsightEvent, InsightMetadata};

/// In-memory HDC index rebuilt deterministically from chain events.
///
/// This is the Rust-side mirror of the precompile's vector store.
/// On node startup, it is rebuilt by replaying all `InsightPublished`
/// and `InsightPurged` events in block order.
///
/// CONSENSUS SAFETY: The `metadata` HashMap is accessed by key lookup
/// only -- never iterated. If any future code path iterates this map
/// (e.g., for serialization, snapshotting, or garbage collection),
/// it MUST be replaced with `BTreeMap<H256, InsightMetadata>`.
pub struct OnChainHdcIndex {
    /// The underlying HDC index (brute-force or HNSW, auto-switching
    /// at 100K vectors). Provided by `kora-hdc`.
    index: LocalIndex,

    /// Per-vector metadata. Key lookup only, NOT iterated.
    metadata: HashMap<H256, InsightMetadata>,
}

impl OnChainHdcIndex {
    /// Create a new empty index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            index: LocalIndex::new(),
            metadata: HashMap::new(),
        }
    }

    /// Store a vector with its metadata.
    ///
    /// Called during block execution when processing a `storeVector`
    /// precompile call. The insertion order is determined by the
    /// canonical transaction ordering within the block.
    ///
    /// CONSENSUS SAFETY: Insertion order matters for HNSW graph
    /// construction. Vectors must be inserted in the order they
    /// appear in finalized blocks.
    pub fn store(&mut self, key: H256, vector: HdcVector, meta: InsightMetadata) {
        self.index.insert(key, vector);
        self.metadata.insert(key, meta);
    }

    /// Remove a vector from the index.
    ///
    /// Called when processing a `deleteVector` precompile call
    /// (triggered by `purge()` on the InsightBoard contract).
    pub fn remove(&mut self, key: &H256) {
        self.index.remove(*key);
        self.metadata.remove(key);
    }

    /// Search for the top-K most similar vectors.
    ///
    /// Returns `(insight_id, hamming_distance)` pairs sorted by
    /// distance ascending, with ties broken by `insight_id`
    /// lexicographic order.
    ///
    /// CONSENSUS SAFETY: This is a read-only operation. It does
    /// not modify state and can be called from `eth_call` (view)
    /// context without consensus implications.
    pub fn search(&self, query: &HdcVector, top_k: usize) -> Vec<(H256, u32)> {
        let mut results = self.index.search(query, top_k);
        // Deterministic sort: distance first, then key for tiebreaking.
        results.sort_by(|(k1, d1), (k2, d2)| d1.cmp(d2).then_with(|| k1.cmp(k2)));
        results
    }

    /// Look up metadata for a specific insight.
    ///
    /// Returns `None` if the insight has been purged or never existed.
    #[must_use]
    pub fn get_metadata(&self, key: &H256) -> Option<&InsightMetadata> {
        self.metadata.get(key)
    }

    /// Update metadata in place (e.g., after a confirmation event
    /// increments the confirmation count or changes the tier).
    pub fn update_metadata(&mut self, key: &H256, f: impl FnOnce(&mut InsightMetadata)) {
        if let Some(meta) = self.metadata.get_mut(key) {
            f(meta);
        }
    }

    /// Rebuild the entire index from a sequence of events.
    ///
    /// Events MUST be provided in canonical block order (ascending
    /// block number, then transaction index within block). This
    /// guarantees deterministic reconstruction of the HNSW graph
    /// (if the index upgrades past 100K vectors).
    ///
    /// Call this on node startup after loading the event log from
    /// the chain database.
    pub fn rebuild_from_events(&mut self, events: impl Iterator<Item = InsightEvent>) {
        // Clear existing state.
        self.index = LocalIndex::new();
        self.metadata.clear();

        for event in events {
            match event {
                InsightEvent::Published { insight_id, vector, metadata } => {
                    self.store(insight_id, vector, metadata);
                }
                InsightEvent::Confirmed { insight_id, total_confirmations, .. } => {
                    self.update_metadata(&insight_id, |meta| {
                        meta.confirmations = total_confirmations;
                        // Tier auto-promotion mirrors the Solidity logic.
                        if total_confirmations >= 25 {
                            meta.tier = crate::InsightTier::Persistent;
                        } else if total_confirmations >= 10 {
                            meta.tier = crate::InsightTier::Consolidated;
                        } else if total_confirmations >= 3 {
                            meta.tier = crate::InsightTier::Working;
                        }
                    });
                }
                InsightEvent::Deleted { insight_id } => {
                    self.remove(&insight_id);
                }
            }
        }
    }

    /// Number of vectors currently in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.metadata.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty()
    }
}

impl Default for OnChainHdcIndex {
    fn default() -> Self {
        Self::new()
    }
}
```

---

## 5. Fixed-Point Decay (`src/decay.rs`)

All decay computations on-chain must use integer arithmetic. No `f64`.
The functions in this module approximate exponential decay using the
identity `exp(-x) ~ (1 - x/N)^N` for large N. We use N=256.

```rust
/// Approximate `balance * exp(-lambda * delta)` using fixed-point
/// integer arithmetic.
///
/// All inputs are in basis points (bps = 1/10,000). This avoids
/// floating-point entirely.
///
/// # Arguments
///
/// - `balance_bps`: Current balance in basis points (0..=10_000).
/// - `lambda_bps`: Decay rate in basis points per tick.
/// - `delta_ticks`: Number of ticks elapsed since last update.
///
/// # Algorithm
///
/// Uses the approximation `exp(-x) ~ (1 - x/256)^256` where
/// `x = lambda_bps * delta_ticks / 10_000`.
///
/// The inner step `(1 - x/256)` is computed as `(256 - step) / 256`
/// in integer arithmetic, applied 256 times via repeated squaring
/// (or loop).
///
/// # Overflow / underflow guards
///
/// - If `lambda_bps * delta_ticks` overflows u64, we use
///   `saturating_mul` and the result clamps to 0 (full decay).
/// - If `step >= 256` at any point, the factor is 0 and the
///   function returns 0 immediately.
///
/// # Consensus safety
///
/// CONSENSUS SAFE: Pure integer arithmetic. No floats. Identical
/// results on all platforms.
///
/// # Example
///
/// ```
/// // 10,000 bps balance, lambda = 50 bps/tick, 100 ticks elapsed.
/// let result = fixed_point_decay(10_000, 50, 100);
/// // exp(-0.005 * 100) = exp(-0.5) ~ 0.6065
/// // result ~ 6065 bps
/// ```
pub fn fixed_point_decay(balance_bps: u64, lambda_bps: u64, delta_ticks: u64) -> u64 {
    // x_bps = lambda_bps * delta_ticks (may overflow -> saturate)
    let x_bps = lambda_bps.saturating_mul(delta_ticks);

    // step = x_bps / 10_000 (scaled into the (1 - step/256)^256 domain)
    // We need step = x_bps * 256 / 10_000 to avoid losing precision.
    // But we must guard against overflow of x_bps * 256.
    let step = match x_bps.checked_mul(256) {
        Some(v) => v / 10_000,
        None => {
            // Overflow: x is enormous, decay to zero.
            return 0;
        }
    };

    // If step >= 256, the factor (256 - step) is <= 0, full decay.
    if step >= 256 {
        return 0;
    }

    // Compute (256 - step)^256 / 256^256, applied to balance_bps.
    //
    // We iterate 256 times, each time multiplying by (256 - step)
    // and dividing by 256 to keep the value in range.
    //
    // Invariant: `value` stays in [0, balance_bps] throughout.
    let factor_num = 256 - step; // guaranteed < 256 and >= 1
    let mut value = balance_bps;

    for _ in 0..256 {
        value = value * factor_num / 256;
        if value == 0 {
            return 0; // Early exit: fully decayed.
        }
    }

    value
}

/// Approximate `2^(-age / half_life)` in basis points.
///
/// Returns the decay factor in bps (0..=10_000). Used for pheromone
/// intensity decay where the formula is `intensity * 2^(-(current_block
/// - deposit_block) / half_life)`.
///
/// # Algorithm
///
/// Decompose into integer and fractional parts:
/// - `q = age / half_life` (integer division, number of full halvings)
/// - `r = age % half_life` (remainder)
/// - `2^(-age/half_life) = 2^(-q) * 2^(-r/half_life)`
/// - `2^(-q)` is a right-shift by q.
/// - `2^(-r/half_life)` is approximated by linear interpolation
///   between 1.0 and 0.5 using `(10_000 - 5_000 * r / half_life)`.
///
/// CONSENSUS SAFE: Pure integer arithmetic.
pub fn fixed_point_exp2_neg(age: u64, half_life: u64) -> u64 {
    if half_life == 0 {
        return 0;
    }

    let q = age / half_life; // Number of full halvings.

    // After 14 halvings, the value is < 1 bps. Return 0.
    if q >= 14 {
        return 0;
    }

    let r = age % half_life; // Remainder.

    // Fractional part: linear interpolation between 10_000 and 5_000.
    // 2^(-r/half_life) ~ 1 - (r/half_life) * 0.5 (linear approx).
    // In bps: 10_000 - 5_000 * r / half_life.
    let frac_bps = 10_000u64.saturating_sub(5_000 * r / half_life);

    // Integer part: divide by 2^q.
    frac_bps >> q
}
```

---

## 6. Tiered Search (`src/search.rs`)

Gas-optimized 3-tier search pipeline for on-chain use. This implements
the tiered search described in doc 03 / doc 09, adapted for the
precompile's gas metering.

```rust
use alloy_primitives::H256;
use kora_hdc::HdcVector;

use crate::OnChainHdcIndex;

/// Gas cost for a Tier 1 comparison (1 word XOR + POPCNT + compare).
pub const TIER1_GAS: u64 = 100;

/// Gas cost for a Tier 2 comparison (16 words sampled).
pub const TIER2_GAS: u64 = 500;

/// Gas cost for a Tier 3 comparison (full 160 words via precompile).
pub const TIER3_GAS: u64 = 1_500;

/// Tier 1 rejection threshold.
///
/// First-word Hamming distance above which a candidate is rejected.
/// At D=10,240, one word is 64 bits. Expected distance for a random
/// pair is 32. For similar vectors (full distance < 1,024), expected
/// first-word distance is ~6.4. A threshold of 20 is generous enough
/// to avoid false negatives while rejecting ~90% of random candidates.
///
/// CONSENSUS SAFETY: This is a protocol constant. All validators must
/// use the same value. Do not make it configurable per-call.
const TIER1_WORD_THRESHOLD: u32 = 20;

/// Tier 2 rejection threshold.
///
/// Approximate distance (from 16 sampled words, scaled to full vector)
/// above which a candidate is rejected. Sampling 16 of 160 words and
/// multiplying by 10 gives an estimate accurate to +/-5%.
/// Threshold of 1,200 is generous (true RESONANCE_THRESHOLD_HAMMING
/// is 1,024) to avoid false negatives.
const TIER2_APPROX_THRESHOLD: u32 = 1_200;

/// Sample indices for Tier 2: 16 evenly-spaced words from [0..160).
/// Indices: 0, 10, 20, 30, ..., 150.
const TIER2_SAMPLE_INDICES: [usize; 16] = [
    0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150,
];

/// Result of a tiered search.
#[derive(Clone, Debug)]
pub struct TieredSearchResult {
    /// Top-K results: (insight_id, exact_hamming_distance).
    /// Sorted by distance ascending, ties broken by key.
    pub results: Vec<(H256, u32)>,
    /// Total gas consumed by the search.
    pub gas_used: u64,
}

/// Gas-optimized tiered search over the on-chain HDC index.
///
/// The 3-tier pipeline reduces gas consumption by ~25x compared to
/// brute-force full-vector comparison.
///
/// ## Tier 1: First-word filter (~100 gas/candidate)
///
/// Compares only the first `u64` word of the query with each
/// candidate. Rejects candidates whose single-word Hamming distance
/// exceeds `TIER1_WORD_THRESHOLD`. Eliminates ~90% of candidates.
///
/// ## Tier 2: Sample-word comparison (~500 gas/candidate)
///
/// For Tier 1 survivors, compares 16 evenly-spaced words (10% of
/// the vector). The sampled distance is scaled by 10 to estimate
/// the full-vector distance. Eliminates ~90% of remaining candidates.
///
/// ## Tier 3: Full Hamming distance (~1,500 gas/candidate)
///
/// For Tier 2 survivors, computes the exact 160-word Hamming
/// distance via the precompile at `0x09`. Produces the final ranking.
pub struct TieredSearch;

impl TieredSearch {
    /// Execute a tiered search over the on-chain index.
    ///
    /// # Arguments
    ///
    /// - `index`: The on-chain HDC index to search.
    /// - `query`: The query vector (10,240 bits).
    /// - `top_k`: Number of results to return.
    /// - `gas_limit`: Maximum gas budget for this search. The search
    ///   stops early if the budget is exhausted.
    ///
    /// # Returns
    ///
    /// A [`TieredSearchResult`] containing the top-K results and the
    /// total gas consumed.
    ///
    /// # Consensus safety
    ///
    /// CONSENSUS SAFE: All operations are integer arithmetic. The
    /// result ordering uses a composite `(distance, key)` sort key
    /// for deterministic tiebreaking.
    pub fn search(
        index: &OnChainHdcIndex,
        query: &HdcVector,
        top_k: usize,
        gas_limit: u64,
    ) -> TieredSearchResult {
        let mut gas_used = 0u64;

        // ---- Tier 1: first-word filter ----
        let query_word0 = query.0[0];
        let mut tier1_survivors: Vec<(H256, &HdcVector)> = Vec::new();

        for (key, vector) in index.iter_vectors() {
            gas_used += TIER1_GAS;
            if gas_used > gas_limit {
                break;
            }

            let word_dist = (query_word0 ^ vector.0[0]).count_ones();
            if word_dist <= TIER1_WORD_THRESHOLD {
                tier1_survivors.push((key, vector));
            }
        }

        // ---- Tier 2: sample-word comparison ----
        let mut tier2_survivors: Vec<(H256, &HdcVector, u32)> = Vec::new();

        for (key, vector) in &tier1_survivors {
            gas_used += TIER2_GAS;
            if gas_used > gas_limit {
                break;
            }

            let sample_dist = Self::approximate_hamming(&query.0, &vector.0);
            if sample_dist <= TIER2_APPROX_THRESHOLD {
                tier2_survivors.push((*key, vector, sample_dist));
            }
        }

        // ---- Tier 3: full Hamming distance ----
        let mut results: Vec<(H256, u32)> = Vec::new();

        for (key, vector, _approx) in &tier2_survivors {
            gas_used += TIER3_GAS;
            if gas_used > gas_limit {
                break;
            }

            let exact_dist = query.hamming_distance(vector);
            results.push((*key, exact_dist));
        }

        // Deterministic sort: distance ascending, key ascending for ties.
        results.sort_by(|(k1, d1), (k2, d2)| d1.cmp(d2).then_with(|| k1.cmp(k2)));
        results.truncate(top_k);

        TieredSearchResult { results, gas_used }
    }

    /// Approximate Hamming distance by sampling 16 evenly-spaced words.
    ///
    /// Returns the estimated full-vector distance (sampled distance * 10).
    ///
    /// CONSENSUS SAFE: Pure integer arithmetic.
    fn approximate_hamming(a: &[u64; 160], b: &[u64; 160]) -> u32 {
        let mut dist = 0u32;
        for &idx in &TIER2_SAMPLE_INDICES {
            dist += (a[idx] ^ b[idx]).count_ones();
        }
        // Scale: 16 words sampled out of 160 total = 10x.
        dist * 10
    }
}
```

---

## 7. Event Sync (`src/sync.rs`)

```rust
use alloy_primitives::H256;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::{ChainHdcError, InsightEvent, OnChainHdcIndex};

/// Synchronizes the in-memory HDC index with on-chain events.
///
/// Subscribes to `InsightPublished`, `InsightConfirmed`, and
/// `InsightPurged` events from the InsightBoard contract, plus
/// `PheromoneDeposited` events from the PheromoneRegistry.
///
/// ## Startup
///
/// On node startup, call [`NeuroChainSync::replay_from`] to replay
/// all historical events and rebuild the index deterministically.
///
/// ## Runtime
///
/// During normal operation, call [`NeuroChainSync::subscribe`] to
/// receive new events as blocks are finalized. Each event is applied
/// to the `OnChainHdcIndex` in block order.
///
/// ## Off-chain agents
///
/// Off-chain agents (not validators) use this module to stay in sync
/// with the on-chain state by subscribing to events via RPC.
pub struct NeuroChainSync {
    /// The HDC index to keep synchronized.
    index: parking_lot::RwLock<OnChainHdcIndex>,

    /// Block number up to which the index has been synced.
    synced_up_to: std::sync::atomic::AtomicU64,
}

impl NeuroChainSync {
    /// Create a new sync manager with an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self {
            index: parking_lot::RwLock::new(OnChainHdcIndex::new()),
            synced_up_to: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Replay historical events to rebuild the index.
    ///
    /// Events MUST be in canonical order (block number ascending,
    /// then transaction index ascending within each block).
    ///
    /// This is called once at startup before subscribing to new events.
    pub fn replay_from(
        &self,
        events: impl Iterator<Item = InsightEvent>,
        up_to_block: u64,
    ) {
        let mut index = self.index.write();
        index.rebuild_from_events(events);
        self.synced_up_to.store(up_to_block, std::sync::atomic::Ordering::Release);
        info!(
            block = up_to_block,
            vectors = index.len(),
            "HDC index rebuilt from historical events"
        );
    }

    /// Apply a single new event to the index.
    ///
    /// Called for each event received from the subscription channel
    /// during normal operation.
    pub fn apply_event(&self, event: InsightEvent, block_number: u64) {
        let mut index = self.index.write();
        match &event {
            InsightEvent::Published { insight_id, vector, metadata } => {
                index.store(*insight_id, vector.clone(), metadata.clone());
            }
            InsightEvent::Confirmed { insight_id, total_confirmations, .. } => {
                index.update_metadata(insight_id, |meta| {
                    meta.confirmations = *total_confirmations;
                    if *total_confirmations >= 25 {
                        meta.tier = crate::InsightTier::Persistent;
                    } else if *total_confirmations >= 10 {
                        meta.tier = crate::InsightTier::Consolidated;
                    } else if *total_confirmations >= 3 {
                        meta.tier = crate::InsightTier::Working;
                    }
                });
            }
            InsightEvent::Deleted { insight_id } => {
                index.remove(insight_id);
            }
        }
        self.synced_up_to.store(block_number, std::sync::atomic::Ordering::Release);
    }

    /// Get a read-only reference to the index for search operations.
    pub fn index(&self) -> parking_lot::RwLockReadGuard<'_, OnChainHdcIndex> {
        self.index.read()
    }

    /// The latest block number to which the index has been synced.
    pub fn synced_block(&self) -> u64 {
        self.synced_up_to.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl Default for NeuroChainSync {
    fn default() -> Self {
        Self::new()
    }
}
```

---

## 8. WisdomGate (`src/wisdom_gate.rs`)

The WisdomGate is a filter applied when an agent consumes shared
knowledge from the on-chain substrate. It prevents low-quality,
tainted, duplicate, or homogeneous knowledge from entering the
agent's local cognition.

```rust
use std::collections::HashMap;

use alloy_primitives::{Address, H256};
use kora_hdc::HdcVector;

use crate::InsightMetadata;

/// Configuration for the WisdomGate filter.
#[derive(Clone, Debug)]
pub struct WisdomGateConfig {
    /// Minimum trust score (0..=10_000 bps) required to accept knowledge.
    /// Default: 500 bps (0.05 = MIN_TRUST_THRESHOLD from doc 09).
    pub min_trust_bps: u64,

    /// Maximum Hamming distance to the query for relevance.
    /// Default: 1,024 (RESONANCE_THRESHOLD_HAMMING from doc 09).
    pub resonance_threshold: u32,

    /// Hamming distance below which two vectors are considered duplicates.
    /// Default: 512 (DUPLICATE_THRESHOLD from doc 09).
    pub duplicate_threshold: u32,

    /// Maximum number of results from the same author.
    /// Default: 3.
    pub max_per_author: usize,
}

impl Default for WisdomGateConfig {
    fn default() -> Self {
        Self {
            min_trust_bps: 500,
            resonance_threshold: 1_024,
            duplicate_threshold: 512,
            max_per_author: 3,
        }
    }
}

/// Reason a candidate was rejected by the WisdomGate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WisdomRejection {
    /// Trust score below `min_trust_bps`.
    InsufficientTrust { trust_bps: u64, required_bps: u64 },
    /// The insight is in a tainted state (Challenged or Decaying with
    /// low confirmations).
    Tainted { state: crate::InsightState },
    /// Hamming distance exceeds `resonance_threshold` (not relevant).
    NotRelevant { distance: u32, threshold: u32 },
    /// Too similar to an already-accepted result (duplicate).
    Duplicate { duplicate_of: H256, distance: u32 },
    /// Author diversity limit exceeded.
    AuthorSaturation { author: Address, count: usize, max: usize },
}

/// Filter for consuming shared knowledge from the on-chain substrate.
///
/// Checks applied (in order):
///
/// 1. **Trust**: author's trust score > `min_trust_bps`.
/// 2. **Taint**: insight state is not Challenged or Purged.
/// 3. **Relevance**: Hamming distance to query <= `resonance_threshold`.
/// 4. **Duplicate**: not too similar to an already-accepted result.
/// 5. **Diversity**: max `max_per_author` results from the same author.
///
/// ## Known limitation
///
/// **The WisdomGate does NOT inspect content for prompt injection.**
/// A malicious agent could publish an insight whose vector is benign
/// (passes similarity checks) but whose content payload contains
/// prompt-injection attacks targeting the consuming agent's LLM.
/// Content-level safety filtering is the responsibility of the
/// consuming agent's cognitive pipeline, NOT the WisdomGate.
///
/// This is a known gap documented here so that future work can
/// address it (e.g., content hash verification, content scanning
/// in the cognitive pipeline, or a separate ContentGate module).
pub struct WisdomGate {
    config: WisdomGateConfig,
}

impl WisdomGate {
    /// Create a new WisdomGate with the given configuration.
    #[must_use]
    pub fn new(config: WisdomGateConfig) -> Self {
        Self { config }
    }

    /// Create a WisdomGate with default configuration.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(WisdomGateConfig::default())
    }

    /// Filter a batch of search results through the WisdomGate.
    ///
    /// # Arguments
    ///
    /// - `candidates`: Search results as `(insight_id, vector, metadata,
    ///   hamming_distance, trust_score_bps)` tuples.
    /// - `already_accepted`: Vectors that have already been accepted
    ///   (for duplicate detection across multiple calls).
    ///
    /// # Returns
    ///
    /// A tuple of `(accepted, rejected)`. Accepted entries are in
    /// the same order as the input. Rejected entries include the
    /// rejection reason.
    ///
    /// CONSENSUS SAFETY: This function is for LOCAL (off-chain) use
    /// only. It is called by agents consuming shared knowledge, not
    /// by validators during block execution. The trust scores are
    /// provided by the caller and may differ between agents.
    pub fn filter(
        &self,
        candidates: &[(H256, &HdcVector, &InsightMetadata, u32, u64)],
        already_accepted: &[(H256, &HdcVector)],
    ) -> (Vec<H256>, Vec<(H256, WisdomRejection)>) {
        let mut accepted: Vec<(H256, &HdcVector)> = already_accepted.to_vec();
        let mut accepted_ids: Vec<H256> = Vec::new();
        let mut rejected: Vec<(H256, WisdomRejection)> = Vec::new();
        let mut author_counts: HashMap<Address, usize> = HashMap::new();

        // Pre-populate author counts from already-accepted entries.
        for (_, _, meta, _, _) in candidates.iter() {
            // Only count if already accepted.
        }

        for &(id, vector, metadata, distance, trust_bps) in candidates {
            // 1. Trust check.
            if trust_bps < self.config.min_trust_bps {
                rejected.push((id, WisdomRejection::InsufficientTrust {
                    trust_bps,
                    required_bps: self.config.min_trust_bps,
                }));
                continue;
            }

            // 2. Taint check.
            if metadata.state == crate::InsightState::Challenged
                || metadata.state == crate::InsightState::Purged
            {
                rejected.push((id, WisdomRejection::Tainted { state: metadata.state }));
                continue;
            }

            // 3. Relevance check.
            if distance > self.config.resonance_threshold {
                rejected.push((id, WisdomRejection::NotRelevant {
                    distance,
                    threshold: self.config.resonance_threshold,
                }));
                continue;
            }

            // 4. Duplicate check against already-accepted vectors.
            let mut is_dup = false;
            for (accepted_id, accepted_vec) in &accepted {
                let dup_dist = vector.hamming_distance(accepted_vec);
                if dup_dist < self.config.duplicate_threshold {
                    rejected.push((id, WisdomRejection::Duplicate {
                        duplicate_of: *accepted_id,
                        distance: dup_dist,
                    }));
                    is_dup = true;
                    break;
                }
            }
            if is_dup {
                continue;
            }

            // 5. Author diversity check.
            let author_count = author_counts.entry(metadata.author).or_insert(0);
            if *author_count >= self.config.max_per_author {
                rejected.push((id, WisdomRejection::AuthorSaturation {
                    author: metadata.author,
                    count: *author_count,
                    max: self.config.max_per_author,
                }));
                continue;
            }

            // All checks passed.
            *author_count += 1;
            accepted.push((id, vector));
            accepted_ids.push(id);
        }

        (accepted_ids, rejected)
    }
}
```

---

## 9. Error Types (`src/error.rs`)

```rust
use thiserror::Error;

/// Errors from the `kora-hdc-chain` crate.
#[derive(Debug, Error)]
pub enum ChainHdcError {
    /// The event log is missing events (gap in block numbers).
    #[error("event log gap: expected block {expected}, got {got}")]
    EventLogGap { expected: u64, got: u64 },

    /// A vector in the event log has an invalid size.
    #[error("invalid vector size: expected 1280 bytes, got {0}")]
    InvalidVectorSize(usize),

    /// The insight ID was not found in the index.
    #[error("insight not found: {0}")]
    InsightNotFound(alloy_primitives::H256),

    /// The search exceeded its gas budget.
    #[error("search gas budget exceeded: used {used}, limit {limit}")]
    GasBudgetExceeded { used: u64, limit: u64 },

    /// Event subscription channel closed.
    #[error("event subscription closed")]
    SubscriptionClosed,
}
```

---

## 10. On-Chain / Off-Chain Boundary Table

This table defines where each operation runs. Getting this wrong
causes consensus violations (if off-chain-only logic leaks into the
consensus path) or performance issues (if on-chain-only logic is
redundantly replicated off-chain).

| Operation | On-Chain (Consensus) | Off-Chain (Agent) | Notes |
|---|---|---|---|
| `HdcVector::hamming_distance()` | YES (precompile 0x09) | YES (local search) | Integer arithmetic, identical results everywhere |
| `HdcVector::similarity()` | **NO** (uses f64) | YES | Convenience function for local ranking only |
| `OnChainHdcIndex::store()` | YES (via precompile) | NO | Modifies consensus state |
| `OnChainHdcIndex::search()` | YES (via eth_call) | YES (local mirror) | Read-only, no state change |
| `OnChainHdcIndex::rebuild_from_events()` | YES (node startup) | YES (agent sync) | Deterministic replay |
| `TieredSearch::search()` | YES (precompile) | Optional | Gas metering only matters on-chain |
| `fixed_point_decay()` | YES (contract logic) | YES (mirrors contract) | Must match Solidity behavior exactly |
| `fixed_point_exp2_neg()` | YES (pheromone decay) | YES (mirrors contract) | Must match Solidity behavior exactly |
| `WisdomGate::filter()` | **NO** | YES | Agent-local trust decisions |
| `NeuroChainSync::apply_event()` | YES (validator) | YES (agent) | Same event, same index update |
| `BundleAccumulator` | NO | YES | Encoding is off-chain; raw vectors are published |
| `TrigramEncoder` / `ProjectionEncoder` | NO | YES | All encoding is off-chain |

**Rule of thumb**: If an operation uses `f64`, it is off-chain only.
If it modifies state, it is on-chain only. If it is pure integer
read-only, it can run anywhere.

---

## 11. Integration Points

### `kora-executor` (precompile registration)

The executor crate needs to register the HDC precompile at address `0x09`.
In `crates/node/executor/Cargo.toml`, add:

```toml
kora-hdc-chain = { workspace = true }
```

In the REVM executor setup (likely `crates/node/executor/src/revm.rs`),
register the precompile:

```rust
use kora_hdc_chain::OnChainHdcIndex;

// During EVM context construction, register the HDC precompile
// at address 0x09. The precompile reads from and writes to the
// shared OnChainHdcIndex via Arc<RwLock<OnChainHdcIndex>>.
//
// The precompile exposes three operations:
//   - storeVector(bytes32 id, bytes vector) -> writes to index
//   - deleteVector(bytes32 id) -> removes from index
//   - searchSimilar(bytes query, uint8 topK) -> reads from index
//
// See doc 07 (07-precompile-integration.md) for the full
// precompile implementation.
```

### `kora-rpc` (query API)

The RPC crate needs to expose HDC query methods. In
`crates/node/rpc/Cargo.toml`, add:

```toml
kora-hdc-chain = { workspace = true }
```

New RPC methods (see doc 12 for full specification):

```
hdc_searchSimilar(query: Bytes, topK: u8) -> Vec<SearchResult>
hdc_getInsight(id: H256) -> InsightMetadata
hdc_getPheromone(target: H256) -> Vec<PheromoneInfo>
hdc_getIndexStats() -> IndexStats
```

---

## 12. Anti-Patterns

These are mistakes that will cause consensus violations, performance
issues, or subtle bugs. Do not do these.

### 1. Do not use `f64` in the consensus path

Every function called during block execution (precompile calls,
state transitions, decay computation) must use integer arithmetic
only. `f64` operations are not deterministic across platforms (see
IEEE 754 non-associativity of addition, platform-specific rounding
modes, compiler reordering of FP operations).

**Wrong:**
```rust
fn decay(balance: f64, lambda: f64, dt: f64) -> f64 {
    balance * (-lambda * dt).exp()  // CONSENSUS VIOLATION
}
```

**Right:**
```rust
fn decay(balance_bps: u64, lambda_bps: u64, dt: u64) -> u64 {
    fixed_point_decay(balance_bps, lambda_bps, dt)  // Integer only
}
```

### 2. Do not use `HashMap` for iteration

Rust's `HashMap` has non-deterministic iteration order (it uses a
randomly-seeded hasher). If any consensus-path code iterates a
`HashMap`, different validators will process entries in different
orders, potentially producing different results.

**Safe uses of `HashMap`:** Key lookup (`get`, `insert`, `remove`,
`contains_key`). These do not depend on iteration order.

**Unsafe uses:** `iter()`, `values()`, `keys()`, `for (k, v) in map`,
`retain()`, serialization to ordered formats. For any of these, use
`BTreeMap` instead.

The `OnChainHdcIndex::metadata` field uses `HashMap` because it is
only accessed by key lookup. If you add any code that iterates it,
you must change it to `BTreeMap`.

### 3. Do not forget composite tiebreakers in sort

When sorting search results by Hamming distance, equal distances
are common (especially for distant vectors). If you sort by distance
alone, the output order is non-deterministic (Rust's `sort_by` is
stable, but the input order may vary across validators if the index
was iterated in a non-deterministic order).

**Wrong:**
```rust
results.sort_by_key(|(_, dist)| *dist);
```

**Right:**
```rust
results.sort_by(|(k1, d1), (k2, d2)| d1.cmp(d2).then_with(|| k1.cmp(k2)));
```

### 4. Do not skip the WisdomGate prompt injection documentation

The WisdomGate does NOT inspect content payloads. A malicious author
can craft a vector that passes all HDC similarity checks while
embedding prompt-injection attacks in the content. This is a known
gap. Document it prominently. Do not pretend it does not exist.

### 5. Do not store full vectors in contract storage slots

Full vectors are 1,280 bytes = 40 storage slots = ~880K gas to write.
Instead, store the vectorHash (32 bytes, 1 slot) on-chain and emit
the full vector in the event log. The precompile index is rebuilt
from events, not from storage reads.

### 6. Do not make gas constants configurable per-call

The `TIER1_GAS`, `TIER2_GAS`, `TIER3_GAS` constants and the tier
thresholds are protocol constants. If different callers use different
values, the gas accounting diverges and validators cannot agree on
execution results. These must be hardcoded or set at genesis.

---

## 13. Required Stubs from `kora-hdc`

If `kora-hdc` (the core algebra crate) is not yet implemented when
you start this crate, create the following minimal stubs so that
`kora-hdc-chain` compiles. These will be replaced with the real
implementations from `02-kora-hdc-core.md`.

```rust
// In crates/hdc/core/src/lib.rs (stub)

/// 10,240-bit binary hypervector.
#[repr(align(64))]
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HdcVector(pub [u64; 160]);

impl HdcVector {
    /// Hamming distance (stub: correct implementation).
    pub fn hamming_distance(&self, other: &Self) -> u32 {
        let mut dist = 0u32;
        for i in 0..160 {
            dist += (self.0[i] ^ other.0[i]).count_ones();
        }
        dist
    }
}

/// Local index (stub).
pub enum LocalIndex {
    BruteForce(Vec<(alloy_primitives::H256, HdcVector)>),
}

impl LocalIndex {
    pub fn new() -> Self {
        Self::BruteForce(Vec::new())
    }

    pub fn insert(&mut self, key: alloy_primitives::H256, vector: HdcVector) {
        match self {
            Self::BruteForce(v) => v.push((key, vector)),
        }
    }

    pub fn remove(&mut self, key: alloy_primitives::H256) {
        match self {
            Self::BruteForce(v) => v.retain(|(k, _)| *k != key),
        }
    }

    pub fn search(&self, query: &HdcVector, top_k: usize) -> Vec<(alloy_primitives::H256, u32)> {
        match self {
            Self::BruteForce(v) => {
                let mut results: Vec<_> = v.iter()
                    .map(|(k, vec)| (*k, query.hamming_distance(vec)))
                    .collect();
                results.sort_by(|(k1, d1), (k2, d2)| d1.cmp(d2).then_with(|| k1.cmp(k2)));
                results.truncate(top_k);
                results
            }
        }
    }
}
```

---

## 14. Checklist

### Setup

- [ ] Create `crates/hdc/` directory
- [ ] Create `crates/hdc/core/Cargo.toml` (stub or full, per doc 02)
- [ ] Create `crates/hdc/chain/Cargo.toml` (as specified in section 1)
- [ ] Add `"crates/hdc/*"` to workspace members in root `Cargo.toml`
- [ ] Add `kora-hdc` and `kora-hdc-chain` to `[workspace.dependencies]`

### Core Implementation

- [ ] `src/types.rs` -- all enums and structs (section 3)
- [ ] `src/error.rs` -- error types (section 9)
- [ ] `src/index.rs` -- `OnChainHdcIndex` with `store`, `remove`, `search`, `rebuild_from_events` (section 4)
- [ ] `src/decay.rs` -- `fixed_point_decay` and `fixed_point_exp2_neg` (section 5)
- [ ] `src/search.rs` -- `TieredSearch` with 3-tier pipeline (section 6)
- [ ] `src/sync.rs` -- `NeuroChainSync` with replay and apply (section 7)
- [ ] `src/wisdom_gate.rs` -- `WisdomGate` with all 5 checks (section 8)
- [ ] `src/lib.rs` -- re-exports (section 2)

### Integration

- [ ] Add `kora-hdc-chain` dependency to `kora-executor/Cargo.toml`
- [ ] Add `kora-hdc-chain` dependency to `kora-rpc/Cargo.toml`
- [ ] Verify: `cargo build -p kora-hdc-chain`
- [ ] Verify: `cargo clippy -p kora-hdc-chain`
- [ ] Verify: `cargo doc -p kora-hdc-chain --no-deps`

### Quality

- [ ] All public items have doc comments
- [ ] All consensus-path functions have `CONSENSUS SAFE` annotation
- [ ] No `f64` in any consensus-path function
- [ ] No `HashMap` iteration in any consensus-path function
- [ ] All sorts use composite `(distance, key)` tiebreakers
- [ ] WisdomGate prompt-injection gap is documented in doc comment

---

## 15. Test Plan

### Unit Tests (`src/decay.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_zero_delta_returns_balance() {
        assert_eq!(fixed_point_decay(10_000, 50, 0), 10_000);
    }

    #[test]
    fn decay_large_delta_returns_zero() {
        assert_eq!(fixed_point_decay(10_000, 50, 1_000_000), 0);
    }

    #[test]
    fn decay_moderate_delta_approximates_exp() {
        // exp(-0.5) ~ 0.6065 -> 6065 bps
        let result = fixed_point_decay(10_000, 50, 100);
        // Allow 5% tolerance for the integer approximation.
        assert!(result > 5700 && result < 6400,
            "expected ~6065, got {result}");
    }

    #[test]
    fn decay_overflow_saturates_to_zero() {
        // lambda * delta overflows u64.
        assert_eq!(fixed_point_decay(10_000, u64::MAX, u64::MAX), 0);
    }

    #[test]
    fn exp2_neg_zero_age_returns_10000() {
        assert_eq!(fixed_point_exp2_neg(0, 100), 10_000);
    }

    #[test]
    fn exp2_neg_one_half_life_returns_5000() {
        // 2^(-1) = 0.5 -> 5000 bps
        assert_eq!(fixed_point_exp2_neg(100, 100), 5_000);
    }

    #[test]
    fn exp2_neg_two_half_lives_returns_2500() {
        // 2^(-2) = 0.25 -> 2500 bps
        assert_eq!(fixed_point_exp2_neg(200, 100), 2_500);
    }

    #[test]
    fn exp2_neg_large_age_returns_zero() {
        assert_eq!(fixed_point_exp2_neg(2000, 100), 0);
    }

    #[test]
    fn exp2_neg_zero_half_life_returns_zero() {
        assert_eq!(fixed_point_exp2_neg(100, 0), 0);
    }
}
```

### Unit Tests (`src/index.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kora_hdc::HdcVector;

    fn random_vector(seed: u64) -> HdcVector {
        // Deterministic random vector from seed.
        let mut v = [0u64; 160];
        let mut s = seed;
        for word in &mut v {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            *word = s;
        }
        HdcVector(v)
    }

    #[test]
    fn store_and_search_returns_exact_match() {
        let mut index = OnChainHdcIndex::new();
        let v = random_vector(42);
        let key = H256::from_low_u64_be(1);
        let meta = /* construct InsightMetadata */;
        index.store(key, v.clone(), meta);

        let results = index.search(&v, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, key);
        assert_eq!(results[0].1, 0); // Exact match, distance 0.
    }

    #[test]
    fn rebuild_from_events_is_deterministic() {
        // Build index twice from the same events in the same order.
        // Assert the search results are identical.
        // This validates the determinism guarantee.
    }

    #[test]
    fn remove_deletes_vector_and_metadata() {
        let mut index = OnChainHdcIndex::new();
        let key = H256::from_low_u64_be(1);
        index.store(key, random_vector(1), /* meta */);
        assert_eq!(index.len(), 1);

        index.remove(&key);
        assert_eq!(index.len(), 0);
        assert!(index.get_metadata(&key).is_none());
    }

    #[test]
    fn search_results_sorted_by_distance_then_key() {
        // Insert multiple vectors at known distances.
        // Verify sort order is (distance_asc, key_asc).
    }
}
```

### Unit Tests (`src/search.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiered_search_gas_accounting() {
        // Insert 100 vectors. Run tiered search.
        // Verify gas_used = (100 * TIER1_GAS) + (survivors * TIER2_GAS)
        //                  + (final * TIER3_GAS).
    }

    #[test]
    fn tiered_search_respects_gas_limit() {
        // Insert 1000 vectors. Set gas_limit to 500.
        // Verify search stops early and returns partial results.
    }

    #[test]
    fn approximate_hamming_scales_correctly() {
        // Two identical vectors: approximate distance should be 0.
        let a = [0u64; 160];
        let b = [0u64; 160];
        assert_eq!(TieredSearch::approximate_hamming(&a, &b), 0);

        // Two complementary vectors: approximate distance should be ~10,240.
        let c = [u64::MAX; 160];
        assert_eq!(TieredSearch::approximate_hamming(&a, &c), 10_240);
    }
}
```

### Unit Tests (`src/wisdom_gate.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_low_trust() {
        let gate = WisdomGate::with_defaults();
        // Candidate with trust_bps = 100 (below 500 threshold).
        // Assert: rejected with InsufficientTrust.
    }

    #[test]
    fn rejects_challenged_insight() {
        // Candidate with state = Challenged.
        // Assert: rejected with Tainted.
    }

    #[test]
    fn rejects_irrelevant_by_distance() {
        // Candidate with distance = 2000 (above 1024 threshold).
        // Assert: rejected with NotRelevant.
    }

    #[test]
    fn rejects_duplicate() {
        // Two candidates with Hamming distance < 512 between them.
        // Second should be rejected with Duplicate.
    }

    #[test]
    fn rejects_author_saturation() {
        // 4 candidates from the same author. Max is 3.
        // 4th should be rejected with AuthorSaturation.
    }

    #[test]
    fn accepts_valid_candidate() {
        // Candidate passing all 5 checks.
        // Assert: accepted.
    }
}
```

### Integration Tests

```bash
# Compile the crate and all dependents.
cargo build -p kora-hdc-chain

# Run unit tests.
cargo test -p kora-hdc-chain

# Run clippy with workspace lints.
cargo clippy -p kora-hdc-chain

# Generate docs and verify no broken links.
cargo doc -p kora-hdc-chain --no-deps
```

---

## 16. Dependencies on Other Documents

| This doc needs | From doc | What |
|---|---|---|
| `HdcVector`, `LocalIndex`, `BundleAccumulator` | 02-kora-hdc-core.md | Core algebra types |
| Tiered search gas model | 03-vector-search.md (design) / 06-vector-search.md (design) | Tier thresholds and gas costs |
| Knowledge kinds, decay half-lives, tier multipliers | 04-knowledge-store.md | `InsightKind`, `InsightTier` values |
| Precompile registration at 0x09 | 07-precompile-integration.md | REVM precompile wiring |
| InsightBoard contract ABI | 08-insight-board-contract.md | Event signatures for sync |
| PheromoneRegistry contract ABI | 09-pheromone-registry.md | Pheromone event signatures |
| Trust scoring function | 11-trust-pipeline.md | Trust scores consumed by WisdomGate |
| RPC method signatures | 12-rpc-extensions.md | JSON-RPC API surface |

---

## Audit Findings

Audit performed 2026-05-08 against the implementation in `crates/hdc/chain/`.

### F01 -- Missing Modules (5 of 8 spec modules absent)

The spec defines 8 source modules. The implementation has 6 files but only covers
3 of the spec's modules, adds 2 that the spec does not define (precompile, rpc),
and is missing 5 spec modules entirely.

| Spec Module | Spec File | Impl File | Status |
|---|---|---|---|
| `lib.rs` | `src/lib.rs` | `src/lib.rs` | PARTIAL -- re-exports present but missing most types |
| `types.rs` | `src/types.rs` | -- | MISSING entirely |
| `error.rs` | `src/error.rs` | -- | MISSING entirely |
| `index.rs` | `src/index.rs` | `src/index.rs` | PARTIAL -- heavily simplified |
| `decay.rs` | `src/decay.rs` | -- | MISSING entirely |
| `search.rs` | `src/search.rs` | -- | MISSING entirely |
| `sync.rs` | `src/sync.rs` | -- | MISSING entirely |
| `wisdom_gate.rs` | `src/wisdom_gate.rs` | `src/wisdom.rs` | PARTIAL -- wrong file name, wrong design |
| -- (not in spec) | -- | `src/precompile.rs` | EXTRA -- belongs in spec but was not specified as a chain module |
| -- (not in spec) | -- | `src/event.rs` | EXTRA -- stub placeholder for sync.rs |
| -- (not in spec) | -- | `src/rpc.rs` | EXTRA -- belongs in kora-rpc, not in kora-hdc-chain |

### F02 -- `precompile.rs` is not spec'd in this crate but is well-implemented

The spec says the precompile registration happens in `kora-executor`, with the
chain crate providing index/search/types. The implementation places the full
precompile dispatch logic in `crates/hdc/chain/src/precompile.rs`. This is
arguably a reasonable design choice (the chain crate provides the precompile
function, the executor just registers it), but it deviates from the spec which
says the precompile implementation lives in doc 07. The implementation quality
is good -- opcodes, gas accounting, input parsing, and tests are all present.

- File: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/precompile.rs`
- 6 opcodes (0x01-0x06): HammingDistance, Bind, Bundle, Permute, VectorId, IsSimilar
- Gas constants are hardcoded (correct per spec anti-pattern #6)
- 8 unit tests covering happy path, error cases, and gas exhaustion

### F03 -- `index.rs` is a simplified substitute, not the spec design

**Spec design**: `OnChainHdcIndex` wraps `kora_hdc::LocalIndex` (which delegates
to brute-force or HNSW). It has `store()`, `remove()`, `search()`,
`get_metadata()`, `update_metadata()`, and `rebuild_from_events()`. The metadata
type is `InsightMetadata` (from `types.rs`) with fields: author, publish_block,
kind, tier, state, confirmations, last_confirmed_block, content_hash.

**Actual implementation** (`/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`):

1. Does NOT use `kora_hdc::LocalIndex`. Uses a raw `HashMap<B256, HdcVector>` instead (line 14). This means no HNSW auto-switching at 100K vectors.

2. `InsightMeta` (line 21) has only 3 fields (publisher, block_number, state) vs the spec's 8 fields. Missing: kind, tier, confirmations, last_confirmed_block, content_hash.

3. `InsightState` (line 32) has 7 variants (Draft, Submitted, Challenged, Voting, Accepted, Rejected, Expired) which do NOT match the spec's 7 variants (Submitted, Verified, Active, Challenged, Decaying, Archived, Purged). The enum values are entirely different -- "Draft", "Voting", "Expired" do not exist in the spec; "Verified", "Active", "Decaying", "Archived", "Purged" are missing from the implementation.

4. `insert_insight()` (line 67) always sets state to `InsightState::Submitted`. The spec's `store()` takes metadata as a parameter and does not hardcode state.

5. `search()` (line 98) filters to `InsightState::Accepted` only. The spec's search has no state filter -- the `WisdomGate` handles filtering downstream.

6. `search()` sorts by distance only (`sort_by_key(|r| r.distance)`, line 117), without composite tiebreaker on key. This is the exact anti-pattern #3 the spec warns about. The sort is non-deterministic when distances are equal.

7. `rebuild_from_events()` is completely missing. The spec considers this critical for deterministic index reconstruction on node startup.

8. `remove()` is missing. The spec requires this for processing `InsightPurged` events.

9. `update_metadata()` with closure is missing. Only `update_state()` exists, which is a narrow subset.

10. `record_pheromone()` (line 130) is a no-op stub with `// TODO`.

### F04 -- `event.rs` is a hollow stub

File: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`

The spec calls for `src/sync.rs` containing `NeuroChainSync` -- a struct with
`RwLock<OnChainHdcIndex>`, `AtomicU64` for synced block tracking, `replay_from()`,
`apply_event()`, `index()` read accessor, and `synced_block()`.

The actual `event.rs` provides:

1. Five topic constants, ALL set to `B256::ZERO` with `// TODO: compute actual hash` (lines 13-26). These are non-functional.

2. A `process_log()` function (line 29) that takes raw log data but does nothing -- every branch is just `info!(...)` with commented-out logic.

3. No `NeuroChainSync` struct. No `RwLock` wrapping. No `AtomicU64` block tracking. No `replay_from()`. No `apply_event()`. No `synced_block()`.

4. The function signature takes `&mut OnChainHdcIndex` directly instead of going through `NeuroChainSync`, which means there is no concurrency safety.

5. No `InsightEvent` enum. The spec defines `Published`, `Confirmed`, `Deleted` variants; the implementation tries to match on raw topic hashes instead.

### F05 -- `wisdom.rs` is a different design than spec's `wisdom_gate.rs`

File: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom.rs`

**Spec design**: `WisdomGate` is a stateless filter that takes a batch of
search results (with trust scores, metadata, distances) and applies 5 checks:
Trust, Taint, Relevance, Duplicate, Author Diversity. It returns
`(accepted_ids, rejected_with_reasons)`. Configuration via `WisdomGateConfig`
with 4 tunable parameters.

**Actual implementation**: `WisdomGate` is a stateful submit/challenge/resolve
lifecycle manager -- an entirely different concept. It is a WisdomGate in the
Optimistic-Rollup challenge-window sense, not the knowledge-filtering sense.

Specific issues:

1. The `WisdomGate` struct (line 38) holds a `HashMap<B256, WisdomSubmission>` and a `challenge_window: u64`. This is a challenge-window state machine, not a quality filter.

2. `WisdomState` (line 25) has 4 variants (Pending, Challenged, Accepted, Rejected) which do not match the spec's `WisdomRejection` enum (InsufficientTrust, Tainted, NotRelevant, Duplicate, AuthorSaturation).

3. There is no `WisdomGateConfig`. No `min_trust_bps`, `resonance_threshold`, `duplicate_threshold`, or `max_per_author`.

4. There is no `filter()` method. The implementation has `submit()`, `challenge()`, `resolve()` -- lifecycle operations, not filtering operations.

5. The 5-check pipeline (Trust, Taint, Relevance, Duplicate, Diversity) does not exist.

6. The prompt-injection gap is not documented as the spec requires (anti-pattern #4).

7. `WisdomSubmission` has a `bond: u64` field (line 18), suggesting this was modeled after an on-chain staking mechanism, not the off-chain agent filter the spec describes.

### F06 -- `rpc.rs` provides functionality that belongs in `kora-rpc`

File: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs`

The spec says `kora-hdc-chain` provides types and index; the RPC layer lives in
`kora-rpc` (`crates/node/rpc/`). The implementation places `HdcApi` with 7 RPC
methods directly in this crate. This creates a layer violation.

Specific issues with the implementation:

1. `HdcApi` (line 15) holds `Arc<parking_lot::RwLock<OnChainHdcIndex>>`. This is the correct concurrency pattern from the spec's `NeuroChainSync`, but it is in the wrong module.

2. RPC methods duplicate precompile logic: `hamming_distance_rpc()`, `bind_rpc()`, `bundle_rpc()` all re-implement what the precompile already does. This creates dual maintenance burden.

3. `search_rpc()` (line 59) delegates to `index.search()` which has the non-deterministic sort issue from F03.

4. `SearchResultRpc` (line 99) drops the metadata from `OnChainSearchResult` -- callers cannot see publisher, block_number, or state.

5. The spec defines 4 RPC methods: `hdc_searchSimilar`, `hdc_getInsight`, `hdc_getPheromone`, `hdc_getIndexStats`. The implementation provides 7 methods but misses `hdc_getInsight`, `hdc_getPheromone`, and `hdc_getIndexStats`.

6. `encode_rpc()` (line 83) does not return a `Result` -- it cannot report errors to callers.

### F07 -- `lib.rs` re-exports are incomplete

File: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/lib.rs`

The spec's `lib.rs` re-exports 17 items from 7 modules. The implementation
re-exports 2 items from 2 modules:

```rust
pub use precompile::{hdc_precompile, PRECOMPILE_ADDRESS};
pub use index::OnChainHdcIndex;
```

Missing re-exports per spec: `fixed_point_decay`, `fixed_point_exp2_neg`,
`TieredSearch`, `TieredSearchResult`, `TIER1_GAS`, `TIER2_GAS`, `TIER3_GAS`,
`NeuroChainSync`, `WisdomGate`, `WisdomGateConfig`, `WisdomRejection`,
`InsightEvent`, `InsightKind`, `InsightMetadata`, `InsightState`, `InsightTier`,
`PheromoneDeposit`, `PheromoneType`, `ChainHdcError`.

### F08 -- Cargo.toml deviations

File: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/Cargo.toml`

1. `revm` is listed as a dependency (line 16) but is not used anywhere in the crate source. No file imports from `revm`. This is dead weight.

2. `tokio` is missing. The spec requires `tokio = { workspace = true, features = ["sync", "rt"] }` for async event subscription in `NeuroChainSync`.

3. `serde` is present but no `features = ["derive"]`. The spec's types need `#[derive(Serialize, Deserialize)]`.

4. `description` says "InsightBoard" (line 8); the spec says "On-chain HDC integration layer for Kora".

5. `[dev-dependencies]` is empty. The spec requires `rstest.workspace = true` and `tokio = { workspace = true, features = ["macros", "test-util"] }`.

### F09 -- Consensus safety violations

1. **Non-deterministic search sort** (`index.rs` line 117): `results.sort_by_key(|r| r.distance)` without composite tiebreaker. The spec explicitly warns against this in anti-pattern #3.

2. **`HashMap` used for vector storage** (`index.rs` line 14): `vectors: HashMap<B256, HdcVector>`. The `search()` method (line 99) iterates this HashMap (`self.vectors.iter()`), producing non-deterministic iteration order. This is anti-pattern #2 from the spec. If two validators iterate in different orders and produce different top-k results (due to distance ties), consensus diverges.

3. **No `InsightEvent` enum**: Without the typed event enum, there is no type-safe replay path, making it impossible to guarantee deterministic reconstruction.

---

## Implementation Status

| Spec Section | Description | Status | Quality |
|---|---|---|---|
| S1. Workspace changes | Cargo.toml setup | DONE with issues | Has unused `revm` dep, missing `tokio` |
| S2. Module structure | 8 modules | 3/8 present | 5 modules completely missing |
| S3. Types (`types.rs`) | InsightEvent, InsightMetadata, enums | NOT STARTED | No file exists |
| S4. On-chain index (`index.rs`) | OnChainHdcIndex wrapping LocalIndex | PARTIAL (~30%) | Missing LocalIndex delegation, rebuild_from_events, remove, most metadata fields |
| S5. Fixed-point decay (`decay.rs`) | fixed_point_decay, fixed_point_exp2_neg | NOT STARTED | No file exists |
| S6. Tiered search (`search.rs`) | TieredSearch 3-tier pipeline | NOT STARTED | No file exists |
| S7. Event sync (`sync.rs`) | NeuroChainSync | STUB (~5%) | `event.rs` exists but is non-functional |
| S8. WisdomGate (`wisdom_gate.rs`) | 5-check filter | WRONG DESIGN | Implements challenge lifecycle instead of quality filter |
| S9. Error types (`error.rs`) | ChainHdcError | NOT STARTED | No file exists |
| S10. On-chain/off-chain boundary | Documentation table | NOT IMPLEMENTED | No boundary annotations in code |
| S11. Integration points | Executor + RPC wiring | PARTIAL | Precompile in wrong location; RPC in wrong crate |
| S12. Anti-patterns compliance | 6 anti-patterns avoided | VIOLATIONS | Anti-patterns #2 and #3 violated |
| S13. Stubs from kora-hdc | Core algebra types | N/A | kora-hdc appears to be fully implemented |
| S14. Checklist | 18 items | ~5/18 done | See below |
| S15. Test plan | Unit + integration tests | PARTIAL | Precompile tests good; index tests minimal; no decay/search/wisdom tests |

### Checklist Status

- [x] Create `crates/hdc/` directory
- [x] Create `crates/hdc/core/Cargo.toml`
- [x] Create `crates/hdc/chain/Cargo.toml` (with deviations, see F08)
- [x] Add `"crates/hdc/*"` to workspace members
- [x] Add workspace dependencies
- [ ] `src/types.rs` -- MISSING
- [ ] `src/error.rs` -- MISSING
- [x] `src/index.rs` -- EXISTS but incomplete (see F03)
- [ ] `src/decay.rs` -- MISSING
- [ ] `src/search.rs` -- MISSING
- [ ] `src/sync.rs` -- MISSING (event.rs is not equivalent)
- [ ] `src/wisdom_gate.rs` -- WRONG DESIGN (see F05)
- [x] `src/lib.rs` -- EXISTS but incomplete re-exports (see F07)
- [ ] All public items have doc comments -- PARTIAL (precompile good, index ok, event/wisdom sparse)
- [ ] All consensus-path functions have CONSENSUS SAFE annotation -- NOT DONE
- [ ] No `f64` in consensus path -- PASS (no f64 used anywhere)
- [ ] No `HashMap` iteration in consensus path -- FAIL (index.rs search iterates HashMap)
- [ ] All sorts use composite tiebreakers -- FAIL (index.rs line 117)
- [ ] WisdomGate prompt-injection gap documented -- NOT DONE

---

## Anti-Patterns & Duct Tape

### AP-01: HashMap iterated in consensus path (CRITICAL)

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`, line 99-115
- **Function**: `OnChainHdcIndex::search()`
- **Issue**: `self.vectors.iter()` iterates a `HashMap<B256, HdcVector>`. HashMap iteration order is non-deterministic across processes due to random hashing. Two validators iterating the same map may encounter entries in different orders. Combined with AP-02 (no composite tiebreaker), this means equal-distance candidates can appear in different final orders, causing consensus divergence.
- **Fix**: Replace `vectors: HashMap<B256, HdcVector>` with a `BTreeMap<B256, HdcVector>` for deterministic iteration, OR delegate to `kora_hdc::LocalIndex` which handles ordering internally, as specified.

### AP-02: Sort without composite tiebreaker (CRITICAL)

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`, line 117
- **Code**: `results.sort_by_key(|r| r.distance);`
- **Issue**: This is the exact anti-pattern the spec warns about in section 12.3. When multiple vectors have the same Hamming distance to the query, the output order depends on the input order (stable sort), which is non-deterministic due to AP-01.
- **Fix**: `results.sort_by(|a, b| a.distance.cmp(&b.distance).then_with(|| a.id.cmp(&b.id)));`

### AP-03: WisdomGate is the wrong abstraction entirely

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom.rs`
- **Issue**: The implementation builds a challenge-window state machine (submit/challenge/resolve with bonds and block-based timeouts). The spec calls for a stateless quality filter with 5 checks (trust, taint, relevance, duplicate, diversity). These are fundamentally different designs solving different problems. The implementation appears to have been written from a different spec or mental model.
- **Impact**: The entire file needs to be rewritten. No code from the current `wisdom.rs` is salvageable for the spec's `wisdom_gate.rs`.

### AP-04: Event processing is entirely stubbed out

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`
- **Issue**: All 5 event topic hashes are `B256::ZERO` (lines 13-26). The `process_log()` function matches on these zero hashes and logs messages but does not actually decode or process any data. Every branch is a no-op. This means the on-chain index can never be populated from chain events.
- **Impact**: The index can only be populated via direct `insert_insight()` calls, not from event replay. This makes startup reconstruction impossible.

### AP-05: Precompile function signature does not integrate with REVM

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/precompile.rs`, line 74
- **Function**: `pub fn hdc_precompile(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError>`
- **Issue**: REVM precompiles use `PrecompileResult` (a type alias for `Result<PrecompileOutput, PrecompileError>` from the `revm` crate). The custom `PrecompileError` in this file is not the REVM `PrecompileError`. Despite `revm` being listed in `Cargo.toml`, it is not imported or used. The executor will need an adapter layer to bridge these types.
- **Fix**: Either return REVM's native types, or provide a `to_revm_result()` adapter method on the output.

### AP-06: RPC module in wrong crate (layer violation)

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs`
- **Issue**: The `HdcApi` struct with 7 RPC methods belongs in `crates/node/rpc/`, not in the chain crate. The chain crate should provide types and index; the RPC crate wires them to jsonrpsee. Placing RPC logic here means the chain crate has an implicit dependency on the RPC framework's design patterns, and the actual RPC crate (`kora-rpc`) must either duplicate this logic or re-export it awkwardly.
- **Fix**: Move `HdcApi` to `crates/node/rpc/src/hdc.rs`. Keep only types and index in the chain crate.

### AP-07: Dual `InsightState` enums with conflicting variants

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`, lines 32-47 (InsightState)
- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom.rs`, lines 24-34 (WisdomState)
- **Issue**: Two separate state enums exist with different variant sets. `InsightState` has {Draft, Submitted, Challenged, Voting, Accepted, Rejected, Expired}. `WisdomState` has {Pending, Challenged, Accepted, Rejected}. Neither matches the spec's `InsightState` {Submitted, Verified, Active, Challenged, Decaying, Archived, Purged}. This creates type confusion and makes it impossible to correctly map between on-chain contract states and Rust-side tracking.
- **Fix**: Define a single `InsightState` enum in `types.rs` matching the spec, with `#[repr(u8)]` discriminants matching the Solidity enum.

### AP-08: `_unused` parameters mask missing implementation

- **Files**: Multiple
- `event.rs` line 33: `_data: &[u8]` and `_log_address: &Address` -- the log data is never decoded
- `wisdom.rs` line 79: `_challenger: Address` -- challenger identity is discarded
- `index.rs` line 130: `_topic`, `_region`, `_strength` all unused in `record_pheromone()`
- **Issue**: Prefixing with `_` silences compiler warnings but hides the fact that these functions are stubs. A reader might assume they work.
- **Fix**: Either implement the logic or use `todo!()` / `unimplemented!()` to make the stub status explicit at runtime.

### AP-09: No `#[must_use]` annotations

- **Files**: `index.rs`, `wisdom.rs`, `rpc.rs`
- **Issue**: The spec marks `new()`, `is_empty()`, `len()`, `get_metadata()` with `#[must_use]`. The implementation has none. This allows callers to silently discard return values.

### AP-10: `serialize` import unused in `index.rs`

- **File**: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`, line 8
- **Code**: `use kora_hdc::{HdcVector, hamming_distance, serialize};`
- **Issue**: `serialize` is imported but never used. This will trigger a compiler warning or error under the workspace's `unused-must-use = "deny"` lint configuration.

---

## Second-Pass Remediation Detail

This pass should treat the current `crates/hdc/chain` implementation as a
prototype and replace the unsafe/stubbed surfaces with event-sourced,
deterministic primitives. Do not patch around the existing `HashMap` iteration,
zero topic constants, or chain-local RPC wrapper; those are structural issues.

### Target module layout

Target files and public ownership:

```text
crates/hdc/chain/src/
  lib.rs             # crate docs and public re-exports only
  precompile.rs      # pure HDC precompile opcodes: hdc_precompile(), read_vector(), exec_*
  types.rs           # InsightEvent, InsightMetadata, InsightKind, InsightState, InsightTier,
                     # PheromoneDeposit, PheromoneType, IndexStats
  error.rs           # ChainHdcError and Result<T>
  index.rs           # OnChainHdcIndex deterministic event-sourced mirror
  event.rs           # topic constants and ABI log decoding, no index mutation
  sync.rs            # NeuroChainSync replay/apply orchestration
  search.rs          # optional chain-local tiered search wrapper if not using kora_hdc::search::TieredSearchPipeline
  wisdom_gate.rs     # off-chain WisdomGate quality filter only
```

`src/rpc.rs` should be removed from this crate's public surface. RPC belongs in
`crates/node/rpc/src/hdc.rs`. `src/wisdom.rs` should not remain as the
WisdomGate implementation; on-chain lifecycle state is represented by
`InsightState` in `types.rs`, while agent-local acceptance policy lives in
`wisdom_gate.rs`.

`src/lib.rs` should re-export only crate-level primitives:

```rust
pub mod event;
pub mod error;
pub mod index;
pub mod precompile;
pub mod sync;
pub mod types;
pub mod wisdom_gate;

pub use error::{ChainHdcError, Result};
pub use index::OnChainHdcIndex;
pub use precompile::{hdc_precompile, PRECOMPILE_ADDRESS};
pub use sync::NeuroChainSync;
pub use types::{InsightEvent, InsightKind, InsightMetadata, InsightState, InsightTier, IndexStats, PheromoneDeposit, PheromoneType};
pub use wisdom_gate::{WisdomGate, WisdomGateConfig, WisdomRejection};
```

### Event sync and ABI decoding

Current file: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`

Replace the current `topics::* = B256::ZERO` placeholders and no-op
`process_log()` with decode-only functions:

- `event::topic0(signature: &str) -> B256`
- `event::decode_log(log: &HdcRawLog) -> Result<Option<InsightEvent>>`
- `event::decode_insight_published(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_insight_confirmed(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_insight_challenged(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_insight_state_changed(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_insight_renewed(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_insight_purged(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_pheromone_deposited(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_pheromone_confirmed(log: &HdcRawLog) -> Result<InsightEvent>`
- `event::decode_pheromone_pruned(log: &HdcRawLog) -> Result<InsightEvent>`

The actual Solidity interfaces define these event signatures:

- `InsightPublished(bytes32,bytes32,address,bytes,bytes,uint8,uint8)`
- `InsightConfirmed(bytes32,address,uint64)`
- `InsightChallenged(bytes32,bytes32,address)`
- `InsightStateChanged(bytes32,uint8,uint8)`
- `InsightRenewed(bytes32,address)`
- `InsightPurged(bytes32,address)`
- `PheromoneDeposited(bytes32,bytes32,address,uint8,uint64,uint64)`
- `PheromoneConfirmed(bytes32,address,uint64,uint64)`
- `PheromonePruned(bytes32,address)`

External refs already verified and relevant here:

- Solidity event `topic0` is the Keccak hash of the canonical event signature,
  e.g. `keccak256("InsightConfirmed(bytes32,address,uint64)")`. It does not
  include `indexed`, parameter names, spaces, or the `event` keyword.
- `alloy_primitives::Keccak256` exposes streaming `update()` and `finalize()`;
  use it in `topic0()` or tests, or use `alloy_primitives::keccak256()` for the
  one-shot case.
- `InsightPublished` has dynamic `bytes vector` and `bytes content`; do not
  hand-slice those fields. Decode with ABI-aware code, then validate
  `vector.len() == kora_hdc::BYTES`, `keccak256(vector) == vectorHash`, and
  compute `InsightMetadata.content_hash = keccak256(content)`.

`process_log()` should either be deleted or changed to:

```rust
pub fn process_log(index: &mut OnChainHdcIndex, log: &HdcRawLog) -> Result<()> {
    if let Some(event) = decode_log(log)? {
        index.apply_event(event)?;
    }
    Ok(())
}
```

The preferred split is cleaner: `event.rs` decodes logs, `sync.rs` applies them.

Target sync file: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/sync.rs`

Required functions:

- `NeuroChainSync::new(index: OnChainHdcIndex) -> Self`
- `NeuroChainSync::replay_from_logs<I>(&self, logs: I) -> Result<()>`
- `NeuroChainSync::apply_log(&self, log: HdcRawLog) -> Result<()>`
- `NeuroChainSync::apply_event(&self, event: InsightEvent, cursor: EventCursor) -> Result<()>`
- `NeuroChainSync::index(&self) -> parking_lot::RwLockReadGuard<'_, OnChainHdcIndex>`
- `NeuroChainSync::synced_block(&self) -> u64`

Replay order must be canonical: `(block_number, transaction_index, log_index)`.
`EventCursor` should carry those three fields and `log_address`; `apply_event()`
must reject out-of-order replay instead of silently accepting it.

### Deterministic index

Current file: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`

Replace the current storage:

```rust
vectors: HashMap<B256, HdcVector>,
metadata: HashMap<B256, InsightMeta>,
```

with deterministic storage and core-index delegation:

```rust
use std::collections::BTreeMap;
use kora_hdc::search::{H256 as CoreH256, LocalIndex, SearchIndex};

pub struct OnChainHdcIndex {
    index: LocalIndex,
    vectors: BTreeMap<B256, HdcVector>,
    metadata: BTreeMap<B256, InsightMetadata>,
    pheromones: BTreeMap<B256, PheromoneDeposit>,
}
```

External ref already verified: Rust `HashMap` is randomly seeded. Key lookup is
fine, but iteration order is not deterministic across processes. Because
`OnChainHdcIndex::search()` currently iterates `self.vectors.iter()`, it must
not use `HashMap` on the consensus path.

Required `OnChainHdcIndex` functions:

- `OnChainHdcIndex::new() -> Self`
- `OnChainHdcIndex::store(&mut self, insight_id: B256, vector: HdcVector, metadata: InsightMetadata) -> Result<()>`
- `OnChainHdcIndex::remove(&mut self, insight_id: &B256) -> Result<()>`
- `OnChainHdcIndex::apply_event(&mut self, event: InsightEvent) -> Result<()>`
- `OnChainHdcIndex::rebuild_from_events<I>(&mut self, events: I) -> Result<()>`
- `OnChainHdcIndex::update_metadata(&mut self, insight_id: &B256, f: impl FnOnce(&mut InsightMetadata)) -> Result<()>`
- `OnChainHdcIndex::search(&self, query: &HdcVector, top_k: usize) -> Result<Vec<OnChainSearchResult>>`
- `OnChainHdcIndex::get_vector(&self, insight_id: &B256) -> Option<&HdcVector>`
- `OnChainHdcIndex::get_metadata(&self, insight_id: &B256) -> Option<&InsightMetadata>`
- `OnChainHdcIndex::get_pheromone(&self, pheromone_id: &B256) -> Option<&PheromoneDeposit>`
- `OnChainHdcIndex::stats(&self) -> IndexStats`
- `OnChainHdcIndex::len(&self) -> usize`
- `OnChainHdcIndex::is_empty(&self) -> bool`

`search()` must use the core `SearchIndex::search()` result, convert
`CoreH256` to `B256`, attach metadata, and then sort again with the explicit
composite key:

```rust
results.sort_by(|a, b| {
    a.distance.cmp(&b.distance).then_with(|| a.id.cmp(&b.id))
});
```

State filtering should not be hardcoded to `InsightState::Accepted`; that state
does not exist in the Solidity enum. The consensus mirror indexes events and
state. Agent-local acceptance filtering belongs in `WisdomGate`.

### Type model

Target file: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/types.rs`

Create one canonical type model matching `contracts/src/IInsightBoard.sol` and
`contracts/src/IPheromoneRegistry.sol`:

- `InsightKind` with `#[repr(u8)]`: `Insight = 0`, `Heuristic = 1`,
  `AntiKnowledge = 2`, `Warning = 3`, `CausalLink = 4`, `Strategy = 5`.
- `InsightState` with `#[repr(u8)]`: `Submitted = 0`, `Verified = 1`,
  `Active = 2`, `Challenged = 3`, `Decaying = 4`, `Archived = 5`,
  `Purged = 6`.
- `InsightTier` with `#[repr(u8)]`: `Transient = 0`, `Working = 1`,
  `Consolidated = 2`, `Persistent = 3`.
- `PheromoneType` with `#[repr(u8)]`: `Threat = 0`, `Opportunity = 1`,
  `Wisdom = 2`.
- `EventCursor { block_number, transaction_index, log_index }` for replay
  ordering.
- `HdcRawLog { log_address, topics, data, cursor }` as the chain-crate log
  input type used by `event::decode_log()` and `NeuroChainSync::apply_log()`.
- `InsightMetadata { vector_hash, author, publish_block, kind, tier, state,
  confirmations, last_confirmed_block, content_hash }`.
- `PheromoneDeposit { pheromone_id, location_hash, depositor, pheromone_type,
  intensity, deposit_block, confirmations, effective_half_life }`.
- `InsightEvent::Published { insight_id, vector_hash, author, vector,
  content_hash, kind, tier, publish_block }`.
- `InsightEvent::Confirmed { insight_id, confirmer, total_confirmations }`.
- `InsightEvent::Challenged { insight_id, challenging_insight_id, challenger }`.
- `InsightEvent::StateChanged { insight_id, old_state, new_state }`.
- `InsightEvent::Renewed { insight_id, renewer }`.
- `InsightEvent::Purged { insight_id, purger }`.
- `InsightEvent::PheromoneDeposited { deposit }`.
- `InsightEvent::PheromoneConfirmed { pheromone_id, confirmer,
  new_confirmation_count, new_effective_half_life }`.
- `InsightEvent::PheromonePruned { pheromone_id, pruner }`.
- `OnChainSearchResult { id, distance, metadata }`.
- `IndexStats { vectors, pheromones, synced_block }`.

Add `TryFrom<u8>` implementations for `InsightKind`, `InsightState`,
`InsightTier`, and `PheromoneType`; invalid values must return
`ChainHdcError::InvalidEnumValue`.

### Error model

Target file: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/error.rs`

Define:

```rust
pub type Result<T> = std::result::Result<T, ChainHdcError>;
```

Required `ChainHdcError` variants:

- `UnknownTopic0(B256)`
- `InvalidTopicCount { event: &'static str, expected: usize, got: usize }`
- `AbiDecode { event: &'static str, reason: String }`
- `InvalidVectorLength { expected: usize, got: usize }`
- `VectorHashMismatch { expected: B256, got: B256 }`
- `InvalidEnumValue { ty: &'static str, value: u8 }`
- `DuplicateInsight(B256)`
- `InsightNotFound(B256)`
- `PheromoneNotFound(B256)`
- `Index(String)` for `kora_hdc::search::HdcIndexError`
- `EventOrder { last: EventCursor, got: EventCursor }`
- `GasBudgetExceeded { used: u64, limit: u64 }`
- `SubscriptionClosed`

Do not hide decode failures with `info!()` logs. Every malformed event must
return a typed error so replay can fail closed.

### Wisdom gate split

Current file: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom.rs`

The current `WisdomGate::submit()`, `WisdomGate::challenge()`, and
`WisdomGate::resolve()` model a lifecycle FSM. That lifecycle already exists in
`InsightBoard.sol` and is mirrored through `InsightStateChanged` events. Keep
that state in `InsightMetadata.state`.

Create `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom_gate.rs` for
the off-chain filter:

- `WisdomGate::new(config: WisdomGateConfig) -> Self`
- `WisdomGate::with_defaults() -> Self`
- `WisdomGate::filter(&self, candidates: &[WisdomCandidate<'_>], already_accepted: &[AcceptedWisdom<'_>]) -> WisdomGateOutput`
- `WisdomGate::check_trust(...) -> std::result::Result<(), WisdomRejection>`
- `WisdomGate::check_taint(...) -> std::result::Result<(), WisdomRejection>`
- `WisdomGate::check_relevance(...) -> std::result::Result<(), WisdomRejection>`
- `WisdomGate::check_duplicate(...) -> std::result::Result<(), WisdomRejection>`
- `WisdomGate::check_author_diversity(...) -> std::result::Result<(), WisdomRejection>`

`WisdomGate` must remain off-chain only. It may use caller-provided trust
scores and local policy. It must not be called from precompile execution or
validator state transition code. Its doc comment must explicitly say it does
not inspect content payloads for prompt injection.

### RPC relocation

Current chain file: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs`
Current RPC wrapper: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs`

Move the chain-local `HdcApi` methods into `crates/node/rpc/src/hdc.rs` and
remove `pub mod rpc;` from `kora-hdc-chain`. The RPC crate should own:

- `HdcRpcApi::hamming_distance()`
- `HdcRpcApi::similarity()`
- `HdcRpcApi::bind()`
- `HdcRpcApi::bundle()`
- `HdcRpcApi::search()`
- `HdcRpcApi::vector_id()`
- `HdcRpcApi::encode()`
- `HdcRpcApi::get_insight()`
- `HdcRpcApi::get_pheromone()`
- `HdcRpcApi::get_index_stats()`

`HdcApiImpl` should hold `Arc<parking_lot::RwLock<OnChainHdcIndex>>` directly
or a small RPC-local service wrapper. `parse_vector()` belongs in
`crates/node/rpc/src/hdc.rs`, not in the chain crate. RPC result structs should
be serde-enabled in the RPC crate and should wrap chain types instead of
forcing chain types to depend on jsonrpsee concerns.

### Second-pass tests

Add tests with these exact targets:

- `crates/hdc/chain/src/event.rs`
  - `topic0_matches_solidity_signature_hashes`
  - `decode_insight_published_extracts_indexed_and_dynamic_fields`
  - `decode_insight_published_rejects_bad_vector_len`
  - `decode_insight_published_rejects_vector_hash_mismatch`
  - `decode_state_changed_rejects_invalid_state`
  - `decode_unknown_topic_returns_unknown_topic_error`
- `crates/hdc/chain/src/index.rs`
  - `search_ties_sort_by_distance_then_id`
  - `rebuild_from_events_is_deterministic`
  - `rebuild_rejects_duplicate_insight`
  - `purged_event_removes_vector_and_metadata`
  - `confirmed_event_updates_confirmations_and_tier`
  - `hashmap_iteration_regression_many_equal_distances`
- `crates/hdc/chain/src/sync.rs`
  - `replay_from_logs_applies_block_tx_log_order`
  - `apply_log_rejects_out_of_order_cursor`
  - `synced_block_tracks_last_applied_block`
- `crates/hdc/chain/src/wisdom_gate.rs`
  - `filter_rejects_insufficient_trust`
  - `filter_rejects_challenged_or_purged_state`
  - `filter_rejects_non_resonant_distance`
  - `filter_rejects_duplicate_vector`
  - `filter_enforces_author_diversity`
- `crates/node/rpc/src/hdc.rs`
  - `get_insight_returns_metadata`
  - `get_pheromone_returns_deposit`
  - `get_index_stats_returns_counts`
  - `search_returns_metadata_and_deterministic_order`
- `crates/e2e/src/tests/hdc.rs`
  - `event_replay_populates_hdc_index_after_submit`
  - `purge_event_removes_hdc_index_entry`

### Phased checklist

Phase 0 - Lock the boundary:

- [ ] Mark `crates/hdc/chain/src/rpc.rs` as slated for removal from the chain
  crate.
- [ ] Mark `crates/hdc/chain/src/wisdom.rs` as the wrong abstraction and stop
  adding lifecycle behavior there.
- [ ] Add `ChainHdcError` and `types.rs` first so later modules use one shared
  type model.

Phase 1 - Event ABI and topics:

- [ ] Replace all `event::topics::* = B256::ZERO` constants with computed
  topic hashes for the actual Solidity signatures listed above.
- [ ] Implement `event::topic0()`.
- [ ] Implement `event::decode_log()` and per-event decode functions.
- [ ] Add topic and ABI decode tests before wiring sync.

Phase 2 - Deterministic index:

- [ ] Replace `HashMap` vector iteration with `BTreeMap` plus
  `kora_hdc::search::LocalIndex`.
- [ ] Implement `OnChainHdcIndex::store()`, `remove()`, `apply_event()`,
  `rebuild_from_events()`, `update_metadata()`, and `stats()`.
- [ ] Enforce composite `(distance, id)` ordering in `search()`.
- [ ] Remove `Accepted` state filtering from `search()`.

Phase 3 - Sync orchestration:

- [ ] Create `sync.rs` with `NeuroChainSync`.
- [ ] Replay logs in `(block_number, transaction_index, log_index)` order.
- [ ] Reject cursor regressions with `ChainHdcError::EventOrder`.
- [ ] Track `synced_block()` from the last applied cursor.

Phase 4 - Wisdom gate split:

- [ ] Replace `wisdom.rs` exports with `wisdom_gate.rs`.
- [ ] Implement the five off-chain checks as separate helper functions.
- [ ] Document that content prompt-injection filtering is outside
  `WisdomGate`.

Phase 5 - RPC relocation:

- [ ] Remove `pub mod rpc;` from `crates/hdc/chain/src/lib.rs`.
- [ ] Move vector parsing and HDC RPC method bodies into
  `crates/node/rpc/src/hdc.rs`.
- [ ] Add `get_insight`, `get_pheromone`, and `get_index_stats` RPC methods.
- [ ] Keep chain crate independent of jsonrpsee/RPC response types.

Phase 6 - Verification:

- [ ] Run `cargo test -p kora-hdc-chain`.
- [ ] Run `cargo test -p kora-rpc hdc`.
- [ ] Run `cargo nextest run --workspace --all-features hdc`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo +nightly fmt --all -- --check`.

---

## Recommended Changes Checklist

Priority levels: **P0** = consensus-critical / will cause bugs, **P1** = spec compliance / required for feature completeness, **P2** = quality / maintainability.

### P0 -- Consensus Safety Fixes

- [ ] **P0-1**: Replace `HashMap<B256, HdcVector>` with `BTreeMap` or delegate to `LocalIndex` in `index.rs` line 14. HashMap iteration in `search()` is non-deterministic.
- [ ] **P0-2**: Add composite tiebreaker to `search()` sort in `index.rs` line 117. Change `sort_by_key(|r| r.distance)` to `sort_by(|a, b| a.distance.cmp(&b.distance).then_with(|| a.id.cmp(&b.id)))`.
- [ ] **P0-3**: Align `InsightState` enum (`index.rs` lines 32-47) with spec: {Submitted=0, Verified=1, Active=2, Challenged=3, Decaying=4, Archived=5, Purged=6} with `#[repr(u8)]` discriminants.

### P1 -- Missing Module Implementation

- [ ] **P1-1**: Create `src/types.rs` with `InsightEvent`, `InsightMetadata`, `InsightKind`, `InsightState`, `InsightTier`, `PheromoneDeposit`, `PheromoneType` as specified in spec section 3.
- [ ] **P1-2**: Create `src/error.rs` with `ChainHdcError` enum as specified in spec section 9: `EventLogGap`, `InvalidVectorSize`, `InsightNotFound`, `GasBudgetExceeded`, `SubscriptionClosed`.
- [ ] **P1-3**: Create `src/decay.rs` with `fixed_point_decay()` and `fixed_point_exp2_neg()` as specified in spec section 5. Include all 8 unit tests from the spec.
- [ ] **P1-4**: Create `src/search.rs` with `TieredSearch` 3-tier pipeline as specified in spec section 6. Include gas constants `TIER1_GAS=100`, `TIER2_GAS=500`, `TIER3_GAS=1500`.
- [ ] **P1-5**: Replace `src/event.rs` with `src/sync.rs` containing `NeuroChainSync` as specified in spec section 7: `RwLock<OnChainHdcIndex>`, `AtomicU64`, `replay_from()`, `apply_event()`, `index()`, `synced_block()`.
- [ ] **P1-6**: Rewrite `src/wisdom.rs` as `src/wisdom_gate.rs` implementing the 5-check stateless filter (Trust, Taint, Relevance, Duplicate, Diversity) with `WisdomGateConfig` as specified in spec section 8. Document the prompt-injection gap.

### P1 -- Index Rewrite

- [ ] **P1-7**: Refactor `OnChainHdcIndex` to wrap `kora_hdc::LocalIndex` instead of raw `HashMap`. Use `LocalIndex::insert()`, `LocalIndex::remove()`, `LocalIndex::search()`.
- [ ] **P1-8**: Expand `InsightMeta` to full `InsightMetadata` with all 8 fields: author, publish_block, kind, tier, state, confirmations, last_confirmed_block, content_hash.
- [ ] **P1-9**: Add `rebuild_from_events()` method to `OnChainHdcIndex` for deterministic startup reconstruction.
- [ ] **P1-10**: Add `remove()` method to `OnChainHdcIndex`.
- [ ] **P1-11**: Add `update_metadata()` with closure to `OnChainHdcIndex`.
- [ ] **P1-12**: Remove state-filtering from `search()` (currently filters to `Accepted` only). Filtering belongs in `WisdomGate`.

### P1 -- lib.rs and Cargo.toml Fixes

- [ ] **P1-13**: Update `src/lib.rs` re-exports to match spec: add all 17+ type/function re-exports.
- [ ] **P1-14**: Remove unused `revm` dependency from `Cargo.toml`.
- [ ] **P1-15**: Add `tokio = { workspace = true, features = ["sync", "rt"] }` to `Cargo.toml`.
- [ ] **P1-16**: Add `features = ["derive"]` to `serde` dependency in `Cargo.toml`.
- [ ] **P1-17**: Add `rstest` and `tokio` test features to `[dev-dependencies]`.

### P1 -- RPC Relocation

- [ ] **P1-18**: Move `HdcApi` from `crates/hdc/chain/src/rpc.rs` to `crates/node/rpc/src/hdc.rs`. The chain crate should only export types and index.
- [ ] **P1-19**: Add the 3 missing RPC methods: `hdc_getInsight`, `hdc_getPheromone`, `hdc_getIndexStats`.

### P2 -- Quality Improvements

- [ ] **P2-1**: Add `#[must_use]` to `OnChainHdcIndex::new()`, `len()`, `is_empty()`, `get()`.
- [ ] **P2-2**: Add `CONSENSUS SAFE` / `CONSENSUS SAFETY` doc annotations to all consensus-path functions.
- [ ] **P2-3**: Remove unused `serialize` import from `index.rs` line 8.
- [ ] **P2-4**: Replace `_`-prefixed unused parameters with `todo!()` or actual implementations to make stub status explicit.
- [ ] **P2-5**: Add `Serialize`/`Deserialize` derives to `InsightMeta`, `InsightState`, `OnChainSearchResult`.
- [ ] **P2-6**: Add `#[repr(u8)]` to `InsightState` for Solidity enum compatibility.
- [ ] **P2-7**: Add integration tests for event replay -> index rebuild -> search cycle.
- [ ] **P2-8**: Add `TIER1_WORD_THRESHOLD` and `TIER2_APPROX_THRESHOLD` documentation once `search.rs` is created.
- [ ] **P2-9**: Connect `precompile.rs`'s `PrecompileError` to REVM's `PrecompileError` type, or provide adapter methods.

---

## Implementation Reality Check

### What exists (DONE)

- [x] `OnChainHdcIndex` with `BTreeMap`-backed metadata storage -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`
- [x] `InsightState` FSM with 7 states (Submitted, Verified, Active, Challenged, Decaying, Archived, Purged) -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`
- [x] `insert_insight()`, `search()`, `update_state()` methods on the index -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`
- [x] `event.rs` with event decoders for: InsightPublished, InsightConfirmed, InsightChallenged, InsightStateChanged, InsightPurged, PheromoneDeposited -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`
- [x] `rpc.rs` with `HdcApi` trait exposing: `hamming`, `similarity`, `bind`, `bundle`, `permute`, `vectorId`, `encode`, `search`, `getInsight` -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs`
- [x] `precompile.rs` with opcode dispatch table -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/precompile.rs` (handlers are STUBBED)
- [x] `wisdom.rs` with `WisdomGate` submit path -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom.rs` (challenge/resolve STUBBED)

### What is missing or incomplete

- [ ] **Complete precompile handler implementations** -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/precompile.rs` dispatches opcodes but handlers return placeholder values or are no-ops
- [ ] **Complete WisdomGate `challenge()` and `resolve()`** -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/wisdom.rs` has only `submit()`; challenge and resolve are stubbed
- [ ] **Add decay arithmetic (integer-only, no floats on-chain)** -- No `decay.rs` module exists yet; the spec in this doc (section 5) defines `fixed_point_decay()` and `fixed_point_exp2_neg()` but they are not implemented
- [ ] **Wire event sync into `FinalizedReporter`** -- `event.rs` decodes events but there is no integration with the block finalization pipeline to trigger index updates
- [ ] **Add index persistence model** -- Currently in-memory only; decide between: (a) rebuild from events on startup, or (b) snapshot/checkpoint. The spec (section 7) prescribes event replay via `NeuroChainSync` but no `sync.rs` file exists
- [ ] **Implement `record_pheromone()` backing storage** -- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs` line ~130 is a TODO stub
- [ ] **Create `search.rs` with tiered search** -- The spec (section 6) defines a 3-tier gas-optimized search but no `search.rs` file exists in the chain crate
- [ ] **Create `types.rs` with shared types** -- Types are currently scattered across `index.rs` and `event.rs`; the spec calls for a unified `types.rs`

### Note: kora-precompiles duplication

PR #42 introduced a parallel precompile implementation (`StigmergyPrecompile` at `0xA0D`)
that overlaps with `precompile.rs` in this crate. The two need to be reconciled:

- `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/precompile.rs` -- this crate's precompile (stub handlers)
- PR #42's `StigmergyPrecompile` -- separate precompile focused on stigmergy/pheromones

Decision needed: merge both into a single precompile, or keep them as separate precompiles
at different addresses (one for HDC vector ops, one for stigmergy/pheromones).

### Verification commands

```bash
# Check compilation
cargo check -p kora-hdc-chain

# Run tests
cargo test -p kora-hdc-chain

# Find all stubs and TODOs
grep -rn "TODO\|todo!\|unimplemented!\|stub" crates/hdc/chain/src/

# Check event handler wiring
grep -rn "process_log\|FinalizedReporter\|event_handler" crates/hdc/chain/src/

# Verify what the precompile actually handles
grep -rn "fn execute\|opcode\|dispatch" crates/hdc/chain/src/precompile.rs
```
