# Roko & mirage-rs HDC Implementation Analysis

What exists across the roko and mirage-rs codebases, what works well, what has
consensus-safety issues, and what the spec calls for that neither implements.

> **Audit note (2026-05-08):** Verified against source at
> `uniswap/bardo/roko/`. Several crate names, line counts, file paths, and
> type attributions in the original draft were incorrect. Corrections are
> inline. The `roko-neuro` crate **does not exist** in the repository. Types
> the original draft attributed to `roko-neuro` (KnowledgeHdcEncoder,
> ResonanceDetector, KnowledgeEntry with 23 fields, KnowledgeStore backed by
> JSONL, etc.) do not exist in any roko crate. Some of the described types
> (BundleAccumulator, ItemMemory) actually live in `golem-core/src/hdc.rs`
> in the sibling `bardo/` workspace, not in any roko crate. The
> `roko-learn/src/hdc_fingerprint.rs` file also does not exist. All
> code snippets and line references for the `roko-neuro` and
> `roko-learn/hdc_fingerprint.rs` sections are **fabricated** -- they describe
> plausible but non-existent code.

---

## Codebase Map

### roko crates (verified paths and line counts)

| File | Lines | Key types / functions | Role |
|------|------:|----------------------|------|
| `bardo-primitives/src/hdc.rs` | 311 | `HdcVector`, `from_seed()`, `similarity()`, `bind()`, `bundle()`, `permute()` | Core vector type and algebra |
| `roko-index/src/hdc.rs` | 355 | `HdcFingerprint`, `fingerprint_symbol()`, `fingerprint_file()` | Code fingerprinting via trigrams |
| `roko-learn/src/hdc_clustering.rs` | 442 | `k_medoids()`, `KMedoidsConfig`, `ClusterResult`, `HdcCluster` | K-medoids (PAM) clustering |

> **Audit note:** The original draft listed the following files that **do not
> exist** in the repository:
>
> - `roko-primitives/src/hdc.rs` (718 lines) -- The actual crate is named
>   `bardo-primitives`, contains 311 lines, and does NOT include
>   `BundleAccumulator`, `DecayingBundleAccumulator`, `ItemMemory`,
>   `fingerprint()`, or `text_fingerprint()`. Those types are partly found in
>   `golem-core/src/hdc.rs` (735 lines) in the sibling bardo workspace.
> - `roko-neuro/src/hdc.rs` (689 lines) -- No `roko-neuro` crate exists.
> - `roko-neuro/src/knowledge_store.rs` (4,751 lines) -- Does not exist.
> - `roko-neuro/src/context.rs` (2,979 lines) -- Does not exist.
> - `roko-neuro/src/lib.rs` (1,641 lines) -- Does not exist.
> - `roko-learn/src/hdc_fingerprint.rs` (130 lines) -- Does not exist.
>
> The `KnowledgeHdcEncoder`, `RoleFillerEncoder`, `ResonanceDetector`,
> `KnowledgeEntry` (23-field struct), `KnowledgeStore` (JSONL-backed),
> `ContextAssemblyWeights`, `EmotionalProvenance`, `ValidationArc`,
> `fingerprint_episode()`, and all associated code snippets in subsections
> 2-5 and 8 of "What Exists and Works Well" (plus Issues 4, 5, 8, and 9 in
> "What Needs Fixing") are **fabricated descriptions of non-existent code**.

### mirage-rs apps (verified paths and line counts)

| File | Lines | Key types / functions | Role |
|------|------:|----------------------|------|
| `mirage-rs/src/chain/projection.rs` | 245 | `ProjectionMatrix`, `project_bytes()`, `project_tokens()` | Float-embedding-to-HDC projection |
| `mirage-rs/src/chain/insight.rs` | 453 | `InsightEntry`, `InsightId`, `KnowledgeKind`, `KnowledgeState` (7-state FSM) | On-chain knowledge entry + lifecycle |
| `mirage-rs/src/chain/hdc_index.rs` | 233 | `HdcIndex`, `IndexedVector`, `Hit` | Brute-force flat vector index |
| `mirage-rs/src/chain/hnsw.rs` | 476 | `HnswBinaryIndex`, `HnswConfig` | HNSW binary index |
| `mirage-rs/src/chain/knowledge.rs` | 599 | `KnowledgeStore`, `PostOutcome`, `KnowledgeSnapshot`, `KnowledgeError` | Dual-index knowledge store |
| `mirage-rs/src/chain/pheromone.rs` | 414 | `PheromoneField`, `Pheromone`, `PheromoneKind`, `PheromoneHit` | Stigmergic pheromone field |

**Total (verified):** ~3,528 lines across 9 verified HDC-related files, split
across 3 independent HDC implementations (bardo-primitives, roko-index,
golem-core). The original draft's "~12,762 lines across 14 files" figure
included ~10,190 lines from non-existent `roko-neuro` and
`roko-learn/hdc_fingerprint.rs` files, plus 5-15% inflation on the real
file counts.

---

## What Exists and Works Well

### 1. Core HDC Algebra (bardo-primitives/src/hdc.rs)

**Status: Production-complete. Well-designed.**

> **Audit note:** The original draft called this `roko-primitives/src/hdc.rs`
> at 718 lines. The actual crate is `bardo-primitives` with 311 lines. Line
> references below have been corrected to match the actual file. The types
> `BundleAccumulator`, `DecayingBundleAccumulator`, `ItemMemory`,
> `fingerprint()`, and `text_fingerprint()` do NOT exist in this file. A
> `BundleAccumulator` and `ItemMemory` exist in `golem-core/src/hdc.rs` (735
> lines) in the sibling workspace, with different signatures than described
> below. There is no `DecayingBundleAccumulator` anywhere in the codebase.

The fundamental vector type is a 10,240-bit binary vector stored as 160 `u64` words:

```rust
// bardo-primitives/src/hdc.rs, lines 24-26
pub struct HdcVector {
    bits: [u64; 160],
}
```

Key operations are clean and correct:

```rust
// Bind (XOR) -- lines 107-113
pub fn bind(&self, other: &Self) -> Self {
    let mut bits = [0u64; 160];
    for (slot, (left, right)) in bits.iter_mut().zip(self.bits.iter().zip(other.bits.iter())) {
        *slot = left ^ right;
    }
    Self { bits }
}

// Similarity (Hamming) -- lines 211-218
pub fn similarity(&self, other: &Self) -> f32 {
    let mut differing_bits = 0u32;
    for (left, right) in self.bits.iter().zip(other.bits.iter()) {
        differing_bits += (left ^ right).count_ones();
    }
    let differing_bits = u16::try_from(differing_bits).unwrap_or(u16::MAX);
    1.0_f32 - (f32::from(differing_bits) / 10_240.0_f32)
}

// Bundle (majority vote, tie -> 0) -- lines 117-138
pub fn bundle(vectors: &[&Self]) -> Self {
    // ...
    for bit_index in 0..64 {
        let mut ones = 0usize;
        for vector in vectors {
            ones += ((vector.bits[word_index] >> bit_index) & 1) as usize;
        }
        if ones * 2 > len {       // <-- strict inequality: tie -> 0
            word |= 1u64 << bit_index;
        }
    }
    // ...
}

// Permute (cyclic rotation) -- lines 142-164
pub fn permute(&self, n: usize) -> Self { /* word_shift + bit_shift rotation */ }

// Deterministic seeded generation (FNV-1a + splitmix64) -- lines 193-208
pub fn from_seed(seed: &[u8]) -> Self {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for &byte in seed {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3); // FNV prime
    }
    // ... fill 160 words via splitmix64
}
```

Also includes:

- **Serde support** (lines 28-75) -- Custom `Serialize`/`Deserialize` for `HdcVector`
  using raw 1280-byte representation. Handles both `visit_bytes` and `visit_seq`.

- **rkyv zero-copy support** (lines 225-234, behind `#[cfg(feature = "rkyv")]`) --
  `similarity_archived()` reads directly from mmap'd rkyv buffers with no
  deserialization on little-endian platforms.

> **Audit note:** The following items from the original draft do NOT exist in
> `bardo-primitives/src/hdc.rs`:
>
> - `BundleAccumulator` -- exists in `golem-core/src/hdc.rs` (line 251), but
>   uses `f32` votes (not `i32` as the draft claimed), and the `add_weighted()`
>   signature takes `f32` weight (not `i32`).
> - `DecayingBundleAccumulator` -- does not exist anywhere in the codebase.
> - `ItemMemory` -- exists in `golem-core/src/hdc.rs` (line 160), but provides
>   `role()` and `fillers()` methods. There is no `top_k()` or
>   `insert_seeded()` method as claimed.
> - `fingerprint()` and `text_fingerprint()` -- do not exist anywhere in the
>   codebase.

**Assessment:** The actual `bardo-primitives` code is solid. The algebra is
correct. The API is clean and the implementation uses pure integer operations
(no floating point in the core path except for the final similarity
normalization). The `f32` similarity return type is appropriate for off-chain
use. Tests cover involution, self-similarity, tie-breaking, serde roundtrip, and
seeded determinism.

### 2. Cognitive Encoder (roko-neuro/src/hdc.rs)

> **AUDIT: THIS ENTIRE SECTION IS FABRICATED.** There is no `roko-neuro` crate
> in the repository. The types `KnowledgeHdcEncoder`, `RoleFillerEncoder`,
> `ResonanceDetector`, `ResonancePair`, `encode_generic_entry()`,
> `encode_causal_link()`, `encode_structured()`, `query_role()`, and all code
> snippets in this section describe non-existent code. No file at
> `roko-neuro/src/hdc.rs` exists. No grep across the entire `bardo/roko/`
> workspace returns any of these type names.
>
> The golem-triage crate (`golem-triage/src/hdc_encoder.rs`) has a
> role-filler encoder for **transactions** (not knowledge entries), but it uses
> a completely different API shape (thermometer encoding for ordinal fields,
> `ItemMemory` from golem-core) and is unrelated to the code described here.

### 3. Resonance Detection (roko-neuro/src/hdc.rs)

> **AUDIT: THIS ENTIRE SECTION IS FABRICATED.** `ResonanceDetector` does not
> exist anywhere in the codebase. The 0.526 threshold, the pairwise comparison
> logic, the domain extraction -- none of this code exists. The statistical
> motivation for the threshold (5.26 sigma above chance at D=10,240, using
> the correct SD of similarity = 1/(2*sqrt(D)) ~ 0.00494) is mathematically
> sound as a *design proposal*, but it is not implemented.

### 4. Knowledge Entry Model (roko-neuro/src/lib.rs)

> **AUDIT: THIS ENTIRE SECTION IS FABRICATED.** The 23-field `KnowledgeEntry`
> struct does not exist anywhere in the codebase. No `KnowledgeTier`,
> `EmotionalProvenance`, `ValidationArc`, or `EmotionalTag` types exist.
>
> The actual on-chain knowledge entry is `InsightEntry` in
> `mirage-rs/src/chain/insight.rs` (453 lines), which has 14 fields (not 23):
> `id`, `author`, `kind`, `content`, `vector`, `enabled_by`, `state`,
> `created_at`, `half_life_seconds`, `initial_weight`, `weight`,
> `confirmations`, `challenges`, `stake_wei`.
>
> The `KnowledgeKind` enum does exist in `insight.rs` with 6 variants, but
> the half-lives differ from what was listed:
>
> | Kind | Actual half-life | Draft claimed |
> |------|-----------------|---------------|
> | `Warning` | 3 minutes | 1 hour |
> | `Insight` | 7 days | 30 days |
> | `Heuristic` | 15 days | 90 days |
> | `CausalLink` | 15 days | 60 days |
> | `StrategyFragment` | 15 days | 14 days |
> | `AntiKnowledge` | 15 days | 30 days |
>
> There is no tier system (`Transient`/`Working`/`Consolidated`/`Persistent`)
> in the actual code. The lifecycle is managed via the 7-state FSM
> (`KnowledgeState`) described in the mirage-rs section below.

### 5. Knowledge Store (roko-neuro/src/knowledge_store.rs)

> **AUDIT: THIS ENTIRE SECTION IS FABRICATED.** There is no
> `knowledge_store.rs` file anywhere in the roko crates. The `KnowledgeStore`
> struct, JSONL-backed storage, `read_all()` pattern, `ContextAssemblyWeights`,
> anti-knowledge thresholds, and all code snippets are non-existent.
>
> The actual `KnowledgeStore` lives in `mirage-rs/src/chain/knowledge.rs`
> (599 lines) and uses an in-memory `HashMap<InsightId, InsightEntry>` with
> `HdcIndex` and optional `HnswBinaryIndex` -- described correctly in the
> mirage-rs section below.

### 6. Code Fingerprinting (roko-index/src/hdc.rs)

**Status: Complete. Independent HDC implementation.**

> **Audit note:** Line numbers corrected. Code snippets verified against
> actual source. Content is accurate.

```rust
// roko-index/src/hdc.rs, lines 100-118
pub struct HdcFingerprint {
    bits: [u64; WORDS],   // WORDS=160, same as HdcVector but different type
}

impl HdcFingerprint {
    pub fn similarity(&self, other: &Self) -> f64 {   // Returns f64, not f32
        let dist = hamming_distance(&self.bits, &other.bits);
        1.0 - (f64::from(dist) / TOTAL_BITS as f64)
    }
}
```

Fingerprinting uses `role_vector XOR bundle(name, context)`:

```rust
// roko-index/src/hdc.rs, lines 173-181
pub fn fingerprint_symbol(symbol: &Symbol, context: &[u8]) -> HdcFingerprint {
    let role_vec = role_vector(&symbol.kind);     // Deterministic by SymbolKind
    let name_vec = encode_name(&symbol.name);     // Trigram encoding
    let ctx_vec = vector_from_seed(context);
    let combined = bundle(&[name_vec, ctx_vec]);
    HdcFingerprint { bits: bind(&role_vec, &combined) }
}
```

This is a **second, independent HDC implementation** that duplicates
`splitmix64`, `fnv1a`, `vector_from_seed`, `bundle`, `bind`, and
`hamming_distance` from bardo-primitives. It uses `[u64; 160]` arrays directly
rather than `HdcVector`.

> **Audit note:** The original draft said "third, independent" counting the
> non-existent `roko-neuro` as one. There are actually two independent
> implementations in the roko workspace (`bardo-primitives` and `roko-index`),
> plus a third in the sibling workspace (`golem-core`). mirage-rs depends on
> `bardo-primitives::HdcVector` and adds its own `splitmix64` in `projection.rs`
> and `hnsw.rs`.

### 7. K-Medoids Clustering (roko-learn/src/hdc_clustering.rs)

**Status: Correct. O(n^2) distance precomputation.**

> **Audit note:** Code verified against actual source. Line numbers corrected.
> Actual file is 442 lines (not 498 as originally stated).

```rust
// roko-learn/src/hdc_clustering.rs, lines 81-152
pub fn k_medoids(vectors: &[HdcVector], config: &KMedoidsConfig) -> ClusterResult {
    let dist = precompute_distances(vectors);  // O(n^2) pairwise distances
    let mut medoid_indices = seed_medoids(&dist, n, k);  // Farthest-first seeding
    // ... alternating assign/update until convergence or max_iterations
}

fn precompute_distances(vectors: &[HdcVector]) -> Vec<f32> {
    let n = vectors.len();
    let mut dist = vec![0.0_f32; n * n];  // Full n*n matrix in RAM
    for i in 0..n {
        for j in (i + 1)..n {
            let d = 1.0 - vectors[i].similarity(&vectors[j]);
            dist[i * n + j] = d;
            dist[j * n + i] = d;
        }
    }
    dist
}
```

> **Audit note:** Uses `bardo_primitives::HdcVector` (verified in the imports
> at line 34: `use bardo_primitives::HdcVector;`). The doc example at line 22
> also shows the correct import path.

### 8. Episode Fingerprinting (roko-learn/src/hdc_fingerprint.rs)

> **AUDIT: THIS ENTIRE SECTION IS FABRICATED.** The file
> `roko-learn/src/hdc_fingerprint.rs` does not exist. There is no
> `fingerprint_episode()` function anywhere in the codebase. The struct
> `EpisodeFingerprintInput`, the manual base64 encode/decode, and all code
> snippets are non-existent.

---

## What Needs Fixing

### Issue 1: Floating-Point Non-Determinism in Similarity

**Severity: Consensus-breaking. Must fix before on-chain use.**

bardo-primitives uses `f32`:

```rust
// bardo-primitives/src/hdc.rs, line 211
pub fn similarity(&self, other: &Self) -> f32 {
    // ...
    1.0_f32 - (f32::from(differing_bits) / 10_240.0_f32)
}
```

roko-index uses `f64`:

```rust
// roko-index/src/hdc.rs, line 114
pub fn similarity(&self, other: &Self) -> f64 {
    1.0 - (f64::from(dist) / TOTAL_BITS as f64)
}
```

mirage-rs `InsightEntry::decayed_weight` uses `f32::exp()`:

```rust
// mirage-rs/src/chain/insight.rs, lines 254-262
pub fn decayed_weight(&self, now_secs: u64) -> f32 {
    let tau = self.effective_half_life_seconds() as f32;
    if tau <= 0.0 { return 0.0; }
    let age = self.age_seconds(now_secs) as f32;
    let decay = (-age / tau * std::f32::consts::LN_2).exp();  // <-- non-deterministic
    self.initial_weight * decay
}
```

> **Audit note:** Line numbers corrected. Code verified against actual source.
> The `if tau <= 0.0` guard was omitted from the original snippet.

`f32::exp()` is **not** IEEE-754-deterministic across platforms (x87 vs SSE vs
ARM NEON produce different ULP rounding). This means two validators can compute
different decayed weights and disagree on entry state transitions.

**Fix:** On-chain similarity should return raw Hamming distance as `u32` and let
the consumer normalize. Decay should use fixed-point arithmetic or lookup tables.

### Issue 2: Wall-Clock Time Dependencies

**Severity: Consensus-breaking.**

> **Audit note:** The `roko-neuro` references are fabricated. The actual
> wall-clock concern applies only to mirage-rs.

mirage-rs uses `u64` unix timestamps and relies on external `now_secs`
parameters:

```rust
// mirage-rs/src/chain/insight.rs, line 194
pub created_at: u64,
```

All mirage-rs decay/refresh functions (`decayed_weight`, `refresh_weight`,
`apply_decay`, `evaporate`, `current_intensity`) take `now_secs: u64` as a
parameter. The caller provides this value, which means different callers
(validators) could disagree if they use their own wall clocks.

**Fix:** All on-chain time references must use block numbers (monotonic,
deterministic). Off-chain can continue using wall-clock but the types must be
separate to prevent accidental mixing.

### Issue 3: golem-core BundleAccumulator f32 Non-Determinism

**Severity: Consensus-fragile.**

The `bardo-primitives::HdcVector::bundle()` function itself is deterministic --
ties go to 0 via strict inequality (line 131: `ones * 2 > len`). This is
well-defined and not a consensus concern:

```rust
// bardo-primitives/src/hdc.rs, line 131
if ones * 2 > len {  // strict inequality: 50% ones -> bit stays 0
    word |= 1u64 << bit_index;
}
```

The actual concern is the `golem-core::BundleAccumulator`, which uses `f32`
votes and a decay method that introduces non-determinism:

> **Audit note:** The original draft described a `BundleAccumulator::finish()`
> in `roko-primitives` with `i32` votes and a `DecayingBundleAccumulator` with
> `f32` votes. Neither exists in `bardo-primitives`. A `BundleAccumulator`
> does exist in `golem-core/src/hdc.rs` (line 251) with `f32` votes and a
> `snapshot()` method that uses `votes[i] > threshold` where
> `threshold = total_weight / 2.0`. There is no `DecayingBundleAccumulator`
> anywhere. The `golem-core::BundleAccumulator::decay(factor: f32)` method
> applies multiplicative decay to existing `f32` votes, which has the same
> non-determinism concern described in the original.

The `golem-core::BundleAccumulator` uses `f32` votes with a decay method:

```rust
// golem-core/src/hdc.rs, lines 306-311
pub fn decay(&mut self, factor: f32) {
    for v in &mut self.votes {
        *v *= factor;
    }
    self.total_weight *= factor;
}
```

The `f32` decay path means accumulated votes can have non-deterministic rounding
across platforms when `decay_factor` is applied repeatedly. A vote that should
be exactly zero might be epsilon-positive on one platform and epsilon-negative
on another, flipping a bit.

**Fix:** For on-chain use, either use fixed-point `i32` decay with explicit
truncation rules, or document that `BundleAccumulator` is off-chain only.

### Issue 4: O(n^2) Resonance Detection

> **AUDIT: THIS ISSUE REFERENCES NON-EXISTENT CODE.** The `ResonanceDetector`
> with O(n^2) pairwise comparison does not exist in any crate. The code snippet
> and line references are fabricated. The k-medoids clustering in
> `roko-learn/src/hdc_clustering.rs` does have O(n^2) distance precomputation,
> which is a legitimate scaling concern, but that is a different feature.

**Fix (if resonance detection is implemented in the future):** Use the HNSW
index for approximate search, or compute resonances incrementally (only compare
new entries against existing ones).

### Issue 5: read_all() on Every Query

> **AUDIT: THIS ISSUE REFERENCES NON-EXISTENT CODE.** There is no
> `knowledge_store.rs` in any roko crate. There is no JSONL-backed
> `KnowledgeStore`, no `read_all()` method, no `query_hits_filtered()`. The
> entire description of a file-backed knowledge store is fabricated.
>
> The actual mirage-rs `KnowledgeStore` (knowledge.rs line 56) already uses an
> in-memory `HashMap<InsightId, InsightEntry>`, which is the correct pattern.
> This is not a bug to fix -- the in-memory approach is already in place.

### Issue 6: O(N log N) top_k Instead of O(N log k)

**Severity: Performance. Minor but fixable.**

> **Audit note:** The `roko-primitives` snippet (`ItemMemory::top_k`) is
> fabricated -- `bardo-primitives` has no `ItemMemory` or `top_k`. The
> mirage-rs `HdcIndex::top_k` snippet is verified and accurate.

`HdcIndex::top_k` (and `PheromoneField::query_top_k`) collect all scores, sort
the entire list, then truncate:

```rust
// mirage-rs/src/chain/hdc_index.rs, lines 107-128
// WARNING: CONSENSUS-UNSAFE — partial_cmp lacks tiebreaker key (equal-score
// entries retain HashMap iteration order, which is non-deterministic), and
// uses f32 scores. On-chain: use u32 Hamming + total_cmp + composite key.
pub fn top_k(&self, query: &HdcVector, k: usize) -> Vec<Hit> {
    let mut hits: Vec<Hit> = self.entries.iter()
        .filter(|e| e.weight > 0.0)
        .map(|e| { /* compute scores */ })
        .collect();
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(k);
    hits
}
```

**Fix:** Use a bounded `BinaryHeap` of size `k` for O(N log k) selection. For
k=10 and N=100K, this is ~3x faster.

### Issue 7: Multiple Separate HDC Implementations

**Severity: Architecture. Maintenance burden and divergence risk.**

> **Audit note:** Corrected crate names and count.

The codebase contains three independent implementations of the same HDC
primitives (across two workspaces):

1. **bardo-primitives/src/hdc.rs** (311 lines) -- `HdcVector` struct with full
   algebra, `similarity() -> f32`, `splitmix64()`, `from_seed()` via FNV-1a.

2. **roko-index/src/hdc.rs** (355 lines) -- `HdcFingerprint` struct with
   `[u64; 160]` arrays, `similarity() -> f64`, its own `splitmix64()`, its own
   `fnv1a()`, its own `vector_from_seed()`, `bundle()`, `bind()`,
   `hamming_distance()`. Does NOT use `HdcVector` from bardo-primitives.

3. **golem-core/src/hdc.rs** (735 lines, sibling workspace) -- Its own
   `HdcVector` struct with `xorshift64` (not splitmix64), its own `bind()`,
   `bundle()`, `permute()`, `similarity()`. Also provides `BundleAccumulator`
   and `ItemMemory` not present in bardo-primitives.

Additionally, mirage-rs uses `HdcVector` from bardo-primitives but duplicates
`splitmix64()` in both `projection.rs` (line 51) and `hnsw.rs` (line 76).

The roko-index implementation uses a different similarity return type (`f64`
vs `f32`), which could produce subtly different results at the precision
boundary. The golem-core implementation uses a completely different PRNG
(xorshift64 vs splitmix64), meaning `golem_core::HdcVector::deterministic()`
and `bardo_primitives::HdcVector::from_seed()` produce different vectors for
the same input.

**Fix:** Consolidate all HDC operations onto `bardo_primitives::HdcVector`. Make
`HdcFingerprint` a newtype over `HdcVector`. Extract `splitmix64()` into a
shared utility.

### Issue 8: hdc_vector Stored as Option<Vec<u8>>

> **AUDIT: THIS ISSUE REFERENCES NON-EXISTENT CODE.** The `roko-neuro/src/lib.rs`
> file and its 23-field `KnowledgeEntry` with `pub hdc_vector: Option<Vec<u8>>`
> do not exist. The actual mirage-rs `InsightEntry` already stores `vector: HdcVector`
> as a typed, always-present field. This issue is already solved in the actual code.

### Issue 9: 23-Field KnowledgeEntry

> **AUDIT: THIS ISSUE REFERENCES NON-EXISTENT CODE.** The 23-field
> `KnowledgeEntry` struct does not exist. The actual `InsightEntry` in
> mirage-rs has 14 well-organized fields and does not have the decomposition
> problem described here.

### Issue 10: HNSW Consensus Issues (mirage-rs)

**Severity: Consensus-breaking in HNSW path.**

> **Audit note:** Verified against actual source. This issue is real and
> correctly described. Line numbers are slightly off; corrected below.

**No deletion support.** `HnswBinaryIndex` has no `remove()` method. When entries
are pruned via `apply_decay()` in the knowledge store (knowledge.rs line 262),
they are removed from `HdcIndex` but remain as ghost nodes in the HNSW graph:

```rust
// mirage-rs/src/chain/knowledge.rs, lines 261-263
for id in to_prune {
    self.hdc.remove(id);
    // Note: no hnsw.remove(id) -- HNSW has no remove method
}
```

**Insertion order dependency.** The HNSW graph topology depends on insertion order
because `random_level()` consumes RNG state sequentially (hnsw.rs line 178):

```rust
// mirage-rs/src/chain/hnsw.rs, lines 178-186
// WARNING: CONSENSUS-BREAKING — f64::ln() is a transcendental function
// not covered by IEEE 754 bit-exactness guarantees. Different libm
// implementations (glibc, musl, macOS) may return different results.
// Also: sequential RNG consumption makes graph topology insertion-order-
// dependent. FIX: Use integer-only level assignment via leading_zeros
// (see doc 06 deterministic_level function).
fn random_level(&mut self) -> usize {
    let raw = splitmix64(&mut self.rng_state);   // Sequential RNG consumption
    let mantissa = (raw >> 11) as f64;
    let uniform = mantissa / (1u64 << 53) as f64;
    let u = uniform.max(f64::MIN_POSITIVE);
    let m_l = 1.0 / (self.config.m as f64).ln();
    let level = (-u.ln() * m_l).floor() as usize;   // f64::ln() non-deterministic
    level.min(16)
}
```

Two issues: (1) `f64::ln()` is platform-dependent for the same reason as `exp()`,
and (2) reordering insertions changes which nodes get which layers, producing
different graph topologies and different search results.

**swap_remove changes iteration order.** `HdcIndex::remove()` uses `swap_remove`:

```rust
// mirage-rs/src/chain/hdc_index.rs, lines 93-99
pub fn remove(&mut self, id: InsightId) -> bool {
    if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
        self.entries.swap_remove(pos);   // Changes order of remaining entries
        true
    } else { false }
}
```

This changes the position of the last element, which affects tie-breaking in
`top_k` (ties broken by insertion order, per the doc comment at line 105).

### Issue 11: HNSW Threshold Effectively Disabled

**Severity: Functionality. Dead code.**

The default `KnowledgeStore::new()` sets `hnsw_threshold: usize::MAX`:

```rust
// mirage-rs/src/chain/knowledge.rs, lines 91-98
pub fn new() -> Self {
    Self {
        entries: HashMap::new(),
        hdc: HdcIndex::new(),
        hnsw: None,
        hnsw_threshold: usize::MAX,   // Never triggers
    }
}
```

The HNSW index is only used when `with_hnsw()` is called explicitly. No code
in the codebase calls `with_hnsw()` except tests. The auto-switchover is
effectively dead code.

### Issue 12: HashMap Iteration Order Non-Determinism

**Severity: Consensus-fragile.**

The mirage-rs `KnowledgeStore` (knowledge.rs line 57), `PheromoneField`
(pheromone.rs line 119), and HNSW `id_to_node` (hnsw.rs line 68) all use
`HashMap`, whose iteration order is non-deterministic:

```rust
// mirage-rs/src/chain/knowledge.rs, line 57
pub struct KnowledgeStore {
    entries: HashMap<InsightId, InsightEntry>,
    // ...
}

// mirage-rs/src/chain/pheromone.rs, line 119
pub struct PheromoneField {
    pheromones: HashMap<PheromoneId, Pheromone>,
    // ...
}
```

`apply_decay()` iterates `entries.values_mut()` (knowledge.rs line 248), which
produces different orderings across runs. If decay produces borderline prune
decisions that depend on which entries are processed first (via shared mutable
state), results diverge.

Similarly, `PheromoneField::evaporate()` (pheromone.rs line 233) uses
`self.pheromones.retain()` which iterates in HashMap order.

**Fix:** Use `BTreeMap` or sort before processing for deterministic iteration.

---

## mirage-rs Detailed Analysis

### ProjectionMatrix (projection.rs)

**Architecture:** A seeded `{0,1}^(HDC_BITS x ceil(input_dim/64))` bit-packed
projection matrix. For bge-small-en-v1.5 (1536D input), this is:
- 10,240 rows x 24 u64 words = 245,760 words = **~1.9 MB**

```rust
// mirage-rs/src/chain/projection.rs, lines 40-49
pub struct ProjectionMatrix {
    pub input_dim: usize,     // e.g. 1536
    pub output_dim: usize,    // Always 10,240
    rows: Vec<Vec<u64>>,      // rows[row_idx] = Vec<u64> of length ceil(input_dim/64)
    pub seed: u64,
}
```

Three projection modes:
1. **`project_floats()`** (line 89) -- Sign-projection from float embeddings.
   For each output bit j: `bit_j = sign(sum_i matrix[j,i] ? +input[i] : -input[i])`.
   Uses `f32` accumulation. Tie-breaking: `sum > 0.0` (zero goes to 0).

2. **`project_bytes()`** (line 131) -- Direct FNV-1a seeding via `HdcVector::from_seed()`.
   Bypasses embeddings entirely. Exact-match semantics only.

3. **`project_tokens()`** (line 146) -- Whitespace-split bag-of-tokens bundled via
   majority vote. Crude semantic fingerprint without an embedding model.

**Consensus concern:** `project_floats()` accumulates `f32` sums across 1536
dimensions. The sum `sum += if bit { x } else { -x }` involves 1536 additions,
and IEEE-754 addition is not associative. However, the compiler will typically
emit sequential additions in source order, and the matrix is fixed, so in
practice this is deterministic for identical binaries. Cross-platform
determinism is not guaranteed.

### InsightEntry (insight.rs)

**Architecture:** 7-state FSM lifecycle for on-chain knowledge entries:

```
Created --> Active --> Confirmed --> Decaying --> Pruned
                  \-> Challenged --> Active/Confirmed/Pruned
                                \-> Decaying --> Pruned
Created/Active/Confirmed/Decaying/Challenged --> Stale (5x half-life)
```

```rust
// mirage-rs/src/chain/insight.rs, lines 134-170
pub enum KnowledgeState {
    Created,     // Just posted, not yet indexed
    Active,      // Indexed and searchable; weight >= 0.25 * initial
    Confirmed,   // >= 3 confirmations
    Decaying,    // Weight below 0.25 * initial but still searchable
    Challenged,  // Open challenge contesting this entry
    Pruned,      // Challenge resolved against or weight < 0.01 * initial
    Stale,       // Aged out (>= 5 * half_life elapsed)
}
```

Terminal states (`Pruned`, `Stale`) are sinks -- no outbound transitions.
State transitions are validated via `can_transition_to()` (line 160).

Confirmation extends effective half-life (line 270):

```rust
// mirage-rs/src/chain/insight.rs, lines 270-275
pub fn effective_half_life_seconds(&self) -> u64 {
    let base = self.half_life_seconds as f32;
    let confirms = self.confirmations.len() as f32;
    let extension = confirms.sqrt() * 0.5 * base;
    (base + extension).max(0.0) as u64
}
```

Formula: `tau_eff = tau_base * (1 + sqrt(confirmations) * 0.5)`.
At 4 confirmations: `tau_eff = tau_base * (1 + 2 * 0.5) = 2 * tau_base`.

> **Audit note:** This formula differs from the pheromone formula (see below),
> which uses `tau_eff = tau_base * (1 + sqrt(confirmations))` -- i.e. the
> pheromone version omits the `* 0.5` factor, making confirmations 2x more
> impactful on pheromone lifetime than on insight lifetime. This inconsistency
> is not flagged anywhere in the codebase.

### HdcIndex (hdc_index.rs)

**Architecture:** Flat `Vec<IndexedVector>` with brute-force linear scan.

```rust
// mirage-rs/src/chain/hdc_index.rs, lines 46-49
pub struct HdcIndex {
    entries: Vec<IndexedVector>,  // Not HashMap -- allows swap_remove
}
```

- `insert()` scans for existing ID (O(n) linear find)
- `remove()` uses `swap_remove()` -- O(n) find + O(1) removal but changes order
- `top_k()` computes `similarity * weight` for all entries, sorts, truncates
- `set_weight()` does O(n) linear find

Adequate for <100K vectors. At 10K entries, a full scan takes ~2ms.

### HnswBinaryIndex (hnsw.rs)

**Architecture:** Standard HNSW with binary Hamming distance.

Configuration:

```rust
// mirage-rs/src/chain/hnsw.rs, lines 31-40
pub struct HnswConfig {
    pub m: usize,               // Max neighbours per node (default: 16)
    pub m_max_0: usize,         // Max neighbours at layer 0 (default: 32 = 2*M)
    pub ef_construction: usize, // Build-time beam width (default: 200)
    pub seed: u64,              // Deterministic seed (default: 0xC0FF_EE_C0FF_EE)
}
```

Layer assignment uses splitmix64 (line 178) with `level = floor(-ln(U) * m_L)`
where `m_L = 1/ln(M)`. Hard ceiling at layer 16.

Insert algorithm (line 191):
1. Phase 1: Greedy descent from max_level to node_level+1
2. Phase 2: Beam search of size ef_construction at layers [0..node_level]
3. Bidirectional links with shrink step

Search algorithm (line 254):
1. Greedy descent from max_level to layer 1
2. Beam search on layer 0 with `ef = max(k*4, 40, min(ef_construction, 64))`
3. Sort by `similarity * weight`, truncate to k

The beam search (line 321) uses a negated-distance max-heap as a min-heap for
the frontier, and a `Nearest` wrapper for the top-k candidates.

### KnowledgeStore (knowledge.rs)

**Architecture:** Dual-index store with HDC + optional HNSW.

```rust
// mirage-rs/src/chain/knowledge.rs, lines 56-62
pub struct KnowledgeStore {
    entries: HashMap<InsightId, InsightEntry>,
    hdc: HdcIndex,                    // Always present
    hnsw: Option<HnswBinaryIndex>,    // Only with with_hnsw()
    hnsw_threshold: usize,            // Default: usize::MAX (disabled)
}
```

Key operations:
- **`post()`** (line 126) -- Content-addressed dedup + HDC duplicate detection
  (>95% similarity = duplicate). Returns `PostOutcome::Accepted`, `Duplicate`,
  or `ExactMatch`.
- **`search()`** (line 272) -- Routes to HNSW above threshold, brute-force below.
- **`apply_decay()`** (line 246) -- Walks all entries, refreshes weights, prunes
  those below 1% of initial.
- **`confirm()`** / **`challenge()`** -- Updates entry state and syncs index weights.

### PheromoneField (pheromone.rs)

**Architecture:** Time-decaying signals with 3 kinds and bucketed decay.

Three kinds with different half-lives:

| Kind | Default Half-life | Use case |
|------|------------------|----------|
| `Threat` | 2 hours | "this contract just rugged" |
| `Opportunity` | 4 hours | "fresh arb window in pool X" |
| `Wisdom` | 24 hours | "this trick generalises across L2 bridges" |

Decay formula (pheromone.rs line 103):

```rust
// mirage-rs/src/chain/pheromone.rs, lines 103-110
// WARNING: CONSENSUS-UNSAFE — f32::exp() is platform-dependent (different
// libm implementations produce different results). If used on-chain, replace
// with fixed_point_decay() using integer arithmetic. See doc 09.
pub fn current_intensity(&self, now_secs: u64) -> f32 {
    let tau = self.effective_half_life_seconds() as f32;
    if tau <= 0.0 { return 0.0; }
    let elapsed = now_secs.saturating_sub(self.deposited_at) as f32;
    self.base_intensity * (-elapsed / tau * std::f32::consts::LN_2).exp()
}
```

Bucketed decay optimization (16 buckets, line 211): computes one `exp()` per
`(bucket, kind, confirmations)` combination instead of per-pheromone. However,
the bucket assignment is just `now_secs % 16` (line 182), which is simplistic
and means pheromones deposited 16 seconds apart land in the same bucket despite
potentially having very different ages.

Confirmation extends tau: `tau_eff = tau_base * (1 + sqrt(confirmations))`.

`query_top_k()` (line 245) does brute-force scan with `similarity * intensity`
ranking. Same O(N log N) sort-and-truncate pattern as the HDC indices.

**Consensus concern:** `f32::exp()` in `current_intensity()` is non-deterministic.
`HashMap` iteration in `evaporate()` and `query_top_k()` is non-deterministic.
The `sort_by(|a, b| b.score.total_cmp(&a.score))` is stable (Rust's `sort_by`
is a stable sort), but entries with equal scores will retain their HashMap
iteration order, which is non-deterministic across runs.

---

## Gap Analysis: Spec vs Implementation

What the target architecture (docs 04-09 in this series) calls for that
**neither** roko nor mirage-rs implements:

### 1. No Tiered Search Pipeline

The spec describes a three-tier search:

```
Tier 1: First-word filter (~100 gas) -- eliminates ~90% of candidates
Tier 2: Approximate search (16/160 compressed words) -- narrows to ~1%
Tier 3: Exact Hamming search        -- final ranking
```

**Current state:** Both bardo-primitives and mirage-rs do full 160-word brute-force
comparison on every query. The HNSW index provides approximate search but there
is no first-word pre-filter and no compressed-word approximate tier.

### 2. No VCG Attention Auction for Context Assembly

The spec describes a Vickrey-Clarke-Groves (VCG) mechanism for allocating context
budget across knowledge entries.

> **Audit note:** The code snippet below was from the non-existent
> `roko-neuro/src/knowledge_store.rs`. No `ContextAssemblyWeights` struct
> exists in the actual codebase. The mirage-rs `KnowledgeStore::search()`
> ranks by `similarity * weight` only -- there is no keyword relevance,
> predictive foraging, or freshness component.

**Current state:** mirage-rs uses a simple `similarity * weight` ranking.
There is no multi-signal scoring, no truthful bidding, no second-price
payment, and no budget constraint enforcement.

### 3. No Dream Cycle / Offline Consolidation

The spec describes a "dream cycle" where the agent periodically:
- Replays episodes using Mattar-Daw replay scoring
- Performs NREM-like consolidation (re-encode with fresh projections)
- Performs REM-like creativity (random cross-binding for novel associations)

**Current state:** There is no background consolidation process.

> **Audit note:** The tier promotion snippet below was from the non-existent
> `roko-neuro/src/knowledge_store.rs`. There is no `KnowledgeTier` enum and
> no tier promotion logic in the actual codebase. The mirage-rs knowledge
> lifecycle uses a 7-state FSM (`KnowledgeState`) with confirmation-driven
> promotion to `Confirmed` (>= 3 confirmations), but this is not the same
> as the dream-cycle consolidation described in the spec.

### 4. No Structural Anti-Knowledge

The spec calls for anti-knowledge to live in a distinct subspace that cannot be
confused with regular knowledge during retrieval.

> **Audit note:** The `roko-neuro` references and anti-knowledge thresholds
> are fabricated. There is no `roko-neuro/src/lib.rs` or
> `knowledge_store.rs`.

**Current state in mirage-rs:** Anti-knowledge is just another kind variant:

```rust
// mirage-rs/src/chain/insight.rs, line 80
AntiKnowledge,   // Just another KnowledgeKind variant
```

The `KnowledgeStore::find_hdc_duplicate()` checks for HDC-similar entries of
the same `KnowledgeKind` at > 95% similarity to prevent duplicates, but this
is deduplication, not structural anti-knowledge separation. All kinds (including
`AntiKnowledge`) live in the same vector space and the same index. At retrieval
time, there is no mechanism to prevent anti-knowledge from being returned
alongside (and potentially conflicting with) regular knowledge.

### 5. No Verification Cells

The spec defines four verification approaches for on-chain HDC:
- ZK verification (prove search result without revealing index)
- Optimistic verification (challenge window)
- TEE verification (hardware attestation)
- Binius verification (binary field proofs)

**Current state:** None implemented. The mirage-rs challenge mechanism
(insight.rs line 302) provides basic dispute resolution but not cryptographic
verification of search results.

### 6. No Pheromone SINR Interference Model

The spec describes a Signal-to-Interference-plus-Noise Ratio (SINR) model where
overlapping pheromones attenuate each other based on spectral distance.

**Current state:** Pheromones are independent. No interference, no spectral
distance, no attenuation between competing signals. The `query_top_k()` just
ranks by `similarity * intensity` without considering signal interference.

### 7. No Resonator Networks

The spec describes iterative resonator networks for factorizing complex bound
structures (capacity from ~sqrt(D) to ~N^2 bound pairs).

**Current state:** Only simple threshold comparison. No iterative factorization.

### 8. Half-Life Values Diverge from Target Spec

The mirage-rs `KnowledgeKind` half-lives (see the audit note under "4.
Knowledge Entry Model" above) differ
significantly from the target values specified in doc 09, which defines
kind-specific base half-lives (Insight: 72h, Heuristic: 168h, CausalLink: 120h,
Warning: 48h, StrategyFragment: 96h, AntiKnowledge: 336h) with per-tier
multipliers (Transient 0.1x through Persistent 5.0x). The current mirage-rs
code uses flat per-kind values (Warning: 3 min, Insight: 7 days, others: 15
days) with no tier system. Aligning these will require introducing the tier
model from doc 09.

---

## Recommended Fix Priority

Ordered by impact/effort ratio. Consensus-breaking issues first.

### P0: Consensus-Critical (Must fix before on-chain)

**1. Replace f32/f64 similarity with integer Hamming distance for on-chain path**

Impact: Consensus-breaking. Effort: Low.

```rust
// Add to bardo-primitives/src/hdc.rs
pub fn hamming_distance(&self, other: &Self) -> u32 {
    let mut diff = 0u32;
    for (left, right) in self.bits.iter().zip(other.bits.iter()) {
        diff += (left ^ right).count_ones();
    }
    diff
}
```

Consumers normalize: `similarity = 1.0 - (hamming as f32 / 10_240.0)`.
On-chain code uses raw `u32` for comparisons.

**2. Replace f32::exp() decay with fixed-point or lookup table**

Impact: Consensus-breaking. Effort: Medium.

```rust
// Fixed-point decay: weight = initial_weight >> (age / half_life)
// For fractional half-lives, use pre-computed 2^(-k/N) lookup table
// with N=16 or N=32 sub-half-life steps
pub fn decayed_weight_fixed(initial: u32, age_blocks: u64, half_life_blocks: u64) -> u32 {
    if half_life_blocks == 0 { return 0; }
    let full_halves = age_blocks / half_life_blocks;
    if full_halves >= 32 { return 0; }
    initial >> full_halves as u32  // Exact powers of two
}
```

**3. Replace wall-clock time with block numbers on-chain**

Impact: Consensus-breaking. Effort: Medium.

Add `created_at_block: u64` and `half_life_blocks: u64` fields.

> **Audit note:** The original claimed `BLOCKS_PER_DAY = 43_200` is defined
> in `roko-neuro/src/lib.rs`. That file does not exist. No such constant
> exists in the codebase.

**4. Use BTreeMap instead of HashMap for deterministic iteration**

Impact: Consensus-fragile. Effort: Low.

Replace `HashMap<InsightId, InsightEntry>` with `BTreeMap<InsightId, InsightEntry>`
in mirage-rs knowledge.rs and `HashMap<PheromoneId, Pheromone>` with
`BTreeMap<PheromoneId, Pheromone>` in pheromone.rs.

### P1: Architecture (Should fix for maintainability)

**5. Consolidate HDC implementations**

Impact: Maintenance burden. Effort: Medium.

- Make `HdcFingerprint` a newtype over `HdcVector`
- Remove duplicate `splitmix64()`, `fnv1a()`, `bundle()`, `bind()` from roko-index
- Use `bardo_primitives::HdcVector` everywhere
- Extract shared `splitmix64()` into a `bardo-primitives::util` module
- Reconcile `golem_core::HdcVector` (xorshift64) with `bardo_primitives::HdcVector`
  (splitmix64) -- these are currently incompatible PRNGs

**6. ~~Change hdc_vector from Option<Vec<u8>> to Option<HdcVector>~~**

> **Audit note:** This item references the non-existent `roko-neuro`
> `KnowledgeEntry`. The actual mirage-rs `InsightEntry` already stores
> `vector: HdcVector` (typed, non-optional). This fix is already done.

**7. ~~Add in-memory index to roko-neuro KnowledgeStore~~**

> **Audit note:** This item references the non-existent JSONL-backed
> `roko-neuro` `KnowledgeStore`. The actual mirage-rs `KnowledgeStore` already
> uses an in-memory `HashMap` + `HdcIndex` + optional `HnswBinaryIndex`. This
> fix is already done.

**8. Add HNSW deletion support**

Impact: Ghost nodes degrade search quality over time. Effort: Medium.

Options:
- Tombstone marking + periodic rebuild
- Lazy deletion with backfill of orphaned neighbors
- Full rebuild when tombstone count exceeds threshold (e.g., 10%)

### P2: Performance (Should fix for scale)

**9. Replace O(N log N) top_k with O(N log k) bounded heap**

Impact: ~3x speedup for typical k=10 queries at large N. Effort: Low.

```rust
use std::collections::BinaryHeap;
use std::cmp::Reverse;

pub fn top_k(&self, query: &HdcVector, k: usize) -> Vec<Hit> {
    let mut heap: BinaryHeap<Reverse<(OrderedFloat<f32>, usize)>> = BinaryHeap::new();
    for (idx, entry) in self.entries.iter().enumerate() {
        let score = query.similarity(&entry.vector) * entry.weight;
        if heap.len() < k {
            heap.push(Reverse((OrderedFloat(score), idx)));
        } else if let Some(&Reverse((OrderedFloat(min_score), _))) = heap.peek() {
            if score > min_score {
                heap.pop();
                heap.push(Reverse((OrderedFloat(score), idx)));
            }
        }
    }
    // Extract and sort
}
```

**10. ~~Make resonance detection incremental~~**

> **Audit note:** Resonance detection does not exist in the codebase (see
> Issue 4). This is a future-work recommendation for when it is implemented,
> not a fix to existing code.

**11. Add first-word pre-screening tier**

Impact: ~10x speedup for large indices. Effort: Medium.

Compare only the first u64 word of each vector as a cheap pre-filter (as
specified in the tiered search pipeline of doc 09). Vectors whose first-word
distance exceeds a lenient threshold can be safely rejected with high
probability, eliminating ~90% of candidates before full Hamming comparison.

### P3: Functionality (Future work)

**12. Structural anti-knowledge via subspace separation**

**13. Tiered search pipeline (first-word -> sample-word -> exact)**

**14. Dream cycle / offline consolidation process**

**15. SINR pheromone interference model**

**16. VCG attention auction for context assembly**

**17. Verification cells (start with optimistic verification)**

---

## Audit Summary (2026-05-08)

### Issues Found Not Mentioned in Original Draft

1. **Inconsistent confirmation extension formulas.** `InsightEntry` uses
   `tau_eff = tau_base * (1 + sqrt(confirms) * 0.5)` while `Pheromone` uses
   `tau_eff = tau_base * (1 + sqrt(confirms))`. The pheromone formula gives
   confirmations 2x more impact on lifetime. This should be intentional and
   documented, or unified.

2. **golem-core is a third independent HDC implementation.** The `golem-core/src/hdc.rs`
   (735 lines) has its own `HdcVector` type using xorshift64 (not splitmix64),
   `BundleAccumulator`, and `ItemMemory`. This is completely incompatible with
   `bardo_primitives::HdcVector` despite having the same struct name and
   dimensionality. Vectors generated by one will not match vectors generated by
   the other for the same input seed.

3. **golem-triage role-filler encoder not analyzed.** The
   `golem-triage/src/hdc_encoder.rs` has a real, working role-filler encoder
   for transactions (using thermometer encoding for ordinals), but it was not
   mentioned in the original draft. This is the closest existing code to the
   `KnowledgeHdcEncoder` described in the fabricated subsection 2 ("Cognitive
   Encoder") above.

### Credibility Assessment

Of the 14 files listed in the original draft's Codebase Map:

- **9 files exist** and are largely correctly described (the mirage-rs files
  plus roko-index and roko-learn clustering)
- **5 files are fabricated** (all 4 roko-neuro files + hdc_fingerprint.rs)
- Line counts for existing files were inflated by 5-15%
- The crate name `roko-primitives` is wrong (actual: `bardo-primitives`)
- ~10,190 lines of supposed code (roko-neuro + hdc_fingerprint) do not exist

Subsections 2, 3, 4, 5, and 8 under "What Exists and Works Well" describe
plausible but entirely non-existent code. Issues 4, 5, 8, and 9 in the "What
Needs Fixing" section reference this non-existent code. Fix priorities 6, 7,
and 10 also reference it.

The mirage-rs detailed analysis (ProjectionMatrix through PheromoneField) is
accurate and well-done. Issues 1, 2, 3, 6, 7, 10, 11, and 12 are real,
correctly identified, and properly prioritized.
The gap analysis (items 1-8 under "Gap Analysis: Spec vs Implementation") is
sound, though the VCG context assembly gap referenced fabricated code for the
"current state" comparison.
