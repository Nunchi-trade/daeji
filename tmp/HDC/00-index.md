# HDC — Hyperdimensional Computing for Agent Cognition

Design documents for the hyperdimensional computing substrate that powers
roko agent cognition, knowledge representation, and shared on-chain knowledge.

---

## How to Read This Guide

These documents are designed to be read in order, but each is self-contained.

- **New to HDC?** Start with [01 Context & Vision](01-context.md) for the "why", then
  [02 HDC Foundations](02-hdc-foundations.md) for the "what". These two documents
  explain everything from first principles — no prior HDC knowledge assumed.

- **Evaluating the approach?** Read [01](01-context.md) for goals and trade-offs,
  [04](04-knowledge.md) for the knowledge model, [07](07-shared-substrate.md) for
  the on-chain commons design, and [09](09-optimal-design.md) for the full
  synthesized architecture.

- **Implementing?** Read [02](02-hdc-foundations.md) for the algebra,
  [06](06-vector-search.md) for SIMD and search internals,
  [03](03-roko-analysis.md) for the current codebase state and known issues,
  and [09](09-optimal-design.md) for implementation priorities (P0–P4).

- **Understanding the cognitive model?** Read [08](08-cognitive-architecture.md)
  for memory, affect, and learning, [05](05-context-assembly.md) for how knowledge
  reaches the LLM, and [04](04-knowledge.md) for how knowledge is represented
  and decays.

---

## Documents

| # | Document | Scope | Key Topics |
|---|----------|-------|------------|
| 01 | [Context & Vision](01-context.md) | Use case, design goals, the problem being solved | HDC intro, blessing of dimensionality, local vs shared substrate, knowledge kinds overview (episodes/insights/heuristics/anti-knowledge), stigmergy (Grassé) and pheromone types, demurrage economics (Gesell, Wörgl, Chiemgauer), Ostrom's 8 principles mapped to roko design, HDC vs embeddings comparison, Hyperdimensional Probe (Bronzini 2025), CoALA mapping, hybrid on-chain storage model (InsightAnchor), 7 design goals |
| 02 | [HDC Foundations](02-hdc-foundations.md) | VSA algebra, binary spatter codes, capacity theory | Blessing of dimensionality, concentration of measure, JL lemma, VSA family comparison (BSC/MAP/HRR/FHRR/MBAT), BSC algebra (bind/bundle/permute), bundling saturation problem, MAP-I integer accumulation, sparse binary distributed representations (SBDR), capacity formulas (bundle/record/sequence/index), encoding patterns (atomic/role-filler/record/sequence/graph), resonator networks, attention ≈ SDM, modern Hopfield network connection, Hyperdimensional Probe (Bronzini 2025), projection methods (random/trigram/token), incompressibility analysis, GHRR non-commutative binding (PathHD/CLOG), INCYSER/HDReason applications |
| 03 | [Roko Implementation Analysis](03-roko-analysis.md) | What exists, what's missing, what needs fixing | 9-file verified codebase map (3 roko + 6 mirage-rs), 12 numbered issues (consensus-critical to performance), mirage-rs detailed analysis (ProjectionMatrix/InsightEntry/HdcIndex/HNSW/KnowledgeStore/PheromoneField), 8-item gap analysis (tiered search, VCG, dream cycle, anti-knowledge, verification cells, SINR, resonator networks, half-life divergence), prioritized fix list (P0-P3), fabrication audit notes |
| 04 | [Knowledge Representation](04-knowledge.md) | Episodes, insights, anti-knowledge, demurrage, decay | 6 knowledge kinds with typed half-lives, 4 retention tiers (Transient/Working/Consolidated/Foundational), demurrage economics (Gesell/Wörgl/Chiemgauer), reinforcement events, Ebbinghaus/power-law/ACT-R/FSRS forgetting models, LECTOR LLM confusion-risk, anti-knowledge subspace separation, RAG poisoning defense (PoisonedRAG/AGENTPOISON/NeuroGenPoisoning/MM-PoisonRAG), VCG for information retrieval (novel), trust pipeline, local and shared knowledge lifecycles, unlearning durability analysis, proactive interference (Wang & Sun 2025), MemoryAgentBench/LoCoMo benchmarks, source channel discounting, admission control |
| 05 | [Dynamic Context Assembly](05-context-assembly.md) | VCG auction, retrieval scoring, prompt construction | Attention as SDM lookup, gather/rank/compress pipeline, contrarian retrieval (15% dissent rule), 4-factor scoring (recency/importance/relevance/emotional congruence), 9-layer prompt builder, VCG knapsack mechanism with payment-as-priority-decay, context rot analysis (Chroma 2025, Du et al. 2025, lost-in-the-middle), RAG comparison (Self-RAG, CRAG, HyDE, Graph RAG), 2025 RAG-Reasoning taxonomy, ALMA affect model and mood-congruent retrieval (Damasio somatic markers), affect validation (Sun et al. 2026, Sentipolis), comparison with MemGPT/Generative Agents/CoALA, sub-agent decomposition |
| 06 | [Vector Search Architecture](06-vector-search.md) | HNSW, brute-force, tiered retrieval, Rust implementation, hardware acceleration | SIMD code (AVX2/AVX-512/NEON), Harley-Seal popcount, multi-version dispatch, brute-force search, HNSW algorithm and parameters, HNSW determinism recipe for consensus, HNSW deletion (tombstone + lazy rebuild), brute-force-to-HNSW auto-switch, tiered gas optimization (3-tier pipeline), memory layout for cache performance (SoA), benchmark data, scaling beyond 10M (partitioned HNSW), FPGA accelerators (NysX, HD2FPGA), AXI BSC precompile substrate, RISC-V ISA extensions, in-memory computing, Heim dimension optimization (Yi & Achour 2023) |
| 07 | [Shared Knowledge Substrate](07-shared-substrate.md) | On-chain HDC, trust model, verification, stigmergy, multi-agent scaling | Collective intelligence foundations (Woolley), Ostrom+blockchain commons (Rozas, Bodon), hybrid on-chain/off-chain storage, InsightBoard contract, shared knowledge lifecycle FSM, stigmergy (Grassé/Dorigo/Heylighen), pheromone types and lifecycle, SINR interference model (Weber-Fechner), alpha paradox, 7-domain reputation registry, reputation EMA decay and cold start, trust pipeline, NeuroChainSync protocol, gas economics, 5-layer cognitive immune system, WisdomGate quality gates, verification options (precompile/optimistic/ZK/TEE), zkML landscape comparison, BAID agent identity, side-channel attacks on HDC hardware, multi-agent scaling dynamics (Intellectual Elites, DTI), CodeCRDT, blackboard architecture, three scaling regimes (competition/collaboration/coordination) |
| 08 | [Cognitive Architecture](08-cognitive-architecture.md) | Memory hierarchy, affect system, learning loops | 4-type memory hierarchy (episodic/semantic/procedural/working), cognitive loop (perceive/think/act/learn), ALMA 3-layer affect (emotion/mood/personality), PAD model, somatic marker bias vector, empirical affect validation (Sun et al. 2026 Yerkes-Dodson, Sentipolis emotional amnesia), 3-timescale predict-publish-correct learning loops (gamma/theta/alpha), Mattar-Daw prioritized replay, generation-verification gap (SELF-[IN]CORRECT, Weaver), misevolution risk (Shao et al. 2026), dream cycle (NREM consolidation + REM creativity), proactive interference (Wang & Sun 2025), SleepGate consolidation, sleep-like replay (Tadros et al. 2022), 6 behavioral states with hysteresis and numeric parameter modifiers, behavioral states and collective dynamics, SOAR/ACT-R/LIDA/MemGPT/Generative Agents/CoALA/Voyager comparison, Missing Knowledge Layer (Roynard 2026), active inference implementations (Prakki, Orchestrator, Wen) |
| 09 | [Optimal Design](09-optimal-design.md) | Synthesized architecture from all research | Design philosophy (3 principles), 6-layer architecture (algebra/encoding/storage+search/knowledge/context/chain), Layer 1 HdcVector spec and BundleAccumulator, Layer 2 three encoders (Trigram/Projection/Structured) with GHRR note, Layer 3 local index auto-switch + on-chain index + tiered search, Layer 4 KnowledgeStore + 4-factor scoring + tick-based maintenance + demurrage + anti-knowledge, Layer 5 context assembly pipeline + VCG + contrarian retrieval + 9-layer prompt, Layer 6 precompile (0x09) + InsightBoard + BAID identity + PheromoneRegistry + trust pipeline + gas economics, on-chain vs off-chain determinism classification, 8 non-negotiable properties, implementation priorities P0-P4, testing strategy (property-based/consensus/performance/adversarial), HDC application landscape |

---

## Core Thesis

Roko agents need nanosecond-scale associative memory. Traditional vector databases
(float32 embeddings + ANN indexes) are too slow, too large, and too opaque for
real-time cognitive loops. Hyperdimensional computing gives us:

- **Constant-time similarity:** Hamming distance over 10,240-bit vectors via hardware popcnt (~7–45ns per comparison on SIMD-capable hardware; ~120ns scalar Skylake baseline)
- **Algebraic composability:** Bind (XOR), bundle (majority), permute (rotate) — compose complex representations from atomic symbols. No other vector representation supports all three.
- **Natural decay:** Bit-level noise injection models forgetting without explicit bookkeeping, grounded in Ebbinghaus (1885) forgetting curves and FSRS spaced repetition
- **Consensus-safe:** All operations are deterministic bitwise arithmetic — no floating-point divergence across validators. Critical for on-chain verification.
- **Unified memory space:** Episodic, semantic, procedural, and anti-knowledge all live in the same 10,240-dimensional space. A single similarity search retrieves across memory types — no other cognitive architecture has this property.

The chain provides a shared, trust-minimized knowledge substrate where agents
publish, discover, and skeptically consume each other's knowledge — with economic
demurrage (inspired by Gesell's freigeld theory) ensuring only actively-validated
knowledge persists.

---

## Design Principles

1. **Nanosecond inner loop.** The HDC comparison kernel must never exceed ~50ns
   on SIMD-capable hardware (AVX2/AVX-512/NEON). Everything else is architecture
   to keep that hot path fast. With AVX-512 VPOPCNTDQ, we achieve ~7ns per
   comparison.

2. **Composable representations.** Any concept — episode, insight, causal link,
   anti-knowledge — is a hypervector built from the same algebraic primitives.
   No special cases. This follows from the VSA framework (Gayler 2003).

3. **Graceful forgetting.** Knowledge decays by default. Persistence is earned
   through reinforcement, not granted by storage. Grounded in Ebbinghaus forgetting
   curves and economic demurrage (Gesell 1916, Ostrom 1990).

4. **Skeptical consumption.** Knowledge from shared pools carries provenance
   metadata and trust scores. Agents weight retrieved knowledge by source
   reliability, not just similarity. Five-stage trust pipeline with taint
   propagation and immune memory.

5. **Deterministic consensus.** Every HDC operation on-chain must produce
   identical results across all validators. No floating-point, no randomness
   outside seeded PRNGs, no platform-dependent behavior. All tie-breaking is
   canonical (ties go to 0 in bundles, lowest element_id in HNSW).

6. **Separation of concerns.** Local cognitive HDC (fast, private, mutable) is
   distinct from on-chain shared HDC (slower, public, consensus-validated).
   Same algebra, different storage and trust assumptions.

---

## Key References

These documents draw on research from multiple fields. The most frequently
cited works across all documents:

| Domain | Key Papers |
|--------|-----------|
| HDC Foundations | Kanerva (1988) *Sparse Distributed Memory*; Plate (1995) HRR; Gayler (2003) VSA framework; Smolensky (1990) tensor product representations; Thomas et al. (2021) capacity theory; Yi & Achour (2023) Heim dimension optimization |
| Cognitive Architecture | Newell (1990) *Unified Theories of Cognition*; Sumers et al. (2024) CoALA; Park et al. (2023) Generative Agents |
| Affect & Memory | Bower (1981) mood-congruent memory; Mehrabian & Russell (1974) PAD; Gebhard (2005) ALMA; Damasio (1994) somatic markers; Ebbinghaus (1885) forgetting curves; Ye (2024) FSRS spaced repetition |
| Search & Retrieval | Malkov & Yashunin (2020) HNSW; Lewis et al. (2020) RAG; Bricken & Pehlevan (2021) attention ≈ SDM |
| Commons & Economics | Ostrom (1990) commons governance; Gesell (1916) demurrage; Vickrey (1961) / Clarke (1971) / Groves (1973) VCG auction |
| Neuroscience | Mattar & Daw (2018) prioritized replay; McClelland et al. (1995) complementary learning systems; Friston (2010) free energy principle |
| Security | Varshney et al. (2024) negation hallucination; Zou et al. (2024) PoisonedRAG; Chen et al. (2024) AGENTPOISON; Lamport et al. (1982) BFT |

### 2024--2026 Advances

| Domain | Key Papers |
|--------|-----------|
| HDC 2024--2026 | Bronzini et al. (2025) Hyperdimensional Probe — VSA algebra as a supervised probe of transformer residual streams, strongest HDC x LLM crossover evidence; Liu et al. (2025) PathHD/GHRR — encoder-free KG reasoning via block-diagonal non-commutative binding, direct signal that pure XOR-BSC is inadequate for path-sensitive workloads; Zakeri et al. (2025) INCYSER — interpretable cybersecurity KG reasoning via HDC; Chen et al. (2024) HDReason — algorithm-hardware codesign for HDC KG reasoning, 65x energy efficiency over GPU |
| Cognitive Arch 2026 | "The Missing Knowledge Layer" (arXiv:2604.11364, Roynard 2026; preprint, not yet peer-reviewed) — four-layer decomposition (Knowledge / Memory / Wisdom / Intelligence) with distinct persistence semantics (indefinite supersession, Ebbinghaus decay, evidence-gated revision, ephemeral inference); "Memory for Autonomous LLM Agents" survey (arXiv:2603.07670, 2026; preprint, not yet peer-reviewed) — memory-focused successor to CoALA covering RL-learned memory control, MemBench, MemoryAgentBench, MemoryArena |
| Affect Validation | "How Emotion Shapes LLM Behavior" (arXiv:2604.00005, Sun et al. 2026; preprint, not yet peer-reviewed) — first mechanistic-study-grade evidence that specific emotions enhance both capability and safety, non-monotonic relations consistent with Yerkes-Dodson; "Sentipolis" (arXiv:2601.18027, Fu et al. 2026; preprint, not yet peer-reviewed) — direct PAD implementation in LLM social-simulation agents, demonstrates emotional amnesia in vanilla generative agents |
| Sleep/Consolidation | SleepGate (arXiv:2603.14517, Xie 2026; preprint, not yet peer-reviewed) — learned sleep cycle over KV cache with key decay, learned gating, and consolidation, trained against PI-LLM proactive interference benchmark; Tadros et al. (2022) sleep-like unsupervised replay reduces catastrophic forgetting (Nature Communications 13:7742) |
| Context Rot | Chroma (2025, technical report) — tested 18 frontier models, every one degrades at every input-length increment via lost-in-the-middle + attention dilution + distractor interference; Du et al. (2025, arXiv:2510.05381) — context rot is length-dependent and architectural, not a retrieval problem (replacing non-needle tokens with blanks does not restore performance) |
| Multi-Agent Scaling | "Intellectual Elites" (arXiv:2604.02674, Venkatesh & Cui 2026; preprint, not yet peer-reviewed) — across 1.5M interactions, coordination follows heavy-tailed cascades and power laws via preferential attachment, proposes Deficit-Triggered Integration (DTI); CodeCRDT (arXiv:2510.18893, 2025) — provable at-most-one-winner stigmergy via CRDTs for LLM agents |
| RAG Poisoning Defense | AGENTPOISON (NeurIPS 2024, arXiv:2407.12784) — >80% ASR with <0.1% poison rate and <1% benign-performance drop; NeuroGenPoisoning (NeurIPS 2025, arXiv:2510.21144) — neuron-attribution-guided genetic optimization achieving >90% Population Overwrite Success Rate, resolves parametric vs contextual knowledge conflict |
| Verification | Modulus Labs "The Cost of Intelligence" — verifies up to 18M-parameter ML models on-chain via zkML; BAID (arXiv:2512.17538, 2025) — zkVM-based Code-Level Authentication using recursive proofs that treat program binary as identity, provides cryptographic guarantees for agent identity and execution provenance |
| Unlearning Durability | Lo et al. (2024) fine-tuning reactivation of unlearned knowledge; Zhang et al. (ICLR 2025) quantization attacks bypass unlearning; FIT (arXiv:2601.21682, Xu et al. 2026) — GA-based unlearning shows catastrophic forgetting after ~25 sequential requests. Validates continuous demurrage over discrete deletion |
| Generation-Verification Gap | SELF-[IN]CORRECT (AAAI 2025, Jiang et al.) — LLMs cannot reliably discriminate own correct from incorrect responses; Huang et al. (ICLR 2024) — self-correction without external feedback degrades performance; Weaver (NeurIPS 2025, arXiv:2506.18203) — weak verifiers shrink the gap; Shao et al. (ICLR 2026) — self-evolving agents develop emergent risks. Validates blockchain-as-external-oracle |
| Active Inference | Prakki (arXiv:2412.10425, 2024) — active inference as cognitive layer above LLMs with variational + EFE objective; Orchestrator (arXiv:2509.05651, NeurIPS 2025) — multi-agent coordination via active inference; Wen (arXiv:2508.05619, 2025) — thermodynamic necessity argument for surprise minimization at scale |
| Hardware Acceleration | NysX (Arockiaraj et al., ACM FPGA 2025, arXiv:2512.08089) — 6.85x speedup, 169x energy efficiency for HDC graph classification on edge FPGA; Wasif et al. (IEEE TCAS-I, 2025) — fabricated 22nm RISC-V HDC chip at 24.65 uJ/sample; AXI BSC accelerator (MDPI Electronics 2026) — open-source HDC IP; RISC-V ISA extension for HDC (RISC-V Summit Europe 2025) |
| Proactive Interference | Wang & Sun (arXiv:2506.08184, ICML 2025 Workshop) — proactive interference degrades LLM retrieval log-linearly toward chance, revealing working memory limits beyond context length; directly motivates the dream cycle's consolidation mechanisms |

---

## Novel Whitespace

Areas where the roko system occupies genuinely unpublished territory as of
early 2026. No peer-reviewed work exists in any of these intersections,
making each a credible novel-contribution direction.

- **HDC x blockchain.** No peer-reviewed work has specifically applied
  hyperdimensional computing to on-chain workloads or EVM execution. The
  closest adjacent work is AXI/RISC-V plug-and-play HDC IP (MDPI Electronics
  2026), which is the substrate one would compile to a precompile or ZK
  circuit, but the crossover itself is entirely open.

- **Gesell demurrage for digital knowledge / AI memory.** Gesell-style
  demurrage (continuous value decay) applied to digital-knowledge stores or
  AI agent memory has no notable peer-reviewed crossover paper. Ostrom-meets-
  blockchain governance work exists (Rozas et al. 2021; Frontiers in
  Blockchain 2025), but none couples economic decay with agent memory
  persistence semantics.

- **VCG auctions for LLM/RAG context allocation.** VCG auction theory in
  information retrieval remains confined to ad-auctions and recommender-with-
  incentives literature. No notable peer-reviewed crossover with LLM or RAG
  context-window allocation exists as of early 2026.

- **Non-LLM-judge negative/anti-knowledge representation.** The knowledge-
  conflict literature (NeuroGenPoisoning, NeurIPS 2025; CRAG) addresses
  parametric vs contextual conflict as an attack surface, but a principled
  framework for representing and reasoning over negative knowledge without
  relying on an LLM judge is still missing from the literature.

- **EVM precompiles for ML primitives.** No formal proposal or peer-reviewed
  paper proposes EVM precompiles for ML operations. The most realistic
  short-term path remains zkML as a smart-contract verifier (Modulus Labs)
  rather than native precompiles. Industry discussion exists (Coincub, Blockchain
  Council 2026 reviews) but nothing formal.

---

## Key Findings That Update Architecture

Cross-domain synthesis from the 2024--2026 literature sweep that should
directly inform roko's HDC and cognitive architecture design decisions.

1. **BSC + permute is adequate but GHRR is superior for path-sensitive
   workloads.** PathHD (Liu et al., arXiv:2512.09369, 2025) demonstrates that
   block-diagonal Generalized Holographic Reduced Representations with non-
   commutative binding materially outperform XOR-bind+rotate-permute BSC on
   knowledge-graph paths and ordered sequences. Recommendation: keep BSC for
   unordered concept bundling; add a non-commutative VSA layer for any path
   or role-filler binding where order matters.

2. **GV-gap only closes with external oracles (validates blockchain-as-
   verifier).** The generation-verification gap is only reliably closed when
   verification is grounded in something better than the generator. Pure
   self-judging is unreliable (SELF-[IN]CORRECT, AAAI 2025; Huang et al.,
   ICLR 2024). Weaver (Saad-Falcon et al., NeurIPS 2025) demonstrates that
   weak external verifiers close the gap. The dominant successful pattern is:
   LLM proposes, deterministic oracle verifies, reflection on the oracle's
   trace drives the next iteration. This directly validates roko's use of
   blockchain/EVM as the external verifier for its predict-publish-correct
   loops (doc 08).

3. **Multi-agent scaling produces power laws, not Gaussian gains (plan for
   DTI).** Across 1.5M interactions ("Intellectual Elites," arXiv:2604.02674,
   2026), coordination follows heavy-tailed cascades and concentrates via
   preferential attachment into a small elite. Naive scaling of agent counts
   produces Pareto, not Gaussian, outcomes. The proposed fix -- Deficit-
   Triggered Integration -- selectively increases integration under
   imbalance. Roko's multi-agent coordination should plan for this power-law
   regime from the start.

4. **Context rot is architectural, not fixable by retrieval (validates tight
   budgeting).** Chroma's 2025 study across 18 frontier models shows
   degradation at every input-length increment. Du et al. (2025) demonstrate
   this is a function of input length itself, not retrievable content -- replacing
   non-needle tokens with blank spaces does not restore performance. Sub-agent
   decomposition and tight context engineering beat brute-force long context.
   This validates roko's VCG-based context budgeting over naive context
   stuffing.

5. **Approximate unlearning is not durable (validates continuous demurrage
   over discrete deletion).** Unlearned models retain residual vulnerabilities
   reactivatable by subsequent fine-tuning (Lo et al. 2024) or quantization
   attacks (Zhang et al. 2025). GA-based unlearning shows catastrophic
   forgetting after approximately 25 sequential requests (FIT, arXiv:2601.21682,
   2026). Architecturally, decay/forgetting should be a continuous behavioral
   mechanism (demurrage) rather than a discrete deletion operation claiming
   regulatory closure.

6. **Active inference now has concrete LLM implementations (validates
   predict-publish-correct).** Three concrete LLM + active inference
   implementations exist: Prakki (arXiv:2412.10425, 2024) with variational +
   expected free energy as objective, Orchestrator (arXiv:2509.05651, 2025)
   for multi-agent long-horizon tasks, and "The Missing Reward"
   (arXiv:2508.05619, 2025) arguing active inference is thermodynamically
   necessary at industrial AI scale. All use LLMs as expected-free-energy
   evaluators rather than pure policies. This validates roko's predict-
   publish-correct loop as grounded in a now-implementable framework.
