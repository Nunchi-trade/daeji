# HDC Foundations — Vector Symbolic Architectures

## What is Hyperdimensional Computing?

Hyperdimensional Computing (HDC) is a computational framework built on a
single, surprising fact about high-dimensional geometry: **random vectors in
high-dimensional spaces are nearly orthogonal to each other.** This property,
sometimes called the "blessing of dimensionality," stands in contrast to
the well-known "curse of dimensionality" that plagues machine learning. Where
low-dimensional methods struggle as dimensions increase, HDC *exploits*
high dimensionality as a resource.

### The Geometry of High Dimensions

Consider a space of D-dimensional binary vectors (each component is 0 or 1).
If you generate two such vectors uniformly at random, each bit position
independently has a 50% chance of matching. The number of differing bits —
the Hamming distance — follows a binomial distribution with parameters D
and p=0.5. For our operating dimension D=10,240:

- **Expected Hamming distance:** D/2 = 5,120 bits differ
- **Standard deviation:** sqrt(D/4) = sqrt(2,560) ~ 50.6 bits
- **99.99% of random pairs** have Hamming distance in [5,120 - 4*50.6, 5,120 + 4*50.6] = [4,918, 5,322]

This concentration is extreme. Out of 10,240 bits, the distance between any
two random vectors is pinned to within ~2% of D/2. In normalized similarity
terms (1 - hamming/D), random pairs cluster tightly around 0.500 +/- 0.010
(95% of pairs) or +/- 0.020 (99.99% of pairs).

The implication: if you draw thousands of random vectors, they are all
*approximately equidistant* from each other. They are **quasi-orthogonal** —
not exactly orthogonal (true orthogonality requires at most D vectors in D
dimensions), but so close to orthogonal that they can serve the same purpose.
Each random vector is a unique, distinguishable atomic symbol with negligible
cross-talk.

### Concentration of Measure

This phenomenon is a special case of **concentration of measure**, one of the
deepest results in probability theory and geometry. The general principle:
in high dimensions, well-behaved functions of many independent random variables
concentrate sharply around their expected value.

For binary vectors, Hoeffding's inequality gives the precise tail:

```
P(|hamming(A,B) - D/2| > t) <= 2 * exp(-2t^2 / D)
```

For D=10,240 and t=200 (less than 2% deviation from expected):

```
P(|hamming(A,B) - 5120| > 200) <= 2 * exp(-2 * 40000 / 10240) ~ 2 * exp(-7.8) ~ 0.00082
```

This means that with probability >99.9%, any two random 10,240-bit vectors
differ in between 4,920 and 5,320 bit positions. You can generate millions
of random vectors and they all sit in this thin shell of quasi-orthogonality.

### The Johnson-Lindenstrauss Lemma

The theoretical backbone for why high-dimensional random projections work is
the **Johnson-Lindenstrauss (JL) lemma**:

> For any set of n points in high-dimensional space and any epsilon > 0,
> there exists a projection into D = O(log(n) / epsilon^2) dimensions that
> preserves all pairwise distances to within a factor of (1 +/- epsilon).

Johnson, W.B. and Lindenstrauss, J. (1984). "Extensions of Lipschitz
mappings into a Hilbert space." *Conference in Modern Analysis and
Probability*, Contemporary Mathematics, 26:189-206.

This lemma tells us two things:
1. **Sufficient dimension:** To faithfully represent n items, you need only
   D = O(log n) dimensions. For n = 10^6 items with epsilon = 0.1, D ~ 2,000
   suffices. Our D=10,240 provides generous headroom.
2. **Random projections work:** The projection matrix can be random — no
   careful optimization needed. This is why random binary vectors work as
   atomic symbols and why random projection preserves similarity structure.

### HDC as a Computing Paradigm

With quasi-orthogonal random vectors as atomic symbols, HDC builds complex
representations through three algebraic operations: **BIND** (association),
**BUNDLE** (superposition), and **PERMUTE** (sequencing). The resulting
"algebra of representation" was first articulated by Kanerva in his work
on Sparse Distributed Memory:

Kanerva, P. (1988). *Sparse Distributed Memory.* MIT Press.

The framework has since been developed under many names — Vector Symbolic
Architecture (VSA), holographic reduced representations, multiply-add-permute
codes — but the core idea is the same: represent concepts as points in
high-dimensional space, compose them algebraically, and retrieve them by
similarity search.

Key properties of the paradigm:
- **Distributed representation:** Information is spread across all D bits.
  No single bit is critical; damage to any subset degrades gracefully.
- **Fixed-width composition:** Binding two 10,240-bit vectors produces a
  10,240-bit vector. Bundling ten vectors produces a 10,240-bit vector.
  Representations do not grow as structures become more complex.
- **Similarity = semantic relatedness:** The Hamming distance between two
  vectors directly measures their conceptual similarity. Related items have
  distance < D/2; unrelated items have distance ~ D/2.
- **Noise tolerance:** Because the quasi-orthogonal gap is so wide (~101 sigma
  at D=10,240, since the gap from random is 0.5 and the standard deviation of
  random similarity is 1/(2*sqrt(D)) ~ 0.005), operations can tolerate
  substantial noise and still recover the correct answer.

---

## VSA Family Comparison

Five major VSA models have been proposed in the literature, each making
different algebraic and representational trade-offs:

| Model | Element Type | Bind | Bundle | Similarity | HW Efficiency |
|-------|-------------|------|--------|-----------|---------------|
| **BSC** (Binary Spatter Code) | {0,1} | XOR | Majority vote | Hamming | Excellent — bitwise ops |
| **MAP-B** (Multiply-Add-Permute Binary) | {-1,+1} | Hadamard (x) | Sum + threshold | Dot product | Good — SIMD multiply |
| **MAP-C** (MAP Complex) | e^{i*theta} | Hadamard (x) | Sum | Cosine | Moderate — complex arith |
| **HRR** (Holographic Reduced Representation) | R | Circular convolution | Sum | Cosine | Poor — FFT required |
| **FHRR** (Fourier HRR) | e^{i*theta} | Hadamard (x) | Sum | Cosine | Best dimension-efficiency |

### Full Citations

- **BSC:** Kanerva, P. (1994). "The Spatter Code for Encoding Concepts at
  Many Levels." *Proceedings of the International Conference on Artificial
  Neural Networks (ICANN)*, pp. 226-229. Springer.

- **MAP-B:** Gayler, R.W. (1998). "Multiplicative Binding, Representation
  Operators, and Analogy." *Analogical Connections: Proceedings of the IAAI
  Workshop on Analogical Reasoning.*

- **MAP-C:** Plate, T.A. (1991). "Holographic Reduced Representations:
  Convolution Algebra for Compositional Distributed Representations."
  *Proceedings of the 12th International Joint Conference on Artificial
  Intelligence (IJCAI)*, pp. 30-35. (The complex variant of MAP was
  introduced alongside HRR as an alternative algebraic framework.)

- **HRR:** Plate, T.A. (1995). "Holographic Reduced Representations."
  *IEEE Transactions on Neural Networks*, 6(3):623-641.

- **FHRR:** Plate, T.A. (2003). *Holographic Reduced Representation:
  Distributed Representation for Cognitive Structures.* CSLI Publications,
  Stanford University.

### Algebraic Comparison

All five models share the same abstract algebra: they support a binding
operation (associative, invertible, dissimilar to inputs), a bundling
operation (commutative, similar to inputs), and a permutation operation
(creates ordered sequences). The differences lie in the element domain and
the computational cost of each operation:

| Property | BSC | MAP-B | MAP-C | HRR | FHRR |
|----------|-----|-------|-------|-----|------|
| Element size | 1 bit | 1 bit + sign | 2 floats | 1 float | 2 floats |
| Bind cost | XOR (1 cycle) | Multiply (1 cycle) | Complex mult (4 cycles) | FFT + mult (D log D) | Complex mult (4 cycles) |
| Bundle cost | Vote (scan) | Sum + threshold | Sum (2 adds) | Sum (1 add) | Sum (2 adds) |
| Memory/vector | D/8 bytes | D/8 bytes | 8D bytes | 4D bytes | 8D bytes |
| Capacity/dim | ~sqrt(D) | ~sqrt(D) | ~D/log(D) | ~D/log(D) | ~D/log(D) |

### Why BSC?

Roko uses Binary Spatter Codes (BSC) for these reasons:

1. **Hardware acceleration.** XOR and popcnt are single-cycle instructions on all
   modern CPUs. No FPU needed. No SIMD shuffle complexity. The Intel `POPCNT`
   instruction computes the Hamming weight of a 64-bit word in 1 cycle with
   1-cycle latency. A full D=10,240 similarity comparison requires 160 XORs
   and 160 POPCNTs — roughly 25-50ns on modern hardware.

2. **Minimal memory.** 10,240 bits = 1,280 bytes per vector. Compare to HRR
   at 10,240 x f32 = 40,960 bytes (32x larger) or FHRR at 10,240 x complex64
   = 81,920 bytes (64x larger). In a system that may store millions of
   vectors, this 32-64x reduction is significant.

3. **Deterministic.** No floating-point rounding. XOR is XOR on every platform,
   every compiler, every architecture. The IEEE 754 standard permits different
   rounding modes and intermediate precision, meaning two validators computing
   the same HRR circular convolution could get different results. BSC
   eliminates this class of consensus bugs entirely.

4. **Composability preserved.** BSC has the same algebraic properties as other
   VSAs — bind is invertible (XOR is self-inverse), bundle is commutative,
   permute creates ordered sequences. Every algorithm designed for the
   abstract VSA algebra works identically on BSC.

5. **Proven at scale.** Google's TPU-based HDC work, IBM's NVSA (244x faster
   inference), and academic benchmarks all validate BSC at D>=10,000.

The trade-off: BSC has lower capacity per dimension than FHRR (~sqrt(D) vs ~D/log(D)).
But at D=10,240, this gives capacity ~100 items per bundle, which is sufficient
for cognitive operations (context windows are bounded). If future workloads
demand higher capacity, the algebra is the same — only the element domain
changes.

### BSC Limitations and Alternative Algebras

BSC's simplicity is a genuine strength — XOR is a single-cycle instruction,
deterministic across all platforms, and ideal for consensus. But that simplicity
carries a structural limitation that becomes visible in path-sensitive and
order-dependent reasoning tasks.

**The commutativity problem.** XOR binding is commutative: `A XOR B = B XOR A`.
This means that pure BSC binding cannot distinguish "A causes B" from "B causes A,"
or "founded_by -> CEO_of" from "CEO_of -> founded_by." The two encodings
produce identical vectors. For single-hop associations this is manageable (our
`encode_causal` already works around it with permute), but for multi-hop path
composition — where the *sequence* of relations matters — commutativity becomes
a deeper problem. A chain like `r1 XOR r2 XOR r3` is identical regardless of
the order in which the relations are applied, collapsing distinct reasoning
paths into the same point in hyperspace.

Our current system addresses this with the **permute-before-bind** pattern:
`rho^2(r1) XOR rho^1(r2) XOR rho^0(r3)` produces a position-aware encoding
where reordering the relations changes the result. This works, and is the
standard solution in the BSC literature. However, it introduces a two-step
protocol (permute then bind) where the algebra itself provides no ordering
guarantee — the ordering discipline is imposed by convention, not by the
operator's algebraic properties.

**GHRR: non-commutative binding by construction.** Generalized Holographic
Reduced Representations (GHRR) offer a fundamentally different approach.
Where BSC operates on single bits and FHRR on complex phasors (scalars on the
unit circle), GHRR replaces each scalar element with an m x m unitary matrix.
Binding becomes element-wise matrix multiplication:

```
H1 * H2 = [a1*b1, a2*b2, ..., aD*bD]    where ai, bi in U(m)
```

Since matrix multiplication is non-commutative (`a1*b1 != b1*a1` in general),
GHRR binding is inherently order-sensitive without requiring a separate permute
step. The parameter m controls the degree of non-commutativity: at m=1, GHRR
reduces exactly to FHRR (commutative); as m increases, the binding becomes
progressively more non-commutative, providing a tunable knob between capacity
and order-sensitivity.

GHRR also offers increased memorization capacity for nested and compositional
structures. Empirically, GHRR sustains high decoding accuracy at greater
nesting depths than FHRR, because the block-diagonal projection captures
richer cross-dimensional interactions than the purely diagonal projection
used by FHRR.

> Yeung, C., Zou, Z., and Imani, M. (2024).
> "Generalized Holographic Reduced Representations." *arXiv:2405.09689.*

**PathHD: GHRR for knowledge-graph reasoning.** The PathHD system (Liu et al.,
2025) demonstrates the practical value of non-commutative binding for multi-hop
knowledge graph reasoning. PathHD encodes relation paths as GHRR hypervectors
via left-to-right binding:

```
v_path = v_r1 * v_r2 * ... * v_rL    (GHRR bind, non-commutative)
```

This encoding naturally distinguishes "founded_by -> CEO_of" from "CEO_of ->
founded_by" without any permutation step. PathHD retrieves candidate paths
using a calibrated blockwise cosine similarity with top-K pruning, then passes
the results to a single LLM call for adjudication. On WebQSP, PathHD achieves
86.2% Hits@1, outperforming commutative alternatives: XOR binding scored 83.9%,
element-wise product 84.4%, and standard HRR 85.1%. The 2.3 percentage-point
gap between XOR and GHRR directly quantifies the cost of commutativity in
path-sensitive tasks.

> Liu, Y., Chung, W.Y., Chen, H., Yeung, C., and Imani, M. (2025).
> "Encoder-Free Knowledge-Graph Reasoning with LLMs via Hyperdimensional
> Path Retrieval." *arXiv:2512.09369.*

**Conjunctive block coding for structured graphs.** A related approach, CLOG
(Conjunctive block coding for hyperdimensional graph representation), uses
block encoding with masking schemes to improve memory capacity for sparse
graph representations. CLOG leverages HDC's variable binding in a block
structure that preserves approximate structural similarity beyond simple
edge correspondence — enabling graph reconstruction and link prediction
tasks that are difficult with flat BSC encodings. The block structure
provides a richer representational substrate for graphs where local
connectivity patterns carry semantic information.

> Zakeri, A., Zou, Z., Chen, H., Latapie, H., and Imani, M. (2024).
> "Conjunctive block coding for hyperdimensional graph representation."
> *Array*, 21, 100338. ScienceDirect.

**Recommendation for the current system.** BSC + rotate-permute is adequate
for our current architecture. We already layer permute for sequence encoding
(causal links, episodes, n-grams), and the permute-before-bind pattern
correctly handles the directionality requirements of the current knowledge
representation layer. The determinism, hardware efficiency, and consensus
compatibility of BSC remain strong advantages that outweigh the algebraic
elegance of GHRR for our present workload.

However, for future path-sensitive reasoning — multi-hop knowledge graph
queries, transaction sequence analysis, causal chain inference across
multiple hops — GHRR or block-coded VSA should be considered as an
alternative binding algebra. The key architectural signal is this: **BSC's
simplicity is a strength for consensus (pure XOR, bit-identical across all
validators) but a limitation for complex structured reasoning where the
binding operator itself must enforce ordering.** If the system evolves to
require deep compositional path queries, switching the binding algebra from
BSC to GHRR at the encoding layer — while preserving the same bundling and
similarity operations — would provide inherent non-commutativity without
the fragility of relying on permutation conventions.

---

## Core Algebra

All operations on D=10,240 bit vectors, stored as `[u64; 160]`.

### Bind (XOR) — Association

```
A XOR B = bitwise XOR of all 160 words
```

**Intuition:** Binding creates a new vector that represents the *association*
between two concepts. Think of it as creating a "key-value pair" or a
"role-filler" connection. The bound result is dissimilar to both inputs —
it is a new, unique point in the space that encodes the *relationship* rather
than either concept individually.

**Self-inverse property:** The most important property of XOR binding is that
it is its own inverse:

```
A XOR B XOR B = A XOR 0 = A
```

This means you can **unbind** by applying the same operation with the same
key. If you have a bound pair `C = A XOR B`, you recover A by computing
`C XOR B = A XOR B XOR B = A`. No separate "inverse" operation is needed.

**Worked example (8-bit vectors for illustration):**

```
Let A = 11010010
Let B = 01101001

BIND:   C = A XOR B
        C = 11010010
          XOR 01101001
          ----------
            10111011

UNBIND with B:  C XOR B
                10111011
            XOR 01101001
            ----------
                11010010  = A  (recovered exactly)

UNBIND with A:  C XOR A
                10111011
            XOR 11010010
            ----------
                01101001  = B  (recovered exactly)
```

Full properties:
- **Self-inverse:** `A XOR B XOR B = A` — unbinding recovers the original
- **Distributive over bundle:** `A XOR [B, C] ~ [A XOR B, A XOR C]` (approximate,
  because majority vote introduces quantization)
- **Dissimilar to inputs:** `sim(A XOR B, A) ~ 0.5` (quasi-orthogonal).
  Each bit has a 50% chance of flipping, making the result uncorrelated with
  either input.
- **Commutative:** `A XOR B = B XOR A`
- **Associative:** `(A XOR B) XOR C = A XOR (B XOR C)` — bindings can be
  composed in any order

Use: Associate two concepts. "Key-value" pairs, role-filler bindings,
causal links, negation.

### Bundle (Majority Vote) — Superposition

```
[A, B, C] = for each bit position, take majority of {A_i, B_i, C_i}
```

**Intuition:** Bundling creates a superposition — a single vector that is
simultaneously similar to all of its constituent vectors. It is the HDC
analog of a "set" or "bag." You can later test whether an item was in the
bundle by checking if it has high similarity with the bundled result.

**Tie-breaking rule:** For an even number of inputs, some bit positions will
have an equal number of 0s and 1s. Roko uses a deterministic tie-breaking
rule: **ties go to 0.** This ensures that every validator computing the same
bundle gets the same result, which is critical for consensus. Alternative
tie-breaking rules (random, alternating, lowest-bit-of-position) exist in
the literature, but determinism is non-negotiable in a distributed system.

**The BundleAccumulator pattern:** In practice, bundling is implemented not
by collecting all vectors and voting at the end, but by maintaining a running
tally. Each bit position has an `i32` accumulator that counts the excess of
1s over 0s:

```
struct BundleAccumulator {
    counts: [i32; 10240],   // one counter per bit position
    n: usize,               // number of vectors added
}

impl BundleAccumulator {
    fn new() -> Self {
        Self { counts: [0i32; 10240], n: 0 }
    }

    fn add(&mut self, v: &HdcVector) {
        for i in 0..10240 {
            if v.bit(i) == 1 {
                self.counts[i] += 1;
            } else {
                self.counts[i] -= 1;
            }
        }
        self.n += 1;
    }

    /// Number of vectors added so far.
    fn count(&self) -> usize {
        self.n
    }

    /// Finalize: majority vote across all bit positions.
    /// Ties (count == 0) resolve to 0 (deterministic).
    fn finalize(&self) -> HdcVector {
        let mut result = HdcVector::zero();
        for i in 0..10240 {
            if self.counts[i] > 0 {
                result.set_bit(i, 1);
            }
            // counts[i] == 0 -> bit stays 0 (deterministic tie-break)
            // counts[i] < 0 -> bit stays 0
        }
        result
    }

    // Legacy alias for finalize() -- used in older code.
    fn finish(&self) -> HdcVector { self.finalize() }
}
```

This pattern has two advantages: (1) it processes vectors in a streaming
fashion without storing them all in memory, and (2) the i32 counters enable
**weighted bundling** — add a vector multiple times to increase its influence.

**Capacity:** Each additional item bundled adds noise to every other item's
signal. At each bit position, the signal from one constituent is a single
vote (magnitude 1), while the other N-1 constituents contribute noise with
standard deviation sqrt(N-1). The **per-bit** signal-to-noise ratio is:

```
SNR_per_bit = 1 / sqrt(N - 1)
```

When aggregated over all D bit positions, the expected normalized similarity
between the bundle and any constituent exceeds the random baseline (0.5) by
approximately `1 / (sqrt(2*pi) * sqrt(N-1))`. The **aggregate** SNR — the
ratio of this gap to the standard deviation of random similarity
(1/(2*sqrt(D))) — is proportional to `sqrt(D) / sqrt(N-1)`.

For practical bundling at D=10,240, retrieval remains reliable up to
approximately **N ~ 100 items** for codebooks of moderate size (~1,000
entries). The exact limit depends on the codebook size and the desired
confidence level; see the Capacity Theory section below.

Thomas, A., Dasgupta, S., and Rosing, T. (2021). "A Theoretical Perspective
on Hyperdimensional Computing." *Journal of Artificial Intelligence
Research*, 72, 215-249. arXiv:2010.07426.

Properties:
- **Similar to all inputs:** `sim([A, B, C], A) > 0.5`
- **Capacity:** ~sqrt(D) ~ 100 items before retrieval degrades below threshold
- **Commutative:** Order does not matter
- **Weighted bundle:** Repeat a vector K times to give it Kx weight

Use: Merge multiple concepts into one. Sets, bags, episode summaries,
evidence aggregation.

#### The Bundling Saturation Problem

Bundling is the most capacity-constrained operation in BSC. Understanding its
failure mode is essential for designing systems that remain reliable.

**The mechanism of saturation.** When you bundle N vectors via majority vote,
each target vector's signal is a single vote per bit position. The other N-1
vectors act as independent noise sources. At each bit position, the vote for
any particular constituent vector has magnitude 1, while the sum of all other
votes behaves as a random walk with standard deviation sqrt(N-1). As N grows,
the noise term dominates:

```
After bundling N vectors:
  Signal (any one constituent):  1 vote
  Noise (all other constituents): ~sqrt(N-1) votes (std dev)
  SNR per bit position:          1 / sqrt(N-1)

Expected normalized similarity between bundle and any constituent:
  E[sim] = 0.5 + 0.5 * erf(1 / sqrt(2*(N-1)))
         ~ 0.5 + 1 / (sqrt(2*pi) * sqrt(N-1))    for large N

Aggregate SNR (gap / std of random similarity):
  gap = E[sim] - 0.5 ~ 1 / (sqrt(2*pi) * sqrt(N-1))
  std_random = 1 / (2*sqrt(D))
  SNR_aggregate = gap / std_random ~ 2*sqrt(D) / (sqrt(2*pi) * sqrt(N-1))
                ~ sqrt(2*D/pi) / sqrt(N-1)

At D=10,240, N=101:
  E[sim] ~ 0.540, gap ~ 0.040
  SNR_aggregate ~ 8.1 sigma above random
  Retrieval is reliable for codebooks up to ~10,000 items.

At D=10,240, N=300:
  E[sim] ~ 0.523, gap ~ 0.023
  SNR_aggregate ~ 4.7 sigma above random
  Retrieval still works for moderate codebooks (~1,000 items).
```

The practical capacity depends on both the number of bundled items N and the
codebook size M. The rule of thumb **N_max ~ sqrt(D)** (approximately 100 for
D=10,240) reflects the point where the per-bit SNR becomes small enough that
retrieval requires increasingly favorable conditions (small codebook, no other
noise sources). Beyond this point, the system is not broken but is operating
with thinner margins.

At the saturation limit, every bit position in the bundled result has roughly
a 50/50 split between 0s and 1s -- the majority vote converges toward random
noise. The bundled vector becomes quasi-orthogonal to *all* of its constituents,
not just to unrelated vectors. At that point, no amount of similarity search
can recover the original components. This is the fundamental capacity wall,
and it is a hard limit of binary majority-vote bundling.

**Why naive binary majority is insufficient for recent HDC systems.** Nearly
every recent HDC paper with strong empirical results uses some form of
weighted or thresholded bundling, not raw majority vote. The raw majority
approach treats all bundled vectors as equally important, discards all
magnitude information when binarizing, and has no mechanism to prioritize
recent or relevant information. The literature has converged on three
principal alternatives:

**1. Weighted bundling.** Rather than adding each vector once, add vectors
multiple times in proportion to their importance. A vector added K times
receives K votes at each bit position, effectively amplifying its signal
relative to the noise floor:

```
Standard bundle:   signal = 1,  noise ~ sqrt(N-1)
Weighted (weight K): signal = K,  noise ~ sqrt(sum_of_other_weights)
```

The `BundleAccumulator` with `i32` counters already supports this directly --
calling `add()` multiple times for the same vector, or using `add_weighted(v, k)`
if implemented, gives that vector K times the influence in the final majority
vote. The `DecayingBundleAccumulator` pattern (used in some roko subsystems)
applies exponentially decreasing weights to older additions, ensuring that
recent vectors dominate the bundle while older vectors gracefully fade.
This is critical for temporal context: a context window that bundles the
last 50 events should weight the most recent events more heavily.

**2. Thresholded bundling.** Only include a vector in the bundle if its
similarity to the current accumulator exceeds a threshold. This prevents
near-orthogonal (irrelevant) vectors from adding noise without contributing
signal. Variants include:

- **Pre-filter threshold:** Check `sim(v, current_bundle) > t` before adding.
  Reject vectors that are too dissimilar to the emerging consensus.
- **Post-bundle pruning:** After bundling N vectors, check each constituent's
  similarity to the result and remove those below threshold, then re-bundle.
- **Adaptive threshold:** Start with a low threshold and raise it as the
  bundle fills, preserving capacity for the most important items.

**3. Normalized bundling.** Periodically normalize the accumulator to prevent
any single dimension from dominating. In the binary setting, this means
periodically binarizing the accumulator (threshold at 0, emit 1 or 0) and
restarting the counters. This is equivalent to "snapshotting" the bundle
and then treating the snapshot as the first vector in a new bundle:

```
// Normalized bundling: snapshot every K additions
if num_vectors % K == 0 {
    let snapshot = self.finalize();  // binarize current state
    self.counts = [0i32; 10240];     // reset counters
    self.add(&snapshot);             // snapshot becomes first vote
    self.num_vectors = 1;
}
```

This bounds the effective load factor to K, preventing saturation regardless
of how many total vectors are added. The trade-off is lossy compression: the
snapshot cannot distinguish its individual constituents, so late additions
are biased toward the already-accumulated consensus.

Neubert, P. and Schubert, S. (2021). "Hyperdimensional Computing as a
Framework for Systematic Aggregation of Image Descriptors." *CVPR 2021.*

Schmuck, M., Benini, L., and Rahimi, A. (2019). "Hardware Optimizations of
Dense Binary Hyperdimensional Computing: Rematerialization of Hypervectors,
Binarized Bundling, and Combinational Associative Memory." *ACM Journal on
Emerging Technologies in Computing Systems (JETC)*, 15(4):1-25.

#### The MAP-I Alternative: Integer Accumulation

The MAP-I (Multiply-Add-Permute, Integer variant) vector symbolic architecture
avoids binary saturation entirely by operating directly in the integer domain.
Instead of binarizing votes at the end, MAP-I keeps the integer accumulator
as the representation itself:

```
BSC bundle:     accumulate i32 votes  ->  binarize at threshold  ->  binary vector
MAP-I bundle:   accumulate i32 votes  ->  keep integer vector    ->  integer vector
```

Because MAP-I never binarizes, it preserves the full magnitude information in
the accumulator. A vector added 10 times has 10x the influence, and this
information persists through subsequent operations (unlike BSC, where
binarization discards magnitude after each bundle). The capacity of MAP-I
scales linearly with the integer range, not as sqrt(D).

MAP-I has become the default in several FPGA-based HDC accelerators precisely
because integer accumulation maps naturally to hardware adder trees, avoids
the saturation-and-rebinarize cycle, and supports wider dynamic range. The
trade-off is that MAP-I loses the extreme compactness of BSC (1 bit per
dimension) and the simplicity of XOR binding. For roko, BSC remains the right
choice because hardware efficiency and consensus determinism outweigh the
capacity benefits of MAP-I -- but the `BundleAccumulator` with `i32` counters
is effectively MAP-I during the accumulation phase, only collapsing to BSC
at `finalize()`.

Kleyko, D., Rachkovskij, D.A., Osipov, E., and Rahimi, A. (2022). "A
Survey on Hyperdimensional Computing aka Vector Symbolic Architectures,
Part I: Models and Data Transformations." *ACM Computing Surveys*, 55(6),
Article 130.

Schlegel, K., Neubert, P., and Protzel, P. (2022). "A comparison of Vector
Symbolic Architectures." *Artificial Intelligence Review*, 55(6), 4523-4555.

#### Sparse Binary Distributed Representations (SBDR)

An alternative to dense BSC that dominates when memory footprint matters more
than compute. In SBDR (also called BSDC-S or BSDC-SEG), vectors have the
same dimensionality D but only a small fraction of bits are set to 1 (the
"activity ratio" or "density," typically 1-5% rather than BSC's 50%).

Key properties:
- **Memory:** Sparse vectors can be stored as lists of set-bit indices rather
  than full bitmaps, reducing storage from D/8 bytes to ~k * log2(D) bits
  where k is the number of active bits. At 2% density and D=10,240, this is
  ~205 active bits, storable in ~205 * 14 bits ~ 360 bytes vs BSC's 1,280.
- **Similarity:** Uses overlap (set intersection size) rather than Hamming
  distance. Two random SBDR vectors have expected overlap near zero, so any
  non-trivial overlap indicates genuine similarity.
- **Bundling:** OR-based bundling (set union) followed by "thinning" -- random
  deletion of active bits to restore the target density. Without thinning,
  repeated bundling increases density toward 50%, converging back to dense BSC.
- **Binding:** Segment-wise permutation or context-dependent thinning (CDT)
  rather than XOR, since XOR on sparse vectors produces dense vectors.

SBDR is relevant to roko's design space for two reasons. First, at very large
index sizes (millions of vectors), the 3-4x memory reduction from sparse
storage may justify the more complex binding and thinning operations. Second,
the thinning operation after bundling is itself a form of normalized bundling --
it prevents the density drift that is the sparse analog of binary saturation.
The current architecture does not use SBDR, but the algebra is compatible
with a future migration path if memory pressure demands it.

Rachkovskij, D.A. and Kussul, E.M. (2001). "Binding and Normalization of
Binary Sparse Distributed Representations by Context-Dependent Thinning."
*Neural Computation*, 13(2), 411-452.

#### Practical Impact for Roko

The `BundleAccumulator` with `i32` vote counters already implements the
core mechanism of weighted bundling. The counters preserve full vote magnitude
during accumulation, and the `finalize()` step applies a threshold (at 0)
to collapse back to BSC. This means:

1. **Weighted bundling is free.** Call `add()` multiple times or implement
   `add_weighted()` with a multiplier. The i32 range (+-2 billion) provides
   effectively unlimited dynamic range.
2. **The saturation wall applies at `finalize()`.** Even with perfect integer
   accumulation, the binarization step loses magnitude information. If 200
   vectors are bundled with equal weight, the finalized binary vector is
   unreliable. The sqrt(D) ~ 100 limit is the capacity of the *binarized
   output*, not of the accumulator itself.
3. **Consumers must respect the limit.** Any subsystem that bundles more than
   ~100 items at D=10,240 should either: (a) use weighted bundling to ensure
   important items dominate, (b) apply normalized bundling with periodic
   snapshots, or (c) split the bundle into multiple vectors (e.g., segment
   by time window).
4. **The DecayingBundleAccumulator pattern** (exponentially down-weighting
   older additions) is the recommended approach for temporal contexts. It
   naturally limits the effective load factor because old items' votes decay
   below the noise floor, keeping the effective N well below the capacity wall.

### Permute (Cyclic Rotation) — Sequence

```
rho(A) = rotate all 160 words left by 1 bit (with carry across words)
rho^k(A) = rotate left by k bits
```

**Intuition:** Permutation takes a vector and produces a new vector that is
quasi-orthogonal to the original — but in a *deterministic, invertible* way.
Unlike binding (which requires a second vector), permutation transforms a
single vector into a new, unrelated one. The key use is **sequence encoding:**
by permuting a vector differently depending on its position, you create
position-aware representations.

**How permute creates quasi-orthogonal vectors:** A cyclic rotation by 1 bit
shifts every bit to a new position. Since the original vector is random, the
bit at position i has no correlation with the bit at position i+1. Therefore,
`rho(A)` is quasi-orthogonal to A, and `rho^k(A)` is quasi-orthogonal to
`rho^j(A)` for any j != k. You get up to D distinct quasi-orthogonal vectors
from a single starting vector — one for each rotation amount.

**Position-dependent representations:** To encode a sequence [x, y, z] with
position information, permute each element by its position index:

```
seq = rho^2(x) XOR rho^1(y) XOR rho^0(z)
```

Now `seq` encodes not just *which* items are present, but *where* they appear.
`rho^2(x) XOR rho^1(y)` is different from `rho^2(y) XOR rho^1(x)` because
permutation breaks the commutativity of XOR.

**Worked example (8-bit vectors):**

```
Let x = 11010010

rho^1(x) = 10100101    (rotated left by 1)
rho^2(x) = 01001011    (rotated left by 2)

sim(x, rho^1(x)):
  x         = 11010010
  rho^1(x)  = 10100101
  XOR       = 01110111  → popcount = 6, hamming = 6/8 = 0.75
  sim = 1 - 0.75 = 0.25   (dissimilar, as expected for small D)

For D=10,240, sim(x, rho^k(x)) ~ 0.500 for any k != 0.
```

Properties:
- **Dissimilar to input:** `sim(rho(A), A) ~ 0.5`
- **Invertible:** `rho^{-1}(rho(A)) = A` (rotate right undoes rotate left)
- **Breaks commutativity:** `bind(rho(A), B) != bind(rho(B), A)`
- **Composition:** `rho^j(rho^k(A)) = rho^{j+k}(A)` — rotations compose additively

Use: Encode ordered sequences. Position-aware representations. N-gram encoding.

### Similarity (Hamming Distance)

```
sim(A, B) = 1.0 - (hamming_distance(A, B) / D)
```

Where `hamming_distance = popcount(A XOR B)` — count differing bits.

**Intuition:** Similarity is the fundamental retrieval operation. Given a
query vector, find the closest match in a codebook or memory. Because random
vectors concentrate around sim ~ 0.5, any similarity significantly above 0.5
indicates a genuine match.

**Normalized similarity:** The formula `1 - hamming/D` maps Hamming distance
to a [0, 1] similarity score:
- **1.0** = identical vectors (hamming = 0)
- **0.5** = orthogonal / unrelated (hamming = D/2, the random baseline)
- **0.0** = complementary vectors (hamming = D, every bit differs)

> **CONSENSUS SAFETY:** The division `hamming / D` produces a floating-point
> value. On-chain code MUST NOT use this normalized similarity. Use the raw
> `hamming_distance()` (u32) for all on-chain comparisons, thresholds, and
> ranking. The normalized similarity function is a convenience for off-chain
> display, logging, and local scoring only. On-chain threshold checks should
> compare `hamming_distance < THRESHOLD_HAMMING` (integer comparison), not
> `similarity > 0.526` (float comparison). For D=10,240, the equivalent
> integer threshold is `THRESHOLD_HAMMING = (1.0 - 0.526) * 10240 = 4854`.

**The resonance threshold:** For D=10,240, the standard deviation of random
similarity is 1/(2*sqrt(D)) ~ 0.00494. Roko uses a resonance threshold of
**0.526**, which is approximately 5.26 standard deviations above the random
mean of 0.5:

```
threshold = 0.5 + 5.26 * (1/(2*sqrt(D)))
          = 0.5 + 5.26 * 0.00494
          = 0.5 + 0.026
          = 0.526
```

At 5.26 sigma, the false positive rate (a random vector exceeding threshold)
is vanishingly small -- approximately 7e-8. For a codebook of 1,000 items,
the expected number of false matches per query is ~0.00007 -- essentially zero.
This makes the threshold extremely conservative for filtering noise.

Performance: `popcount` over 160 u64s takes ~25-50 CPU cycles on modern x86
with hardware POPCNT. At 3 GHz, that is ~8-17ns per comparison. This means
brute-force search over 100,000 vectors takes ~1ms — fast enough for
real-time cognitive operations.

---

## Capacity Theory

Understanding the capacity limits of HDC is critical for system design.
Capacity determines how many items can be stored, how complex structures
can be before they degrade, and ultimately whether D=10,240 is sufficient.

### Bundle Capacity

Bundle capacity has two distinct measures: **per-bit accuracy** (what
fraction of bits in the bundled vector correctly reflect each constituent)
and **retrieval accuracy** (whether nearest-neighbor lookup in a codebook
recovers the correct constituent). Retrieval accuracy depends on both N
(items bundled) and D (dimensionality), while per-bit accuracy depends
only on N.

**Per-bit analysis.** When N vectors are bundled by majority vote, the
probability that a given bit position in the result matches the
corresponding bit of any one constituent is:

```
P(bit correct) = Phi(1 / sqrt(N-1))
               = 0.5 + 0.5 * erf(1 / sqrt(2*(N-1)))
```

where Phi is the standard normal CDF. This follows because the constituent
contributes a signal of +1 while the other N-1 vectors contribute noise
with standard deviation sqrt(N-1). Note that per-bit accuracy depends only
on N, not on D.

**Retrieval analysis.** For retrieval from a codebook of M items, the
relevant quantity is the **aggregate signal-to-noise ratio** — the gap
between the expected similarity to the correct constituent and the random
baseline (0.5), divided by the standard deviation of random similarity:

```
gap = E[sim(bundle, constituent)] - 0.5
    = 0.5 * erf(1 / sqrt(2*(N-1)))
    ~ 1 / (sqrt(2*pi) * sqrt(N-1))           for large N

std_random = 1 / (2*sqrt(D))

SNR_aggregate = gap / std_random ~ sqrt(2*D/pi) / sqrt(N-1)
```

For reliable retrieval, the aggregate SNR must be large enough that no
wrong codebook entry is likely to exceed the correct entry's similarity.
The probability of a retrieval error from M items is approximately:

```
P(retrieval error) ~ M * Phi(-SNR_aggregate)
```

**Derivation.** The similarity between the bundle and any *wrong* codebook
entry is approximately Gaussian with mean 0.5 and standard deviation
sigma = 1/(2*sqrt(D)). The similarity to the *correct* entry has mean
0.5 + gap. A retrieval error occurs when any wrong entry has higher
similarity than the correct entry. By a union bound over M-1 wrong entries,
each independently exceeding the correct entry's expected similarity with
probability Phi(-gap/sigma) = Phi(-SNR_aggregate), the total error
probability is at most (M-1) * Phi(-SNR_aggregate) ~ M * Phi(-SNR_aggregate).
This bound is tight when M is moderate and SNR is large (the dominant
failure mode is a single wrong entry crossing the threshold, not multiple).

Thomas, A., Dasgupta, S., and Rosing, T. (2021). "A Theoretical Perspective
on Hyperdimensional Computing." *Journal of Artificial Intelligence
Research*, 72, 215-249. arXiv:2010.07426.

**Concrete values for D=10,240, M=1,000 codebook:**

| N (items bundled) | Per-bit accuracy | E[sim] | Aggregate SNR | P(retrieval error) |
|-------------------|------------------|--------|---------------|-------------------|
| 10 | 63.1% | 0.631 | 26.4 sigma | ~0 |
| 30 | 57.4% | 0.574 | 14.9 sigma | ~0 |
| 50 | 55.7% | 0.557 | 11.5 sigma | ~0 |
| 100 | 54.0% | 0.540 | 8.1 sigma | ~10^-13 |
| 200 | 52.8% | 0.528 | 5.7 sigma | ~5 x 10^-6 |
| 300 | 52.3% | 0.523 | 4.7 sigma | ~0.002 |

For roko's use case (context assembly typically selects 10-30 items),
D=10,240 provides overwhelming headroom. Even bundling 100 items, retrieval
from a 1,000-entry codebook has an error probability below 10^-12.

**The sqrt(D) rule of thumb.** The widely cited limit N_max ~ sqrt(D)
(approximately 100 for D=10,240) reflects the point where the per-bit SNR
(1/sqrt(N-1)) becomes small enough that individual bits are only marginally
better than random. However, because the aggregate SNR benefits from
summing over all D bits, retrieval can remain reliable well beyond this
per-bit threshold — the aggregate signal is amplified by a factor of
sqrt(D). The sqrt(D) rule is a useful conservative guideline, but the
actual limit depends on the codebook size and the required retrieval
confidence.

### Record Capacity (Role-Filler Pairs)

Structured records use binding and bundling together:

```
record = bind(role_1, filler_1) XOR bind(role_2, filler_2) XOR ... XOR bind(role_K, filler_K)
```

Each bound pair contributes one "item" to the bundle, so the theoretical
bundle capacity (~sqrt(D) ~ 100) is an upper bound. However, structured
records have **lower practical capacity** than simple bundles because the
unbinding step introduces additional noise: the noise from other bound pairs
must be compared against the full codebook, not just evaluated for
similarity to a known target.

**Why records have lower capacity than bundles.** To retrieve filler_k from
a record, you compute `record XOR role_k`. This produces `filler_k XOR
noise`, where `noise = bind(role_1, filler_1) XOR ... XOR bind(role_{k-1},
filler_{k-1}) XOR bind(role_{k+1}, filler_{k+1}) XOR ... XOR bind(role_K,
filler_K)` -- the XOR of K-1 random-looking bound pairs. Each noise term
is quasi-orthogonal to every codebook vector, but the query result must be
compared against the *entire* filler codebook (size M) via nearest-neighbor
search. In a simple bundle, the "correct" entry has expected similarity
~0.5 + gap, and wrong entries have expected similarity ~0.5. In a record,
the unbinding noise has the same statistics as bundle noise (K-1 interfering
terms), but the noise is correlated across the query -- it is the *same*
noise vector added to every codebook comparison. This correlation slightly
increases the effective false-positive rate compared to independent noise
in pure bundling.

In practice:

- **5-9 role-filler pairs** can be reliably stored and retrieved per record.
- Beyond ~12 pairs, retrieval accuracy drops below 90% even at D=10,240.

Frady, E.P., Kleyko, D., and Sommer, F.T. (2018). "A Theory of Sequence
Indexing and Working Memory in Recurrent Neural Networks." *Neural
Computation*, 30(6):1449-1513.

### Sequence Capacity

Sequences encoded with permutation-based position markers degrade as length
increases, because each position-permuted element becomes an additional noise
source for every other element. The capacity is similar to bundle capacity
(~sqrt(D) elements) but with faster degradation due to the interaction
between permutation and binding:

- For position-only encoding (permute + bundle): ~sqrt(D) ~ 100 positions
- For n-gram encoding (permute + bind + bundle): capacity decreases with
  n-gram order; trigrams support ~60-80 elements at D=10,240
- For deep recursive binding (nested bind chains): each level of nesting
  roughly halves the effective capacity

### Index Capacity

The index (not the vectors themselves) scales differently:

| Vector count | Search algorithm | Memory | Query time |
|-------------|-----------------|--------|-----------|
| < 1K | Brute-force | ~1.3 MB | ~50us |
| 1K - 100K | Brute-force | ~130 MB | ~5ms |
| 100K - 1M | HNSW | ~200 MB | ~0.1ms |
| 1M - 10M | HNSW + partitioning | ~2 GB | ~0.5ms |

---

## Encoding Patterns

### Atomic Symbols

Each atomic concept gets a random D-bit vector from a seeded PRNG:

```rust
// CONSENSUS-SAFE: ChaCha20 is a deterministic PRNG. Given the same seed,
// all validators produce identical vectors. No floats, no platform-dependent
// behavior. Safe for on-chain use.
fn random_vector(seed: u64) -> HdcVector {
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let mut v = [0u64; 160];
    for w in &mut v {
        *w = rng.gen();
    }
    HdcVector(v)
}
```

The item memory maps concept names to their random vectors. To convert a
symbol name (an arbitrary string like `"ROLE_insight"` or `"ETH_USDC"`)
into its seed, hash the name:

```rust
// CONSENSUS-SAFE: Deterministic hash -> deterministic seed -> deterministic
// vector. Same symbol name produces same vector on all validators.
/// Deterministic symbol-to-vector mapping. The same symbol name always
/// produces the same vector on any validator.
fn symbol_vector(name: &str) -> HdcVector {
    // FNV-1a hash of the symbol name -> u64 seed
    let seed = fnv1a_hash(name.as_bytes());
    random_vector(seed)
}
```

This ensures that (a) the same symbol always gets the same vector across
all validators, and (b) different symbols get quasi-orthogonal vectors
(because ChaCha20 with different seeds produces independent random
bitstreams). No coordination or pre-shared table is required -- any node
can derive any symbol's vector on demand from its name alone.

The seed-based approach means vectors are **lazily generated:** you do not
need to store a table of all possible atomic vectors. Given a concept's
identifier, any node can recompute its vector on demand. The ChaCha20 PRNG
ensures cryptographic-quality randomness (uniform bit distribution,
independence across seeds).

### Role-Filler Binding

The role-filler binding pattern implements what Smolensky called **Tensor
Product Variable Binding** — a method for representing structured symbolic
information in distributed vectors:

Smolensky, P. (1990). "Tensor Product Variable Binding and the Representation
of Symbolic Structures in Connectionist Systems." *Artificial Intelligence*,
46(1-2):159-216.

In the tensor product framework, a structured representation like
{name: Alice, role: engineer} is encoded as:

```
record = bind(ROLE_name, FILLER_alice) XOR bind(ROLE_role, FILLER_engineer)
```

where ROLE_name, ROLE_role, FILLER_alice, FILLER_engineer are all random
atomic vectors from the item memory.

**Querying:** To retrieve the filler for a given role, bind the record with
the role vector:

```
query = record XOR ROLE_name
      = [bind(ROLE_name, FILLER_alice) XOR bind(ROLE_role, FILLER_engineer)] XOR ROLE_name
      = FILLER_alice XOR [bind(ROLE_role, FILLER_engineer) XOR ROLE_name]
                                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                                        this term is quasi-orthogonal noise
      ~ FILLER_alice   (with high similarity)
```

The query result is not exactly FILLER_alice — it is FILLER_alice plus noise
from the other role-filler pairs. But the noise is quasi-orthogonal (expected
similarity to any codebook vector ~ 0.5), so the nearest-neighbor lookup in
the codebook cleanly recovers the correct filler.

**Worked example (8-bit vectors):**

```
Codebook (item memory):
  ROLE_name   = 11001010
  ROLE_age    = 01110100
  FILLER_bob  = 10011001
  FILLER_25   = 11100011

Step 1: Bind each role-filler pair
  pair_1 = ROLE_name XOR FILLER_bob
         = 11001010 XOR 10011001 = 01010011
  pair_2 = ROLE_age  XOR FILLER_25
         = 01110100 XOR 11100011 = 10010111

Step 2: Bundle (majority vote, 2 vectors → tie-break to 0)
  For each bit position, take majority of {pair_1, pair_2}:
    pair_1 = 0 1 0 1 0 0 1 1
    pair_2 = 1 0 0 1 0 1 1 1
    vote:    T T 0 1 0 T 1 1    (T = tie → 0)
  record = 00010011

Step 3: Query for "name" — unbind with ROLE_name
  query = record XOR ROLE_name
        = 00010011 XOR 11001010 = 11011001

Step 4: Compare query to all fillers
  sim(query, FILLER_bob) = 1 - hamming(11011001, 10011001)/8
                         = 1 - 1/8 = 0.875   ← MATCH
  sim(query, FILLER_25)  = 1 - hamming(11011001, 11100011)/8
                         = 1 - 4/8 = 0.500

  Nearest neighbor: FILLER_bob. Correct!
```

Note: at D=8 the margin is slim (0.875 vs 0.500). At D=10,240, the gap
between the correct match (sim ~ 0.75 for 2-item records) and noise
(sim ~ 0.50) is hundreds of standard deviations — essentially impossible
to confuse.

### Record Encoding (Structured Data)

Structured data as bound role-filler pairs, bundled:

```
record = bundle([
    bind(ROLE_name, filler_name),
    bind(ROLE_age, filler_age),
    bind(ROLE_action, filler_action),
])
```

Query: `bind(record, ROLE_name)` ~ `filler_name` (similarity > threshold)

### Sequence Encoding (N-gram)

Ordered sequences using permutation:

```
trigram(a, b, c) = bind(rho^2(a), bind(rho^1(b), c))
text = bundle([trigram_1, trigram_2, ..., trigram_n])
```

This captures word order while allowing fuzzy matching. The permutation
encodes position: `rho^2(a)` marks "a" as being two positions from the end,
`rho^1(b)` marks "b" as one position from the end, and `c` is at the final
position. Because `rho^k(a)` is quasi-orthogonal to `rho^j(a)` for k != j,
each position is distinctly encoded.

### Graph Encoding

Directed edges as permuted bindings:

```
edge(A -> B) = bind(rho(A), B)
graph = bundle([edge_1, edge_2, ..., edge_n])
```

The permutation ensures `A -> B != B -> A`. This encoding supports:
- **Edge query:** Does edge A -> B exist? Check `sim(graph, bind(rho(A), B)) > threshold`
- **Neighbor query:** What does A point to? Compute `graph XOR rho(A)` and
  find nearest codebook match (returns B if edge exists)
- **Reverse query:** What points to B? Compute `rho^{-1}(graph XOR B)` and
  find nearest match (returns A)

---

## Resonator Networks

### The Factorization Problem

A fundamental challenge in HDC: given a composed vector — say, a bound pair
`C = A XOR B` — how do you determine *which* A and *which* B were used to
create it? Simple unbinding requires knowing one of the factors. But what if
you know neither, and only know that A and B each came from known codebooks?

This is the **factorization problem**, and it is analogous to the cocktail
party problem in signal processing: separate a superposition back into its
components.

### Simple Cleanup Memory

The naive approach: compare the composed vector against every possible
combination. For two codebooks of size N each, this requires N^2 comparisons.
Worse, for K factors from K codebooks, the search space is N^K — exponential
in the number of factors.

An alternative is **threshold-based cleanup:** unbind with every vector in
one codebook, and check if the result is close to any vector in the other
codebook. This reduces the search to 2N comparisons for two factors, but it
only works when the number of bundled items is small (< sqrt(D)).

### Resonator Network Algorithm

Resonator networks provide a dramatically more efficient solution:

Frady, E.P., Kent, S.J., Olshausen, B.A., and Sommer, F.T. (2020).
"Resonator Networks, 1: An Efficient Solution for Factoring High-Dimensional,
Distributed Representations of Data Structures." *Neural Computation*,
32(12):2311-2331.

The algorithm works by iterative convergence, similar to the EM algorithm or
power iteration:

**Setup:** Given a composed vector `S` and K codebooks {C_1, C_2, ..., C_K},
find factors {f_1, f_2, ..., f_K} such that `f_1 XOR f_2 XOR ... XOR f_K ~ S`.

**Algorithm:**

```
Initialize: x_k = random vector for each factor k = 1..K

Repeat until convergence:
  For each factor k:
    1. Compute "other factors" product:
       others_k = x_1 XOR x_2 XOR ... XOR x_{k-1} XOR x_{k+1} XOR ... XOR x_K

    2. Unbind from composed vector:
       estimate_k = S XOR others_k

    3. Project onto codebook (cleanup):
       x_k = argmax_{c in C_k} sim(estimate_k, c)
       (select the codebook vector most similar to the estimate)

  Check convergence:
    If x_1 XOR x_2 XOR ... XOR x_K ~ S (similarity > threshold), done.
```

**Intuition:** Each factor module "assumes" the current estimates of the
other factors are correct, unbinds them from the composed vector, and looks
up the best match in its own codebook. As each module improves its estimate,
it provides better context for the other modules, creating a positive
feedback loop that converges to the correct factorization.

**Bipolarization step:** In practice, the codebook projection is often
replaced or augmented with a "bipolarize" step (for BSC: threshold to the
nearest binary vector), which acts as a soft cleanup. This is faster than
full codebook search and sufficient when the estimates are already close.

### Capacity

The key result: resonator networks can resolve up to **N^2** bound pairs from
codebooks of size N, compared to ~sqrt(D) for simple threshold cleanup.

| Method | Capacity | Complexity per query |
|--------|----------|---------------------|
| Brute-force search | N^K | O(N^K * D) |
| Threshold cleanup | ~sqrt(D) items | O(N * D) |
| Resonator network | ~N^2 bound pairs | O(I * K * N * D) where I = iterations |

For N=1,000 symbols and K=2 factors, the resonator network can resolve ~10^6
bound pairs — far exceeding the ~100 item limit of simple cleanup. This is
relevant for roko's future scalability: as codebooks grow, resonator networks
can maintain factorization accuracy where simpler methods fail.

### Connection to Modern Hopfield Networks

Resonator networks are closely related to **modern Hopfield networks** with
exponential capacity:

Ramsauer, H., Schafl, B., Lehner, J., Seidl, P., Widrich, M., Adler, T.,
Gruber, L., Holzleitner, M., Pavlovic, M., Sandve, G.K., Unterthiner, T.,
Brandstetter, J., Hochreiter, S. (2021). "Hopfield Networks is All You Need."
*International Conference on Learning Representations (ICLR).*

Classical Hopfield networks (Hopfield, 1982) store ~0.14*D patterns. Modern
Hopfield networks, which use an exponential energy function (equivalent to
softmax attention), store up to ~2^{D/2} patterns — a colossal improvement.

The resonator network's codebook projection step is equivalent to a Hopfield
network update: each module's "cleanup" operation is a single step of pattern
retrieval from an associative memory. The iterative resonator loop is
therefore a coupled system of Hopfield networks, each helping the others
converge to the correct stored pattern.

---

## Attention Approximates Sparse Distributed Memory

One of the most illuminating connections in modern AI:

Bricken, T. and Pehlevan, C. (2021). "Attention
Approximates Sparse Distributed Memory." *Advances in Neural Information
Processing Systems (NeurIPS)*, 34:15301-15315.

### Sparse Distributed Memory (SDM)

Kanerva's Sparse Distributed Memory (1988) is a content-addressable memory
that stores patterns at random addresses in a high-dimensional space. To read
from SDM:

1. Present a query address.
2. Activate all storage locations within Hamming distance r of the query.
3. Sum the data stored at all activated locations (weighted by activation).
4. Threshold the sum to produce the retrieved pattern.

The "sparse" in SDM refers to the fact that only a small fraction of the
exponentially many possible addresses are actually used — the activated
locations form a sparse subset.

### Transformer Attention as SDM

Bricken et al. showed that the transformer's scaled dot-product attention
mechanism is mathematically equivalent to an SDM read with an exponential
(softmax) activation function:

```
SDM read:  output = sum_i  a(query, key_i) * value_i
                    where a() is the activation function

Attention: output = sum_i  softmax(query . key_i / sqrt(d)) * value_i
                    where softmax is the exponential activation
```

The correspondence is exact:
- **Keys** = storage addresses in SDM
- **Values** = stored data at each address
- **Query** = the read address
- **Softmax** = exponential activation function (smooth version of SDM's
  hard threshold)

### External vs. Internal Attention

This equivalence reveals a deep connection between HDC retrieval and
transformer inference:

| | Transformer Attention | HDC / SDM Retrieval |
|---|---|---|
| **Memory** | Context window (KV cache) | Long-term vector store (item memory) |
| **Scope** | "Internal" — attends over tokens *within* the current sequence | "External" — retrieves from a persistent memory *outside* the current context |
| **Capacity** | Bounded by context length (typically 4K-128K tokens) | Bounded by codebook size (can be millions of vectors) |
| **Activation** | Softmax (exponential) | Hamming threshold (hard) or similarity ranking (soft) |
| **Write mechanism** | Append to KV cache during forward pass | Explicit insertion into vector store |

In roko's architecture, this means:
- The LLM's internal attention handles *within-context* reasoning.
- The HDC vector store handles *long-term memory* retrieval — acting as an
  "external attention" mechanism over the agent's accumulated knowledge.
- The two systems are complementary: internal attention is powerful but
  transient (limited to the context window), while external HDC memory is
  persistent but requires explicit encoding.

### Connection to Modern Hopfield Networks

The modern Hopfield network result (Ramsauer et al. 2021) completes the
picture: the exponential energy function that gives softmax attention its
power is the same mechanism that gives modern Hopfield networks their
exponential storage capacity (~2^{D/2} patterns).

This means:
- Classical Hopfield (quadratic energy) ~ linear attention ~ SDM with hard threshold
- Modern Hopfield (exponential energy) ~ softmax attention ~ SDM with exponential activation
- HDC with resonator cleanup ~ coupled modern Hopfield networks

The unifying theme: **all of these are content-addressable memories operating
in high-dimensional spaces.** HDC makes this explicit by working directly
with the high-dimensional vectors, while transformers achieve it implicitly
through learned attention weights.

### Hyperdimensional Probing of LLM Internals

If attention really is SDM, and SDM is native HDC, then it should be possible
to decode the internal representations of a transformer using VSA algebra
directly. Bronzini et al. (2025) demonstrated exactly this.

Bronzini, M., Nicolini, C., Lepri, B., Staiano, J., and Passerini, A.
(2025). "Hyperdimensional Probe: Decoding LLM Representations via Vector
Symbolic Architectures." arXiv:2509.25045.

**What they did.** The authors trained a shallow three-layer MLP (55-71M
parameters) to map a transformer's residual stream activations into a
VSA codebook of MAP-Bipolar hypervectors ({-1, +1}^D, D=4,096). Each
concept in the codebook is a quasi-orthogonal random hypervector (average
pairwise cosine similarity 0 +/- 0.02), and the encoder learns the
mapping M: R^d -> {-1, +1}^D that projects the LLM's continuous
internal state into this discrete symbolic space. Once projected, the
standard VSA operations — binding (Hadamard product), bundling
(element-wise sum), and unbinding (factoring out a known component) —
can decompose the resulting hypervector to extract which concepts the
model is internally representing.

**Key results.** Tested across six LLMs spanning 355M to 109B parameters
(GPT-2-medium, Pythia-1.4B, Phi-4, Llama 3.1-8B, OLMo-2-32B, Llama 4
Scout-109B), the probe achieved:

- 0.89 average cosine similarity between predicted and target hypervectors
- 94% binary element accuracy (only 6% of vector elements deviate)
- 83% concept extraction accuracy (probing@1) averaged across models
- 76% key-target extraction accuracy on structured analogy tasks
- F1=0.69 on SQuAD question-answering in a zero-shot probing setup

These results hold across different embedding dimensions and model
architectures, demonstrating that VSA algebra is sufficient to extract
interpretable, structured features from transformer internals — not as
a toy demonstration, but as a competitive interpretability method that
unifies the strengths of supervised probes, sparse autoencoders, and
logit-based attribution.

**Why this matters for BSC.** The Hyperdimensional Probe provides
empirical evidence for a claim that is load-bearing in roko's architecture:
VSA and transformers are not competing representations but complementary
layers. The transformer handles inference (next-token prediction,
reasoning, planning); the VSA layer handles knowledge management
(encoding, retrieval, composition, decay). Bronzini et al. show that
a simple VSA encoder can read out the concepts a transformer is working
with — which means the information flowing through transformer residual
streams is already compatible with hyperdimensional representation, even
though the model was never trained to produce VSA-structured outputs.

**Connection to the attention-SDM thesis.** This result closes a
conceptual loop. Bricken et al. (2021) showed that attention *is* SDM
— the same content-addressable memory mechanism that underlies all of
HDC. Bronzini et al. (2025) showed that VSA algebra can *decode* what
attention computes. If the attention mechanism is performing SDM-style
associative retrieval internally, then it should not be surprising that
a VSA probe — which speaks the same algebraic language as SDM — can
extract structured meaning from the residual stream. The probe works
*because* attention and VSA share the same mathematical substrate:
high-dimensional distributed representations manipulated by similarity-
based operations.

**The architectural implication.** In roko's design, the LLM's internal
attention handles within-context reasoning (the "fast path"), while the
HDC vector store provides persistent external memory (the "long-term
path"). The Hyperdimensional Probe result validates this division: VSA
is not a weaker alternative to neural representations but a natural
interface to them. Knowledge encoded in BSC hypervectors can be injected
into the LLM context window, processed by attention (which is itself
SDM), and the results can be read back out via VSA decomposition. The
two systems are not merely compatible — they are two views of the same
underlying geometry.

---

## Projection — Embedding to Hypervector

Raw data (text, floats, bytes) must be projected into the 10,240-bit space.
Projection is the bridge between conventional data representations and the
HDC algebra. The quality of the projection determines the quality of
downstream HDC operations.

### 1. Random Projection (Dense Matrix)

```
binary_vector = sign(M * embedding)
```

Where M is a D x d matrix (D=10,240, d=input dimension, e.g., 768 for BERT).
Each element of M is drawn from {-1, +1} uniformly.

**Why random projection works:** The Johnson-Lindenstrauss lemma guarantees
that random linear projections preserve pairwise distances. Specifically, if
two embeddings have cosine similarity s in the original d-dimensional space,
their Hamming similarity in the D-dimensional binary space will be
approximately (1+s)/2. This is because:

```
P(sign(m . x) = sign(m . y)) = 1 - arccos(cos_sim(x,y)) / pi
```

where m is a random projection vector. For cosine similarity s = 0.8, the
expected Hamming similarity is approximately 0.87 — high similarity is
preserved. For s = 0.0 (orthogonal), Hamming similarity is 0.50 (random) —
unrelatedness is also preserved.

Achlioptas, D. (2003). "Database-friendly Random Projections:
Johnson-Lindenstrauss with Binary Coins." *Journal of Computer and System
Sciences*, 66(4):671-687.

- **Pros:** Preserves cosine similarity (Johnson-Lindenstrauss lemma)
- **Cons:** Matrix is large (10,240 x 768 x 1 bit ~ 960 KB)
- **Consensus:** Deterministic from seed. All validators derive same M.

### 2. Character Trigram Encoding

```
text -> character trigrams -> each trigram is a random vector -> bundle
```

Used in roko-index for code fingerprinting. Fast, no model required.

The encoding process in detail:
1. Slide a window of 3 characters across the input text.
2. Map each trigram to a random vector using a hash of the trigram as the seed.
3. Bundle all trigram vectors using majority vote.

For example, the string "hello" produces trigrams: "hel", "ell", "llo".
Each trigram maps to a random vector, and the bundle captures the
distributional character-level signature of the input.

- **Pros:** Zero external dependencies, ~100ns encoding, language-agnostic
- **Cons:** No semantic understanding (purely syntactic). "happy" and "glad"
  will have low similarity because they share few character trigrams.

### 3. Token-Level Encoding

```
tokens -> each token's position-permuted random vector -> bundle
```

Position-aware: `rho^i(token_vector)` where i is position.

This is a "bag-of-words with position weighting" approach:
1. Tokenize the input (using a fixed vocabulary).
2. For each token at position i, compute `rho^i(token_vector)`.
3. Bundle all position-permuted token vectors.

The position-permutation ensures that "the cat sat on the mat" and "the mat
sat on the cat" produce different vectors, because the same word at different
positions gets different permutation amounts.

- **Pros:** Captures order, fast, no model, position-sensitive
- **Cons:** Fixed vocabulary required. Out-of-vocabulary tokens must be
  handled via character-level fallback or hashing.

### Recommendation

Use a **two-tier projection** system:

1. **Fast path (local):** Character trigram or token encoding for real-time
   cognitive operations. ~100ns. Good enough for similarity retrieval.

2. **Quality path (publication):** Random projection from a neural embedding
   (e.g., sentence-transformers). ~10ms. Used when publishing to shared
   substrate where quality matters more than speed.

The same vector algebra works on both — the difference is encoding fidelity.
The fast path sacrifices semantic precision for speed; the quality path
sacrifices speed for semantic faithfulness. Both produce valid 10,240-bit
BSC vectors that participate identically in all algebraic operations.

---

## Incompressibility

A critical property of dense random binary vectors: they are fundamentally
**incompressible** by lossless methods.

### Information-Theoretic Argument

A random 10,240-bit vector where each bit is independently and uniformly
drawn from {0, 1} has **exactly 10,240 bits of Shannon entropy.** This is
the maximum possible entropy for a 10,240-bit string. By the source coding
theorem (Shannon 1948), no lossless compression scheme can reduce the expected
representation below the entropy of the source.

Concretely: if you run `gzip`, `zstd`, `brotli`, or any other lossless
compressor on a random 10,240-bit vector, the output will be *at least* as
large as the input (typically slightly larger, due to compression headers and
framing overhead).

### Why This Is a Feature, Not a Bug

The incompressibility of HDC vectors is a **direct consequence** of their
quasi-orthogonality. If vectors were compressible, they would have structure
— patterns, redundancy, correlations between bits. But structure means
reduced effective dimensionality, which means fewer quasi-orthogonal
directions, which means lower capacity.

The chain of reasoning:
1. High capacity requires high effective dimensionality.
2. High effective dimensionality requires maximal entropy per bit.
3. Maximal entropy per bit means incompressibility.

Therefore: **incompressibility is the price of capacity.** The 1,280 bytes
per vector are not wasted — every bit is carrying useful information.

### Practical Implications

- **On-chain storage:** Each vector costs exactly 1,280 bytes. No shortcuts.
- **Network transfer:** Vectors must be sent in full (or as diffs from a known base).
- **Serialization format:** An `HdcVector` is serialized as 160 consecutive
  `u64` words in **little-endian** byte order (native on x86 and ARM), for a
  total of exactly 1,280 bytes. This format is used for on-chain storage,
  EVM calldata, P2P gossip, and local persistence. **Endianness is consensus-
  critical**: all validators must agree on byte order so that XOR, popcount,
  and Hamming distance produce identical results. Little-endian is chosen
  because it matches the native byte order of all target platforms (x86-64,
  aarch64), allowing zero-copy deserialization via `rkyv` or raw pointer
  casts without byte-swapping overhead.
- **Sparse vectors are different:** For sparse vectors (e.g., 2% density),
  run-length encoding or RRR compression achieves ~7x reduction. But BSC
  vectors are dense (50% ones), so this does not apply.

### Practical Mitigation for On-Chain Cost

- Store only **anchors** (hashes) on-chain, full vectors in event logs.
- Use **delta encoding** for updates: XOR of old and new vector produces a
  diff that may be sparse *if the update is small*. A single role-filler
  change in a bundled record affects ~50% of bits, so delta encoding offers
  no compression on average. But incremental updates (adding one item to a
  large bundle) change fewer bits and compress somewhat.
- **Batch publications** to amortize per-transaction overhead.
- **Lazy materialization:** Store the recipe (list of atomic vector seeds +
  operations) rather than the result. Recompute on demand. This trades
  computation for storage but is only viable when the recipe is shorter
  than 1,280 bytes.

### Wire Format Specification

The canonical wire format for `HdcVector` is used in all contexts:
EVM calldata, P2P gossip, event logs, local persistence, and RPC responses.

```
HDC Vector Wire Format (version 1)
===================================

Total size: 1,280 bytes (fixed, no length prefix needed when standalone)

Layout:
  Offset   Size    Field
  ------   ----    -----
  0        8       words[0]   (u64, little-endian)
  8        8       words[1]   (u64, little-endian)
  ...
  1272     8       words[159] (u64, little-endian)

Bit addressing:
  Bit index b (0 <= b < 10,240) maps to:
    word  = b / 64
    bit   = b % 64
    value = (words[word] >> bit) & 1

Endianness: LITTLE-ENDIAN (consensus-critical).
All validators MUST use LE. On a big-endian platform, each u64 word
must be byte-swapped before storage/transmission.

Integrity: When transmitted as part of a signed message, the raw
1,280-byte payload is hashed with Keccak-256 to produce the vector's
content-addressed ID (H256).

  vector_id = keccak256(words[0..160] as [u8; 1280])
```

```rust
/// Serialize an HdcVector to its canonical 1,280-byte wire format.
fn serialize_vector(v: &HdcVector) -> [u8; 1280] {
    let mut buf = [0u8; 1280];
    for (i, word) in v.0.iter().enumerate() {
        buf[i * 8..(i + 1) * 8].copy_from_slice(&word.to_le_bytes());
    }
    buf
}

/// Deserialize an HdcVector from its canonical wire format.
/// Returns Err if the buffer is not exactly 1,280 bytes.
fn deserialize_vector(buf: &[u8]) -> Result<HdcVector, VectorError> {
    if buf.len() != 1280 {
        return Err(VectorError::InvalidLength {
            expected: 1280,
            actual: buf.len(),
        });
    }
    let mut words = [0u64; 160];
    for i in 0..160 {
        words[i] = u64::from_le_bytes(
            buf[i * 8..(i + 1) * 8].try_into().unwrap()
        );
    }
    Ok(HdcVector(words))
}

/// Compute the content-addressed ID of a vector.
fn vector_id(v: &HdcVector) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let buf = serialize_vector(v);
    let mut hasher = Keccak::v256();
    hasher.update(&buf);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

#[derive(Debug)]
enum VectorError {
    InvalidLength { expected: usize, actual: usize },
    ChecksumMismatch { expected: [u8; 32], actual: [u8; 32] },
}

impl std::fmt::Display for VectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VectorError::InvalidLength { expected, actual } =>
                write!(f, "invalid vector length: expected {} bytes, got {}", expected, actual),
            VectorError::ChecksumMismatch { .. } =>
                write!(f, "vector checksum mismatch"),
        }
    }
}

impl std::error::Error for VectorError {}
```

**Framed wire format** (for streaming / multiplexed channels):

```
Offset   Size    Field
------   ----    -----
0        1       version    (u8, currently 0x01)
1        1       flags      (u8, bit 0 = compressed, bits 1-7 reserved)
2        2       reserved   (u16, must be 0x0000)
4        1280    payload    (1,280 bytes if uncompressed, or compressed length)
                             When flags bit 0 is set, payload is LZ4-compressed
                             and a 2-byte LE length prefix replaces bytes [4..6],
                             followed by the compressed data.
```

The framed format adds 4 bytes of overhead but enables version negotiation
and optional compression. Compression is counterproductive for random BSC
vectors (50% density compresses poorly) but useful for delta-encoded vectors
where the XOR diff may be sparse.

---

## Practical Applications Beyond Classification

The algebraic foundations described in this document are not merely theoretical.
Recent work demonstrates that HDC's core operations -- bind, bundle, permute,
similarity search -- translate directly into practical systems for knowledge
graph reasoning, cybersecurity, and hardware-accelerated inference.

### INCYSER: Interpretable Cybersecurity Reasoning

Zakeri, Chen, Srinivasa, Latapie, and Imani (2025) apply HDC to cybersecurity
knowledge graphs in a system called INCYSER. The system combines embedding-based
unsupervised learning with HDC-based graph representation learning to perform
link prediction and triple classification on cybersecurity knowledge graphs --
tasks like predicting which threat actor is likely to target which vulnerability,
or classifying whether a given (attacker, technique, target) triple is valid.

INCYSER achieves a 1.3% improvement in mean reciprocal rank (MRR) over leading
models with 25.1% faster inference. But the more significant result is
**interpretability.** Because HDC reasoning operates through explicit algebraic
composition of hypervectors -- binding entity vectors with relation vectors,
bundling neighborhood information, computing similarity scores -- the reasoning
process is transparent. You can inspect which vectors contributed to a
prediction, decompose a bundled representation back into its constituents, and
trace the path from input entities to output score.

This stands in direct contrast to GNN-based knowledge graph reasoning, where
the learned representations are opaque. A graph neural network can tell you
that triple (A, r, B) scores 0.87, but it cannot tell you *why* -- the
information is distributed across layers of nonlinear transformations that
resist interpretation. HDC's algebraic transparency is not a post-hoc
explainability add-on; it is intrinsic to the computational model.

For roko agents operating in the DeFi security space, this interpretability
property is not merely desirable -- it is essential. When an agent flags a
transaction as suspicious or recommends a defensive action, the reasoning
must be auditable. HDC provides this auditability by construction.

> Zakeri, A., Chen, H., Srinivasa, N., Latapie, H., & Imani, M. (2025).
> "Enabling Efficient and Interpretable Cybersecurity Reasoning Through
> Hyperdimensional Computing." *IEEE Transactions on Artificial
> Intelligence*. DOI: 10.1109/TAI.2025.3545394.

### HDReason: Hardware-Accelerated Knowledge Graph Reasoning

Chen et al. (2024) take HDC knowledge graph reasoning a step further with
HDReason, an algorithm-hardware codesign framework that implements HDC-based
knowledge graph completion on FPGA. The system encodes entities and relations
as hypervectors using a fixed base matrix (eliminating the expensive weight
updates required by neural approaches), bundles neighborhood information
through element-wise binding, and evaluates candidate triples using a
TransE-style score function -- all operations that map directly to the
BSC algebra described in this document.

The FPGA implementation (Xilinx Alveo U50, 200 MHz) achieves:
- **10.6x speedup** over an NVIDIA RTX 4090 GPU
- **65x energy efficiency improvement** over GPU (0.21-0.93 J/batch vs
  20.88-65.31 J/batch on GPU)
- **4.2x higher performance** than state-of-the-art FPGA-based GCN training
  platforms at similar accuracy
- **Quantization robustness:** 4-bit quantization causes only a 5% accuracy
  drop, versus 45% for GNN models

The quantization robustness result is particularly relevant to this
architecture. It demonstrates empirically what the theory predicts: because
HDC representations are holographic (information distributed across all
dimensions) and the operations are bitwise, the system degrades gracefully
under aggressive quantization. This is the same property that makes BSC
vectors suitable for deterministic on-chain execution -- the binary nature
of our 10,240-bit vectors is not a limitation but an advantage, enabling
exact arithmetic without floating-point indeterminacy.

HDReason's architecture -- systolic arrays for encoding, parallel score
engines, content-addressable caches for hypervector reuse -- also validates
the principle that HDC's computational primitives are inherently
parallelizable. The same bind/bundle/similarity operations that run on FPGA
datapaths can run as SIMD instructions on commodity CPUs, or as bitwise
operations in a blockchain VM.

> Chen, H., Ni, Y., Zakeri, A., Zou, Z., Yun, S., Wen, F., Khaleghi, B.,
> Srinivasa, N., Latapie, H., & Imani, M. (2024). "HDReason:
> Algorithm-Hardware Codesign for Hyperdimensional Knowledge Graph
> Reasoning." arXiv:2403.05763.

### The Interpretability Advantage

INCYSER and HDReason together illustrate a structural advantage of HDC over
neural approaches for knowledge graph reasoning:

| Property | HDC (BSC) | GNN / Deep Learning |
|---|---|---|
| Reasoning transparency | Algebraic: inspect which vectors contributed | Opaque: distributed across nonlinear layers |
| Decomposability | Unbind and unbundle to recover constituents | No general decomposition method |
| Quantization tolerance | 5% accuracy drop at 4-bit | 45% accuracy drop at 4-bit |
| Hardware mapping | Bitwise ops -> FPGA/SIMD/blockchain VM | Matrix multiply -> GPU/TPU |
| Energy efficiency | 65x better than GPU (HDReason) | Baseline |
| Determinism | Exact bitwise arithmetic | Floating-point ordering dependence |

The last row is critical for this architecture. Neural knowledge graph
reasoning requires floating-point matrix operations whose results depend on
execution order, hardware platform, and compiler optimizations. Two validators
running the same GNN inference may produce different floating-point results
and therefore disagree on consensus. HDC's bitwise operations produce
identical results everywhere, always -- the non-negotiable property for
on-chain execution.

---

## References

Achlioptas, D. (2003). "Database-friendly Random Projections:
Johnson-Lindenstrauss with Binary Coins." *Journal of Computer and System
Sciences*, 66(4):671-687.

Bricken, T. and Pehlevan, C. (2021). "Attention
Approximates Sparse Distributed Memory." *Advances in Neural Information
Processing Systems (NeurIPS)*, 34:15301-15315.

Bronzini, M., Nicolini, C., Lepri, B., Staiano, J., and Passerini, A.
(2025). "Hyperdimensional Probe: Decoding LLM Representations via Vector
Symbolic Architectures." arXiv:2509.25045.

Frady, E.P., Kleyko, D., and Sommer, F.T. (2018). "A Theory of Sequence
Indexing and Working Memory in Recurrent Neural Networks." *Neural
Computation*, 30(6):1449-1513.

Frady, E.P., Kent, S.J., Olshausen, B.A., and Sommer, F.T. (2020).
"Resonator Networks, 1: An Efficient Solution for Factoring High-Dimensional,
Distributed Representations of Data Structures." *Neural Computation*,
32(12):2311-2331.

Gayler, R.W. (1998). "Multiplicative Binding, Representation Operators,
and Analogy." *Analogical Connections: Proceedings of the IAAI Workshop on
Analogical Reasoning.*

Johnson, W.B. and Lindenstrauss, J. (1984). "Extensions of Lipschitz
mappings into a Hilbert space." *Conference in Modern Analysis and
Probability*, Contemporary Mathematics, 26:189-206.

Kanerva, P. (1988). *Sparse Distributed Memory.* MIT Press.

Kanerva, P. (1994). "The Spatter Code for Encoding Concepts at Many Levels."
*Proceedings of the International Conference on Artificial Neural Networks
(ICANN)*, pp. 226-229. Springer.

Plate, T.A. (1991). "Holographic Reduced Representations: Convolution Algebra
for Compositional Distributed Representations." *Proceedings of the 12th
International Joint Conference on Artificial Intelligence (IJCAI)*, pp. 30-35.

Plate, T.A. (1995). "Holographic Reduced Representations." *IEEE Transactions
on Neural Networks*, 6(3):623-641.

Plate, T.A. (2003). *Holographic Reduced Representation: Distributed
Representation for Cognitive Structures.* CSLI Publications, Stanford
University.

Ramsauer, H., Schafl, B., Lehner, J., Seidl, P., Widrich, M., Adler, T.,
Gruber, L., Holzleitner, M., Pavlovic, M., Sandve, G.K., Unterthiner, T.,
Brandstetter, J., Hochreiter, S. (2021). "Hopfield Networks is All You Need."
*International Conference on Learning Representations (ICLR).*

Smolensky, P. (1990). "Tensor Product Variable Binding and the Representation
of Symbolic Structures in Connectionist Systems." *Artificial Intelligence*,
46(1-2):159-216.

Thomas, A., Dasgupta, S., and Rosing, T. (2021). "A Theoretical Perspective
on Hyperdimensional Computing." *Journal of Artificial Intelligence
Research*, 72, 215-249. arXiv:2010.07426.

Zakeri, A., Chen, H., Srinivasa, N., Latapie, H., and Imani, M. (2025).
"Enabling Efficient and Interpretable Cybersecurity Reasoning Through
Hyperdimensional Computing." *IEEE Transactions on Artificial Intelligence*.
DOI: 10.1109/TAI.2025.3545394.

Chen, H., Ni, Y., Zakeri, A., Zou, Z., Yun, S., Wen, F., Khaleghi, B.,
Srinivasa, N., Latapie, H., and Imani, M. (2024). "HDReason:
Algorithm-Hardware Codesign for Hyperdimensional Knowledge Graph Reasoning."
arXiv:2403.05763.

Kleyko, D., Rachkovskij, D.A., Osipov, E., and Rahimi, A. (2022). "A
Survey on Hyperdimensional Computing aka Vector Symbolic Architectures,
Part I: Models and Data Transformations." *ACM Computing Surveys*, 55(6),
Article 130.

Liu, Y., Chung, W.Y., Chen, H., Yeung, C., and Imani, M. (2025).
"Encoder-Free Knowledge-Graph Reasoning with LLMs via Hyperdimensional
Path Retrieval." arXiv:2512.09369.

Neubert, P. and Schubert, S. (2021). "Hyperdimensional Computing as a
Framework for Systematic Aggregation of Image Descriptors." *Proceedings
of the IEEE/CVF Conference on Computer Vision and Pattern Recognition
(CVPR)*, pp. 16938-16947.

Rachkovskij, D.A. and Kussul, E.M. (2001). "Binding and Normalization of
Binary Sparse Distributed Representations by Context-Dependent Thinning."
*Neural Computation*, 13(2):411-452.

Schlegel, K., Neubert, P., and Protzel, P. (2022). "A comparison of Vector
Symbolic Architectures." *Artificial Intelligence Review*, 55(6):4523-4555.

Schmuck, M., Benini, L., and Rahimi, A. (2019). "Hardware Optimizations of
Dense Binary Hyperdimensional Computing: Rematerialization of Hypervectors,
Binarized Bundling, and Combinational Associative Memory." *ACM Journal on
Emerging Technologies in Computing Systems (JETC)*, 15(4):1-25.

Yeung, C., Zou, Z., and Imani, M. (2024). "Generalized Holographic Reduced
Representations." arXiv:2405.09689.

Zakeri, A., Zou, Z., Chen, H., Latapie, H., and Imani, M. (2024).
"Conjunctive block coding for hyperdimensional graph representation."
*Array*, 21, 100338.
