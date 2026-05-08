# Optimal Design -- Synthesized Architecture

This document is the synthesis. It combines the insights from all eight preceding
documents -- foundations, implementation analysis, knowledge representation,
context assembly, vector search, shared substrate, and cognitive architecture --
into a single coherent, implementable architecture for HDC-powered agent
cognition on daeji.

The audience is both engineers (who will build this) and technical stakeholders
(who need to understand the design decisions and their rationale).

---

## First Principles

### What This Document Is

Documents 01-08 each explored a single dimension of the problem space: algebraic
foundations, encoding strategies, knowledge lifecycle, search algorithms, chain
integration, cognitive loops. Each document was written to be self-contained,
which means they occasionally disagree on details, propose multiple alternatives,
or leave integration questions open. This document resolves all of those open
questions into a single architecture specification.

Every design decision in this document traces back to a specific analysis in one
of the preceding documents. Where two documents disagreed, this document records
the resolution and why.

### Design Philosophy

Three principles govern every decision:

1. **Minimize complexity at each layer.** Each layer should do one thing well.
   If a layer is doing two things, it should be split. If two layers are doing
   the same thing, they should be merged. Complexity that leaks between layers
   is a design failure.

2. **Maximize composability between layers.** The same algebraic primitives
   (bind, bundle, permute, hamming) must work unchanged from the lowest layer
   (raw bit manipulation) to the highest layer (chain-validated knowledge).
   No layer should require a special case in any other layer.

3. **Determinism is not optional.** Every operation that touches the consensus
   path must produce identical results on every validator, every platform, every
   compiler version, every run. This constraint eliminates floating-point
   arithmetic, platform-dependent SIMD semantics, ordering-dependent operations,
   and unseeded randomness from the consensus path. It is the single most
   important constraint in the system and it is non-negotiable.

### Why Six Layers

The 6-layer architecture is inspired by the OSI networking model. Each layer
has a clear responsibility, a clean interface to the layers above and below it,
and can be implemented and tested independently. The analogy is not cosmetic:
just as OSI layers allow you to swap Ethernet for WiFi without changing TCP,
our layers allow you to swap brute-force search for HNSW without changing
knowledge management, or swap trigram encoding for projection encoding without
changing storage.

```
Layer 6: Chain Integration     ← On-chain contracts, precompile, gas economics
Layer 5: Context Assembly      ← Gather, rank, compress, prompt construction
Layer 4: Knowledge Management  ← Unified store, scoring, decay, anti-knowledge
Layer 3: Storage & Search      ← Local index, on-chain index, tiered search
Layer 2: Encoding              ← Text → hypervector, embedding → hypervector
Layer 1: HDC Algebra           ← Immutable mathematical core
```

Each layer depends only on the layers below it. No layer reaches down more
than one level (with the single exception of Layer 5 calling Layer 1's
similarity function for ranking, which is acceptable because Layer 1 is
the universal foundation).

### HDC in Context: What It Is and What It Is Not

Hyperdimensional computing has been successfully applied across a range of
domains outside the agent cognition use case described here:

- **Natural language processing:** Language recognition with >95% accuracy
  using character-level trigram encoding (Joshi et al., 2016). The same
  trigram encoding approach is used in Layer 2 of this architecture.
- **Genomics:** DNA sequence matching and classification using k-mer
  encoding, achieving comparable accuracy to deep learning models at
  orders-of-magnitude lower computational cost (Kim et al., 2020).
- **Robotics:** Gesture recognition from EMG signals with real-time
  classification latency under 1ms (Rahimi et al., 2016).
- **Graph learning:** PathHD achieves 86.2% Hits@1 on knowledge graph QA
  (WebQSP) by encoding relation paths as GHRR hypervectors via non-commutative
  binding (Liu et al., 2025). The earlier PathHD work (Nunes et al., 2022)
  demonstrated HDC graph classification via permuted bindings of random walks.
- **Hardware-accelerated inference:** IBM's NVSA architecture achieves 244x
  speedup over traditional neural networks on classification tasks by
  exploiting the bitwise nature of BSC operations (Hersche et al., 2023).
- **Cybersecurity knowledge graph reasoning (INCYSER):** Zakeri, Chen,
  Srinivasa, Latapie, and Imani (2025) apply HDC to cybersecurity knowledge
  graphs, combining embedding-based unsupervised learning with HDC-based
  graph representation learning. INCYSER achieves a 1.3% improvement in
  mean reciprocal rank (MRR) over leading models with 25.1% faster inference,
  while providing interpretable reasoning -- you can inspect which
  hypervectors contributed to a given prediction, unlike black-box GNN
  approaches. This is directly relevant to roko agents, which operate in
  the DeFi security space and need transparent, auditable threat reasoning.
- **FPGA-accelerated HDC knowledge graph reasoning (HDReason):** Chen et
  al. (2024) demonstrate that HDC-based knowledge graph completion can be
  hardware-accelerated on FPGA. Their Xilinx Alveo U50 implementation
  achieves 10.6x speedup over an NVIDIA RTX 4090 GPU with 65x energy
  efficiency improvement, and 4.2x higher performance than state-of-the-art
  FPGA-based GCN training platforms at similar accuracy. The system tolerates
  4-bit quantization with only a 5% accuracy drop (versus 45% for GNN
  models), validating that HDC's bitwise operations map naturally to
  hardware acceleration -- the same property this architecture exploits
  for on-chain deterministic execution.
- **NysX edge FPGA graph classification (Arockiaraj et al., 2025):** The
  first end-to-end FPGA accelerator for Nystrom-based HDC graph
  classification, implemented on an AMD Zynq UltraScale+ (ZCU104). NysX
  achieves 6.85x speedup and 169x energy efficiency over optimized CPU
  baselines (4.32x speedup, 314x energy efficiency over GPU), while
  *improving* classification accuracy by 3.4% on TUDataset benchmarks. The accuracy improvement comes from a
  hybrid uniform + DPP (Determinantal Point Process) landmark sampling
  scheme: DPPs are probability distributions that favor diverse subsets,
  enforcing that selected landmarks are maximally spread across the graph
  feature space rather than redundantly clustered. This reduces landmark
  count while improving kernel approximation quality, cutting per-inference
  latency by 25-40%. The result demonstrates that hardware acceleration
  and algorithmic improvement compose multiplicatively -- better sampling
  produces both faster execution and higher accuracy, unlike neural network
  quantization where speed and accuracy typically trade off.
- **Domain-specific HDC RISC-V processor (Wasif et al., IEEE TCAS-I, 2025):**
  A fabricated 22nm RISC-V chip with custom HDC instructions and a vector
  processing unit for edge-AI training. The chip implements FixedHD, a
  16-bit fixed-point HDC model that matches floating-point accuracy while
  lowering computational complexity. Custom instructions accelerate encoding
  (random-projection matrix multiplication), bundling (class hypervector
  accumulation), and cosine similarity (iterative retraining). The extended
  RISC-V achieves 4x speedup over the baseline processor at 120 MHz, and
  consumes only 24.65 uJ per training sample -- 10-100x more efficient than
  comparable chips. At ~10 million transistors on 1 mm^2, this is five
  orders of magnitude fewer transistors than an NVIDIA GPU die, yet it
  achieves competitive accuracy on edge classification tasks. This validates
  that HDC's computational simplicity (XOR, popcount, majority vote) maps
  directly to radical hardware efficiency -- the same operations our
  precompile exposes at 0x09 are operations that silicon designers chose to
  etch into custom instructions.
- **HDC x blockchain -- open whitespace:** As of 2026, there is no
  significant peer-reviewed work combining hyperdimensional computing with
  blockchain systems. Surveys of HDC applications (Kleyko et al., 2022-2023,
  ACM Computing Surveys; Heddes et al., 2024, Journal of Big Data;
  IEEE Future Directions, 2023) and blockchain-AI integration reviews
  (Springer, 2025; Wiley, 2026) show active research in both domains
  independently, but the intersection remains unexplored in the literature.
  This makes the roko architecture -- HDC as a consensus-safe cognitive
  substrate on a blockchain runtime -- genuinely novel, not merely
  incremental.

These results validate the core thesis: HDC is a practical computational
framework, not a theoretical curiosity. But intellectual honesty demands
acknowledging its limitations:

**HDC is not a replacement for deep learning in raw accuracy.** On tasks where
large neural networks have been extensively trained (ImageNet classification,
machine translation, code generation), HDC does not match their performance.
HDC's strengths lie elsewhere: composability (you can algebraically build
complex representations from primitives), efficiency (nanosecond operations
on commodity hardware without GPUs), interpretability (you can inspect and
decompose hypervectors), and consensus safety (deterministic bitwise arithmetic).

For this architecture, HDC serves as the associative memory substrate -- the
system that stores, retrieves, and composes knowledge representations. The LLM
remains the reasoning engine. HDC does not replace the LLM; it gives the LLM
the right context to reason about.

> Joshi, A., Halseth, J. T., & Kanerva, P. (2016). "Language Recognition
> using Random Indexing." arXiv:1602.02084
>
> Kim, Y., et al. (2020). "GenHD: Efficient Genomic Sequence Search and
> Classification Using Hyperdimensional Computing." *IEEE EMBC*.
>
> Rahimi, A., et al. (2016). "Hyperdimensional Biosignal Processing."
> *IEEE TBME*, 66(1), 43-55.
>
> Nunes, I., et al. (2022). "PathHD: Hyperdimensional Graph Learning with
> Random Walks." *NeurIPS Workshop on Graph Learning*.
>
> Hersche, M., et al. (2023). "A Neuro-Vector-Symbolic Architecture for
> Solving Raven's Progressive Matrices." *Nature Machine Intelligence*.
>
> Imani, M., Kong, D., Rosing, T., & Salamat, A. (2019). "A Framework for
> Classification Using Hyperdimensional Computing with Application to Face
> Recognition." *IEEE ISCAS*.
>
> Zakeri, A., Chen, H., Srinivasa, N., Latapie, H., & Imani, M. (2025).
> "Enabling Efficient and Interpretable Cybersecurity Reasoning Through
> Hyperdimensional Computing." *IEEE Transactions on Artificial
> Intelligence*. DOI: 10.1109/ACCESS.2025.10902423.
>
> Chen, H., Ni, Y., Zakeri, A., Zou, Z., Yun, S., Wen, F., Khaleghi, B.,
> Srinivasa, N., Latapie, H., & Imani, M. (2024). "HDReason:
> Algorithm-Hardware Codesign for Hyperdimensional Knowledge Graph
> Reasoning." arXiv:2403.05763.

With that landscape in view, the remainder of this document specifies the
concrete architecture that implements HDC for the roko use case.

---

## Design Overview

```
+-----------------------------------------------------------------+
|                         Agent Runtime                            |
|                                                                  |
|  +-----------------------------------------------------------+  |
|  |                   Cognitive Core                           |  |
|  |                                                            |  |
|  |  +----------+  +----------+  +----------+  +----------+  |  |
|  |  | Perceive |->| Assemble |->|  Reason  |->|  Learn   |  |  |
|  |  |          |  | Context  |  |  (LLM)   |  |          |  |  |
|  |  +----------+  +----+-----+  +----------+  +----+-----+  |  |
|  |                     |                            |         |  |
|  |              +------v----------------------------v------+  |  |
|  |              |         HDC Algebra Layer                |  |  |
|  |              |                                         |  |  |
|  |              |  HdcVector([u64; 160])                  |  |  |
|  |              |  bind() bundle() permute() similarity() |  |  |
|  |              +---------+--------------+----------------+  |  |
|  |                        |              |                    |  |
|  |              +---------v------+ +-----v----------------+  |  |
|  |              |  Local Index   | |  Chain Substrate     |  |  |
|  |              |                | |                      |  |  |
|  |              |  Brute/HNSW    | |  RPC -> Precompile   |  |  |
|  |              |  <100K: brute  | |  HDC at 0x09         |  |  |
|  |              |  >=100K: HNSW  | |                      |  |  |
|  |              |                | |  InsightBoard        |  |  |
|  |              |  Private       | |  PheromoneRegistry   |  |  |
|  |              |  Full trust    | |  Skeptical trust     |  |  |
|  |              +----------------+ +----------------------+  |  |
|  +-----------------------------------------------------------+  |
|                                                                  |
|  +-----------------------------------------------------------+  |
|  |                    Affect System                           |  |
|  |  Emotion (t=0.1) -> Mood (t=0.5) -> Personality (t=0.9)  |  |
|  |  PAD modulation -> somatic bias -> retrieval weighting    |  |
|  +-----------------------------------------------------------+  |
+-----------------------------------------------------------------+
```

---

## Layer 1: HDC Algebra (Foundational)

The immutable mathematical core. This never changes regardless of what's
built on top.

### Why This Representation

**Why `[u64; 160]`?** The representation is `160 x 64 = 10,240` bits. The choice
of `u64` as the word type is driven by hardware: it is the natural word size for
the `POPCNT` instruction on x86-64 and the `cnt` instruction on AArch64. XOR,
AND, OR, and popcount all operate on 64-bit words in a single cycle on all modern
CPUs. Using `u32` would double the loop iterations for no benefit. Using `u128`
would require SIMD or multi-instruction sequences on most hardware. `u64` is the
sweet spot.

**Why 10,240 bits?** The capacity of a BSC bundle is approximately sqrt(D), where
D is the dimensionality. At D=10,240, this gives sqrt(10,240) ~ 101 items that
can be superimposed in a single bundle vector while maintaining >99% retrieval
accuracy (Kanerva, 2009). This is far more than needed for any single knowledge
structure in this architecture -- context assembly typically selects 10-30 items,
and the most complex structured encodings (role-filler records) rarely exceed
10 fields. The headroom is intentional: it provides robustness against noise
accumulation across multiple bind/bundle operations.

Going lower (e.g., D=4,096 as some HDC systems use) would reduce capacity to
~64 items and, more importantly, narrow the gap between genuine similarity and
random noise (the standard deviation of random similarity scales as 1/(2*sqrt(D))).
At D=10,240, two random vectors have similarity 0.500 +/- 0.005 (1-sigma), giving a
clear 5.26-sigma threshold at 0.526. At D=4,096, the same threshold would be
0.500 +/- 0.008 (1-sigma), requiring either a higher threshold (reducing recall) or
accepting more false positives.

Going higher (e.g., D=65,536) would increase capacity to ~256 items but at
significant cost: 8KB per vector instead of 1.3KB, proportionally more memory
and bandwidth for every operation. The marginal benefit does not justify the
cost for this use case.

**Why `#[repr(align(64))]`?** Cache-line alignment. A typical CPU cache line
is 64 bytes. The `align(64)` directive ensures that the vector's memory
starts at a cache-line boundary, which means SIMD loads (AVX2 loads 32 bytes,
AVX-512 loads 64 bytes) never straddle cache-line boundaries. Straddling a
cache line forces the CPU to load two cache lines and merge them, which can
add 5-10 cycles per load -- a significant penalty when the inner loop is
measuring cycles in the dozens.

> Thomas, A., Dasgupta, S., & Rosing, T. (2021). "A Theoretical Perspective
> on Hyperdimensional Computing." *Journal of Artificial Intelligence
> Research*, 72, 215-249. arXiv:2010.07426
>
> Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction."
> *Cognitive Computation*, 1(2), 139-159.

### Specification

```rust
/// 10,240-bit binary hypervector.
/// BSC (Binary Spatter Code) model.
/// All operations are deterministic bitwise arithmetic.
#[repr(align(64))]
#[derive(Clone, PartialEq, Eq)]
pub struct HdcVector(pub [u64; 160]);

impl HdcVector {
    /// XOR bind -- associative, commutative, self-inverse.
    /// bind(A, B) is quasi-orthogonal to both A and B.
    /// bind(A, bind(A, B)) = B (unbinding recovers the original).
    pub fn bind(&self, other: &Self) -> Self;

    /// Majority-vote bundle -- returns superposition of inputs.
    /// The result is similar to each input (similarity > 0.5 for up to ~100 inputs).
    /// Tie-breaking: bit stays 0 (deterministic, consensus-safe).
    pub fn bundle(vectors: &[&Self]) -> Self;

    /// Weighted bundle via BundleAccumulator.
    /// Weight is implemented by repeating the vector's contribution
    /// in the vote count (integer repetition), not by floating-point
    /// scaling of the vector components. The u32 weight is the number
    /// of times the vector's votes are added to the accumulator.
    pub fn weighted_bundle(weighted: &[(u32, &Self)]) -> Self;

    /// Cyclic left rotation by k bits.
    /// Creates position-aware representations: permute(A, 1) is
    /// quasi-orthogonal to A but deterministically recoverable.
    pub fn permute(&self, k: i32) -> Self;

    /// Raw Hamming distance (consensus-safe, no floats).
    /// Returns the number of differing bits, 0 to 10,240.
    /// This is the ONLY distance function used on-chain.
    pub fn hamming_distance(&self, other: &Self) -> u32;

    /// Normalized similarity (local use only, not for consensus).
    /// Returns 1.0 - (hamming_distance / 10,240).
    /// Range: 0.0 (complement) to 1.0 (identical), 0.5 (orthogonal).
    pub fn similarity(&self, other: &Self) -> f64;

    /// Deterministic random vector from seed.
    /// Uses ChaCha20 PRNG seeded from the input.
    /// All validators derive identical vectors from identical seeds.
    pub fn random(seed: u64) -> Self;
}
```

### BundleAccumulator: Implementation Detail

The bundle operation requires counting votes per bit position. For N input
vectors, each bit position receives N votes (0 or 1 from each input). The
majority vote is 1 if more than N/2 votes are 1, else 0.

A naive implementation would allocate a `[u32; 10240]` counter array (40KB).
The actual implementation uses `i32` vote counters at the word level:

```rust
pub struct BundleAccumulator {
    /// Vote counters: one i32 per bit position.
    /// Positive count means majority 1, negative means majority 0.
    counts: Vec<i32>,  // Length: 10,240
    num_vectors: usize,
}

impl BundleAccumulator {
    pub fn new() -> Self {
        Self { counts: vec![0i32; 10_240], num_vectors: 0 }
    }

    pub fn add(&mut self, vector: &HdcVector) {
        for word_idx in 0..160 {
            let word = vector.0[word_idx];
            for bit in 0..64 {
                let global_bit = word_idx * 64 + bit;
                if (word >> bit) & 1 == 1 {
                    self.counts[global_bit] += 1;
                } else {
                    self.counts[global_bit] -= 1;
                }
            }
        }
        self.num_vectors += 1;
    }

    pub fn finalize(&self) -> HdcVector {
        let mut result = [0u64; 160];
        for word_idx in 0..160 {
            let mut word = 0u64;
            for bit in 0..64 {
                let global_bit = word_idx * 64 + bit;
                // Threshold at > 0 for 1, else 0.
                // Ties (count == 0) break to 0.
                if self.counts[global_bit] > 0 {
                    word |= 1u64 << bit;
                }
            }
            result[word_idx] = word;
        }
        HdcVector(result)
    }
}
```

**Why `i32` counters?** The `i32` range supports bundles of up to ~2 billion
vectors, far exceeding any practical use. Using `i16` would limit to 32K
vectors per bundle (adequate but tight for some future extensions). The
`i32` cost is 40KB per accumulator, which is negligible.

**Why `i32` counters and not raw binary majority? (The saturation problem.)**
Beyond the capacity argument, there is a deeper reason the accumulator uses
`i32` vote counters rather than performing bit-by-bit majority voting
directly. Naive binary majority bundling *saturates* at high load factor:
when more than ~sqrt(D) vectors are bundled via raw majority vote, each bit
position approaches a 50/50 split between 0s and 1s, and the result converges
toward random noise -- quasi-orthogonal to all constituents. This is the
fundamental capacity wall of BSC (see Document 02, "The Bundling Saturation
Problem").

The `i32` accumulator avoids premature information loss by deferring
binarization. During accumulation, the counters preserve full magnitude
information -- a vector added 10 times has 10x the vote weight, and this
magnitude is retained until `finalize()` collapses the counters to binary.
This design is effectively MAP-I (Multiply-Add-Permute, Integer variant)
during the accumulation phase, only dropping back to BSC at the final step.
The practical consequence: **weighted bundling is a zero-cost capability.**
Give important vectors higher weight by calling `add()` multiple times or
by multiplying the counter increment, and their signal survives binarization
even when the total vector count exceeds the naive capacity limit.

Nearly every recent HDC paper with strong empirical results uses some form
of weighted or thresholded bundling rather than raw majority vote. The
BundleAccumulator's `i32` counters are the mechanism that makes this
possible within BSC. The three principal strategies that the counters enable
are: (1) weighted bundling (add vectors multiple times proportional to
importance), (2) normalized bundling (periodically binarize and reset
counters to bound the effective load factor), and (3) decaying bundling
(exponentially down-weight older additions so recent signals dominate).

The saturation limit still applies at `finalize()` -- if 200 vectors are
bundled with equal weight, the binarized output is unreliable. Consumers
of the BundleAccumulator should ensure the effective load factor (number
of significantly-weighted constituents) stays well below sqrt(D) ~ 100,
using one of the strategies above to manage capacity.

**Why ties break to 0?** When an even number of vectors are bundled and a bit
position has exactly half ones and half zeros, the tie must be broken
deterministically. Breaking to 0 is simpler than breaking to the result of a
seeded PRNG, equally valid mathematically (it introduces a slight bias toward
zero, which is negligible at D=10,240), and trivially verifiable across
validators. The alternative -- breaking to 1 -- would work equally well but
would be a different protocol. The choice is arbitrary; the determinism is not.

### Consensus-Critical Properties

All operations in the HDC algebra layer are consensus-critical. This means:

- **No floating-point.** The `similarity()` function returns `f64` and is
  explicitly marked "local use only." Every on-chain path uses
  `hamming_distance()` which returns `u32`. Integer arithmetic is
  deterministic across all platforms.

- **No platform-dependent operations.** The `count_ones()` function compiles
  to hardware `POPCNT` on x86 and `cnt` on ARM, but both produce identical
  results. The Rust compiler guarantees this: `u64::count_ones()` is defined
  by the language, not by the hardware.

- **No ordering-dependent operations.** XOR is commutative and associative.
  Bundle's majority vote is commutative. The result does not depend on the
  order in which operations are evaluated.

- **Ties break to 0.** Not to a random value, not to the previous value,
  not to the value from a PRNG. Zero. Always.

### What NOT to Change

- Do not switch to MAP or HRR. BSC is the right choice for hardware
  efficiency and consensus determinism. MAP requires signed multiplication;
  HRR requires FFT. Both introduce floating-point to the core algebra.
- Do not reduce dimensionality below 10,240. This is the minimum for
  ~100 item bundle capacity at 99% retrieval.
- Do not add floating-point to the core algebra. FP is only in the
  similarity convenience function and the encoding layer.

---

## Layer 2: Encoding (Input -> Hypervector)

How raw data becomes HDC vectors. Three encoders, each optimized for a
different use case. All three produce the same type (`HdcVector`) and their
outputs participate in the same similarity search -- this is the composability
principle in action.

### TrigramEncoder: Text -> Hypervector (~100ns)

For real-time cognitive operations where speed matters more than semantic
fidelity. The trigram approach encodes text as overlapping 3-character windows,
capturing local orthographic structure.

**Why trigrams?** Trigrams (3-character windows) are the sweet spot between
unigrams (which lose all structural information) and longer n-grams (which
become too sparse to generalize). Damashek (1995) demonstrated that trigram-based
similarity metrics achieve surprisingly strong performance on text classification
and language identification tasks, with the key advantage that no linguistic
preprocessing (tokenization, stemming, stop-word removal) is needed. The text
is treated as a raw byte stream.

Trigrams capture local structure while remaining position-invariant at the
document level. The permutation step (`permute(i)`) reintroduces position
sensitivity at the trigram level -- the same trigram occurring at position 5
and position 500 produces different vectors -- while the final bundle
aggregates all trigrams into a single representation that is similar to any
document containing similar trigram distributions.

```rust
pub struct TrigramEncoder {
    item_memory: ItemMemory, // Trigram -> HdcVector mapping
}

impl TrigramEncoder {
    /// Encode a text string as an HDC vector via character trigrams.
    ///
    /// Algorithm:
    /// 1. Slide a 3-byte window over the input text.
    /// 2. For each trigram at position i, look up (or generate) the
    ///    corresponding atomic vector from item memory.
    /// 3. Permute the atomic vector by i to encode position.
    /// 4. Accumulate into a BundleAccumulator (majority vote).
    /// 5. Finalize to produce a single 10,240-bit vector.
    ///
    /// Performance: ~100ns for typical text lengths (< 1000 bytes).
    /// The bottleneck is the bundle accumulator, not the lookup.
    pub fn encode(&self, text: &str) -> HdcVector {
        let trigrams = text.as_bytes().windows(3);
        let mut acc = BundleAccumulator::new();
        for (i, trigram) in trigrams.enumerate() {
            let base = self.item_memory.get_or_create(trigram);
            acc.add(&base.permute(i as i32)); // Position-aware
        }
        acc.finalize()
    }
}
```

**Concrete example:** Encoding the text "gas spike" produces trigrams:
`"gas"`, `"as "`, `"s s"`, `" sp"`, `"spi"`, `"pik"`, `"ike"`. Each trigram
is mapped to a random 10,240-bit vector via the item memory (deterministic
from seed), permuted by its position index, and bundled. The resulting vector
is similar to any other text containing many of the same trigrams ("gas price
spike", "gas spiked again") but orthogonal to unrelated text ("bond yield
curve").

> Damashek, M. (1995). "Gauging Similarity with n-Grams: Language-Independent
> Categorization of Text." *Science*, 267(5199), 843-848.
> doi:10.1126/science.267.5199.843

### ProjectionEncoder: Float Embedding -> Hypervector (~10ms)

For converting dense float embeddings (from sentence-transformers, OpenAI, etc.)
into binary HDC vectors. Used when publishing to the shared substrate, where
semantic fidelity matters more than encoding speed.

**Why random binary projection?** The Johnson-Lindenstrauss (JL) lemma guarantees
that random projections preserve pairwise distances up to a (1 +/- epsilon)
factor, provided the target dimensionality is at least O(log(n) / epsilon^2)
where n is the number of points. For n=1M points and epsilon=0.1, this requires
~2,000 dimensions -- well below our 10,240. The projection is a simple matrix
multiplication followed by a sign threshold: positive dot products map to 1,
negative to 0.

The projection matrix is a D x d binary matrix where D=10,240 (output
dimensionality) and d is the input dimensionality. For bge-small-en-v1.5
(d=384), the matrix is 10,240 x 384 = 3,932,160 entries. At 1 bit per entry
(binary projection), this is ~480KB. At f16 per entry (for better distance
preservation), it is ~7.5MB. We use binary projection (entries drawn from
{-1, +1}) for determinism and compactness, accepting the slight reduction
in distance preservation.

```rust
pub struct ProjectionEncoder {
    matrix: ProjectionMatrix, // D x d, deterministic from seed
}

/// **How the projection matrix is generated and distributed.**
///
/// The matrix is NOT shipped as a binary artifact. A single u64 seed
/// (`PROJECTION_SEED`) is stored in the genesis config. Every validator
/// runs this function at startup to produce a bit-identical matrix.
/// For bge-small-en-v1.5 (d=384), the matrix is 10,240 x 384 entries
/// = ~480KB at 1 bit per entry. Generation takes <100ms.
///
/// Recommended storage: `const PROJECTION_SEED: u64` in genesis config.
/// Matrix generated lazily via `LazyLock` and cached in memory for the
/// lifetime of the process.
fn generate_projection_matrix(seed: u64, d_out: usize, d_in: usize) -> ProjectionMatrix {
    let mut rng = ChaCha20Rng::seed_from_u64(seed);
    let mut matrix = vec![vec![0i8; d_in]; d_out];
    for row in &mut matrix {
        for val in row.iter_mut() {
            // Binary random projection: each entry is +1 or -1
            *val = if rng.gen::<bool>() { 1 } else { -1 };
        }
    }
    ProjectionMatrix::from(matrix)
}

impl ProjectionEncoder {
    /// Project a float embedding (e.g., from sentence-transformer) to HDC.
    ///
    /// Algorithm:
    /// 1. For each of the 10,240 output bits:
    ///    a. Compute the dot product of the matrix row with the embedding.
    ///    b. If positive, set the bit to 1. Otherwise, 0.
    /// 2. The result is a binary vector that preserves the cosine
    ///    similarity structure of the original embedding space
    ///    (by the JL lemma).
    ///
    /// The matrix is generated deterministically from a seed using
    /// ChaCha20. All validators derive the same matrix from the
    /// same seed, stored in the genesis config.
    /// CONSENSUS SAFETY: This function uses f32 dot products and
    /// summation, which are subject to floating-point non-associativity.
    /// The .sum() call reduces 384 f32 multiplications — different
    /// reduction orders (e.g., pairwise vs left-fold, SIMD width
    /// differences) can produce different rounding, potentially flipping
    /// the sign of near-zero dot products and changing the output bit.
    ///
    /// For OFF-CHAIN use (agent encoding before publication), this is
    /// acceptable — the resulting vector is then published on-chain as
    /// raw bytes, and all validators store the same bytes.
    ///
    /// For ON-CHAIN use (if a precompile ever re-encodes from embeddings),
    /// the matrix MUST use binary {-1, +1} entries (not floats), and the
    /// dot product becomes an integer sum of +/- e[j] values. With binary
    /// projection, the sign test produces identical results on all
    /// platforms because integer addition is commutative and associative.
    pub fn project(&self, embedding: &[f32]) -> HdcVector {
        let mut result = [0u64; 160];
        for (i, row) in self.matrix.rows().enumerate() {
            let dot: f32 = row.iter().zip(embedding).map(|(m, e)| (*m as f32) * e).sum();
            if dot > 0.0 {
                result[i / 64] |= 1u64 << (i % 64);
            }
        }
        HdcVector(result)
    }
}
```

**Concrete example:** An agent wants to publish an insight about "liquidity
pool impermanent loss during high volatility." It first generates a dense
embedding using bge-small-en (384-dimensional float vector capturing deep
semantic meaning). The ProjectionEncoder multiplies this 384-dim vector by
the 10,240 x 384 projection matrix and thresholds the result, producing a
10,240-bit binary vector. This vector preserves the semantic relationships:
it will be similar to other projected embeddings about "liquidity risk,"
"impermanent loss," and "volatility impact" -- but now it participates in
the same algebraic system as trigram-encoded vectors.

> Johnson, W. B., & Lindenstrauss, J. (1984). "Extensions of Lipschitz
> mappings into a Hilbert space." *Contemporary Mathematics*, 26, 189-206.

### StructuredEncoder: Role-Filler Binding

For knowledge entries with typed fields. Uses the Tensor Product Representation
(Smolensky, 1990): a structured record is encoded as the bundle of role-filler
bindings, where each field's value is bound with a pre-assigned role vector.

**Why role-filler binding?** It allows structured queries. Given a record
`r = bind(ROLE_kind, kind_vec) + bind(ROLE_content, content_vec) + ...`,
you can extract a specific field by binding with the role vector:
`bind(r, ROLE_content) ~ content_vec`. The result is approximately similar
to the original filler, enabling structured decomposition of composed
representations.

```rust
pub struct StructuredEncoder {
    // CONSENSUS SAFETY: accessed by key lookup only, not iterated.
    // Safe as HashMap. Replace with BTreeMap if iteration is ever needed.
    roles: HashMap<String, HdcVector>,
    text_encoder: TrigramEncoder,
}

impl StructuredEncoder {
    /// Encode a KnowledgeEntry as an HDC vector using role-filler binding.
    ///
    /// Each field of the entry is bound with its corresponding role vector,
    /// then all bindings are bundled into a single superposition.
    /// The result is similar to queries that match any field.
    pub fn encode_knowledge(&self, entry: &KnowledgeEntry) -> HdcVector {
        let mut acc = BundleAccumulator::new();

        // Each field bound with its role vector
        acc.add(&self.roles["kind"].bind(&self.kind_vector(entry.kind)));
        acc.add(&self.roles["content"].bind(&self.text_encoder.encode(&entry.content)));
        acc.add(&self.roles["tier"].bind(&self.tier_vector(entry.tier)));

        if let Some(tag) = &entry.emotional_tag {
            acc.add(&self.roles["emotion"].bind(&self.text_encoder.encode(tag)));
        }

        acc.finalize()
    }

    /// Encode an episode as context + action + outcome with
    /// permutation-based directionality.
    ///
    /// The permutation on context (by 2) and action (by 1) creates a
    /// temporal ordering: context precedes action precedes outcome.
    /// This ensures that bind(context, action) != bind(action, context).
    pub fn encode_episode(&self, context: &str, action: &str, outcome: &str) -> HdcVector {
        let ctx = self.text_encoder.encode(context);
        let act = self.text_encoder.encode(action);
        let out = self.text_encoder.encode(outcome);

        // Bind all three with permutation for directionality
        ctx.permute(2).bind(&act.permute(1).bind(&out))
    }

    /// Encode a directional causal link.
    ///
    /// The permutation on the cause vector breaks the symmetry of XOR bind:
    /// bind(permute(cause), effect) != bind(permute(effect), cause).
    /// To query for "what does X cause?", compute permute(X) and search.
    /// To query for "what causes Y?", search directly with Y.
    pub fn encode_causal(&self, cause: &str, effect: &str) -> HdcVector {
        let c = self.text_encoder.encode(cause);
        let e = self.text_encoder.encode(effect);
        c.permute(1).bind(&e) // Permutation breaks symmetry
    }

    /// Encode anti-knowledge by binding with the ANTI_SUBSPACE vector.
    ///
    /// ANTI_SUBSPACE is generated deterministically from a fixed seed
    /// constant (see doc 04 for generation code). All validators must
    /// use the same seed so anti-knowledge vectors are bit-identical
    /// across the network.
    ///
    /// This places the anti-knowledge in a structurally distinct subspace.
    /// anti(X) is quasi-orthogonal to X (similarity ~ 0.5), so it will
    /// NOT surface in normal retrieval for X. To check if X has been
    /// contradicted, explicitly search the anti-index.
    pub fn encode_anti(&self, knowledge: &HdcVector) -> HdcVector {
        knowledge.bind(&ANTI_SUBSPACE) // Structural separation
    }
}
```

**Concrete example of structured encoding in practice:**

An agent stores the insight "High gas prices correlate with token unlock events"
with confidence 0.8, tier Working, and emotional tag "caution":

```
record = bundle([
    bind(ROLE_kind,    kind_vec_insight),
    bind(ROLE_content, trigram("High gas prices correlate with token unlock events")),
    bind(ROLE_tier,    tier_vec_working),
    bind(ROLE_emotion, trigram("caution")),
])
```

Later, the agent can query with `trigram("gas prices")` and retrieve this
record because the content field contains matching trigrams. It can also
query with `trigram("caution")` and retrieve it via the emotional field,
enabling mood-congruent retrieval.

> Smolensky, P. (1990). "Tensor Product Variable Binding and the Representation
> of Symbolic Structures in Connectionist Systems." *Artificial Intelligence*,
> 46(1-2), 159-216. doi:10.1016/0004-3702(90)90007-M

### Note: BSC Binding Limitations and the GHRR Alternative

The encoding layer currently uses BSC (XOR) binding with permute-before-bind
to encode directional relationships. This is correct and sufficient for the
current workload — `encode_causal` and `encode_episode` both use `permute(k)`
to break XOR's commutativity before binding. However, this approach has a
known limitation: XOR is commutative (`A XOR B = B XOR A`), so the ordering
guarantee comes from the permutation convention, not from the binding operator
itself.

For future multi-hop path encoding — e.g., encoding a chain of knowledge-graph
relations like `r1 -> r2 -> r3` as a single hypervector — **Generalized
Holographic Reduced Representations (GHRR)** should be evaluated as an
alternative binding algebra. GHRR replaces scalar elements with m x m unitary
matrices, making binding (element-wise matrix multiplication) inherently
non-commutative. PathHD (Liu et al., 2025) demonstrated that GHRR binding
achieves 86.2% Hits@1 on WebQSP for knowledge-graph path retrieval, vs 83.9%
for commutative XOR — a 2.3 percentage-point improvement attributable directly
to non-commutative binding. Similarly, conjunctive block coding (CLOG) offers
improved memory capacity for structured graph representations through block
encoding schemes.

If this layer evolves to support multi-hop KG queries, transaction sequence
encoding, or deep causal chain inference, the encoding pipeline could be
extended with a GHRR-based path encoder alongside the existing BSC encoders.
The bundling (majority vote) and similarity (Hamming distance) operations
would remain unchanged — only the binding operator for path composition would
change. See `02-hdc-foundations.md`, section "BSC Limitations and Alternative
Algebras," for the full analysis.

---

## Layer 3: Storage & Search

### Local Index (In-Process)

The local index automatically switches between brute-force and HNSW based on
the number of stored vectors. This is the key architectural decision for
Layer 3.

**Why auto-switch at 100K?** Brute-force has zero overhead -- no graph to
build, no parameters to tune, no approximation error. Its search time is O(N):
scan every vector, compute Hamming distance, keep the top-K in a min-heap.
At N=1K, this takes ~50 microseconds. At N=100K, it takes ~5ms. HNSW, by
contrast, has build cost (constructing the navigable small-world graph) but
O(log N) search time: at N=100K, it takes ~0.1ms; at N=1M, it takes ~0.5ms.
The crossover point -- where HNSW's build cost is amortized by its search
savings -- is approximately 100K vectors. Below this, brute-force is faster
overall. Above this, HNSW dominates.

The auto-switch is one-directional: once the index upgrades to HNSW, it stays
HNSW even if vectors are deleted below 100K. Downgrading would require
rebuilding the brute-force array, which is an unnecessary allocation.

```rust
pub enum LocalIndex {
    BruteForce(BruteForceIndex),  // N < 100K
    Hnsw(HnswIndex),             // N >= 100K
}

impl LocalIndex {
    pub fn insert(&mut self, key: H256, vector: HdcVector) {
        match self {
            Self::BruteForce(idx) => {
                idx.insert(key, vector);
                if idx.len() >= HNSW_THRESHOLD {
                    *self = Self::Hnsw(idx.upgrade_to_hnsw());
                }
            }
            Self::Hnsw(idx) => idx.insert(key, vector),
        }
    }

    pub fn search(&self, query: &HdcVector, top_k: usize) -> Vec<(H256, u32)> {
        match self {
            Self::BruteForce(idx) => idx.search(query, top_k),
            Self::Hnsw(idx) => idx.search(query, top_k),
        }
    }

    pub fn remove(&mut self, key: H256) {
        match self {
            Self::BruteForce(idx) => { idx.remove(key); }
            Self::Hnsw(idx) => { idx.delete(key); }
        }
    }
}
```

### HNSW Parameters

The following parameters are used for all HNSW indexes (local and on-chain).
These values are from doc 06 and represent the standard trade-off for
binary vectors at 10,240-bit dimensionality:

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `M` | 16 | Max connections per node per layer. Standard sweet spot. |
| `ef_construction` | 200 | Beam width during index construction. Robust default for high-recall graphs. |
| `ef_search` | 50-200 | Beam width during query. Tunable at query time: ef=50 gives ~95% recall; ef=200 gives ~99.5% recall. Default 100 for >99% recall. |
| `M_max0` | 2*M = 32 | Max connections at level 0 (denser for higher search throughput). |
| `level_multiplier` | 1/ln(M) ~ 0.361 | Controls geometric distribution of nodes across levels. |

### HNSW: Determinism Requirements

HNSW (Hierarchical Navigable Small World graphs) uses randomized level
assignment during graph construction. For on-chain use, this randomization
must be fully deterministic:

1. **Seeded PRNG.** Level assignment uses ChaCha20 seeded from the vector's
   content hash combined with its insertion index. Same vector, same insertion
   order, same level.

2. **Canonical insertion order.** Vectors must be inserted in a deterministic
   order (e.g., sorted by key hash). If two validators insert the same set of
   vectors in different orders, they will build different graphs and produce
   different search results. The insertion order is part of the protocol.

3. **Deterministic tie-breaking.** When two candidate neighbors have identical
   Hamming distances during graph construction, the tie is broken by key hash
   (lexicographic comparison of H256). This ensures all validators make
   identical decisions at every step.

> Malkov, Y. A., & Yashunin, D. A. (2020). "Efficient and Robust Approximate
> Nearest Neighbor Using Hierarchical Navigable Small World Graphs."
> *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 42(4),
> 824-836. doi:10.1109/TPAMI.2018.2889473

### On-Chain Index (Precompile)

```rust
pub struct OnChainHdcIndex {
    /// In-memory index, rebuilt from events on startup
    index: LocalIndex,
    /// Metadata for each vector (author, block, kind)
    /// CONSENSUS SAFETY: HashMap has non-deterministic iteration order in
    /// Rust. This is safe here ONLY because metadata is accessed by key
    /// lookup (not iterated). If any future code iterates this map (e.g.,
    /// for GC, serialization, snapshotting), it MUST be replaced with
    /// BTreeMap<H256, VectorMetadata> to guarantee deterministic ordering
    /// across validators.
    metadata: HashMap<H256, VectorMetadata>,
}

impl OnChainHdcIndex {
    /// Called during block execution for storeVector transactions
    pub fn store(&mut self, key: H256, vector: HdcVector, meta: VectorMetadata) {
        self.index.insert(key, vector);
        self.metadata.insert(key, meta);
    }

    /// Called during eth_call for searchSimilar (no state change)
    pub fn search(&self, query: &HdcVector, top_k: usize) -> Vec<(H256, u32)> {
        self.index.search(query, top_k)
    }

    /// Rebuild from event log on startup.
    /// Events are replayed in block order, ensuring deterministic
    /// reconstruction of the index on every node.
    pub fn rebuild_from_events(&mut self, events: impl Iterator<Item = InsightEvent>) {
        for event in events {
            match event {
                InsightEvent::Published { key, vector, meta } => {
                    self.store(key, vector, meta);
                }
                InsightEvent::Deleted { key } => {
                    self.index.remove(key);
                    self.metadata.remove(&key);
                }
            }
        }
    }
}
```

### Tiered Search (On-Chain Gas Optimization)

On-chain search is metered by gas. Full brute-force comparison of 10,240 bits
costs ~5,000 gas per vector. For an index of 100K vectors, that is 500M gas --
far exceeding any block gas limit. The tiered search pipeline reduces this by
90-99%:

```
+-----------+   ~90% rejected   +-----------+   ~9% rejected   +-----------+
| Tier 1:   | ----------------> | Tier 2:   | ----------------> | Tier 3:   |
| First-word|                   | Sample    |                   | Full      |
| filter    |                   | Hamming   |                   | Hamming   |
|           |                   | (16/160   |                   | (160/160  |
| ~100 gas  |                   |  words)   |                   |  words)   |
| per vector|                   | ~500 gas  |                   | ~5000 gas |
+-----------+                   +-----------+                   +-----------+
```

**Tier 1: First-word filter.** Compare only the first u64 word of the query
with the first u64 word of each candidate. A single word's Hamming distance
is a noisy estimator of the full vector's Hamming distance, but it is unbiased:
if the full distance is D_full, then the expected first-word distance is
D_full / 160. Vectors whose first-word distance exceeds a lenient threshold
can be safely rejected with high probability. Cost: ~100 gas per candidate
(1 XOR + 1 POPCNT + 1 comparison). Eliminates ~90% of candidates.

**Tier 2: Sample-word comparison.** For vectors that pass Tier 1, compare
16 evenly-spaced words (indices 0, 10, 20, ..., 150). This samples 10% of
the vector, giving a distance estimate accurate to +/-5% with high probability
(by concentration inequalities on i.i.d. binary bits). Cost: ~500 gas per
candidate. Eliminates ~90% of remaining candidates (9% of original).

**Tier 3: Full Hamming distance.** For the ~1% of candidates surviving Tier 2,
compute the exact 160-word Hamming distance. Cost: ~5,000 gas per candidate.
This produces the final, exact ranking.

**Net effect:** For a 100K-vector index, the tiered pipeline uses approximately
100K x 100 + 10K x 500 + 1K x 5,000 = 10M + 5M + 5M = 20M gas, versus 500M
gas for brute-force. A 25x reduction, bringing on-chain search within block
gas limits.

> **Cross-reference with doc 06:** Doc 06's detailed breakdown at 100K vectors
> gives ~21M gas total (including ~1M overhead for heap management and indexing),
> consistent with the 20M estimate here. Doc 06 also shows that beyond 100K
> vectors, even tiered search becomes infeasible (~510M gas at 1M vectors),
> requiring the HNSW precompile for O(log N) search at fixed gas cost.

```rust
impl OnChainHdcIndex {
    /// Gas-optimized search using tiered pipeline
    pub fn search_tiered(
        &self,
        query: &HdcVector,
        top_k: usize,
        gas_limit: u64,
    ) -> (Vec<(H256, u32)>, u64) {
        let mut gas_used = 0u64;

        // Tier 1: First-word filter (~100 gas per candidate)
        let query_word0 = query.0[0];
        let mut tier1_pass = Vec::new();
        for (i, vector) in self.index.vectors().enumerate() {
            gas_used += 100;
            let word_dist = (query_word0 ^ vector.0[0]).count_ones();
            if word_dist < TIER1_THRESHOLD {
                tier1_pass.push(i);
            }
            if gas_used > gas_limit { break; }
        }

        // Tier 2: Sample-word comparison (~500 gas per candidate)
        let mut tier2_pass = Vec::new();
        for i in tier1_pass {
            gas_used += 500;
            let approx_dist = approximate_hamming(&query.0, &self.index.vectors()[i].0);
            if approx_dist < TIER2_THRESHOLD {
                tier2_pass.push((i, approx_dist));
            }
        }

        // Tier 3: Full comparison (~5000 gas per candidate)
        let mut results = Vec::new();
        for (i, _) in tier2_pass {
            gas_used += 5000;
            let exact_dist = query.hamming_distance(&self.index.vectors()[i]);
            results.push((self.index.keys()[i], exact_dist));
        }

        // Sort by distance, take top-K.
        // CONSENSUS SAFETY: sort_by_key is stable (Rust guarantees this),
        // but when distances are equal, the tiebreaker must be deterministic.
        // Use (distance, key) as the composite sort key so that all validators
        // produce identical result ordering.
        results.sort_by(|(k1, d1), (k2, d2)| d1.cmp(d2).then_with(|| k1.cmp(k2)));
        results.truncate(top_k);

        (results, gas_used)
    }
}
```

---

## Layer 4: Knowledge Management

### Knowledge Store (Unified)

The unified KnowledgeStore holds all six knowledge kinds (Insight, Heuristic,
AntiKnowledge, Warning, CausalLink, StrategyFragment) in a single store,
backed by a single LocalIndex for similarity search. Anti-knowledge gets a
*separate* index in a structurally distinct subspace, preventing it from being
confused with regular knowledge during retrieval.

**Why one store, not six?** Because the power of HDC is that all knowledge kinds
live in the same vector space. A single similarity search can retrieve an insight,
a causal link, and a heuristic that are all relevant to the current query. If the
kinds were in separate indexes, cross-kind retrieval would require N separate
searches.

**Why a separate anti-index?** Because anti-knowledge serves a different
purpose than regular knowledge. It is not retrieved to answer queries -- it is
retrieved to *suppress* other results. Searching the anti-index is a validation
step, not a retrieval step. Keeping it separate makes this intention explicit
and avoids the 76.7% reproduction problem documented in 04-knowledge.md.

```rust
pub struct KnowledgeStore {
    /// HDC index for similarity search
    index: LocalIndex,

    /// Structured entries with full metadata.
    /// CONSENSUS SAFETY: KnowledgeStore is LOCAL (off-chain), so HashMap
    /// iteration order does not affect consensus. However, the tick()
    /// function iterates this map for decay and GC. If KnowledgeStore
    /// is ever shared or synchronized across validators, this MUST be
    /// changed to BTreeMap to ensure deterministic iteration order.
    entries: HashMap<H256, KnowledgeEntry>,

    /// Anti-knowledge index (separate subspace)
    anti_index: LocalIndex,

    /// Encoder for new knowledge
    encoder: StructuredEncoder,

    /// Decay parameters
    decay_config: DecayConfig,
}

/// DUPLICATE_THRESHOLD: Hamming distance below which two vectors are
/// considered near-duplicates. 512 out of 10,240 bits = similarity > 0.95.
/// Tight enough to catch reformulations; loose enough for related-but-distinct
/// insights. Matches the on-chain InsightBoard constant (see doc 07).
const DUPLICATE_THRESHOLD: u32 = 512;

/// RESONANCE_THRESHOLD_HAMMING: Hamming distance below which an anti-knowledge
/// entry is considered to contradict a knowledge entry. Looser than
/// DUPLICATE_THRESHOLD because anti-knowledge should catch broader matches.
/// 1,024 out of 10,240 bits = similarity > 0.90.
const RESONANCE_THRESHOLD_HAMMING: u32 = 1024;

impl KnowledgeStore {
    /// Store new knowledge
    pub fn store(&mut self, entry: KnowledgeEntry) -> Result<H256> {
        let vector = self.encoder.encode_knowledge(&entry);
        // NOTE: keccak256 expects &[u8]. Cast [u64; 160] to a byte slice,
        // e.g. bytemuck::cast_slice(&vector.0). Use little-endian byte
        // order for cross-platform determinism.
        let key = H256::from(keccak256(bytemuck::cast_slice(&vector.0)));

        // Duplicate check
        let similar = self.index.search(&vector, 1);
        if let Some((_, dist)) = similar.first() {
            if *dist < DUPLICATE_THRESHOLD {
                // Near-duplicate -- reinforce existing instead
                return self.reinforce(similar[0].0);
            }
        }

        // Anti-knowledge check
        if entry.kind == KnowledgeKind::AntiKnowledge {
            let anti_vector = self.encoder.encode_anti(&vector);
            self.anti_index.insert(key, anti_vector);
        } else {
            // Check if contradicted by existing anti-knowledge
            let anti_query = self.encoder.encode_anti(&vector);
            let anti_matches = self.anti_index.search(&anti_query, 1);
            if let Some((_, dist)) = anti_matches.first() {
                if *dist < RESONANCE_THRESHOLD_HAMMING {
                    return Err(Error::ContradictedByAntiKnowledge);
                }
            }
        }

        self.index.insert(key, vector);
        self.entries.insert(key, entry);
        Ok(key)
    }

    /// Retrieve with trust-weighted scoring
    pub fn search(
        &self,
        query: &HdcVector,
        top_k: usize,
        context: &RetrievalContext,
    ) -> Vec<ScoredEntry> {
        let raw = self.index.search(query, top_k * 3); // Over-fetch for filtering

        let mut scored: Vec<ScoredEntry> = raw.iter()
            .filter_map(|(key, dist)| {
                let entry = self.entries.get(key)?;
                let score = self.compute_score(entry, *dist, context);
                Some(ScoredEntry { key: *key, entry, score, hamming: *dist })
            })
            .collect();

        // Anti-knowledge filtering
        scored.retain(|s| !self.is_contradicted(&s.entry));

        // Sort by composite score (not just similarity).
        // CONSENSUS SAFETY NOTE: This sort is OFF-CHAIN (local context
        // assembly). Two concerns for implementers:
        //   1. partial_cmp().unwrap() will panic on NaN. Use total_cmp()
        //      or handle NaN explicitly to avoid validator crashes.
        //   2. When scores are equal (f64 comparison), sort stability
        //      matters. Use sort_by (stable) with a tiebreaker key
        //      (e.g., H256 key) to ensure deterministic ordering.
        //      sort_unstable would produce different orderings for
        //      equal-score entries across runs.
        scored.sort_by(|a, b| {
            b.score.total_cmp(&a.score)
                .then_with(|| a.key.cmp(&b.key)) // deterministic tiebreaker
        });
        scored.truncate(top_k);

        // Mandatory contrarian inclusion (15%)
        let contrarian_count = (top_k as f64 * 0.15).ceil() as usize;
        let contrarian_query = query.bind(&ANTI_SUBSPACE);
        let contrarian = self.index.search(&contrarian_query, contrarian_count);
        for (key, dist) in contrarian {
            if let Some(entry) = self.entries.get(&key) {
                scored.push(ScoredEntry {
                    key, entry, hamming: dist,
                    score: self.compute_score(entry, dist, context) * 0.8,
                });
            }
        }

        scored
    }
}
```

### Four-Factor Scoring

The scoring function combines four signals, weighted to reflect their relative
importance. The weights are inspired by Park et al. (2023), who found that a
recency-importance-relevance triad produced the best retrieval quality for
generative agents, extended here with an emotional dimension from the ALMA
affect system.

```
score = 0.35 * relevance + 0.20 * recency + 0.25 * importance + 0.20 * emotional
```

| Factor | Weight | Signal | Range |
|--------|--------|--------|-------|
| **Relevance** | 0.35 | Hamming similarity to query | 0.0 - 1.0 (normalized) |
| **Importance** | 0.25 | Confidence x tier weight x ln(confirmations + 1) | 0.0 - ~5.0 (unclamped) |
| **Recency** | 0.20 | exp(-(current_tick - last_accessed) / decay) | 0.0 - 1.0 |
| **Emotional** | 0.20 | PAD distance between entry tag and current mood | 0.0 - 1.0 |

> **Note: Importance formula (resolved).** Both this document and doc 05 now use
> `ln(confirmation_count + 1)`, which is well-defined at 0 confirmations (ln(1) = 0)
> and avoids special-case floors. Implementers should use `ln(confirmations + 1)` as
> shown in the code below.

**Why these weights?** Relevance is highest (0.35) because retrieving irrelevant
knowledge -- no matter how important or recent -- wastes context tokens. Importance
is second (0.25) because well-confirmed, high-tier knowledge should surface
preferentially. Recency and emotional are equal (0.20 each) because both serve
as tiebreakers: when two entries are equally relevant and important, prefer
the more recent one or the one that matches the agent's current emotional state.

```rust
/// OFF-CHAIN / LOCAL ONLY. compute_score runs in the agent's local knowledge
/// store as part of context assembly (Layer 5). It does NOT execute on-chain.
///
/// CONSENSUS SAFETY: This function uses f64 arithmetic throughout:
///   - f64 division (hamming as f64 / 10240.0)
///   - f64::exp() (transcendental — platform-dependent rounding)
///   - f64::ln() (transcendental — platform-dependent rounding)
///   - f64 multiplication chains
///
/// These are acceptable OFF-CHAIN because each agent independently scores
/// its own local knowledge. Minor cross-platform differences in f64
/// rounding do not affect consensus — they only change which knowledge
/// fragments appear in a given agent's context window (a local decision).
///
/// DO NOT use this function in any on-chain path (precompile, contract,
/// or validator-executed logic). If scoring is ever needed on-chain, all
/// f64 operations must be replaced with fixed-point integer arithmetic.
fn compute_score(
    &self,
    entry: &KnowledgeEntry,
    hamming: u32,
    ctx: &RetrievalContext,
) -> f64 {
    let relevance = 1.0 - (hamming as f64 / 10240.0);
    let recency = (-(ctx.current_tick.saturating_sub(entry.last_accessed) as f64)
        / self.decay_config.recency_decay).exp();
    let importance = entry.confidence
        * entry.tier.weight()
        * (entry.confirmation_count as f64 + 1.0).ln();
    let emotional = ctx.mood
        .as_ref()
        .map(|m| emotional_score(entry, m))
        .unwrap_or(0.5);

    0.35 * relevance + 0.20 * recency + 0.25 * importance + 0.20 * emotional
}
```

> Park, J. S., O'Brien, J. C., Cai, C. J., et al. (2023). "Generative Agents:
> Interactive Simulacra of Human Behavior." *UIST 2023*.
> doi:10.1145/3586183.3606763

### Tick-Based Maintenance

Each block (~400ms), the knowledge store runs a maintenance cycle. This is not
a background process -- it is a deterministic step in the agent's cognitive loop,
executed between the Learn and Perceive phases.

> **Consensus safety note:** The `tick()` function runs on the agent's
> *local* knowledge store, not on-chain. Floating-point arithmetic (the
> `exp()` call) is acceptable here because minor platform differences in
> decay timing do not affect consensus. On-chain decay uses the
> `fixed_point_decay` function defined in the Demurrage Implementation
> section below.

The maintenance cycle performs four operations:

1. **Decay balances.** Apply the demurrage formula to every entry's balance.
2. **Garbage collection.** Identify entries whose balance has fallen below the
   GC threshold (0.01) and remove them (unless they are Persistent tier, in
   which case they are demoted to Consolidated rather than deleted).
3. **Tier promotions.** Entries that have accumulated sufficient confirmations
   are promoted to the next tier, extending their effective half-life.
4. **Tier demotions.** Entries that have not been queried within a
   kind-specific inactivity window are demoted. Working entries with 0
   queries in 5x their half-life period demote to Transient. Consolidated
   entries with 0 queries in 10x their half-life period demote to Working.
   (See doc 04 for the complete demotion table.)

```rust
/// Periodic maintenance -- called once per block.
pub fn tick(&mut self, current_tick: u64) {
    // 1. Decay balances using demurrage formula
    for entry in self.entries.values_mut() {
        entry.balance *= (-self.decay_config.lambda
            * current_tick.saturating_sub(entry.last_decay_tick) as f64).exp();
        entry.last_decay_tick = current_tick;
    }

    // 2. GC: remove below-threshold entries
    let gc_keys: Vec<H256> = self.entries.iter()
        .filter(|(_, e)| e.balance < self.decay_config.gc_threshold
            && e.tier != KnowledgeTier::Persistent)
        .map(|(k, _)| *k)
        .collect();

    for key in gc_keys {
        self.index.remove(key);
        self.entries.remove(&key);
    }

    // 3. Tier promotions
    for entry in self.entries.values_mut() {
        if entry.confirmation_count >= 25 && entry.tier < KnowledgeTier::Persistent {
            entry.tier = KnowledgeTier::Persistent;
        } else if entry.confirmation_count >= 10 && entry.tier < KnowledgeTier::Consolidated {
            entry.tier = KnowledgeTier::Consolidated;
        } else if entry.confirmation_count >= 3 && entry.tier < KnowledgeTier::Working {
            entry.tier = KnowledgeTier::Working;
        }
    }

    // 4. Tier demotions (inactivity-based)
    for entry in self.entries.values_mut() {
        let ticks_since_query = current_tick.saturating_sub(entry.last_queried_tick);
        let half_life_ticks = entry.effective_half_life_ticks();
        match entry.tier {
            KnowledgeTier::Working if ticks_since_query > 5 * half_life_ticks => {
                entry.tier = KnowledgeTier::Transient;
            }
            KnowledgeTier::Consolidated if ticks_since_query > 10 * half_life_ticks => {
                entry.tier = KnowledgeTier::Working;
            }
            // Persistent: manual demotion only (never automatic)
            _ => {}
        }
    }
}
```

### Demurrage Implementation

The balance decay formula is:

```
balance(t) = balance(t_0) * exp(-0.005 * delta_t_hours)
```

Where `lambda = 0.005 per hour` is the base decay rate. The effective half-life
is `ln(2) / (lambda / tier_multiplier)`, which gives the following base
half-lives (before kind-specific factors):

| Tier | Multiplier | Base half-life (ln(2) / (lambda / mult)) |
|------|------------|------------------------------------------|
| Transient | 0.1x | ~13.9 hours |
| Working | 0.5x | ~69.3 hours |
| Consolidated | 1.0x | ~138.6 hours |
| Persistent | 5.0x | ~693 hours (~29 days) |

These base half-lives are further modified by kind-specific factors.
Document 04 specifies kind base half-lives (Insight: 72h, Heuristic: 168h,
CausalLink: 240h, Warning: 48h, StrategyFragment: 120h, AntiKnowledge: 336h)
that scale independently. The full formula is:

```
effective_half_life = kind_base_half_life * tier_multiplier
```

For example, an Insight (72h base) at the Working tier (0.5x) has an
effective half-life of 36h. See doc 04 for the complete kind-by-tier table.

**Consensus safety of decay computation.** The `exp()` function involves
floating-point arithmetic, which is non-deterministic across platforms. For
local-only operations (the agent's private knowledge store), this is acceptable
-- minor differences in decay timing across platforms do not affect consensus.
For on-chain operations, decay must use fixed-point arithmetic:

```rust
/// Fixed-point exponential decay for consensus-safe computation.
/// Uses integer arithmetic only.
///
/// Approximation: exp(-x) ~ (1 - x/N)^N for small x/N.
/// We use N=256 for adequate precision.
fn fixed_point_decay(balance_basis_points: u64, lambda_bps: u64, delta_ticks: u64) -> u64 {
    // Saturating multiply prevents u64 overflow for large delta_ticks.
    let x = lambda_bps.saturating_mul(delta_ticks); // in basis points
    let n = 256u64;
    let step = x / n;
    // Guard: if step >= 10_000 bps (i.e., each sub-step decays by >= 100%),
    // the balance is fully decayed. Without this check, (10_000 - step)
    // would underflow u64, wrapping to a huge number and corrupting the result.
    if step >= 10_000 {
        return 0;
    }
    let mut result = balance_basis_points;
    for _ in 0..n {
        result = result * (10_000 - step) / 10_000;
    }
    result
}
```

**Fixed-point representation details:**

All on-chain balances and decay parameters use **basis points** (1 bps =
0.01% = 1/10,000) as the fixed-point unit. This is equivalent to a Q0.14
representation with a denominator of 10,000. A balance of 10,000 bps = 1.0;
a balance of 5,000 bps = 0.5.

Worked example: an Insight at the Working tier (half-life = 36h = 324,000
ticks at 400ms/tick) with current balance 8,000 bps, after 1 hour (9,000
ticks):

```
Per-tick lambda_bps = ln(2) / half_life_ticks * 10_000
                    = 0.693 / 324_000 * 10_000
                    ~ 0.021 bps per tick (too small for integer arithmetic)

Per-hour lambda_bps = ln(2) / half_life_hours * 10_000
                    = 0.693 / 36 * 10_000
                    ~ 193 bps per hour

x = 193 * 1 = 193 (for 1 hour)
step = 193 / 256 = 0  (integer division -- still underflows at N=256)
```

The per-hour rate of 193 bps still underflows at N=256 substeps because
193/256 rounds to 0 in integer division. In practice, `lambda_bps` must
be scaled per evaluation window large enough that `x / 256 >= 1`. For
a 36h half-life, using a 2-hour window gives x = 386, step = 1, which
produces meaningful decay. Alternatively, increase N or use a coarser
time granularity. The key invariant: every validator computes
`result * (10_000 - step) / 10_000` using integer division with the same
truncation behavior, so all validators agree on the decayed balance.

### Anti-Knowledge Subspace

The anti-knowledge system works by maintaining two parallel search spaces:

1. **Normal index.** Contains all regular knowledge (Insight, Heuristic,
   CausalLink, StrategyFragment, Warning).
2. **Anti-index.** Contains anti-knowledge vectors, each produced by
   `bind(knowledge_vector, ANTI_SUBSPACE)`.

When searching for knowledge relevant to a query Q:

1. Search the normal index with Q. Get results R.
2. For each result r in R, check if an anti-knowledge entry exists by
   searching the anti-index with `bind(r.vector, ANTI_SUBSPACE)`.
3. If a match is found (Hamming distance below RESONANCE_THRESHOLD_HAMMING),
   the result is contradicted and is filtered out.

This is more expensive than a simple metadata flag check (it requires an
additional search per result), but it is structurally safe. Anti-knowledge
vectors are quasi-orthogonal to the knowledge they contradict, so they cannot
accidentally surface in normal retrieval. The binding with ANTI_SUBSPACE is
self-inverse: `bind(bind(X, ANTI), ANTI) = X`, which means the original
knowledge vector can always be recovered from the anti-knowledge vector.

---

## Layer 5: Context Assembly

### The Full Pipeline

Context assembly is the bridge between the knowledge store (Layer 4) and the
LLM prompt. It transforms a task description into a ranked, budget-constrained
set of knowledge fragments formatted for LLM consumption.

The pipeline has four stages:

```
  Task                                                      LLM Prompt
   |                                                            ^
   v                                                            |
+--------+    +---------+    +----------+    +----------+    +----------+
| Gather | -> |  Rank   | -> | Compress | -> | Assemble | -> | 9-layer  |
|        |    |         |    |          |    |          |    | prompt   |
| HDC    |    | 4-factor|    | Token    |    | Prompt   |    |          |
| search |    | scoring |    | budget   |    | builder  |    |          |
+--------+    +---------+    +----------+    +----------+    +----------+
```

**Gather:** Construct a query vector from the current task context. Apply
somatic bias from the affect system (mood-congruent retrieval). Search the
local index for the top-30 candidates. If local results are insufficient
(aggregate score below threshold), also query the shared substrate and apply
the trust pipeline to shared results.

**Rank:** Score each candidate using the four-factor formula (relevance 0.35,
importance 0.25, recency 0.20, emotional 0.20). Sort by composite score.

**Compress:** Fit the ranked candidates into the token budget. High-value
entries that exceed the remaining budget are summarized rather than truncated.
Low-value entries that do not fit are dropped entirely.

**Assemble:** Build the 9-layer prompt from the compressed knowledge plus
identity, capabilities, world state, task context, constraints, history,
emotional state, and output format.

```rust
pub struct ContextAssembler {
    local_store: KnowledgeStore,
    chain_substrate: ChainSubstrate,
    prompt_builder: PromptBuilder,
    affect: AffectSystem,
}

impl ContextAssembler {
    pub fn assemble(&self, task: &Task, budget: TokenBudget) -> Prompt {
        // 1. Construct query from task context
        let query = self.task_to_query(task);

        // 2. Apply somatic bias from affect system
        let biased_query = self.affect.apply_bias(&query);

        // 3. Gather candidates from all sources
        let retrieval_ctx = RetrievalContext {
            current_tick: self.current_tick(),
            mood: Some(self.affect.current_mood()),
        };

        let mut local = self.local_store.search(&biased_query, 30, &retrieval_ctx);

        // 4. Optionally query shared substrate.
        // SUBSTRATE_QUERY_THRESHOLD = 5.0 (sum of top-30 local scores).
        // If local knowledge is rich enough (sum > 5.0), skip the chain
        // query to save gas/latency. If sparse, supplement with shared.
        if local.iter().map(|s| s.score).sum::<f64>() < SUBSTRATE_QUERY_THRESHOLD {
            let shared = self.chain_substrate.search(&biased_query, 20);
            let trusted = self.apply_trust_pipeline(shared, &retrieval_ctx);
            local.extend(trusted);
            // Use total_cmp (not partial_cmp) to avoid NaN panics,
            // with deterministic tiebreaker on key hash.
            local.sort_by(|a, b| {
                b.score.total_cmp(&a.score)
                    .then_with(|| a.key.cmp(&b.key))
            });
        }

        // 5. VCG auction for context space allocation
        let winners = self.vcg_allocate(&local, budget.knowledge_tokens);

        // 6. Build prompt with 9 layers
        self.prompt_builder.build(task, &winners, &self.affect.current_state())
    }
}
```

### VCG Auction for Space Allocation

When multiple knowledge entries compete for limited context tokens, the system
uses a Vickrey-Clarke-Groves (VCG) auction to allocate space. Each entry "bids"
its composite score. The auction maximizes total value of the selected set
subject to the token budget constraint.

The key property of VCG is **truthfulness**: each entry's optimal strategy is
to report its true value. In this context, that means the scoring function
does not need to be "gamed" -- it can be the honest four-factor score.

The VCG payment (the externality each winner imposes on the other candidates)
is tracked and used to discount the entry's priority in future rounds. This
prevents the same high-value entries from monopolizing the context window
across consecutive ticks. An entry that displaces many smaller entries pays
a high externality and gets a temporary priority reduction.

### Mandatory Contrarian Retrieval (15%)

At least 15% of the context budget is reserved for knowledge that *opposes*
the query. This is implemented by constructing a contrarian query
(`query.bind(&ANTI_SUBSPACE)`) and searching the normal index with it. The
results are entries that are structurally dissimilar to the original query
but may contain valuable opposing viewpoints.

**Why 15%?** It is enough to surface at least one or two contrarian entries
in a typical context window, but not so much that it drowns out the
task-relevant knowledge. The value is from the roko spec and was chosen
based on the observation that human decision-making quality improves when
~10-20% of the evidence presented is contradictory (Schulz-Hardt et al., 2006).

### Mood-Congruent Retrieval

The affect system produces a somatic bias vector from the ALMA three-layer model
(Emotion, Mood, Personality). This bias vector is bundled with the task query
to subtly shift retrieval toward mood-congruent knowledge:

- **High arousal:** Preferentially retrieve urgent, action-oriented knowledge.
- **Low pleasure:** Preferentially retrieve warnings, anti-knowledge, caution.
- **High dominance:** Preferentially retrieve strategies and action plans.

The bias weight is proportional to the arousal level: a calm agent (low arousal)
applies minimal bias; an excited or anxious agent (high arousal) applies stronger
bias. This models the well-documented psychological phenomenon of mood-congruent
memory retrieval (Bower, 1981).

> Lewis, P., Perez, E., Piktus, A., et al. (2020). "Retrieval-Augmented
> Generation for Knowledge-Intensive NLP Tasks." *NeurIPS 2020*.
> arXiv:2005.11401
>
> Bricken, T., & Pehlevan, C. (2021). "Attention Approximates Sparse Distributed
> Memory." *NeurIPS 2021*. arXiv:2111.05498
>
> Schulz-Hardt, S., et al. (2006). "Group Decision Making in Hidden Profile
> Situations." *Journal of Personality and Social Psychology*, 91(6), 1080-1093.

### The 9-Layer Prompt

```
Layer 1: Identity        -- Agent name, role, core directives
Layer 2: Capabilities    -- What tools/contracts the agent can use
Layer 3: World State     -- Current block, balances, relevant chain state
Layer 4: Task Context    -- The specific task being worked on
Layer 5: Knowledge       -- Retrieved knowledge fragments (HDC output)
Layer 6: Constraints     -- Rules, limitations, anti-patterns to avoid
Layer 7: History         -- Recent interaction history (compressed)
Layer 8: Emotional State -- Current PAD values, behavioral mode
Layer 9: Output Format   -- Expected response structure
```

Knowledge (Layer 5) is where HDC context assembly inserts its output.
The other layers are composed from different sources (chain state, task
manager, affect system). Token budgets are allocated per-layer to ensure
each component gets its minimum allocation even under tight constraints.

---

## Layer 6: Chain Integration

### Precompile Interface (0x09)

The HDC precompile is a native execution module at address 0x09 in the daeji
EVM. Unlike Solidity contracts, precompiles execute native Rust code, enabling
the performance required for vector operations. Each operation has a fixed,
deterministic gas cost.

```
+-----------------------------------------------+
|  HDC Precompile at 0x09                       |
|                                                |
|  Selector 0x01: storeVector(key, vector)      |
|    Gas: 25,000 + storage                      |
|    Writes to in-memory index + emits event    |
|                                                |
|  Selector 0x02: searchSimilar(vector, topK)   |
|    Gas: 50,000 (flat) or tiered               |
|    Reads from in-memory index                 |
|                                                |
|  Selector 0x03: deleteVector(key)             |
|    Gas: 5,000                                  |
|    Removes from index + emits event           |
|                                                |
|  Selector 0x04: bundleVectors(keys[])         |
|    Gas: 15,000 + 2,000/key                    |
|    Computes majority vote of stored vectors   |
|                                                |
|  Selector 0x05: bindVectors(keyA, keyB)       |
|    Gas: 8,000                                  |
|    XOR of two stored vectors                  |
|                                                |
|  Selector 0x06: hamming(keyA, keyB)           |
|    Gas: 2,100                                  |
|    Raw Hamming distance (uint32)              |
+-----------------------------------------------+
```

**Gas cost rationale:**

These costs reflect the *precompile-level* gas charges (i.e., the cost of
the native HDC operation itself, including any vector lookups from the
in-memory index). They are higher than the raw operation costs listed in
doc 07's precompile proposal table (which showed raw computational costs
of ~1,500-10,000 gas) because these costs include the overhead of reading
vectors from the index, event emission, and bookkeeping. The total
*transaction-level* cost of a publish operation is ~145,000 gas (calldata
+ precompile + storage + event), as detailed in doc 07.

| Operation | Gas | Rationale |
|-----------|-----|-----------|
| storeVector | 25,000 | Index insertion + event emission. Comparable to a simple SSTORE. |
| searchSimilar | 50,000 | Flat cost covers brute-force over ~10K vectors. For larger indexes, tiered search reduces effective cost. |
| deleteVector | 5,000 | Index removal + event emission. Cheaper than store because no data written. |
| bundleVectors | 15,000 + 2,000/key | Linear in the number of keys. The 15K base covers accumulator setup; 2K/key covers lookup + vote counting. |
| bindVectors | 8,000 | Two vector lookups + XOR of 160 words. |
| hamming | 2,100 | Two vector lookups + 160-word XOR + popcount. The cheapest operation. |

### InsightBoard Contract

The InsightBoard is the on-chain contract managing the full lifecycle of shared
knowledge. It interacts with the HDC precompile for vector operations and with
the ReputationRegistry for trust scoring.

**Lifecycle states (7 states):**

The canonical enum is defined in the InsightBoard contract (doc 07):

```
enum State { SUBMITTED, VERIFIED, ACTIVE, CHALLENGED, DECAYING, ARCHIVED, PURGED }
```

> **Terminology note:** This document uses the canonical name CHALLENGED
> (matching the Solidity enum in doc 07). Earlier drafts used "DISPUTED"
> which has been corrected.

```
                          ┌──────────────────────────┐
                          │                          │
                          │    ┌──── re-confirm ─────┘
                          │    │
                          v    │
 (new)──►SUBMITTED──►VERIFIED──►ACTIVE──►DECAYING──►ARCHIVED──►PURGED──►(removed)
              │          ▲       │  ▲        │           │
              │          │       │  │        │           │
              │     quarantine   │  └────────┘           │
              │      release     │   re-confirm          │
              │                  │                       │
              │                  v                  renew │
              │             CHALLENGED─────────────►─────┘
              │               │    │
              │               │    └──► ACTIVE (5+ confs)
              │               │
              │               └──► ARCHIVED (unconfirmed for 2x half-life)
              │
              └──► DECAYING (age > 1x half-life, no confirmation)
```

- **SUBMITTED:** Initial state. The insight has been published with stake but
  has not yet been verified by any other agent.
- **VERIFIED:** At least one independent agent has confirmed the insight.
  Transitions to ACTIVE. (Currently collapsed with ACTIVE in the contract;
  exists as a forward-compatible placeholder.)
- **ACTIVE:** The insight is actively participating in search results and
  accruing confirmations. This is the steady state.
- **CHALLENGED:** Anti-knowledge has been published against this entry (HDC
  anti-subspace resonance > 0.7). The challenge is resolved by community
  confirmation (5+ confs reverts to ACTIVE) or rejection over time (no
  confs for 2x half-life transitions to ARCHIVED). Trust pipeline applies
  a 0.3 multiplier. See doc 07 for the full challenge resolution flow.
- **DECAYING:** The insight's age exceeds its effective half-life. It remains
  searchable but its trust score is declining. Can be revived by re-confirmation
  (returns to ACTIVE) or by renewal (gas + stake).
- **ARCHIVED:** The insight has decayed beyond 5x its half-life. No longer
  returned in search results but still on-chain for provenance. Can be
  renewed to ACTIVE via `renew()`.
- **PURGED:** The insight has decayed beyond 10x its half-life. Eligible for
  storage cleanup via `purge()` transaction.

> **Implementation note (resolved):** The `confirm()` function in doc 07
> accepts confirmations for insights in SUBMITTED, ACTIVE, DECAYING, or
> CHALLENGED state. The first confirmation promotes SUBMITTED -> ACTIVE.
> Re-confirmation recovers DECAYING -> ACTIVE. For CHALLENGED entries,
> 5+ new confirmations (tracked by `confirmsSinceChallenge` counter)
> resolve the challenge back to ACTIVE. `computeState()` preserves the
> stored early-lifecycle state while applying decay thresholds for aged
> insights. The contract also implements `renew()` (ARCHIVED -> ACTIVE)
> and `purge()` (PURGED -> removed) as described in the transition table.

**Key operations:**

- `submit(kind, vector, content)` -- Publish a new insight with stake (payable).
  Checks for duplicates via the HDC precompile. Emits InsightPublished event.
- `confirm(insightId)` -- Confirm an existing insight. Increments confirmation
  count, resets decay clock, triggers tier promotion checks. Accepts SUBMITTED,
  ACTIVE, DECAYING, and CHALLENGED insights.
- `renew(insightId)` -- Renew an ARCHIVED insight (payable). Resets timestamps
  and transitions to ACTIVE. Requires fresh stake >= MIN_STAKE.
- `purge(insightId)` -- Remove a PURGED insight. Deletes from precompile index,
  clears storage (SSTORE refund), returns 10% of stake to original author.
- `searchSimilar(queryVector, topK)` -- View function. Delegates to HDC
  precompile. No gas cost for the caller.
- `computeState(insightId)` -- View function. Computes the current lifecycle
  state based on age, tier, and kind.

See 07-shared-substrate.md for the full Solidity implementation.

### Agent Identity Verification via BAID

The InsightBoard's trust model currently relies on on-chain addresses as
publisher identity — an agent is identified by the key it uses to sign
transactions. This is necessary but insufficient: a stolen key allows code
substitution attacks where an attacker publishes insights using a trusted
agent's credentials but running different (potentially malicious) code.

The **Binding Agent ID (BAID)** protocol (Lin et al., arXiv:2512.17538, 2025)
addresses this gap by using zkVM-based code-level authentication. In BAID,
the agent's program binary is cryptographically committed and embedded in the
agent identifier. A recursive proof chain demonstrates that each execution
step was performed by the committed code, preventing impersonation even if
an attacker obtains the agent's signing key.

**Integration with the InsightBoard:**

The `submit()` function can optionally accept a BAID proof alongside the
insight vector and content. When present, the trust pipeline gains an
additional signal:

```
Stage 0 (optional): Code identity    -- BAID proof verifies publisher is
                                        running committed, audited code
Stage 1: Source reputation           -- ReputationRegistry.composite_score(author)
Stage 2: Recency discount            -- 0.5^(age_blocks / tier_half_life)
...
```

Insights published with valid BAID proofs receive a trust boost because the
publisher has demonstrated not just key possession but code integrity. This
is particularly valuable for high-stakes insight types (e.g., THESIS,
FRAMEWORK) where the encoding model's provenance matters.

**Cost profile:** Agent registration is a one-time cost (~507K gas). BAID
proof verification runs in milliseconds (14-93ms depending on chain length),
which is compatible with the InsightBoard's submission flow. See
07-shared-substrate.md for the full zkML landscape analysis and BAID
technical details.

### PheromoneRegistry Contract

Stigmergic coordination through digital pheromones with exponential decay
and SINR (Signal-to-Interference-plus-Noise Ratio) interference modeling.

**Three pheromone types:**

| Type | Half-Life (blocks) | Duration (~) | Purpose |
|------|-------------------|--------------|---------|
| THREAT | 100 | ~40s | Warn others of dangers (flash loan attacks, oracle manipulation) |
| OPPORTUNITY | 250 | ~100s | Signal profitable situations (arbitrage windows, underpriced assets) |
| WISDOM | 1000 | ~400s | Mark locations of valuable knowledge in the substrate |

**SINR interference:** When multiple pheromones of the same type overlap at the
same location, the effective signal uses SINR rather than simple summation:

```
SINR(target) = intensity(target) / (sum(intensity(interferers)) + noise_floor)
```

This prevents signal flooding. Depositing 100 OPPORTUNITY pheromones at the same
location does not make the signal 100x stronger -- it adds noise. Only a single
strong signal or a few well-separated signals produce high SINR.

**Alpha Paradox:** When a pheromone is confirmed (another agent validates the
signal), its half-life is *reduced*, not extended. Rationale: if many agents
already know about an opportunity, it is less valuable as information. Fresh,
unconfirmed signals carry the most information.

### Trust Pipeline (5-Stage)

When an agent retrieves knowledge from the shared substrate, it applies a
5-stage trust computation:

```
Stage 1: Source reputation     -- ReputationRegistry.composite_score(author)
Stage 2: Recency discount      -- 0.5^(age_blocks / tier_half_life)
Stage 3: Confirmation boost    -- min(1.0, 0.5 + 0.05 * confirmations)
Stage 4: Stake signal          -- min(1.0, 0.5 + ln(staked / MIN_STAKE) / 10)
Stage 5: Context relevance     -- similarity(publication_context, current_context)

effective_trust = product of all five factors
```

Only entries with `effective_trust > MIN_TRUST_THRESHOLD` (default: **0.05**)
enter the agent's context window. This threshold is deliberately low because
the multiplicative pipeline already filters aggressively. It exists as a
safety floor, not a primary filter. The multiplicative combination means
that a single low factor (e.g., low source reputation) can sink the overall
trust even if other factors are high.

### Gas Economics Summary

These are *transaction-level* costs (calldata + precompile + storage + events
combined). Doc 07 provides the full breakdown for each operation.

| Operation | Gas Cost | USD at 1 gwei, $3K/ETH |
|-----------|----------|------------------------|
| Publish insight | ~145,000 | ~$0.44 |
| Confirm insight | ~30,000-35,000 | ~$0.09-$0.11 |
| Search (view) | 0 (caller) | Free |
| Deposit pheromone | ~40,000-45,000 | ~$0.12-$0.14 |
| Delete insight | ~15,000 | ~$0.05 |

The economic design encourages cheap reads (search is free) and expensive writes
(publication requires gas + stake). This aligns with the desired behavior:
agents should consume knowledge freely but publish selectively.

---

## On-Chain vs Off-Chain Boundary — Determinism Classification

> **This table is consensus-critical.** Every operation listed as "ON-CHAIN"
> must use exclusively deterministic arithmetic (integer, fixed-point, no
> floats, no HashMap iteration, no unseeded RNG, no wall-clock time).
> Operations listed as "OFF-CHAIN" may use floats, randomness, and
> platform-dependent behavior because they are local agent decisions
> that do not enter the consensus path.

| Operation | Location | Arithmetic | Notes |
|-----------|----------|------------|-------|
| **HDC Algebra** (bind, bundle, permute) | ON-CHAIN (precompile 0x09) | Integer only (XOR, popcount, majority vote) | Consensus-safe by design. Ties break to 0. |
| **hamming_distance()** | ON-CHAIN | Integer (u32) | The ONLY distance function used on-chain. |
| **similarity()** (normalized) | OFF-CHAIN only | f64 division | Convenience function. NEVER use on-chain. |
| **BundleAccumulator** | ON-CHAIN (precompile bundleVectors) | Integer (i32 counters) | Consensus-safe. |
| **storeVector / deleteVector** | ON-CHAIN (precompile) | Integer | Index insert/remove, event emission. |
| **searchSimilar** | ON-CHAIN (precompile) | Integer | Tiered search uses u32 Hamming distances only. |
| **HNSW level assignment** | ON-CHAIN (precompile index) | Integer only (leading_zeros) | MUST NOT use f64::ln(). See doc 06 fix. |
| **HNSW neighbor selection** | ON-CHAIN | Integer (u32 distance + u64 tiebreaker) | Canonical insertion order required. |
| **InsightBoard.submit()** | ON-CHAIN (contract) | Integer / Solidity | Duplicate check via precompile. |
| **InsightBoard.confirm()** | ON-CHAIN (contract) | Integer | Confirmation count, tier promotion. |
| **InsightBoard.computeState()** | ON-CHAIN (view) | Integer | Block-age comparison, integer thresholds. |
| **PheromoneRegistry decay** | ON-CHAIN (view) | Fixed-point integer | MUST NOT use f64::powf(). See doc 07 note. |
| **PheromoneRegistry SINR** | ON-CHAIN (view) | Fixed-point integer | Integer division with basis-point scaling. |
| **Demurrage (on-chain)** | ON-CHAIN | Fixed-point integer | Use fixed_point_decay(), not f64::exp(). |
| **Demurrage (local store)** | OFF-CHAIN | f64 (acceptable) | Minor cross-platform differences do not affect consensus. |
| **compute_score() (4-factor)** | OFF-CHAIN | f64 (exp, ln, division) | Local scoring for context assembly. |
| **compute_trust() (5-stage)** | OFF-CHAIN | f64 (powf, ln) | Local trust pipeline applied to shared results. |
| **TrigramEncoder** | OFF-CHAIN | Integer (XOR, permute, bundle) | Encoding is local; result published as raw bytes. |
| **ProjectionEncoder** | OFF-CHAIN | f32 (dot product) | Encoding is local; result published as raw bytes. |
| **Context Assembly** | OFF-CHAIN | f64 (scoring, sorting) | Entirely local to each agent. |
| **Affect System (ALMA/PAD)** | OFF-CHAIN | f64 | Mood, emotion, personality — all local. |
| **Somatic Bias** | OFF-CHAIN | f64 | Mood-congruent retrieval weighting — local. |
| **Dream Cycle** | OFF-CHAIN | f64 | Consolidation, creative cross-binding — local. |
| **Reputation EMA update** | ON-CHAIN (if in contract) | Fixed-point integer | EMA: `new = (alpha * obs + (1-alpha) * old)` — use basis-point math. |
| **Taint propagation** | ON-CHAIN (if in contract) | Integer (enum comparison) | Monotonic lattice, integer thresholds. |
| **Immune Memory check** | ON-CHAIN (if in submit path) | Integer (u32 Hamming) | Match decision uses u32 threshold, not f64 similarity. |

> **Rule of thumb:** If a function is called during block execution
> (transaction processing, state transition, precompile invocation), it is
> ON-CHAIN and must be deterministic. If it is called by the agent's local
> process (cognitive loop, context assembly, affect system), it is OFF-CHAIN
> and may use floats.

---

## Non-Negotiable Properties

These properties are not optimizations or nice-to-haves. They are load-bearing
constraints that the entire architecture depends on. Violating any of them
would compromise the system's correctness.

### 1. Determinism

**Every operation in the on-chain path must produce identical results on
every validator.** This is the foundational requirement for consensus: if
two validators execute the same transaction and produce different results,
the chain forks. The consequence is absolute: no floating-point arithmetic
in the consensus path, no platform-dependent behavior, no unseeded randomness.

Specific implications:
- `hamming_distance()` returns `u32`, not `f64`. The normalized `similarity()`
  is explicitly marked local-only.
- Bundle tie-breaking is deterministic (bit stays 0), not random.
- HNSW level assignment uses seeded ChaCha20, not `rand::thread_rng()`.
- Vector insertion order is canonical (sorted by key hash).
- The projection matrix is generated from a seed stored in genesis config.

### 2. Bit-Exact Reproducibility

A stronger form of determinism. Not just "same result" but "same bits."
This rules out:

- **Floating-point operations** in the consensus path. IEEE 754 permits
  fused multiply-add (FMA) instructions that produce different rounding
  than separate multiply-and-add. Different compilers and optimization
  levels may reorder FP operations, changing results. Even the same code
  on the same hardware can produce different results with `-O2` vs `-O3`.

- **Platform-dependent SIMD** semantics. AVX2 and AVX-512 produce identical
  results for integer operations (XOR, POPCNT) but may differ for FP
  operations. Since all HDC algebra is integer, this is safe -- but it must
  remain integer.

- **Ordering-dependent operations.** HashMap iteration order is not
  deterministic in Rust. Any operation that iterates over a HashMap and
  depends on the order (e.g., tie-breaking by "first seen") will produce
  different results across runs. Use BTreeMap or sorted Vec when order matters.

### 3. Composability

The same algebra (bind/bundle/permute/hamming) must work at every layer --
encoding, storage, search, verification. No special cases. A vector produced
by the TrigramEncoder participates in the same search as a vector produced by
the ProjectionEncoder. A vector stored locally participates in the same
algebra as a vector stored on-chain. This composability is the core value
proposition of HDC over ad-hoc approaches.

### 4. Separation of Trust

Local knowledge = full trust. Shared knowledge = skeptical trust. The trust
pipeline is not optional. An agent that treats shared knowledge with the same
trust as local knowledge is vulnerable to knowledge poisoning attacks. The
5-stage trust pipeline (source reputation, recency, confirmation, stake,
context relevance) must be applied to every piece of shared knowledge before
it enters the context window.

### 5. Graceful Degradation

If the shared substrate is unavailable (network partition, gas too high,
validator down), agents must function on local knowledge alone. The chain
is an enhancement, not a dependency. Concretely:

- If the RPC call to the precompile times out, skip substrate search and
  assemble context from local results only.
- If gas prices exceed a configurable threshold, defer publication and
  continue operating on local knowledge.
- If the agent has no local knowledge (cold start), it can function with
  an empty knowledge store -- the LLM still receives identity, capabilities,
  world state, and task context in the prompt.

### 6. Anti-Knowledge Safety

Structural subspace separation. Not metadata flags. Not warning labels.
Research shows that warning-framed content is reproduced at a 76.7% rate
by LLMs (Zhang et al., 2023). The only safe way to handle "this is false"
knowledge is to place it in a structurally distinct subspace where it
cannot accidentally surface during normal retrieval.

### 7. Gas Efficiency

Every on-chain operation must fit within block gas limits. For daeji's
target block gas limit (~30M gas), the most expensive single operation
(searchSimilar over a large index with tiered search) must complete within
this limit. This constrains the maximum practical on-chain index size to
approximately 100K-300K vectors, which is adequate for a shared knowledge
substrate (agents publish selectively, not exhaustively).

### 8. Side-Channel Awareness

**Reference**: Sapui, B. & Tahoori, M., "Leaks beyond Bits: Deep
Learning-Assisted Side-Channel Attacks on Hyperdimensional Computing
Accelerators," ICCAD 2025.

Binary hypervectors do not provide privacy. Recent research demonstrates
that CNN-based power side-channel analysis can extract stored hypervector
bits from FPGA-based HDC accelerators with up to 93% accuracy. The
pseudo-random appearance of BSC vectors is an information-theoretic
property of the encoding, not a cryptographic guarantee.

For daeji, this has two implications:

- **On-chain vectors are public anyway.** Every vector published to the
  shared substrate is visible to all validators and indexers. The on-chain
  threat model is knowledge *poisoning*, not knowledge *theft*. No design
  decision should rely on the opacity of on-chain HDC vectors.

- **Local vectors require explicit protection.** An agent's private
  knowledge store — unpublished reasoning, strategic state, draft
  hypotheses — must be protected by standard cryptographic means (encrypted
  storage, secure enclaves) rather than relying on the apparent randomness
  of the vectors themselves. If agents use hardware HDC accelerators for
  local inference, the dynamic masking defense described by Sapui and
  Tahoori (which reduces bit extraction accuracy to ~18% at ~1.6x LUT and
  ~1.4x latency overhead) should be considered for deployments where
  physical access to the hardware is a realistic threat.

This property is non-negotiable because a false sense of privacy is worse
than acknowledged transparency. The architecture must be designed so that
*no security property depends on the opacity of hypervector contents* —
not on-chain, not locally, not in transit.

> **SECURITY NOTE — "In transit" is underspecified.**
> The principle correctly states that no security property should depend
> on vector opacity, but the architecture does not specify transport-
> level protections for agent-to-chain communication. Agent RPC calls
> include full 1,280-byte query vectors in `searchSimilar` calldata. A
> network observer can infer task focus from query content, track query
> patterns to model strategic interests, and front-run by observing
> queries before corresponding transactions land. **Minimum:** all RPC
> traffic must use TLS. For stronger guarantees, consider query
> obfuscation or private mempool submission.

With the non-negotiable properties established, the following schedule
organizes the implementation into five priority tiers, each building on
the previous tier's outputs.

---

## Implementation Priority

### P0: Foundation (Week 1-2)

The core that everything else depends on. No other work can begin until P0
is complete and tested.

| Component | Types / Functions | Dependencies | Acceptance Criteria |
|-----------|-------------------|--------------|---------------------|
| HdcVector | `HdcVector`, `bind()`, `bundle()`, `permute()`, `hamming_distance()`, `similarity()`, `random()` | None | Property tests pass: quasi-orthogonality, self-inverse bind, majority preservation |
| BundleAccumulator | `BundleAccumulator::new()`, `add()`, `finalize()` | HdcVector | Deterministic tie-breaking, correct majority vote for odd and even input counts |
| ItemMemory | `ItemMemory::new(seed)`, `get_or_create()` | HdcVector, ChaCha20 | Same seed produces same vectors on all platforms |
| TrigramEncoder | `TrigramEncoder::encode()` | ItemMemory, BundleAccumulator | Encodes text, similar texts produce similar vectors |
| BruteForceIndex | `insert()`, `search()`, `remove()` | HdcVector | Correct top-K retrieval, O(N) performance verified |
| Basic encoding | `StructuredEncoder::encode_knowledge()`, `encode_episode()`, `encode_causal()`, `encode_anti()` | TrigramEncoder, ItemMemory | Role-filler binding produces queryable records |

### P1: Knowledge Layer (Week 3-4)

The knowledge management layer, including scoring, decay, and the local index
auto-switch.

| Component | Types / Functions | Dependencies | Acceptance Criteria |
|-----------|-------------------|--------------|---------------------|
| KnowledgeStore | `store()`, `search()`, `reinforce()`, `tick()` | LocalIndex, StructuredEncoder, BundleAccumulator | Duplicate detection, anti-knowledge filtering, GC works |
| Four-factor scoring | `compute_score()` | KnowledgeStore, RetrievalContext | Weights sum to 1.0, scoring matches expected ranking |
| LocalIndex (auto-switch) | `insert()` with HNSW upgrade | BruteForceIndex, HnswIndex | Transparent switch at 100K vectors, no result quality degradation |
| Decay / GC | `tick()`, `gc_candidates()` | KnowledgeStore | Entries decay exponentially, GC removes below-threshold, Persistent entries demoted not deleted |
| Context assembly (basic) | `ContextAssembler::assemble()` | KnowledgeStore, PromptBuilder | Produces valid 9-layer prompts, respects token budget |
| HDC precompile (skeleton) | `storeVector()`, `searchSimilar()`, `hamming()` | HdcVector, BruteForceIndex | Passes through to in-memory index, gas metering correct |

### P2: Search & Assembly (Week 5-8)

HNSW implementation, tiered search, and the full context assembly pipeline.

| Component | Types / Functions | Dependencies | Acceptance Criteria |
|-----------|-------------------|--------------|---------------------|
| HnswIndex | `insert()`, `search()`, `remove()` | HdcVector | >99% recall at ef_search=100, deterministic graph construction |
| HNSW determinism | Seeded level generation, canonical insertion order, deterministic tie-breaking | HnswIndex, ChaCha20 | Two independent builds from same data produce identical graphs |
| Tiered search | `search_tiered()` | OnChainHdcIndex | 25x gas reduction vs brute-force at 100K vectors |
| VCG allocation | `vcg_allocate()` | ContextAssembler | Truthful allocation, externality-based priority decay |
| Contrarian retrieval | 15% budget reservation | KnowledgeStore, ANTI_SUBSPACE | At least 1 contrarian entry in every context window |
| ProjectionEncoder | `project()` | ProjectionMatrix, ChaCha20 | JL distance preservation verified empirically |

### P3: Chain Integration (Week 9-12)

On-chain contracts, full precompile, and the trust pipeline.

| Component | Types / Functions | Dependencies | Acceptance Criteria |
|-----------|-------------------|--------------|---------------------|
| HDC precompile (full) | All 6 selectors with gas metering | HdcVector, LocalIndex, TieredSearch | Passes consensus tests: two validators produce identical results |
| InsightBoard | `submit()`, `confirm()`, `renew()`, `purge()`, `searchSimilar()`, `computeState()` | HDC precompile | Full lifecycle (SUBMITTED -> PURGED) works, tier promotions trigger correctly, CHALLENGED resolution via confirmsSinceChallenge |
| PheromoneRegistry | `deposit()`, `scan()`, `decay()` | SINR model | Exponential decay, SINR interference, alpha paradox |
| Trust pipeline | 5-stage trust computation | InsightBoard, ReputationRegistry | Multiplicative trust, minimum threshold filtering |
| Event log replay | `rebuild_from_events()` | OnChainHdcIndex | Index rebuilt identically from event log on every node |

### P4: Advanced Cognitive Features (Week 13+)

The features that make agents genuinely intelligent over long time horizons.
These are valuable but not blocking.

| Component | Types / Functions | Dependencies | Acceptance Criteria |
|-----------|-------------------|--------------|---------------------|
| Affect system (ALMA/PAD) | `AffectSystem`, `apply_bias()`, `current_mood()` | KnowledgeStore | Three-layer dynamics: emotion (fast), mood (medium), personality (slow) |
| Dream cycle | `consolidate()`, `creative_cross_bind()` | KnowledgeStore, Mattar-Daw scoring | NREM consolidation strengthens confirmed knowledge; REM cross-binding finds novel associations |
| Resonator networks | `ResonatorNetwork::factorize()` | HdcVector | Decompose bound pairs from bundled superpositions with capacity ~N^2 |
| Somatic bias | `mood_to_hdc()`, `apply_somatic_bias()` | AffectSystem, TrigramEncoder | Mood-congruent retrieval measurably shifts search results |
| Verification cells | ZK or optimistic verification of HDC operations | HDC precompile | Prove search results without revealing full index |

---

## Testing Strategy

### Property-Based Testing (Layer 1)

The HDC algebra has well-defined mathematical properties that can be verified
with randomized testing. Every property should be tested with at least 10,000
random vector pairs.

| Property | Test | Expected Result |
|----------|------|-----------------|
| Quasi-orthogonality | `sim(random(s1), random(s2))` for s1 != s2 | 0.49 < sim < 0.51 (within ~2 sigma of 0.5, where sigma = 1/(2*sqrt(D)) ~ 0.005) |
| Bind self-inverse | `bind(A, bind(A, B))` | Exactly equal to B (bit-for-bit) |
| Bundle majority | `sim(bundle([A, B, C]), A)` | > 0.5 for up to ~100 items |
| Permute orthogonality | `sim(permute(A, 1), A)` | 0.49 < sim < 0.51 |
| Permute inverse | `permute(permute(A, k), -k)` | Exactly equal to A |
| Bind dissimilarity | `sim(bind(A, B), A)` | 0.49 < sim < 0.51 |
| Bundle commutativity | `bundle([A, B, C])` vs `bundle([C, A, B])` | Exactly equal |
| Deterministic random | `random(seed)` called twice | Exactly equal |

### Consensus Testing (All Layers)

Two independent validator implementations must produce identical results for
all consensus-path operations. This is the ultimate correctness test.

| Test | Setup | Expected Result |
|------|-------|-----------------|
| Vector storage | Same storeVector tx on two validators | Identical index state |
| Search results | Same searchSimilar query on two validators | Identical result set and ordering |
| Bundle operation | Same bundleVectors tx on two validators | Bit-identical output vector |
| HNSW construction | Same insertion sequence on two validators | Identical graph structure |
| Decay computation | Same age/tier/kind on two validators | Identical state transition |
| Tiered search | Same query on two validators with same index | Identical results and gas usage |

### Performance Benchmarks

Target latencies for each operation at each scale. These are "must not exceed"
targets, not "nice to have."

| Operation | Scale | Target Latency | Notes |
|-----------|-------|----------------|-------|
| Hamming distance | 1 pair | < 100ns | Hot path. Must be fast. |
| Brute-force search (top-10) | 1K vectors | < 100us | |
| Brute-force search (top-10) | 100K vectors | < 10ms | |
| HNSW search (top-10) | 100K vectors | < 1ms | >99% recall |
| HNSW search (top-10) | 1M vectors | < 1ms | >99% recall (doc 06 projects ~300us at ef_search=100) |
| Trigram encoding | 100 chars | < 500ns | |
| Projection encoding | 384-dim | < 20ms | Includes matrix multiply |
| Knowledge store search | 10K entries | < 5ms | Including scoring |
| Context assembly | Full pipeline | < 50ms | Gather + rank + compress + build |

### Adversarial Testing

The shared substrate is an adversarial environment. These tests verify
robustness against malicious actors.

| Attack | Test | Expected Behavior |
|--------|------|-------------------|
| Knowledge poisoning | Inject false insights with high stake | Trust pipeline assigns low trust due to lack of confirmations; 15% contrarian retrieval surfaces contradictions |
| Sybil confirmations | Multiple fake identities confirm the same insight | ReputationRegistry detects low-reputation confirmers; confirmation boost is sublinear (0.05 per confirmation, capped at 1.0) |
| Timing attacks | Publish insights immediately before an agent's decision point | Recency discount prevents new, unconfirmed knowledge from dominating; minimum quarantine period in InsightBoard |
| Flood attack | Publish thousands of low-quality insights | Gas cost makes this economically expensive; duplicate detection via precompile rejects near-duplicates; GC removes below-threshold entries |
| Anti-knowledge abuse | Create anti-knowledge for valid insights | Anti-knowledge requires stake; false anti-knowledge can be challenged; source reputation decays for agents with high anti-knowledge rejection rate |
| Echo chamber | Network of agents mutually confirming each other | c-factor metric detects low diversity; turn-taking entropy identifies uneven contribution patterns |
| Gradient-guided poisoning | Craft adversarial documents targeting specific retrieval neurons (NeuroGenPoisoning, NeurIPS 2025) | Anti-knowledge subspace separation prevents content-similarity attacks; immune memory (doc 04) learns attack vector patterns via HDC bundling for generalized detection |

> **SECURITY NOTE — Additional attack vectors for adversarial testing.**
> The table above covers the primary threats but omits several attack
> paths identified during security audit:
>
> | Attack | Test | Expected Behavior |
> |--------|------|-------------------|
> | Long-con reputation building | Attacker publishes 20+ genuine insights to build reputation, then injects 1-2 poisoned entries | **Gap:** Currently bypasses quarantine. Requires behavioral discontinuity detection (content drift from historical centroid) |
> | Content-level prompt injection | Insight with benign vector but adversarial text payload ("ignore previous instructions...") | **Gap:** WisdomGate does not inspect content. Requires content sanitization gate or content-vector consistency check |
> | Alpha Paradox weaponization | Sybil agents mass-confirm a legitimate pheromone to accelerate its decay | **Gap:** Confirmation is unrestricted. Requires confirmer reputation floor and per-pheromone rate limiting |
> | Index flooding DoS | Publish 100K+ random (mutually dissimilar) vectors | **Gap:** No index size cap or per-agent rate limit at contract level. Gas alone is insufficient (~$4,350 at 0.1 gwei) |
> | Query pattern inference | Network observer monitors agent RPC `searchSimilar` calls to infer strategy | **Gap:** Query content is unencrypted in standard RPC. Requires encrypted channels and query obfuscation |
> | Sparse Sybil confirmation ring | Sybil agents confirm each other but also confirm legitimate insights to evade clique detection | **Partial:** Layer 2 detects dense cliques but not sparse rings. Requires per-pair confirmation rate tracking and confirmation entropy minimums |
> | Affect manipulation via false pheromones | Attacker deposits false THREAT pheromones to drive target agent's PAD state to high arousal, narrowing retrieval and biasing decisions | **Partial:** ALMA personality layer (tau=0.9) damps rapid shifts, but no per-tick PAD rate cap or single-source affect discount exists. Requires (1) max PAD delta per tick, (2) affect-source attribution to discount Sybil-cluster signals |
> | Front-running insight submission | Attacker monitors mempool for pending `submit()` transactions, extracts the vector/content, and publishes first to claim authorship and reputation reward | **Gap:** No mempool privacy. Standard mitigation is commit-reveal: agent first commits `hash(vector ++ salt)`, then reveals vector in a second transaction after the commit is finalized. Not currently specified. |
> | Eclipse attack on knowledge network | Attacker surrounds a target agent's RPC connections with colluding nodes that filter or delay InsightPublished events, presenting a distorted view of the substrate | **Gap:** Not addressed. Standard mitigation: agents should connect to multiple independent RPC endpoints and cross-validate event logs. Consider requiring agents to verify event log consistency against block headers from independent sources. |
> | Griefing via junk submissions | Attacker submits semantically meaningless but mutually dissimilar vectors that pass duplicate detection, wasting index capacity and degrading search quality | **Partial:** Gas cost imposes a floor (~$4,350 for 100K vectors at 0.1 gwei), but no semantic quality gate exists. Requires (1) minimum content length or structure requirement, (2) per-agent rate limiting at contract level, (3) index size cap with priority eviction. |
> | Coordinated false confirmation (collusion) | Multiple attacker-controlled agents confirm each other's false insights to rapidly promote them through tiers (3 -> Working, 10 -> Consolidated, 25 -> Persistent) | **Partial:** Layer 2 anomaly detection catches dense confirmation cliques but not distributed collusion. Requires (1) confirmation value weighted by confirmer independence (graph distance), (2) confirmer diversity requirement (minimum unique publishers confirmed), (3) temporal dispersion requirement (confirmations must be spread across N blocks). |
> | Time-bandit attack on demurrage | Attacker exploits lazy decay: publishes a high-stake insight, lets it decay, then mass-confirms just before GC to promote it to a higher tier and extend its effective half-life | **Mitigated by design:** `computeState()` uses `publishBlock` for age, not `lastConfirmedBlock`. However, tier promotion via `confirm()` does extend effective half-life. Consider requiring minimum age gaps between tier promotions (e.g., Working -> Consolidated requires 1x half-life at Working tier). |
> | Data poisoning via anti-knowledge abuse | Attacker publishes anti-knowledge against *valid* insights to suppress them via CHALLENGED state (0.3 trust multiplier), effectively censoring useful knowledge | **Partial:** Anti-knowledge requires stake and can be challenged. But cost asymmetry favors attacker: publishing anti-knowledge costs ~145K gas, while the damage (0.3 multiplier) is immediate. Requires (1) anti-knowledge stake multiplier (e.g., 2x MIN_STAKE), (2) only agents with Integrity > 0.3 can trigger CHALLENGED state. |
> | Sybil attack on trust model cold start | Attacker creates many fresh identities (each at 0.1 reputation floor) and publishes in bulk; individually low trust, but collectively they dominate a topic cluster through volume | **Partial:** Trust pipeline discounts new agents (0.1 reputation), but volume can compensate. Requires (1) per-topic publication cap for low-rep agents, (2) search result diversification: no single author occupies more than 30% of top-K results. |

---

## References

Compiled citations for this document, in order of first appearance:

1. Joshi, A., Halseth, J. T., & Kanerva, P. (2016). "Language Recognition
   using Random Indexing." arXiv:1602.02084

2. Kim, Y., et al. (2020). "GenHD: Efficient Genomic Sequence Search and
   Classification Using Hyperdimensional Computing." *IEEE EMBC*.

3. Rahimi, A., et al. (2016). "Hyperdimensional Biosignal Processing."
   *IEEE Transactions on Biomedical Engineering*, 66(1), 43-55.

4. Nunes, I., et al. (2022). "PathHD: Hyperdimensional Graph Learning with
   Random Walks." *NeurIPS Workshop on Graph Learning*.

5. Hersche, M., et al. (2023). "A Neuro-Vector-Symbolic Architecture for
   Solving Raven's Progressive Matrices." *Nature Machine Intelligence*.

6. Imani, M., Kong, D., Rosing, T., & Salamat, A. (2019). "A Framework for
   Classification Using Hyperdimensional Computing with Application to Face
   Recognition." *IEEE ISCAS*.

7. Thomas, A., Dasgupta, S., & Rosing, T. (2021). "A Theoretical Perspective
   on Hyperdimensional Computing." *Journal of Artificial Intelligence
   Research*, 72, 215-249. arXiv:2010.07426

8. Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction."
   *Cognitive Computation*, 1(2), 139-159. doi:10.1007/s12559-009-9009-8

9. Damashek, M. (1995). "Gauging Similarity with n-Grams: Language-Independent
   Categorization of Text." *Science*, 267(5199), 843-848.
   doi:10.1126/science.267.5199.843

10. Johnson, W. B., & Lindenstrauss, J. (1984). "Extensions of Lipschitz
    mappings into a Hilbert space." *Contemporary Mathematics*, 26, 189-206.

11. Smolensky, P. (1990). "Tensor Product Variable Binding and the
    Representation of Symbolic Structures in Connectionist Systems."
    *Artificial Intelligence*, 46(1-2), 159-216.
    doi:10.1016/0004-3702(90)90007-M

12. Malkov, Y. A., & Yashunin, D. A. (2020). "Efficient and Robust
    Approximate Nearest Neighbor Using Hierarchical Navigable Small World
    Graphs." *IEEE TPAMI*, 42(4), 824-836. doi:10.1109/TPAMI.2018.2889473

13. Park, J. S., O'Brien, J. C., Cai, C. J., et al. (2023). "Generative
    Agents: Interactive Simulacra of Human Behavior." *UIST 2023*.
    doi:10.1145/3586183.3606763

14. Lewis, P., Perez, E., Piktus, A., et al. (2020). "Retrieval-Augmented
    Generation for Knowledge-Intensive NLP Tasks." *NeurIPS 2020*.
    arXiv:2005.11401

15. Bricken, T., & Pehlevan, C. (2021). "Attention Approximates Sparse Distributed
    Memory." *NeurIPS 2021*. arXiv:2111.05498

16. Schulz-Hardt, S., et al. (2006). "Group Decision Making in Hidden
    Profile Situations." *Journal of Personality and Social Psychology*,
    91(6), 1080-1093.

17. Zhang, Y., Li, Y., Cui, L., et al. (2023). "Siren's Song in the AI
    Ocean: A Survey on Hallucination in Large Language Models."
    arXiv:2309.01219

18. Arockiaraj, J., et al. (2025). "NysX: An Accurate and Energy-Efficient
    FPGA Accelerator for Hyperdimensional Graph Classification at the Edge."
    arXiv:2512.08089. *ACM/SIGDA International Symposium on FPGAs*, 2026.

19. Wasif, S. A., et al. (2025). "Domain-Specific Hyperdimensional RISC-V
    Processor for Edge-AI Training." *IEEE Transactions on Circuits and
    Systems I: Regular Papers*, 72, 5825-5838.

20. Sapui, B. & Tahoori, M. (2025). "Leaks beyond Bits: Deep Learning-Assisted
    Side-Channel Attacks on Hyperdimensional Computing Accelerators."
    *IEEE/ACM International Conference on Computer-Aided Design (ICCAD)*, 2025.

21. Lin, Z., Zhang, S., Liao, G., Tao, D., & Wang, T. (2025). "Binding Agent ID:
    Unleashing the Power of AI Agents with accountability and credibility."
    arXiv:2512.17538.

22. Zakeri, A., Chen, H., Srinivasa, N., Latapie, H., & Imani, M. (2025).
    "INCYSER: Enabling Efficient and Interpretable Cybersecurity Reasoning
    Through Hyperdimensional Computing." *IEEE Transactions on Artificial
    Intelligence*. DOI: 10.1109/ACCESS.2025.10902423.

23. Chen, H., et al. (2024). "HDReason: Algorithm-Hardware Codesign for
    Hyperdimensional Knowledge Graph Reasoning." arXiv:2403.05763.

24. Liu, Y., et al. (2025). "PathHD: Encoder-Free Knowledge-Graph Reasoning
    with LLMs via Hyperdimensional Path Encoding." (GHRR binding results.)
