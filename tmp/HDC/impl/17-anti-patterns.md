# 17 -- Anti-Patterns: Things NOT to Do

> **Status: REFERENCE MATERIAL -- still current.**
> All anti-patterns documented below remain valid. Section 9 ("Codebase
> Anti-Patterns Found") was added 2026-05-08 with concrete instances
> discovered during the cross-cutting audit (doc 20) and PR #42 review.

> **Read this BEFORE starting implementation.** Every anti-pattern here has
> been encountered during development. Each one causes either a consensus
> divergence (validators disagree), a silent correctness bug, or an
> architectural violation that creates painful rework. The examples use the
> actual types and constants from the daeji/kora codebase.

---

## 1. Consensus Safety Anti-Patterns

These are the most dangerous class. A consensus safety violation means two
honest validators running the same code on the same inputs produce different
results. The chain forks. Recovery requires manual intervention.

### 1.1 NEVER use f32/f64 in on-chain code paths

IEEE 754 floating-point permits different rounding modes, intermediate
precision, and fused multiply-add (FMA) behavior across platforms. An x86
validator and an ARM validator can compute the same float expression and get
different results at the last ULP. This is enough to flip a threshold check.

**WRONG:**
```rust
// On-chain similarity check -- uses f64 division
fn is_relevant(a: &HdcVector, b: &HdcVector) -> bool {
    let sim = 1.0 - hamming_distance(a, b) as f64 / 10240.0;
    sim > 0.526  // Floating-point comparison: may differ across platforms
}
```

**RIGHT:**
```rust
use kora_hdc::THRESHOLD_HAMMING;

// On-chain similarity check -- pure integer comparison
fn is_relevant(a: &HdcVector, b: &HdcVector) -> bool {
    hamming_distance(a, b) < THRESHOLD_HAMMING  // 4,854 -- integer, bit-exact everywhere
}
```

**Why:** On x86 with FMA instructions enabled, `hamming_distance as f64 /
10240.0` may produce a slightly different result than on ARM. If the true
similarity is exactly 0.526, one validator says "yes" and the other says "no".
The chain forks. Integer comparison is identical on every platform, every
optimization level, every CPU.

Use basis points (u64, where 10000 = 100%) for any on-chain percentage
calculations. Use integer Hamming distance thresholds for all on-chain
similarity checks.

---

### 1.2 NEVER use HashMap for iteration

`HashMap` uses a randomized hash seed (SipHash with per-process randomization
by default in Rust). Iteration order is non-deterministic across runs, across
processes, and across platforms.

**WRONG:**
```rust
use std::collections::HashMap;

fn compute_bundle(entries: &HashMap<String, HdcVector>) -> HdcVector {
    let mut acc = BundleAccumulator::new();
    for (_name, vec) in entries {  // Iteration order varies per run!
        acc.add(vec);
    }
    acc.to_vector()
}
```

**RIGHT:**
```rust
use std::collections::BTreeMap;

fn compute_bundle(entries: &BTreeMap<String, HdcVector>) -> HdcVector {
    let mut acc = BundleAccumulator::new();
    for (_name, vec) in entries {  // Sorted by key -- deterministic
        acc.add(vec);
    }
    acc.to_vector()
}
```

**Why:** Although majority-vote bundling is mathematically commutative, edge
cases involving ties and even-count bundles can produce order-dependent
results if the accumulator is finalized mid-stream or if the tie-breaking
logic has any sensitivity to insertion order. Using `BTreeMap` eliminates
this entire class of bugs. The performance difference is negligible for the
collection sizes in this system.

---

### 1.3 NEVER use `sort_unstable`

Rust's `sort_unstable` does not preserve the relative order of equal
elements. When two candidates have the same distance score, their ordering
becomes platform-dependent (the underlying algorithm may make different
choices on different runs). This is a consensus violation.

**WRONG:**
```rust
fn top_k(candidates: &mut [(u32, usize)], k: usize) -> &[(u32, usize)] {
    // Unstable sort: equal-distance elements may appear in any order
    candidates.sort_unstable_by_key(|&(dist, _)| dist);
    &candidates[..k]
}
```

**RIGHT:**
```rust
fn top_k(candidates: &mut [(u32, usize)], k: usize) -> &[(u32, usize)] {
    // Stable sort with composite key: distance first, then element ID as tiebreaker
    candidates.sort_by_key(|&(dist, id)| (dist, id));
    &candidates[..k]
}
```

**Why:** If two vectors have Hamming distance 3,412 to the query, `sort_unstable`
may return them in either order. Different validators may choose differently.
The composite key `(distance, element_id)` ensures a unique total ordering --
ties on distance are broken deterministically by element ID.

---

### 1.4 NEVER use wall-clock time on-chain

`SystemTime::now()` returns the local system clock. Different validators have
different clocks (even with NTP, clocks drift by milliseconds). Any on-chain
logic that branches on wall-clock time will diverge.

**WRONG:**
```rust
use std::time::SystemTime;

fn compute_decay(published_at: u64) -> u64 {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let age_seconds = now - published_at;
    // Decay based on wall-clock age
    base_score * decay_factor(age_seconds)
}
```

**RIGHT:**
```rust
fn compute_decay(last_confirmed_block: u64, current_block: u64) -> u64 {
    let age_blocks = current_block.saturating_sub(last_confirmed_block);
    // Decay based on block-height difference -- identical on all validators
    base_score * decay_factor_blocks(age_blocks)
}
```

**Why:** Block numbers are part of consensus -- all validators agree on the
current block height. Wall-clock time is local state. Use block numbers for
all on-chain age, delay, and scheduling calculations.

---

### 1.5 NEVER use `thread_rng()` or `OsRng`

`thread_rng()` is seeded from OS entropy. `OsRng` reads `/dev/urandom`.
Neither is deterministic. Any vector generated this way will differ across
validators.

**WRONG:**
```rust
use rand::thread_rng;
use rand::Rng;

fn generate_vector() -> HdcVector {
    let mut rng = thread_rng();
    let mut words = [0u64; 160];
    for w in &mut words {
        *w = rng.gen();
    }
    HdcVector(words)  // Different on every validator, every run
}
```

**RIGHT:**
```rust
use rand::SeedableRng;
use rand::Rng;
use rand_chacha::ChaCha20Rng;

fn generate_vector(seed: u64) -> HdcVector {
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let mut words = [0u64; 160];
    for w in &mut words {
        *w = rng.gen();
    }
    HdcVector(words)  // Same seed -> same vector, every platform, every time
}
```

**Why:** `ChaCha20Rng` is a cryptographically secure PRNG with a
specification that guarantees identical output for identical seeds across all
platforms. This is what `HdcVector::random(seed)` and
`HdcVector::symbol(name)` use internally. Every on-chain vector generation
path MUST go through a seeded ChaCha20Rng.

---

### 1.6 NEVER use `f64::ln()` for HNSW levels

HNSW level assignment traditionally uses `-ln(uniform_random) * mL` where
`mL = 1/ln(M)`. The `ln()` function is a transcendental function NOT covered
by IEEE 754's bit-exactness guarantees. Different `libm` implementations
(glibc vs musl vs macOS libsystem_m) can produce different results for the
same input.

**WRONG:**
```rust
fn assign_level(random_float: f64, ml: f64, max_level: usize) -> usize {
    // f64::ln() is not bit-exact across platforms!
    let level = (-random_float.ln() * ml).floor() as usize;
    level.min(max_level)
}
```

**RIGHT:**
```rust
/// Integer-only HNSW level assignment for M=16.
///
/// Uses leading_zeros to implement a geometric distribution:
/// P(level >= k) = (1/16)^k = 2^{-4k}
/// level = floor(leading_zeros(random_bits) / 4)
fn deterministic_level(random_bits: u64, max_level: usize) -> usize {
    let level = (random_bits.leading_zeros() / 4) as usize;
    level.min(max_level)
}
```

**Why:** `leading_zeros()` compiles to a single hardware instruction (`LZCNT`
on x86, `CLZ` on ARM). It is an integer operation, bit-exact on every
platform. The division by 4 corresponds to `log2(M)` for M=16. For
non-power-of-2 M values, use rejection sampling on groups of `ceil(log2(M))`
bits.

---

### 1.7 NEVER use `partial_cmp().unwrap()`

`f64` implements `PartialOrd` but not `Ord` because `NaN != NaN`. Calling
`partial_cmp().unwrap()` panics if either operand is NaN. In sorting
contexts, a single NaN in a list of scores crashes the validator.

**WRONG:**
```rust
// Panics if any score is NaN
scores.sort_by(|a, b| a.partial_cmp(b).unwrap());
```

**RIGHT:**
```rust
// total_cmp treats NaN as greater than all other values -- no panic
scores.sort_by(|a, b| a.total_cmp(b));
```

**Why:** `f64::total_cmp()` provides a total ordering over all f64 values,
including NaN, -0.0, and infinities. It follows IEEE 754-2008 totalOrder,
which is deterministic and never panics. Use it for all off-chain float
sorting. For on-chain code, do not use floats at all (see 1.1).

---

## 2. HDC Algebra Anti-Patterns

These violate the mathematical properties of Binary Spatter Code and produce
incorrect HDC results.

### 2.1 NEVER make bundle ties random

When bundling an even number of vectors, some bit positions will have equal
counts of 0s and 1s (a tie). The tie-breaking rule MUST be deterministic.

**WRONG:**
```rust
fn bundle_to_vector(counts: &[i32]) -> HdcVector {
    let mut rng = thread_rng();
    let mut result = HdcVector::default();
    for i in 0..D {
        if counts[i] > 0 {
            result.set_bit(i, 1);
        } else if counts[i] == 0 {
            // Random tie-break: non-deterministic!
            result.set_bit(i, if rng.gen::<bool>() { 1 } else { 0 });
        }
    }
    result
}
```

**RIGHT:**
```rust
fn bundle_to_vector(counts: &[i32]) -> HdcVector {
    let mut result = HdcVector::default();  // All zeros
    for i in 0..D {
        if counts[i] > 0 {
            result.set_bit(i, 1);
        }
        // count == 0 -> bit stays 0 (deterministic tie-break)
        // count < 0  -> bit stays 0
    }
    result
}
```

**Why:** Ties break to 0 (the bit stays 0). This is the convention used in
`BundleAccumulator::to_vector()`. It is deterministic and consensus-safe. A
random tie-break produces different bundle results on different validators,
causing consensus divergence.

---

### 2.2 NEVER use `const` for ANTI_SUBSPACE

`ChaCha20Rng` is not const-evaluable in Rust -- it performs heap allocation
and loop iterations that the const evaluator cannot handle. Attempting
`const ANTI_SUBSPACE: HdcVector = HdcVector::random(SEED)` will not compile.

**WRONG:**
```rust
// Does not compile: ChaCha20Rng is not const-evaluable
const ANTI_SUBSPACE: HdcVector = HdcVector::random(0xAE71_5B8C_0000_0001);
```

**RIGHT:**
```rust
use std::sync::LazyLock;

const ANTI_SUBSPACE_SEED: u64 = 0xAE71_5B8C_0000_0001;

static ANTI_SUBSPACE: LazyLock<HdcVector> = LazyLock::new(|| {
    HdcVector::random(ANTI_SUBSPACE_SEED)
});
```

**Why:** `LazyLock` initializes the vector exactly once on first access, is
thread-safe, and produces the same vector on every validator (because the
seed is a constant). The generated vector is cached for the lifetime of the
process.

---

### 2.3 NEVER confuse Hamming distance with similarity

Hamming distance is an integer count of differing bits (u32, range 0 to
10,240). Similarity is a normalized float (f64, range 0.0 to 1.0). They are
inversely related. Mixing them up inverts every threshold check.

**WRONG:**
```rust
// Treating distance as similarity -- threshold is inverted
fn is_duplicate(a: &HdcVector, b: &HdcVector) -> bool {
    let dist = hamming_distance(a, b);
    dist > DUPLICATE_THRESHOLD  // WRONG: high distance means DISSIMILAR
}
```

**RIGHT (on-chain, integer):**
```rust
fn is_duplicate(a: &HdcVector, b: &HdcVector) -> bool {
    hamming_distance(a, b) < DUPLICATE_THRESHOLD  // 512: low distance = high similarity
}
```

**RIGHT (off-chain, float):**
```rust
fn is_duplicate_offchain(a: &HdcVector, b: &HdcVector) -> bool {
    let sim = 1.0 - hamming_distance(a, b) as f64 / D as f64;
    sim > 0.95  // High similarity = near-duplicate
}
```

**Why:** Distance = XOR popcount (u32). Similarity = `1.0 - distance / D`
(f64, off-chain only). A distance of 0 means identical (similarity 1.0). A
distance of 5,120 means random/orthogonal (similarity 0.5). Always use
`hamming_distance < threshold` on-chain, never `similarity > threshold`.

---

### 2.4 NEVER use `Reverse<>` in BinaryHeap for top-K search

Rust's `BinaryHeap` is a max-heap. When doing top-K nearest neighbor search
(smallest Hamming distances), a common mistake is to wrap items in `Reverse`
thinking it will keep the best matches. It does the opposite.

**WRONG:**
```rust
use std::cmp::Reverse;
use std::collections::BinaryHeap;

fn top_k_wrong(query: &HdcVector, store: &[HdcVector], k: usize) -> Vec<(u32, usize)> {
    let mut heap: BinaryHeap<Reverse<(u32, usize)>> = BinaryHeap::with_capacity(k);

    for (idx, vec) in store.iter().enumerate() {
        let dist = hamming_distance(query, vec);
        if heap.len() < k {
            heap.push(Reverse((dist, idx)));
        } else if dist < heap.peek().unwrap().0 .0 {
            // BUG: Reverse makes the SMALLEST distance the top of the heap.
            // peek() returns the smallest distance. We pop the BEST match
            // and keep the WORST ones!
            heap.pop();
            heap.push(Reverse((dist, idx)));
        }
    }

    heap.into_iter().map(|Reverse(item)| item).collect()
}
```

**RIGHT:**
```rust
use std::collections::BinaryHeap;

fn top_k(query: &HdcVector, store: &[HdcVector], k: usize) -> Vec<(u32, usize)> {
    // Max-heap: the LARGEST distance is at the top.
    let mut heap: BinaryHeap<(u32, usize)> = BinaryHeap::with_capacity(k);

    for (idx, vec) in store.iter().enumerate() {
        let dist = hamming_distance(query, vec);
        if heap.len() < k {
            heap.push((dist, idx));
        } else if dist < heap.peek().unwrap().0 {
            // The top of the max-heap is the WORST match in our current top-K.
            // If the new candidate is better (smaller distance), evict the worst.
            heap.pop();
            heap.push((dist, idx));
        }
    }

    let mut results: Vec<(u32, usize)> = heap.into_vec();
    results.sort_by_key(|&(dist, id)| (dist, id));  // Sort by distance, stable
    results
}
```

**Why:** For top-K nearest neighbors, you want a max-heap of size K that
tracks the K smallest distances seen so far. The top of the max-heap is the
WORST (largest distance) of your current K best. When a new candidate has a
smaller distance than the top, you evict the top and insert the new candidate.
`Reverse<>` flips the heap to a min-heap, making the BEST (smallest distance)
the eviction target -- you keep the worst matches instead of the best.

---

### 2.5 NEVER skip the carry2 weight in Harley-Seal

The Harley-Seal carry-save accumulation for AVX2 popcount produces three
accumulators: `ones` (weight 1), `twos` (weight 2), and `carry2` (weight 4).
The carry2 accumulator counts bit positions where all 4 inputs had a set bit,
so each set bit in carry2 represents 4 set bits in the original data.

**WRONG:**
```rust
// Missing the <<2 shift on carry2 -- undercount by 4x for those bits
total = _mm256_add_epi64(total, popcount_mm256(carry2));  // weight 1, should be 4!
total = _mm256_add_epi64(total, _mm256_slli_epi64(popcount_mm256(twos), 1));
total = _mm256_add_epi64(total, popcount_mm256(ones));
```

**RIGHT:**
```rust
// carry2 represents groups of 4 bits -- must be shifted left by 2 (multiplied by 4)
total = _mm256_slli_epi64(total, 2);  // total *= 4 (carry2 weight)
total = _mm256_add_epi64(
    total,
    _mm256_slli_epi64(
        _mm256_sad_epu8(popcount_mm256(twos), _mm256_setzero_si256()),
        1,  // twos count as 2 each
    ),
);
total = _mm256_add_epi64(
    total,
    _mm256_sad_epu8(popcount_mm256(ones), _mm256_setzero_si256()),
);
```

**Why:** Without the `<<2` shift, Hamming distances computed via AVX2 are
systematically lower than the true value. The scalar baseline and AVX2
path produce different results -- a consensus divergence between validators
using different code paths, or incorrect search results if the SIMD path
is used exclusively.

---

## 3. Solidity Anti-Patterns

These apply to the InsightBoard and PheromoneRegistry contracts.

### 3.1 NEVER require only ACTIVE in confirm()

The `confirm()` function must accept confirmations for insights in multiple
states, not just ACTIVE. Requiring only ACTIVE prevents first confirmations
(SUBMITTED has no confirmations yet) and blocks recovery of decaying or
challenged insights.

**WRONG:**
```solidity
function confirm(uint256 insightId) external {
    Insight storage insight = insights[insightId];
    require(insight.state == State.ACTIVE, "not active");  // Rejects SUBMITTED, DECAYING, CHALLENGED
    insight.confirmations += 1;
}
```

**RIGHT:**
```solidity
function confirm(uint256 insightId) external {
    Insight storage insight = insights[insightId];
    require(
        insight.state == State.SUBMITTED ||
        insight.state == State.ACTIVE ||
        insight.state == State.DECAYING ||
        insight.state == State.CHALLENGED,
        "cannot confirm in current state"
    );

    insight.confirmations += 1;
    insight.lastConfirmedBlock = block.number;

    // State transitions triggered by confirmation:
    if (insight.state == State.SUBMITTED) {
        insight.state = State.ACTIVE;  // First confirmation promotes
    } else if (insight.state == State.DECAYING) {
        insight.state = State.ACTIVE;  // Re-confirmation recovers
    } else if (insight.state == State.CHALLENGED) {
        insight.confirmsSinceChallenge += 1;
        if (insight.confirmsSinceChallenge >= 5) {
            insight.state = State.ACTIVE;  // 5+ confirmations resolve challenge
            insight.confirmsSinceChallenge = 0;
        }
    }
}
```

**Why:** The InsightBoard lifecycle has 7 states:
`SUBMITTED -> VERIFIED -> ACTIVE -> DECAYING -> ARCHIVED -> PURGED`, with
`CHALLENGED` as a side-state reachable from `ACTIVE`. Confirmations are
the mechanism for state transitions: SUBMITTED->ACTIVE (first confirmation),
DECAYING->ACTIVE (re-confirmation), CHALLENGED->ACTIVE (5+ confirmations).
Restricting confirm() to only ACTIVE breaks the entire lifecycle.

---

### 3.2 NEVER use publishBlock for age calculation

The `publishBlock` records when the insight was first submitted. The
`lastConfirmedBlock` records when it was last confirmed. Age-based decay
must use `lastConfirmedBlock`, not `publishBlock`, because re-confirmation
resets the decay clock.

**WRONG:**
```solidity
function computeState(uint256 insightId) public view returns (State) {
    Insight storage insight = insights[insightId];
    uint256 age = block.number - insight.publishBlock;  // Never resets!
    if (age > HALF_LIFE_BLOCKS) {
        return State.DECAYING;
    }
    return insight.state;
}
```

**RIGHT:**
```solidity
function computeState(uint256 insightId) public view returns (State) {
    Insight storage insight = insights[insightId];
    uint256 age = block.number - insight.lastConfirmedBlock;  // Resets on re-confirmation
    if (insight.state == State.ACTIVE && age > HALF_LIFE_BLOCKS) {
        return State.DECAYING;
    }
    if (insight.state == State.DECAYING && age > HALF_LIFE_BLOCKS * 5) {
        return State.ARCHIVED;
    }
    return insight.state;
}
```

**Why:** If you use `publishBlock`, an insight published at block 100 and
re-confirmed at block 10,000 still shows an age of `current - 100`. The
re-confirmation has no effect on decay. The insight decays as if it was
never touched after publication. Using `lastConfirmedBlock` ensures that
re-confirmation resets the decay timer, which is the entire point of the
re-confirmation mechanism.

---

### 3.3 NEVER skip existence checks

In Solidity, uninitialized storage slots are zero. A struct that has never
been written has all fields at their zero values. For the InsightBoard, this
means `state = 0`, which maps to `State.SUBMITTED` (the first enum variant).
Operations on non-existent insight IDs silently succeed because the
zero-initialized struct looks like a valid SUBMITTED insight.

**WRONG:**
```solidity
function confirm(uint256 insightId) external {
    Insight storage insight = insights[insightId];
    // No existence check -- if insightId was never submitted,
    // insight.state is 0 (SUBMITTED), and this succeeds silently
    require(insight.state == State.SUBMITTED || insight.state == State.ACTIVE, "bad state");
    insight.confirmations += 1;  // Incrementing a ghost entry
}
```

**RIGHT:**
```solidity
function confirm(uint256 insightId) external {
    Insight storage insight = insights[insightId];
    require(insight.author != address(0), "insight does not exist");  // Existence check
    require(
        insight.state == State.SUBMITTED ||
        insight.state == State.ACTIVE ||
        insight.state == State.DECAYING ||
        insight.state == State.CHALLENGED,
        "cannot confirm in current state"
    );
    insight.confirmations += 1;
}
```

**Why:** Without the existence check, anyone can call `confirm(99999)` on
a non-existent insight. The zero-initialized struct passes the state check
(state == 0 == SUBMITTED), the confirmation count increments from 0 to 1,
and the state changes to ACTIVE -- all on a phantom entry. This wastes gas,
corrupts state, and can be exploited to create fake insights without staking.
Always check `author != address(0)` (or use a dedicated `exists` flag) before
any operation.

---

### 3.4 NEVER use floating-point in Solidity

Solidity has no floating-point types. Any computation requiring fractional
values must use fixed-point integer math. A common mistake is implementing
formulas using basis-point division incorrectly, losing precision.

**WRONG:**
```solidity
// Integer division truncates: 7 / 10000 = 0, not 0.0007
function decayScore(uint256 score, uint256 basisPoints) public pure returns (uint256) {
    return score * basisPoints / 10000 / 10000;  // Double division loses precision
}
```

**RIGHT:**
```solidity
// Scale up before dividing to preserve precision
function decayScore(uint256 score, uint256 decayBps) public pure returns (uint256) {
    // decayBps in basis points: 10000 = 100% (no decay), 9500 = 95%
    return (score * decayBps) / 10000;
}
```

**Why:** Integer division truncates toward zero. If you divide by 10000
before multiplying, small values collapse to 0. Always multiply first, then
divide. Use basis points (10000 = 100%) consistently. For multi-step
calculations, accumulate in a larger intermediate type before the final
division.

---

## 4. Mattar-Daw Anti-Pattern

### 4.1 NEVER make EVB a 3-factor product (gain x need x EBU)

The Expected Value of Backup (EVB) from Mattar & Daw (2018) is defined as
`gain * need` -- exactly two factors. "EBU" (Expected Backup Utility) is the
NAME of the framework described in the paper, not a third multiplicative
factor. Some secondary sources incorrectly add a third term.

**WRONG:**
```rust
/// WRONG: 3-factor EVB -- this is not Mattar-Daw
fn evb_wrong(gain: f64, need: f64, priority: f64) -> f64 {
    gain * need * priority  // Three factors -- not the original formulation
}
```

**RIGHT:**
```rust
/// Compute the Expected Value of Backup for a single replay candidate.
///
/// EVB = gain * need  (Mattar & Daw, 2018 -- exactly 2 factors)
///   gain = prediction_error (how surprising was the outcome?)
///   need = recurrence_count * discount_factor (how likely to recur?)
pub fn mattar_daw_evb(entry: &ReplayEntry) -> f64 {
    let gain = entry.prediction_error;
    let need = entry.recurrence_count as f64 * entry.discount_factor;
    gain * need
}
```

**Why:** The original Mattar & Daw (2018) paper in *Nature Neuroscience*
defines EVB as `gain(s,a) * need(s)` -- the product of how much replaying an
episode would improve future performance (gain, backward-looking) and how
likely the agent is to encounter similar situations again (need,
forward-looking). Adding a third factor changes the mathematical semantics
and no longer corresponds to the published formulation. "EBU" (Expected
Backup Utility) is what the paper calls this framework -- it is not a
variable in the equation.

---

## 5. Scoring Anti-Patterns

### 5.1 NEVER use `0.8^consecutive_wins` for monopolist penalty

The VCG anti-monopolization mechanism penalizes entries that win consecutive
context slots. The penalty amplifier must GROW with more wins. Using
`PAYMENT_DECAY_PER_TICK^wins` (0.8^wins) makes the amplifier SHRINK toward
zero, reducing the penalty and rewarding monopolization.

**WRONG:**
```rust
const PAYMENT_DECAY_PER_TICK: f64 = 0.8;

// WRONG: 0.8^5 = 0.328 -- penalty SHRINKS, rewarding monopolization
let amplifier = PAYMENT_DECAY_PER_TICK.powi(consecutive_wins as i32);
let penalty = accumulated_payment * amplifier;
```

**RIGHT:**
```rust
const PAYMENT_DECAY_PER_TICK: f64 = 0.8;

// RIGHT: (1/0.8)^5 = 1.25^5 = 3.05 -- penalty GROWS, discouraging monopolization
let amplifier = (1.0 / PAYMENT_DECAY_PER_TICK).powi(consecutive_wins as i32);
let penalty = accumulated_payment * amplifier;
```

**Why:** The monopolist penalty is meant to make it progressively harder for
the same entry to keep winning. With the wrong formula:
- 0 wins: `0.8^0 = 1.0` (no effect)
- 5 wins: `0.8^5 = 0.328` (penalty reduced to 33% -- easier to win!)
- 10 wins: `0.8^10 = 0.107` (penalty reduced to 11% -- total monopoly)

With the correct formula:
- 0 wins: `1.25^0 = 1.0` (no effect)
- 5 wins: `1.25^5 = 3.05` (penalty tripled -- harder to win)
- 10 wins: `1.25^10 = 9.31` (penalty 9x -- very hard to monopolize)

---

### 5.2 NEVER forget to convert Hamming distance to similarity

When mixing HDC distance results with f64 scores (for off-chain ranking,
display, or context assembly), you must convert Hamming distance to
similarity. Forgetting this conversion means that "closer" vectors get
LOWER scores in a system that treats higher as better.

**WRONG:**
```rust
// Off-chain ranking: mixes distance (lower = closer) with score (higher = better)
fn combined_score(query: &HdcVector, candidate: &HdcVector, trust: f64) -> f64 {
    let dist = hamming_distance(query, candidate);
    // BUG: dist is 0-10240 where 0 = identical. Multiplying by trust
    // gives HIGHER scores to LESS relevant entries.
    dist as f64 * trust
}
```

**RIGHT:**
```rust
// Off-chain ranking: convert distance to similarity first
fn combined_score(query: &HdcVector, candidate: &HdcVector, trust: f64) -> f64 {
    let dist = hamming_distance(query, candidate);
    let similarity = 1.0 - dist as f64 / D as f64;  // 0.0 to 1.0, higher = closer
    similarity * trust
}
```

**Why:** Hamming distance and similarity are inverse: `similarity = 1.0 -
distance / D`. If you use raw distance in a scoring formula that expects
higher values to mean "better", you rank the most dissimilar entries highest.
This is a silent correctness bug -- search results are returned in exactly
the wrong order. Note: the conversion to similarity uses f64 and is
OFF-CHAIN ONLY.

---

## 6. Architecture Anti-Patterns

### 6.1 NEVER put chain dependencies in kora-hdc

The `kora-hdc` crate is the core HDC algebra library. It must have zero
blockchain dependencies. Chain-specific code (precompiles, contract
bindings, event processing) goes in `kora-hdc-chain`.

**WRONG:**
```toml
# crates/hdc/core/Cargo.toml
[dependencies]
revm = { workspace = true }           # Chain dependency in core crate!
kora-primitives = { workspace = true } # Chain dependency in core crate!
```

```rust
// crates/hdc/core/src/lib.rs
use revm::precompile::PrecompileResult;  // Core crate depends on REVM!

pub fn hdc_precompile(input: &[u8]) -> PrecompileResult {
    // ...
}
```

**RIGHT:**
```toml
# crates/hdc/core/Cargo.toml -- NO chain dependencies
[dependencies]
rand = { workspace = true }
rand_chacha = { workspace = true }
bytemuck = { workspace = true }
tiny-keccak = { workspace = true }

# crates/hdc/chain/Cargo.toml -- chain dependencies go HERE
[dependencies]
kora-hdc = { workspace = true }        # Depends on core
revm = { workspace = true }            # Chain dependency
kora-primitives = { workspace = true } # Chain dependency
```

**Why:** Keeping kora-hdc chain-agnostic means it can be used in:
- Off-chain agent cognition (no chain needed)
- Testing (no chain runtime required)
- Other chain integrations (if daeji ever supports multiple VMs)
- Standalone HDC research and benchmarking

If the core algebra crate depends on REVM, none of these use cases work
without pulling in the entire blockchain dependency tree.

---

### 6.2 NEVER create circular crate dependencies

Rust does not allow circular dependencies between crates. But it is easy to
accidentally create them when two crates need to share types.

**WRONG:**
```
kora-hdc depends on kora-hdc-chain  (for on-chain types)
kora-hdc-chain depends on kora-hdc  (for algebra)
  -> Circular dependency! cargo refuses to compile.
```

**RIGHT:**
```
kora-hdc          -- core algebra, no chain deps
  ^
  |
kora-hdc-chain    -- depends on kora-hdc, adds chain integration
```

**Why:** The dependency graph must be a DAG (directed acyclic graph). If
crate A depends on crate B and crate B depends on crate A, `cargo` reports a
cycle error and refuses to build. If you need shared types, extract them into
a third crate that both depend on, or define traits in the lower crate and
implement them in the higher crate.

---

### 6.3 NEVER use `unwrap()` in production paths

`unwrap()` panics on `None` or `Err`. In a validator node, a panic in a
consensus-critical path crashes the process. The node goes offline, reducing
the validator set and potentially halting the chain.

**WRONG:**
```rust
// Panic if the vector is not found -- crashes the validator
fn get_vector(store: &VectorStore, id: &[u8; 32]) -> HdcVector {
    store.get(id).unwrap()  // Panics on missing ID
}
```

**RIGHT:**
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HdcError {
    #[error("vector not found: {0:?}")]
    VectorNotFound([u8; 32]),
}

fn get_vector(store: &VectorStore, id: &[u8; 32]) -> Result<HdcVector, HdcError> {
    store.get(id).ok_or(HdcError::VectorNotFound(*id))
}
```

**Why:** Production paths must handle errors gracefully. Use `Result<T, E>`
for fallible operations. Reserve `unwrap()` for cases where the invariant is
proven (and add a comment explaining why it cannot fail). In consensus-critical
code, `expect("reason")` is acceptable when the condition is a true invariant,
but `Result` is always preferred.

---

### 6.4 NEVER block the async runtime

HDC operations (distance computation, search, bundling) are CPU-intensive.
Running them directly in an async context (e.g., an RPC handler on a Tokio
runtime) blocks the executor thread and starves other tasks (networking,
consensus messages).

**WRONG:**
```rust
// Blocks the Tokio executor thread for the entire search
async fn handle_search_rpc(query: HdcVector, store: &VectorStore) -> Vec<SearchResult> {
    // This is CPU-bound and takes milliseconds -- blocks the async runtime!
    store.search_top_k(&query, 10)
}
```

**RIGHT:**
```rust
// Offload CPU-heavy work to a blocking thread pool
async fn handle_search_rpc(query: HdcVector, store: Arc<VectorStore>) -> Vec<SearchResult> {
    tokio::task::spawn_blocking(move || {
        store.search_top_k(&query, 10)
    })
    .await
    .expect("blocking task panicked")
}
```

**Why:** Tokio's async runtime has a limited number of worker threads
(typically equal to the number of CPU cores). A CPU-bound task that takes 5ms
blocks one worker thread for 5ms. During that time, no other async task can
run on that thread -- including consensus message handling, peer
communication, and other RPC requests. `spawn_blocking` moves the work to a
dedicated thread pool designed for blocking operations.

---

## 7. Deployment Anti-Patterns

### 7.1 NEVER use interactive DKG for testnet

Distributed Key Generation (DKG) requires all participants to be online
simultaneously and exchange multiple rounds of messages. This is complex to
orchestrate, fragile to network partitions, and unnecessary for a testnet.

**WRONG:**
```bash
# Interactive DKG: requires all 3 validators to be online simultaneously
# Each validator runs:
kora-keygen dkg --peers "validator1:9000,validator2:9000,validator3:9000" --threshold 2
# Fails if any validator is not reachable during the ceremony
```

**RIGHT:**
```bash
# Trusted dealer: single operator generates all key shares
kora-keygen generate --validators 3 --threshold 2 --output ./keys/
# Distribute key files to each validator via secure channel
scp keys/validator-0.key validator1:/etc/kora/
scp keys/validator-1.key validator2:/etc/kora/
scp keys/validator-2.key validator3:/etc/kora/
```

**Why:** Trusted dealer is simpler (one command), faster (no multi-round
protocol), and sufficient for testnet where you control all validators. Use
interactive DKG only for production mainnets where no single party should
hold all key shares.

---

### 7.2 NEVER hardcode IP addresses

IP addresses change when containers restart, services redeploy, or
infrastructure migrates. Hardcoded IPs cause silent connection failures.

**WRONG:**
```toml
# config.toml
[network]
bootstrap_peers = ["10.0.1.5:9000", "10.0.1.6:9000", "10.0.1.7:9000"]
```

**RIGHT:**
```toml
# config.toml -- use DNS names
[network]
# Railway internal DNS
bootstrap_peers = [
    "validator-0.railway.internal:9000",
    "validator-1.railway.internal:9000",
    "validator-2.railway.internal:9000",
]

# Or Docker service names
# bootstrap_peers = ["validator-0:9000", "validator-1:9000", "validator-2:9000"]
```

**Why:** DNS names are resolved at connection time. When a container restarts
with a new IP, the DNS record updates automatically (Railway internal DNS,
Docker embedded DNS). Hardcoded IPs require manual config updates on every
infrastructure change.

---

### 7.3 NEVER skip health checks

Without health checks, orchestrators cannot sequence startup correctly.
Validators may attempt to connect to peers that are not ready, causing
connection failures and retry storms during boot.

**WRONG:**
```dockerfile
# No health check -- orchestrator has no way to know when the node is ready
CMD ["kora-node", "--config", "/etc/kora/config.toml"]
```

**RIGHT:**
```dockerfile
HEALTHCHECK --interval=5s --timeout=3s --start-period=30s --retries=3 \
    CMD curl -sf http://localhost:8545/health || exit 1

CMD ["kora-node", "--config", "/etc/kora/config.toml"]
```

```rust
// In the node binary: expose a /health endpoint
async fn health_handler() -> impl IntoResponse {
    // Check that consensus is initialized and peer connections are established
    if node.is_ready() {
        (StatusCode::OK, "healthy")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready")
    }
}
```

**Why:** Health checks enable:
1. Orchestrators (Railway, Docker Compose, Kubernetes) to wait for readiness
   before routing traffic or starting dependent services.
2. Automatic restart of unhealthy nodes.
3. Rolling deployments that do not cause downtime.

Without health checks, a validator that crashes during initialization appears
"running" to the orchestrator but cannot participate in consensus.

---

### 7.4 NEVER use public networking for P2P

Validator-to-validator communication (consensus messages, block propagation,
P2P gossip) should traverse internal/private networks. Exposing P2P ports
on public IPs creates attack surface for DoS, eclipse attacks, and
unauthorized connections.

**WRONG:**
```toml
# config.toml -- P2P on public interface
[network]
listen_addr = "0.0.0.0:9000"  # Listens on all interfaces, including public
```

**RIGHT:**
```toml
# config.toml -- P2P on internal network only
[network]
# Railway private network
listen_addr = "0.0.0.0:9000"  # Bind all interfaces
# But only expose via Railway private networking, not public

# Or Docker bridge network
# listen_addr = "172.18.0.0:9000"  # Internal bridge IP only
```

```yaml
# docker-compose.yml
services:
  validator-0:
    networks:
      - internal  # P2P on internal network
    ports:
      - "8545:8545"  # Only RPC is exposed publicly

networks:
  internal:
    internal: true  # No external access
```

**Why:** P2P ports carry consensus-critical messages. An attacker with access
to P2P ports can attempt eclipse attacks (surrounding a validator with
malicious peers), flood with invalid messages (DoS), or probe for
vulnerabilities. Keep P2P on internal networks; expose only RPC endpoints
(and those behind authentication/rate limiting).

---

## 8. Testing Anti-Patterns

### 8.1 NEVER run e2e tests in parallel

QMDB (the storage engine) uses file-based storage. Parallel tests writing to
the same QMDB data directory cause file locking conflicts, data corruption,
and non-deterministic test failures.

**WRONG:**
```bash
# Default: cargo test runs tests in parallel across threads
cargo test -p kora-e2e
# Test A and Test B both write to ./test-data/qmdb/ -> corruption
```

**RIGHT:**
```bash
# Force sequential execution for e2e tests
cargo test -p kora-e2e -- --test-threads=1
```

```rust
// Or use per-test temporary directories
#[test]
fn test_insight_lifecycle() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let db = QmdbHandle::open(tmp_dir.path()).unwrap();
    // Test uses isolated storage -- safe for parallel execution
}
```

**Why:** QMDB uses memory-mapped files with exclusive write access. Two tests
writing to the same database directory simultaneously cause:
1. File lock contention (test hangs or fails with "resource busy")
2. Data races on the memory-mapped region (undefined behavior)
3. Non-deterministic test outcomes depending on scheduling

Use `--test-threads=1` for integration tests, or give each test its own
temporary directory.

---

### 8.2 NEVER hardcode block numbers

Tests that assume specific block numbers break when the test harness changes
its startup sequence, when other tests run before them, or when the genesis
configuration changes.

**WRONG:**
```rust
#[test]
fn test_decay_at_block_1000() {
    let node = start_test_node();
    // Assumes we are at block 0 and can mine exactly to block 1000
    mine_blocks(&node, 1000);
    // Assumes the insight was submitted at block 5
    let insight = node.get_insight(insight_id);
    assert_eq!(compute_age(insight, 1000), 995);  // Fragile: depends on exact block numbers
}
```

**RIGHT:**
```rust
#[test]
fn test_decay_after_half_life() {
    let node = start_test_node();
    let submit_block = node.current_block();

    // Submit insight
    let insight_id = node.submit_insight(/* ... */);

    // Mine past one half-life using relative offset
    let half_life_blocks = 1_296_000;  // 72h at 200ms blocks
    mine_blocks(&node, half_life_blocks);

    let current = node.current_block();
    let age = current - submit_block;
    assert!(age >= half_life_blocks);
    assert_eq!(node.compute_state(insight_id), State::DECAYING);
}
```

**Why:** Relative block offsets make tests independent of the starting block
number. The test works regardless of how many blocks were mined during node
startup or by preceding tests. The test asserts on the age (a relative
quantity) rather than absolute block numbers.

---

### 8.3 NEVER skip determinism tests

Every on-chain operation must produce identical results across runs.
Determinism tests verify this property by running the same operation twice
(or on two separate instances) and comparing results bit-for-bit.

**WRONG:**
```rust
#[test]
fn test_vector_search() {
    let store = build_test_store();
    let results = store.search(&query, 10);
    // Only checks that results are non-empty -- does not verify determinism
    assert!(!results.is_empty());
}
```

**RIGHT:**
```rust
#[test]
fn test_vector_search_deterministic() {
    // Build two independent stores with the same data in the same order
    let store1 = build_test_store();
    let store2 = build_test_store();

    let query = HdcVector::random(42);

    let results1 = store1.search(&query, 10);
    let results2 = store2.search(&query, 10);

    // Results must be identical: same vectors, same distances, same order
    assert_eq!(results1.len(), results2.len());
    for (r1, r2) in results1.iter().zip(results2.iter()) {
        assert_eq!(r1.vector_id, r2.vector_id, "vector IDs must match");
        assert_eq!(r1.distance, r2.distance, "distances must match");
    }
}

#[test]
fn test_bundle_deterministic() {
    // Same inputs, same order -> same output, every time
    let vectors: Vec<HdcVector> = (0..10).map(|i| HdcVector::random(i)).collect();
    let refs: Vec<&HdcVector> = vectors.iter().collect();

    let result1 = bundle(&refs);
    let result2 = bundle(&refs);

    assert_eq!(result1, result2, "bundle must be deterministic");
}

#[test]
fn test_hnsw_insertion_order_deterministic() {
    // Two indexes built with the same vectors in the same order
    // must produce identical search results
    let vectors: Vec<HdcVector> = (0..1000).map(|i| HdcVector::random(i)).collect();

    let mut index1 = HnswIndex::new(16, 200);
    let mut index2 = HnswIndex::new(16, 200);

    for (i, v) in vectors.iter().enumerate() {
        index1.insert(i, v);
        index2.insert(i, v);
    }

    let query = HdcVector::random(9999);
    let results1 = index1.search(&query, 10);
    let results2 = index2.search(&query, 10);

    assert_eq!(results1, results2, "HNSW search must be deterministic");
}
```

**Why:** If any on-chain operation is non-deterministic, validators computing
state transitions from the same block will arrive at different state roots.
The chain forks. Determinism tests are the primary defense against this
failure mode. Every new on-chain operation should have a corresponding
determinism test that runs the operation twice and asserts bit-identical
output.

---

## Summary: Quick Reference

| # | Anti-Pattern | Consequence |
|---|---|---|
| 1.1 | f32/f64 on-chain | Consensus fork (platform-dependent rounding) |
| 1.2 | HashMap iteration | Consensus fork (randomized hash seed) |
| 1.3 | sort_unstable | Consensus fork (non-deterministic equal-element order) |
| 1.4 | Wall-clock time on-chain | Consensus fork (clock skew) |
| 1.5 | thread_rng() / OsRng | Consensus fork (non-deterministic vectors) |
| 1.6 | f64::ln() for HNSW levels | Consensus fork (libm differences) |
| 1.7 | partial_cmp().unwrap() | Validator crash on NaN |
| 2.1 | Random bundle ties | Consensus fork (non-deterministic bundles) |
| 2.2 | const ANTI_SUBSPACE | Compilation error |
| 2.3 | Distance/similarity confusion | Inverted threshold checks |
| 2.4 | Reverse<> in BinaryHeap for top-K | Returns worst matches instead of best |
| 2.5 | Missing carry2 weight | Incorrect Hamming distances (SIMD path) |
| 3.1 | confirm() only ACTIVE | Broken insight lifecycle |
| 3.2 | publishBlock for age | Decay clock never resets |
| 3.3 | Missing existence checks | Ghost entries in InsightBoard |
| 3.4 | Float emulation in Solidity | Precision loss, overflow |
| 4.1 | EVB as 3-factor product | Wrong replay prioritization |
| 5.1 | 0.8^wins for penalty | Rewards monopolization |
| 5.2 | Raw distance in float scores | Inverted search ranking |
| 6.1 | Chain deps in kora-hdc | Core crate unusable standalone |
| 6.2 | Circular crate dependencies | Build failure |
| 6.3 | unwrap() in production | Validator crash |
| 6.4 | Blocking async runtime | Starved networking/consensus |
| 7.1 | Interactive DKG for testnet | Unnecessary complexity, fragile startup |
| 7.2 | Hardcoded IP addresses | Silent connection failures on redeploy |
| 7.3 | No health checks | Unsequenced startup, zombie processes |
| 7.4 | Public networking for P2P | Attack surface for DoS/eclipse |
| 8.1 | Parallel e2e tests | QMDB file conflicts, data corruption |
| 8.2 | Hardcoded block numbers | Fragile tests, false failures |
| 8.3 | No determinism tests | Undetected consensus divergence |

---

## Audit Findings -- Anti-Pattern Verification

> **Audit date:** 2026-05-08
> **Scope:** All 26 anti-patterns checked against actual source code.

### Anti-Patterns AVOIDED (Correctly Implemented)

| # | Anti-Pattern | Status | Evidence |
|---|---|---|---|
| 1.1 | f32/f64 on-chain | AVOIDED | All on-chain code paths (precompile, HNSW, search indexes, SIMD Hamming distance) use integer-only arithmetic. Float code is consistently marked `OFF-CHAIN ONLY` (e.g., `crates/hdc/core/src/vector.rs:104`, `crates/hdc/core/src/context.rs:6`, `crates/hdc/core/src/trust.rs:3`, `crates/hdc/core/src/encode.rs:63`). |
| 1.3 | sort_unstable | AVOIDED | All sort calls in search indexes use `sort_by_key` with composite keys (`(distance, key)`) for deterministic tie-breaking. See `crates/hdc/core/src/search/brute.rs` and `crates/hdc/core/src/search/hnsw.rs`. |
| 1.4 | Wall-clock time on-chain | AVOIDED | All time references use block numbers or tick counts. No `SystemTime` or `Instant` in any HDC crate. |
| 1.5 | thread_rng() / OsRng | AVOIDED | `HdcVector::random()` uses `ChaCha20Rng::seed_from_u64()`. Explicit documentation at `crates/hdc/core/src/vector.rs:67`: "Uses ChaCha20 -- never OsRng or thread_rng." No `thread_rng` or `OsRng` in any HDC crate. |
| 1.6 | f64::ln() for HNSW levels | AVOIDED | `crates/hdc/core/src/search/hnsw.rs:90-92` uses `deterministic_level()` with keccak256 hash + `leading_zeros()`. Comment explicitly states "CONSENSUS SAFETY: Integer-only arithmetic. No f64::ln()." |
| 2.1 | Random bundle ties | AVOIDED | `BundleAccumulator::to_vector()` in `crates/hdc/core/src/bundle.rs` breaks ties to 0 (deterministic, count > 0 sets bit, otherwise stays 0). |
| 2.2 | const ANTI_SUBSPACE | AVOIDED | `crates/hdc/core/src/knowledge/anti.rs:13` uses `LazyLock<HdcVector>`, exactly as recommended. |
| 2.3 | Distance/similarity confusion | AVOIDED | On-chain comparisons use `hamming_distance < THRESHOLD`. Off-chain similarity uses `1.0 - dist as f64 / D as f64` with clear `OFF-CHAIN ONLY` annotations. |
| 2.5 | Missing carry2 weight | AVOIDED | `crates/hdc/core/src/search/simd.rs:102` correctly applies `total = _mm256_slli_epi64(total, 2)` (multiply by 4 for carry2), followed by `<<1` for twos and raw for ones. SIMD/scalar agreement test validates this. |
| 4.1 | EVB as 3-factor product | AVOIDED | `crates/hdc/core/src/cognitive/replay.rs:23-26` implements exactly `gain * need` (2 factors), matching Mattar & Daw (2018). |
| 5.2 | Raw distance in float scores | AVOIDED | `crates/hdc/core/src/knowledge/scoring.rs:43` correctly converts: `let relevance = 1.0 - (hamming as f64 / D as f64)`. |
| 6.1 | Chain deps in kora-hdc | AVOIDED | `crates/hdc/core/Cargo.toml` has no chain dependencies (`revm`, `kora-primitives`, etc.). Chain code is isolated in `crates/hdc/chain/`. |
| 6.2 | Circular dependencies | AVOIDED | Dependency flows `kora-hdc-chain -> kora-hdc`, never the reverse. |

### Anti-Patterns PARTIALLY PRESENT (Risks Exist)

| # | Anti-Pattern | Status | Location | Risk |
|---|---|---|---|---|
| 1.2 | HashMap iteration | **PRESENT** | Multiple files | See "New Anti-Patterns Found" below. |
| 1.7 | partial_cmp().unwrap() | **PRESENT** | `crates/hdc/core/src/knowledge/store.rs:98,157` | Uses `partial_cmp().unwrap_or(Ordering::Equal)` -- not a panic risk, but NaN scores would sort incorrectly. Should use `total_cmp()` instead. |
| 6.3 | unwrap() in production | **PRESENT** | Multiple production paths | See "New Anti-Patterns Found" below. |

### Anti-Patterns NOT APPLICABLE YET

| # | Anti-Pattern | Why |
|---|---|---|
| 2.4 | Reverse<> in BinaryHeap | HNSW uses a max-heap with `Reverse` wrapping for the candidate set and a regular max-heap for results. The usage in `hnsw.rs` is correct -- candidates are popped smallest-first via `Reverse`, results are capped via a max-heap. |
| 3.1-3.4 | Solidity anti-patterns | Contracts now exist under `contracts/src`; see "Second-Pass Remediation Detail" for Solidity-specific ABI, precompile, storage, and test-harness risks. |
| 7.1-7.4 | Deployment anti-patterns | Not auditable from source code alone. |
| 8.1-8.3 | Testing anti-patterns | Determinism tests ARE present for HNSW, brute-force, and bundle. E2E test configuration would need separate review. |

---

## New Anti-Patterns Found

### N1. HashMap iteration in consensus-adjacent code

**Severity: HIGH (potential consensus divergence)**

Three locations use `HashMap` where iteration order affects outputs:

**Location 1:** `crates/hdc/core/src/knowledge/store.rs:175`
```rust
for (key, entry) in self.entries.iter_mut() {
    // Step 1: Decay, Step 2: Promotion, Step 3: Demotion, Step 4: GC
}
```
The `KnowledgeStore.entries` field is `HashMap<[u8; 32], KnowledgeEntry>` (line 13). The `tick()` method iterates over it, collecting promotions, demotions, and GC candidates. If two entries interact (e.g., both compete for promotion but one depends on the other's state), iteration order matters. Even if currently safe, refactoring to `BTreeMap` eliminates the risk.

**Location 2:** `crates/hdc/chain/src/wisdom.rs:39`
```rust
submissions: std::collections::HashMap<B256, WisdomSubmission>,
```
`WisdomGate::resolve()` (line 92) iterates `submissions.values_mut()`. If two submissions have the same `submitted_at + challenge_window`, the order they are resolved is non-deterministic.

**Location 3:** `crates/hdc/chain/src/index.rs:14-16`
```rust
vectors: HashMap<B256, HdcVector>,
metadata: HashMap<B256, InsightMeta>,
```
`OnChainHdcIndex::search()` iterates `self.vectors.iter()` (line 83). When multiple vectors have the same Hamming distance to the query, their ordering in results is HashMap-iteration-order-dependent. The subsequent `sort_by_key(|(_, d)| *d)` is stable but the initial iteration order determines tie-breaking.

**Recommendation:** Replace all three with `BTreeMap` per anti-pattern 1.2.

---

### N2. Production unwrap() calls in HNSW graph operations

**Severity: MEDIUM (validator crash risk)**

The HNSW index uses `unwrap()` in 6 production code paths (not tests):

| Line | File | Expression | Risk |
|------|------|-----------|------|
| 107 | `search/hnsw.rs` | `hash[0..8].try_into().unwrap()` | Low (slice length guaranteed). Still, `expect()` preferred. |
| 206 | `search/hnsw.rs` | `result.peek().unwrap()` | Medium -- panics if result is empty. |
| 222 | `search/hnsw.rs` | `result.peek().unwrap()` | Medium -- panics if result is empty after pop. |
| 262 | `search/hnsw.rs` | `self.nodes.get_mut(&id_a).unwrap()` | High -- panics if node was deleted (e.g., tombstoned). |
| 267 | `search/hnsw.rs` | `self.nodes.get_mut(&id_b).unwrap()` | High -- panics if node was deleted. |
| 274 | `search/hnsw.rs` | `self.nodes.get_mut(&node_id).unwrap()` | High -- panics if node was deleted. |
| 356 | `search/hnsw.rs` | `self.entry_point.unwrap()` | Medium -- panics if index is empty despite guard. |

Additional production unwrap():
| Line | File | Expression | Risk |
|------|------|-----------|------|
| 158 | `chain/precompile.rs` | `data[BYTES..BYTES+4].try_into().unwrap()` | Medium -- panics if slice is wrong length (should be caught by length check above, but fragile). |
| 150 | `vector.rs` | `bytes[i*8..(i+1)*8].try_into().unwrap()` | Low (loop bounds match), but `expect()` preferred. |
| 396 | `trust.rs` | `registry.agents.get(&agent).unwrap()` | High -- panics if agent not in registry. |

**Recommendation:** Replace with `Result`-based error handling per anti-pattern 6.3, or at minimum `expect("invariant: ...")` with documented justification.

---

### N3. Event topic hashes are all `B256::ZERO`

**Severity: HIGH (feature non-functional)**

`crates/hdc/chain/src/event.rs:9-26` -- All 5 event topic constants are `B256::ZERO`:
```rust
pub const INSIGHT_PUBLISHED: B256 = B256::ZERO; // TODO: compute actual hash
pub const INSIGHT_ACCEPTED: B256 = B256::ZERO;   // TODO: compute actual hash
pub const INSIGHT_REJECTED: B256 = B256::ZERO;   // TODO: compute actual hash
pub const INSIGHT_CHALLENGED: B256 = B256::ZERO;  // TODO: compute actual hash
pub const PHEROMONE_DEPOSITED: B256 = B256::ZERO;  // TODO: compute actual hash
```

Since all are `B256::ZERO`, every event matches `INSIGHT_PUBLISHED` and none of the other branches ever execute. The `process_log()` function body is also entirely commented out.

---

### N4. Excessive `clone()` on `HdcVector` (1,280-byte struct)

**Severity: LOW (performance)**

`HdcVector` is 1,280 bytes. Multiple locations clone it unnecessarily:

| File | Line | Context |
|------|------|---------|
| `cognitive/replay.rs` | 40 | `candidates.clone()` -- clones entire Vec of candidates |
| `search/hnsw.rs` | 248 | `candidates.clone()` -- clones candidate list |
| `search/hnsw.rs` | 341 | `vector: vector.clone()` -- stored at insertion time (necessary) |
| `search/hnsw.rs` | 312 | `node.vector.clone()` -- during compaction rebuild |
| `knowledge/store.rs` | 40 | `entry.vector.clone()` -- duplicates vector into flat index |
| `knowledge/store.rs` | 91,139 | `entry.clone()` -- clones entire entry including 1,280-byte vector |
| `context.rs` | 344 | `candidate.clone()` -- clones during context assembly |
| `cognitive/state_machine.rs` | 153,264 | `self.current.clone()` -- clones behavioral state |

Most are necessary but some (e.g., `candidates.clone()` at `replay.rs:40` and `hnsw.rs:248`) appear to be convenience clones that could be avoided with borrows or moved values.

---

### N5. `is_anti()` uses `f64` similarity for on-chain-adjacent detection

**Severity: MEDIUM (potential consensus issue if used on-chain)**

`crates/hdc/core/src/knowledge/anti.rs:38`:
```rust
let sim = 1.0 - (dist as f64 / D as f64);
sim > ANTI_RESONANCE_THRESHOLD  // 0.90
```

This uses `f64` arithmetic. Currently used only in the off-chain knowledge store, but if this function is ever called from an on-chain path, it would be a consensus violation (anti-pattern 1.1). The equivalent integer threshold would be `hamming_distance < 1,024` (i.e., `dist < D - (0.90 * D) = 10,240 - 9,216 = 1,024`), which equals `RESONANCE_THRESHOLD_HAMMING`.

**Recommendation:** Provide an integer-based `is_anti_onchain()` variant using `hamming_distance < RESONANCE_THRESHOLD_HAMMING`.

---

### N6. `unsafe impl Send/Sync for HdcVector` without justification

**Severity: LOW (correctness)**

`crates/hdc/core/src/vector.rs:23-24`:
```rust
unsafe impl Send for HdcVector {}
unsafe impl Sync for HdcVector {}
```

`HdcVector` contains `[u64; 160]`, which already implements `Send` and `Sync`. These manual impls are unnecessary and mask the auto-derive. They are not dangerous (the impls are correct), but they suppress any future compiler warnings if the struct gained a non-Send field.

---

### N7. `search_with_anti_check` uses `f64` for confidence scoring

**Severity: LOW (off-chain only, but fragile)**

`crates/hdc/core/src/knowledge/store.rs:124-136`:
```rust
let mut confidence_modifier = 1.0_f64;
// ...
let anti_sim = 1.0 - (*anti_dist as f64 / crate::constants::D as f64);
if anti_sim > 0.9 { continue; }
else if anti_sim > 0.7 { confidence_modifier = 0.5; }
```

This is off-chain, but the thresholds (0.9, 0.7) are magic numbers not defined as named constants. They should be extracted to constants alongside `ANTI_RESONANCE_THRESHOLD`.

---

## Recommended Changes Checklist

### Critical (Consensus Safety)

- [ ] **Replace `HashMap` with `BTreeMap`** in `crates/hdc/chain/src/index.rs` (lines 14-16), `crates/hdc/chain/src/wisdom.rs` (line 39), and `crates/hdc/core/src/knowledge/store.rs` (line 13). These are iterated in consensus-adjacent code paths. (Anti-pattern 1.2)
- [ ] **Compute actual keccak256 event topic hashes** in `crates/hdc/chain/src/event.rs` (lines 13-25). All are currently `B256::ZERO`, making event processing non-functional. (New anti-pattern N3)
- [ ] **Replace `partial_cmp().unwrap_or()`** with `total_cmp()` in `crates/hdc/core/src/knowledge/store.rs` lines 98 and 157. (Anti-pattern 1.7)

### High Priority (Crash Prevention)

- [ ] **Eliminate production `unwrap()` calls** in HNSW graph operations (`crates/hdc/core/src/search/hnsw.rs` lines 206, 222, 262, 267, 274, 356). Replace with `Result` propagation or `expect("invariant: ...")`. (Anti-pattern 6.3)
- [ ] **Eliminate production `unwrap()`** in `crates/hdc/chain/src/precompile.rs` line 158. (Anti-pattern 6.3)
- [ ] **Eliminate production `unwrap()`** in `crates/hdc/core/src/trust.rs` line 396. (Anti-pattern 6.3)

### Medium Priority (Robustness)

- [ ] **Create integer-based `is_anti_onchain()`** as an alternative to the `f64`-based `is_anti()` in `crates/hdc/core/src/knowledge/anti.rs`. Use `hamming_distance < RESONANCE_THRESHOLD_HAMMING` for any future on-chain usage. (New anti-pattern N5)
- [ ] **Extract magic thresholds** in `crates/hdc/core/src/knowledge/store.rs` lines 129, 132 (`0.9`, `0.7`) into named constants. (New anti-pattern N7)

### Low Priority (Code Quality)

- [ ] **Remove unnecessary `unsafe impl Send/Sync`** from `crates/hdc/core/src/vector.rs` lines 23-24. The auto-derived impls are correct for `[u64; 160]`. (New anti-pattern N6)
- [ ] **Audit `HdcVector` clone sites** for unnecessary copies. Priority targets: `crates/hdc/core/src/cognitive/replay.rs:40`, `crates/hdc/core/src/search/hnsw.rs:248`. (New anti-pattern N4)

---

## Second-Pass Remediation Detail

> **Second-pass date:** 2026-05-08
> **Scope:** HDC core/chain crates, node precompile integration, Solidity contracts, Solidity tests, and HDC e2e tests.
> **Ownership note:** This pass only updates this markdown file. No code was changed.

This section groups the audit findings by implementation smell rather than by
source file. The goal is to make ad-hoc and duct-tape patterns mechanically
detectable before they become consensus or contract-level behavior.

### Remediation Taxonomy

| Taxonomy | Evidence in current HDC code | Why it matters | Correct replacement pattern | Detection rule |
|---|---|---|---|---|
| **Protocol ABI drift** | Rust precompile dispatch defines `0x01=hamming`, `0x02=bind`, `0x03=bundle`, `0x04=permute`, `0x05=vector_id`, `0x06=is_similar` in `crates/hdc/chain/src/precompile.rs:37-63`. Solidity `HdcLib` expects `0x01=storeVector`, `0x02=searchSimilar`, `0x03=deleteVector`, `0x04=bundle`, `0x05=bind`, `0x06=hamming`, `0x07=permute` in `contracts/src/HdcPrecompile.sol:6-8` and call sites at lines 19, 31, 42, 58, 69, 84, 93. | Solidity calls either hit the wrong operation, use an invalid opcode, or decode an incompatible return. `InsightBoard.submit()` depends on `searchSimilar()` and `storeVector()` (`contracts/src/InsightBoard.sol:79-117`), so the primary contract path can revert or silently test the wrong surface. | Define one canonical precompile ABI and generate both Rust opcode dispatch and Solidity constants from it. Payload widths must match exactly; for example bundle count is `u32` in Rust (`precompile.rs:132-135`) but `uint16` in Solidity (`HdcPrecompile.sol:42`). Add a contract-to-precompile integration test that executes `HdcLib` against the actual REVM provider. | Search for opcode literals, `Opcode`, `storeVector`, `searchSimilar`, `deleteVector`, and `abi.encodePacked(uint8` in Rust precompile, Solidity library, contracts, and e2e tests. |
| **Stateful precompile illusion** | Solidity models `storeVector`, `searchSimilar`, and `deleteVector` as calls to address `0x09` (`HdcPrecompile.sol:64-95`). Rust precompile is pure algebra only and has no store/search/delete opcodes (`precompile.rs:84-90`). Node-local `OnChainHdcIndex` is described as ephemeral and rebuilt from events (`crates/hdc/chain/src/index.rs:1-3`), but event sync is still placeholder (`crates/hdc/chain/src/event.rs:35-53`). | A mutable in-memory precompile is not EVM state. Unless the state is part of block execution and state-root calculation, validators can agree on transaction execution while off-chain indexes diverge after restart, log replay, or event decoder bugs. | Keep the precompile stateless and pure, or explicitly model any mutable index as consensus state. For current architecture, contracts should emit canonical events, node-local indexes should rebuild from finalized logs, and contract read paths should not assume a hidden precompile index exists. | Search for `in-memory index`, `storeVector`, `deleteVector`, `searchSimilar`, `process_log`, `record_pheromone`, and `B256::ZERO`. |
| **Consensus ordering leak** | `OnChainHdcIndex` stores vectors and metadata in `HashMap` (`crates/hdc/chain/src/index.rs:5,14-16`) and sorts only by distance (`index.rs:117`). `WisdomGate` uses `HashMap` for submissions (`crates/hdc/chain/src/wisdom.rs:39`). `KnowledgeStore` iterates a `HashMap` during `tick()` (`crates/hdc/core/src/knowledge/store.rs:13,175`). | Equal-distance search results, same-window state transitions, and GC/promotion ordering can become process-seed-dependent. If any of these outputs affect transactions, events, roots, or replayed indexes, validators can diverge. | Use `BTreeMap` for keyed consensus-adjacent storage, or store deterministic `Vec` insertion order plus a unique key. Every result sort must use a total composite key such as `(distance, id)`, not distance alone. | Search consensus-adjacent code for `HashMap`, distance-only `sort_by_key`, and result sorts that do not include a unique ID tie-breaker. |
| **Float boundary leak** | Off-chain modules use `f64` with comments, but `is_anti()` lives in the core knowledge module and computes similarity as `1.0 - dist / D` (`crates/hdc/core/src/knowledge/anti.rs:17-39`). `KnowledgeStore` repeats magic float thresholds `0.9` and `0.7` (`store.rs:124-135`). Sorting uses `partial_cmp().unwrap_or(...)` (`store.rs:98,157`; `cognitive/replay.rs:37`; `cognitive/somatic.rs:79`). | Float code is acceptable off-chain, but becomes a consensus bug when reused from chain/precompile paths. NaN handling and libm functions can also create nondeterministic ordering or silent score collapse. | Split APIs by determinism boundary: `*_offchain` may use `f64`; `*_onchain` must use integer Hamming thresholds or basis points. For anti-knowledge, `similarity > 0.90` should become `dist < 1024` in any chain path. Use `total_cmp()` for off-chain float sorting. | Search for `f32`, `f64`, `ln(`, `partial_cmp`, `0.9`, and `0.7` under HDC core, chain, and executor paths. |
| **Panic-as-control-flow** | Production unwraps remain in precompile parsing (`crates/hdc/chain/src/precompile.rs:158`), HNSW graph operations (`crates/hdc/core/src/search/hnsw.rs:206,222,262,267,274,356`), and vector decoding (`crates/hdc/core/src/vector.rs:150`). | Malformed transaction input or a broken graph invariant should return a precompile error or index error. A panic can crash a validator, abort block execution, or turn bad input into a liveness issue. | Make fallible parsing return `PrecompileError::InvalidInput`. Make HNSW graph mutations return `Result` where invariants can be invalidated by deletes/compaction. Use `expect("invariant: ...")` only for locally proven impossible states, and keep those out of external-input paths. | Search production paths for `unwrap(` and `expect(`, excluding test modules and Solidity test files. |
| **Placeholder sentinel implementation** | All event topics are `B256::ZERO` (`crates/hdc/chain/src/event.rs:12-25`), `process_log()` only logs TODO branches (`event.rs:35-53`), and the event names include stale `InsightAccepted`/`InsightRejected` topics while the current interface emits `InsightConfirmed`, `InsightStateChanged`, `InsightRenewed`, and `InsightPurged` (`contracts/src/IInsightBoard.sol:56-81`). `OnChainHdcIndex::record_pheromone()` is TODO (`index.rs:129-132`), and `PheromoneRegistry.readPheromones()` returns empty arrays (`contracts/src/PheromoneRegistry.sol:254-263`). | Zero sentinels and no-op implementations make tests pass while production features are absent. Event sync currently cannot distinguish event kinds, and pheromone reads always report no results. | Either implement the feature or fail closed behind an explicit feature flag. Event topics should be generated from the actual Solidity ABI, and the Rust event enum must match current contract events. Placeholder public methods should revert with a precise "not implemented" error until semantics are complete. | Search for `TODO`, `Placeholder`, `B256::ZERO`, zero-length array returns, and `skip precompile`. |
| **Solidity mock gap** | `contracts/test/InsightBoard.t.sol:8-13` defines `TestableInsightBoard` specifically to skip precompile calls; it also skips `HdcLib.storeVector()` and `HdcLib.deleteVector()` (`InsightBoard.t.sol:50,79`). E2E tests call raw Rust precompile opcodes directly (`crates/e2e/src/tests/hdc.rs:44-86`) instead of exercising `HdcLib`. | The exact layer most likely to break, Rust/Solidity precompile compatibility, is not tested. The current tests can validate FSM logic while missing ABI drift, payload width mismatches, and wrong return decoding. | Keep pure FSM unit tests, but add at least one integration test that deploys the real contract and routes `HdcLib` calls through the actual HDC precompile provider. The test must cover hamming, bind, bundle, permute, search/submit behavior, and revert surfaces. | Search for `TestableInsightBoard`, `skip precompile`, `call_hdc_precompile`, and `HdcLib` across contract tests and e2e tests. |
| **Solidity storage sentinel and external-call fragility** | Existence checks use `publishBlock != 0` (`InsightBoard.sol:147-150,204-211,240-243,266-269`) and `depositBlock != 0` (`PheromoneRegistry.sol:104-107,136-147,172-178`). `purge()` calls the precompile before deleting storage (`InsightBoard.sol:279-283`) and later sends ETH with `author.call{value: legacy}("")` (`InsightBoard.sol:288-291`). | The current checks are mostly present, which avoids the ghost-entry bug, but the sentinel is coupled to block numbering and storage layout. Low-level calls and precompile side effects need deliberate revert semantics and reentrancy posture. | Prefer an explicit `exists` flag for mappings whose zero value can look valid. Keep state transitions before external ETH calls, add reentrancy protection if additional callbacks become possible, and specify whether precompile failure should revert the whole state transition. | Search Solidity for `publishBlock != 0`, `depositBlock != 0`, `author.call`, `.call(payload)`, `delete insights`, and `delete pheromones`. |
| **Shadow HDC crate** | Workspace dependency points `kora-hdc` at `crates/hdc/core` (`Cargo.toml:77`), while a second package named `kora-hdc` exists at `crates/kora-hdc/Cargo.toml:2`. | Workers can patch or audit the wrong copy. CI may only compile the workspace crate while a stale duplicate keeps misleading future audits. | Remove the shadow copy, rename it as an archived reference outside the workspace, or add a CI guard that fails if multiple `Cargo.toml` files declare `name = "kora-hdc"`. | Search `Cargo.toml` files for `name = "kora-hdc"` and root dependency path declarations for `kora-hdc`. |

### Consensus Safety Checklist

- [ ] No consensus, precompile, finalized-log replay, or state-root path uses `f32`, `f64`, `SystemTime`, `Instant`, `thread_rng`, `OsRng`, `HashMap` iteration, or `sort_unstable`.
- [ ] Every consensus-visible search result is sorted by a unique total key: `(distance, vector_id)` or `(distance, insertion_id)`.
- [ ] Every precompile input parser returns a typed error for malformed input; no external input path can panic.
- [ ] Any local index rebuilt from chain events is reproducible from finalized logs alone, with real event topics and ABI decoding.
- [ ] HNSW, brute-force, SIMD, and precompile paths have scalar/reference determinism tests that compare bit-for-bit outputs.
- [ ] Any off-chain float helper is named or documented as off-chain and is not imported by `crates/hdc/chain` or `crates/node/executor`.

### Solidity-Specific Checklist

- [ ] `contracts/src/HdcPrecompile.sol` and `crates/hdc/chain/src/precompile.rs` share one opcode table, payload schema, output schema, and gas schedule.
- [ ] `HdcLib` integration tests call the actual precompile provider; FSM-only tests may mock precompile behavior but cannot be the only contract coverage.
- [ ] `storeVector`, `searchSimilar`, and `deleteVector` are either implemented as consensus-safe stateful operations or removed from the Solidity library and replaced with event-driven off-chain indexing.
- [ ] All mapping-backed structs have explicit existence checks; prefer `exists` over relying on `publishBlock` or `depositBlock` if block-zero execution or layout changes are possible.
- [ ] Contract IDs are domain-separated and collision-resistant for same sender, same vector, and same block cases; use `abi.encode(...)` or a documented fixed-width `abi.encodePacked(...)` tuple.
- [ ] Public placeholder methods either implement the advertised behavior or revert with a named error; they should not silently return empty arrays.
- [ ] Low-level `.call` uses are reviewed for CEI, revert behavior, and reentrancy posture.

### Remediation Order

1. **Align the Rust/Solidity precompile ABI first.** This blocks meaningful contract integration tests and currently invalidates `HdcLib`.
2. **Decide whether the precompile is pure or stateful.** If pure, remove Solidity store/search/delete assumptions. If stateful, specify how the index participates in consensus state and replay.
3. **Replace placeholder event sync with generated topics and ABI decoding.** Until this is done, node-local HDC indexes cannot be trusted after restart.
4. **Remove deterministic-ordering hazards.** Convert `HashMap` in consensus-adjacent indexes to `BTreeMap` or add composite sorting that includes IDs.
5. **Turn production panics into errors.** Start with precompile parsing and HNSW graph operations.
6. **Fence off float code.** Add integer anti-knowledge checks for any path that can be called from chain/precompile code.
7. **Resolve duplicate HDC crate ownership.** Make it impossible to patch a non-workspace `kora-hdc` copy by mistake.

---

## 9. Codebase Anti-Patterns Found (2026-05-08)

> These are concrete anti-pattern instances discovered in the actual codebase
> during the cross-cutting audit (doc 20) and PR #42 reconciliation analysis.
> Each entry includes WHERE the problem lives, what FIX is needed, and how to
> VERIFY the fix worked.

---

### AP-01: Two trust implementations competing

Two separate `trust.rs` files implement overlapping trust-tracking logic. Both
define agent trust scores, EMA updates, and registry lookup, but they diverge
in API surface and data structures.

**WHERE:**
- `crates/hdc/core/src/trust.rs` -- standalone `TrustRegistry` with `HashMap<H256, AgentTrust>`, EMA-based scoring, penalty/reward methods, and `f64` arithmetic.
- `crates/hdc/core/src/knowledge/trust.rs` -- `TrustTracker` with per-agent `TrustScore` structs, similar EMA updates, but scoped to the knowledge subsystem.

**Consequence:** A caller importing `kora_hdc::trust::TrustRegistry` gets different
behavior than one using `kora_hdc::knowledge::trust::TrustTracker`. If both are
wired into the same node, trust scores for the same agent can diverge, leading to
inconsistent knowledge scoring and replay prioritization.

**FIX:**
1. Choose one canonical trust implementation. `knowledge/trust.rs` is more
   tightly scoped and better factored; promote it or merge the best of both
   into a single `crates/hdc/core/src/trust.rs`.
2. Delete the redundant file.
3. Update all callers (`knowledge/store.rs`, `knowledge/scoring.rs`,
   `cognitive/replay.rs`, `context.rs`) to use the canonical type.

**VERIFY:**
```bash
# After fix: only one trust module should exist
grep -rn "TrustRegistry\|TrustTracker" crates/hdc/core/src/ | grep -v test
# Should show exactly one type, not two
# Confirm compilation:
cargo check -p kora-hdc --all-features
```

---

### AP-02: EMA double-application bug in TrustRegistry

The `TrustRegistry::update()` method applies an EMA (exponential moving average)
update to the trust score, but the `record_outcome()` method in some call paths
also applies its own smoothing. When both are called in sequence (which happens
during knowledge store `tick()` processing), the EMA is effectively applied
twice, causing trust scores to converge to steady-state values faster than
intended. A single bad outcome over-penalizes an agent.

**WHERE:**
- `crates/hdc/core/src/trust.rs` -- `TrustRegistry::update()` (applies EMA with
  alpha = 0.1).
- `crates/hdc/core/src/trust.rs` -- `TrustRegistry::record_outcome()` calls
  `update()` internally, then the caller in `knowledge/store.rs` may also call
  `update()` during `tick()`.

**Consequence:** Agent trust scores drop (or rise) ~2x faster than the intended
EMA rate. Agents with a single bad prediction can be effectively blacklisted
within a few ticks instead of the intended gradual decay.

**FIX:**
1. Audit all call sites of `TrustRegistry::update()` and
   `TrustRegistry::record_outcome()`.
2. Ensure each trust-affecting event triggers exactly one EMA update.
3. If `record_outcome()` internally calls `update()`, callers must not call
   `update()` again.
4. Add a unit test that applies N outcomes and verifies the trust score matches
   the expected single-EMA trajectory.

**VERIFY:**
```bash
# Add test to crates/hdc/core/src/trust.rs:
#[test]
fn ema_applied_exactly_once_per_outcome() {
    let mut registry = TrustRegistry::new();
    let agent = H256::random();
    registry.register(agent);
    // Record 10 positive outcomes
    for _ in 0..10 {
        registry.record_outcome(agent, true);
    }
    let score = registry.score(agent);
    // Expected: EMA(0.5, alpha=0.1, 10 positive) should be ~0.8868
    // If double-applied, score would be ~0.97 (converges too fast)
    assert!(score < 0.95, "EMA applied more than once: score={score}");
}
```

---

### AP-03: TestableInsightBoard copy-paste duplication

The Foundry test contract `TestableInsightBoard` in
`contracts/test/InsightBoard.t.sol` copies and pastes the `submit()` and
`purge()` functions from `InsightBoard.sol`, then removes the precompile calls
(`HdcLib.searchSimilar()`, `HdcLib.storeVector()`, `HdcLib.deleteVector()`).
This means any change to the production `submit()` or `purge()` logic must be
manually duplicated in the test contract, and the test contract does not
actually verify the precompile integration.

**WHERE:**
- `contracts/test/InsightBoard.t.sol` lines 8-13 (class definition) and
  lines ~50, ~79 (overridden `submit()`, `purge()`).

**Consequence:**
1. Production logic changes to `submit()` or `purge()` can silently diverge
   from the test version.
2. The Rust/Solidity precompile ABI mismatch (opcode table drift) is completely
   hidden because tests bypass the precompile entirely.

**FIX:**
1. Replace `TestableInsightBoard` with a mock precompile approach: deploy a
   contract at `0xA0C` that returns canned responses for `hamming`,
   `searchSimilar`, etc.
2. Or use Foundry's `vm.mockCall()` to intercept precompile calls and return
   expected values, while still exercising the real `submit()` code.
3. Remove the copy-pasted function overrides entirely.

**VERIFY:**
```bash
# After fix: no function overrides in the test contract
grep -n "function submit\|function purge" contracts/test/InsightBoard.t.sol
# Should return zero matches (test uses the real functions)
forge test --match-contract InsightBoardTest -vvv
# All tests should pass with the mock precompile
```

---

### AP-04: Stored vs computed state inconsistency in InsightBoard

`InsightBoard.sol` stores `state` in the `InsightAnchor` struct (slot 2, 1 byte)
but also has `computeState()` which derives the state from age and thresholds.
The stored state and computed state can disagree: an insight might have
`anchor.state == ACTIVE` in storage while `computeState()` returns `DECAYING`
because time has passed. Some code paths read `anchor.state` directly (e.g.,
`confirm()` at line 754), while others call `computeState()` (e.g., `purge()`
at line 891). This creates windows where the insight behaves as ACTIVE for
confirmations but as DECAYING for display.

**WHERE:**
- `contracts/src/InsightBoard.sol` -- `confirm()` reads `insight.anchor.state`
  directly.
- `contracts/src/InsightBoard.sol` -- `purge()` calls `computeState()`.
- `contracts/src/InsightBoard.sol` -- `computeState()` is a view function that
  overrides stored state based on age.

**Consequence:** Confirmations can be accepted for insights that `computeState()`
would report as ARCHIVED or even PURGED, because `confirm()` checks the stored
state which is never updated by time passage alone.

**FIX:**
1. In `confirm()`, replace `insight.anchor.state` reads with `computeState(insightId)`.
2. Any write function that needs to check state should call `computeState()` first.
3. Add a test that submits an insight, mines past the half-life, and confirms
   that `confirm()` rejects the call (because `computeState()` returns DECAYING
   or later, not because `anchor.state` still says ACTIVE).

**VERIFY:**
```solidity
// In InsightBoard.t.sol:
function test_confirm_respects_computed_state() public {
    bytes32 id = submitTestInsight();
    // Mine past 10x half-life
    vm.roll(block.number + 648_000 * 10 + 1);
    // Should revert because computeState returns PURGED
    vm.expectRevert("InsightBoard: not confirmable");
    board.confirm(id);
}
```

---

### AP-05: Precompile opcode mismatch between Solidity and Rust

The Solidity library `HdcLib` in `contracts/src/HdcPrecompile.sol` and the Rust
precompile in `crates/hdc/chain/src/precompile.rs` use completely different
opcode assignments. A Solidity call to `HdcLib.hamming()` sends opcode `0x06`,
but Rust interprets `0x06` as `is_similar`. A call to `HdcLib.bind()` sends
opcode `0x05`, but Rust interprets `0x05` as `vector_id`.

**WHERE:**
- `contracts/src/HdcPrecompile.sol` -- Opcode table:
  `0x01=storeVector, 0x02=searchSimilar, 0x03=deleteVector, 0x04=bundle, 0x05=bind, 0x06=hamming, 0x07=permute`
- `crates/hdc/chain/src/precompile.rs` -- Opcode table:
  `0x01=hamming, 0x02=bind, 0x03=bundle, 0x04=permute, 0x05=vector_id, 0x06=is_similar`

**Consequence:** Every `HdcLib` call from Solidity either hits the wrong Rust
operation, sends an invalid opcode, or receives an incompatible return format.
`InsightBoard.submit()` cannot function because `searchSimilar()` (opcode
`0x02`) actually triggers `bind()` in Rust.

**FIX:**
1. Decide on one canonical dispatch scheme. PR #42 uses 4-byte function
   selectors (keccak of Solidity signatures); this is the recommended approach.
2. Update `HdcLib` to use the canonical selectors.
3. Update the Rust precompile dispatch to match.
4. Add a cross-language integration test that deploys InsightBoard against the
   real Rust precompile and verifies end-to-end operation.

**VERIFY:**
```bash
# After fix: Solidity and Rust opcodes must match
# Run the integration test:
cargo test -p kora-e2e test_solidity_precompile_integration
# Run Foundry tests WITH real precompile:
forge test --match-contract InsightBoardIntegrationTest -vvv
```

---

### AP-06: HashMap in iteration-sensitive paths

Multiple files use `HashMap` where iteration order can affect output
determinism. This is anti-pattern 1.2 manifested in the actual codebase.

**WHERE:**

| File | Field | Line | Impact |
|------|-------|------|--------|
| `crates/hdc/chain/src/index.rs` | `vectors: HashMap<B256, HdcVector>` | 14 | Search result ordering on ties |
| `crates/hdc/chain/src/index.rs` | `metadata: HashMap<B256, InsightMeta>` | 15 | Metadata lookup iteration |
| `crates/hdc/chain/src/wisdom.rs` | `submissions: HashMap<B256, WisdomSubmission>` | 39 | Resolution ordering for same-window submissions |
| `crates/hdc/core/src/knowledge/store.rs` | `entries: HashMap<[u8;32], KnowledgeEntry>` | 13 | `tick()` GC/promotion/demotion ordering |
| `crates/hdc/core/src/trust.rs` | `agents: HashMap<H256, AgentTrust>` | 73 | Iteration during batch operations |

**Consequence:** If any of these iteration paths produce outputs that affect
transactions, events, state roots, or replayed indexes, two validators using
different process seeds can diverge. The brute-force search in `index.rs` sorts
results by distance but ties on equal distance are broken by HashMap iteration
order, which is non-deterministic.

**FIX:**
Replace each `HashMap` with `BTreeMap` in the five locations listed above.
```rust
// Before:
use std::collections::HashMap;
vectors: HashMap<B256, HdcVector>,

// After:
use std::collections::BTreeMap;
vectors: BTreeMap<B256, HdcVector>,
```

**VERIFY:**
```bash
# After fix: no HashMap in consensus-adjacent HDC code
grep -rn "HashMap" crates/hdc/chain/src/ crates/hdc/core/src/knowledge/ crates/hdc/core/src/trust.rs \
  | grep -v "//.*HashMap" | grep -v test | grep -v "BTreeMap"
# Should return zero matches
cargo test -p kora-hdc -p kora-hdc-chain --all-features
```

---

### AP Summary Table

| ID | Anti-Pattern | Severity | Status |
|----|-------------|----------|--------|
| AP-01 | Two trust implementations competing | HIGH | OPEN -- must choose one |
| AP-02 | EMA double-application in TrustRegistry | MEDIUM | OPEN -- audit call sites |
| AP-03 | TestableInsightBoard copy-paste | MEDIUM | OPEN -- replace with mock precompile |
| AP-04 | Stored vs computed state in InsightBoard | MEDIUM | OPEN -- use computeState() in write paths |
| AP-05 | Precompile opcode mismatch | CRITICAL | OPEN -- blocks all Solidity/Rust integration |
| AP-06 | HashMap in iteration-sensitive paths | HIGH | OPEN -- replace with BTreeMap in 5 locations |
