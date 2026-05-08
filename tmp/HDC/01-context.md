# Context & Vision

## What is Hyperdimensional Computing?

Hyperdimensional Computing (HDC), also called Vector Symbolic Architecture (VSA),
is a computational framework that represents information as points in a
high-dimensional space — typically vectors of 1,000 to 10,000+ dimensions — and
manipulates them using a small set of algebraic operations. The idea traces back
to Pentti Kanerva's work on Sparse Distributed Memory (Kanerva, 1988) and was
formalized into the modern VSA framework by Gayler (2003) and Plate (2003).

The core insight is counterintuitive: **high-dimensional spaces are mostly
empty, and that emptiness is useful.**

In a 10,240-bit binary vector space, there are 2^10,240 possible vectors.
If you draw two vectors at random, the number of agreeing bits follows
Binomial(10,240, 0.5) with mean 5,120 and standard deviation ~50.6.
The probability that they share more than 53% of their bits is less than
10^{-8} (by normal approximation, 53% agreement is ~6.1 standard deviations
from the mean). Even a 51% agreement threshold has only ~2% probability.
This means random vectors are *quasi-orthogonal* with overwhelming
probability. You get an astronomically large set of nearly non-interfering
"addresses" for free, without any coordination or allocation scheme.

This property is known as the **blessing of dimensionality** — the counterpart
to the "curse of dimensionality" that plagues traditional machine learning.
In ML, high dimensions make distance metrics less discriminative and sampling
harder. In HDC, high dimensions are exactly what gives you:

1. **Enormous capacity.** You can superimpose dozens of vectors into a single
   "bundle" vector and still retrieve each individual component with high
   accuracy, because the random noise from the other components averages
   out across 10,240 dimensions. The exact capacity depends on the
   retrieval method and the size of the item memory: with a clean-up memory
   of 1,000 candidate vectors at D=10,000, bundling up to ~70 vectors still
   permits reliable retrieval; at D=10,240, the capacity is modestly higher
   (Kanerva, 2009; Kleyko et al., 2023).

2. **Robustness to noise.** Flipping a few hundred bits in a 10,240-bit
   vector barely changes its similarity to the original. Knowledge degrades
   gracefully rather than catastrophically.

3. **Algebraic composability.** Three simple operations — *bind* (XOR),
   *bundle* (majority vote), and *permute* (cyclic rotation) — are sufficient
   to build arbitrarily complex structured representations from atomic symbols.
   These are the HDC equivalents of multiplication, addition, and sequencing.

4. **Constant-time comparison.** Determining how similar two concepts are
   reduces to a single Hamming distance computation: XOR two bit vectors
   and count the 1s. On modern CPUs with hardware `POPCNT`, this takes
   ~25-50 cycles for a 10,240-bit vector (using AVX2 SIMD, processing 32
   bytes per iteration across 40 iterations) — roughly 8-17 nanoseconds
   at 3 GHz.

For autonomous agents on a blockchain, these properties are not just convenient
— they are load-bearing. Agents must make decisions at block speed (historically
~3s on BSC, now ~0.75s post-Maxwell and trending toward ~0.45s post-Fermi),
which means every millisecond spent on memory retrieval is a millisecond not
spent on reasoning. Traditional vector databases (float embeddings + approximate
nearest neighbor indexes) introduce 100-1000 microseconds of latency per query,
require GPU inference for encoding, and — critically — produce non-deterministic
results across different hardware, making them unsuitable for consensus-validated
computation. HDC eliminates all three problems: nanosecond retrieval, no model
inference for encoding, and bitwise-exact determinism on every platform.

> Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction to Computing
> in Distributed Representation with High-Dimensional Random Vectors."
> *Cognitive Computation*, 1(2), 139-159. doi:10.1007/s12559-009-9009-8
>
> Kanerva, P. (1988). *Sparse Distributed Memory*. MIT Press.
>
> Gayler, R. W. (2003). "Vector Symbolic Architectures Answer Jackendoff's
> Challenges for Cognitive Neuroscience." In *Proceedings of the Joint
> International Conference on Cognitive Science*, 133-138. arXiv:cs/0412059
>
> Plate, T. A. (2003). *Holographic Reduced Representations: Distributed
> Representation for Cognitive Structures*. CSLI Publications.

---

## The Problem

Roko agents are autonomous software entities that think, learn, and collaborate.
Their cognitive loop runs continuously: perceive → reason → act → learn. At
every iteration, the agent must assemble a context window — the "working memory"
injected into the LLM prompt — from potentially millions of stored knowledge
fragments.

This assembly must be:
- **Fast:** The agent's inner loop targets sub-second latency. Context assembly
  that takes 100ms is a significant fraction of the budget.
- **Associative:** The agent needs "things related to X" not "things at key K."
  Exact-match lookup is useless for cognition.
- **Compositional:** Complex queries like "episodes where I failed at task T
  in context C" require combining multiple concepts.
- **Decay-aware:** Old, unreinforced knowledge should fade. The agent shouldn't
  drown in stale memories.

Traditional approaches fail:

| Approach | Problem |
|----------|---------|
| SQL/KV store | No similarity search. Exact match only. |
| Float vector DB (Pinecone, Qdrant) | 100-1000µs per query. 768+ floats per vector = large. Non-deterministic across platforms. |
| Embedding model + FAISS | Requires GPU. Model inference adds 10-50ms. Not consensus-safe. |
| Full-text search | No semantic similarity. Keyword matching is brittle. |

HDC solves all four:
- **~8-50ns** per comparison (XOR + hardware POPCNT on 1,280 bytes; lower
  bound is raw SIMD, upper bound includes function-call and cache overhead)
- **Associative** by construction (Hamming distance = semantic similarity)
- **Compositional** via algebraic operations (bind, bundle, permute)
- **Decay-aware** via bit-level noise injection and balance-based demurrage
  (see [04-knowledge.md](04-knowledge.md))

Additionally, HDC provides a critical fifth property for blockchain agents:
- **Deterministic** (bitwise arithmetic, no floats — consensus-safe by construction)

---

## Why Not Just Use Embeddings?

A natural question: modern neural embeddings (OpenAI `text-embedding-3`,
sentence-transformers, etc.) are extremely good at capturing semantic similarity.
Why invent a new representation?

The answer is not that embeddings are bad — they are excellent for offline
semantic search. The answer is that they have properties that are *incompatible
with the constraints of real-time, consensus-validated agent cognition.*

| Property | HDC (Binary Spatter Codes) | Neural Embeddings (float32) |
|----------|----------------------------|----------------------------|
| **Element type** | Binary: {0, 1} | Continuous: float32 |
| **Vector size** | 10,240 bits = 1,280 bytes | 768-3072 floats = 3-12 KB |
| **Comparison** | XOR + POPCNT, ~8-17ns | Dot product / cosine, ~500ns-50µs |
| **Encoding** | Trigram / projection, ~100ns-10ms | Model inference, 10-50ms (GPU) |
| **Composability** | Full algebra: bind ⊕ bundle ⊕ permute | None — cannot algebraically compose two embeddings into a meaningful third |
| **Consensus safety** | Exact: XOR is XOR everywhere | Unsafe: IEEE 754 permits fused multiply-add, different rounding modes, and compiler reordering. Same inputs → different results on AMD vs Intel vs ARM. |
| **Invertibility** | Bind is self-inverse: A ⊕ B ⊕ B = A | No inverse — you cannot "unbind" a dot product |
| **GPU required?** | No — bitwise ops on any CPU | Effectively yes for encoding |
| **Structured representation** | Native: role-filler binding, sequences via permute | Must be learned from data; no principled way to encode "A causes B" |

The composability gap is the most significant. With HDC, you can build complex
cognitive structures from primitives:

```
episode = bind(context, bind(action, outcome))
negation = bind(knowledge, ANTI_SUBSPACE)
causal_link = bind(permute(cause), effect)
```

Each of these is a single vector that participates in the same similarity search
as every other vector. You cannot do this with neural embeddings — there is no
operation that takes "the embedding of X" and "the embedding of Y" and produces
"the embedding of X-causes-Y" that is mathematically guaranteed to behave
correctly in similarity search.

The consensus safety gap is equally critical. On a blockchain, every validator
must produce identical state transitions for the same inputs. Floating-point
arithmetic violates this: the IEEE 754 standard permits hardware vendors to
implement fused multiply-add (FMA) instructions that produce different rounding
than separate multiply and add. The C/Rust compilers may reorder floating-point
operations for performance, changing results. Even the same code on the same
hardware can produce different results with different optimization levels. HDC's
bitwise operations have none of these problems: XOR is XOR, POPCNT is POPCNT,
on every platform, every compiler, every architecture.

For these reasons, roko uses HDC as its native knowledge representation and
reserves neural embeddings for the optional "quality path" encoding when
publishing to the shared substrate — where a random projection binarizes the
embedding into HDC form before it enters consensus-validated storage.

Importantly, this is not a forced choice between two incompatible worlds.
Bronzini et al. (2025) demonstrated that VSA algebra can directly decode
the internal representations of large language models — a shallow encoder
maps transformer residual stream activations into VSA hypervectors, and
standard binding/unbinding operations then extract interpretable concepts
with 83% accuracy across models ranging from 355M to 109B parameters. This
confirms that HDC and neural embeddings are complementary: the transformer
produces rich internal representations during inference, and VSA provides
the algebraic structure to encode, compose, and retrieve knowledge derived
from those representations. The two are not in tension — they operate at
different layers of the cognitive stack.

> Bronzini, M., Nicolini, C., Lepri, B., Staiano, J., & Passerini, A.
> (2025). "Hyperdimensional Probe: Decoding LLM Representations via Vector
> Symbolic Architectures." arXiv:2509.25045.
>
> Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction to Computing
> in Distributed Representation with High-Dimensional Random Vectors."
> *Cognitive Computation*, 1(2), 139-159. doi:10.1007/s12559-009-9009-8
>
> Kleyko, D., Rachkovskij, D. A., Osipov, E., & Rahimi, A. (2023). "A Survey
> on Hyperdimensional Computing aka Vector Symbolic Architectures, Part I:
> Models and Data Transformations." *ACM Computing Surveys*, 55(6), Article 130,
> 1-61. doi:10.1145/3538531
>
> Goldberg, D. (1991). "What Every Computer Scientist Should Know About
> Floating-Point Arithmetic." *ACM Computing Surveys*, 23(1), 5-48.
> doi:10.1145/103162.103163

---

## The Use Case

### Local Cognitive HDC

Each roko agent maintains a private HDC index — its "mind." This local index
is the agent's primary cognitive substrate: fast (nanosecond access), private
(never leaves the agent's process), and fully trusted (all entries are
self-authored or explicitly imported).

The design of this local index maps directly to the **CoALA** (Cognitive
Architectures for Language Agents) framework proposed by Sumers et al. (2024),
discussed in depth in [08-cognitive-architecture.md](08-cognitive-architecture.md).
CoALA formalizes LLM-based agents as having four memory types, each
with a distinct cognitive role:

- **Working memory** = the LLM's context window (transient, assembled per-tick)
- **Episodic memory** = records of past experiences ("what happened")
- **Semantic memory** = general knowledge and derived beliefs ("what is true")
- **Procedural memory** = action patterns and strategies ("how to do things")

In roko, all three long-term memory types (episodic, semantic, procedural) are
stored as HDC vectors in the same 10,240-dimensional space. Working memory is
assembled each tick by retrieving the most relevant vectors from the other three
pools and injecting their associated content into the LLM prompt. This is the
"dynamic context assembly" step of the cognitive loop (detailed in
[05-context-assembly.md](05-context-assembly.md)).

The six knowledge types stored in the local index correspond to CoALA's memory
categories as follows (for full encoding details, decay semantics, and
reinforcement mechanics, see [04-knowledge.md](04-knowledge.md)):

**Episodes** — *Episodic memory: records of what happened.*
```
episode = bind(context_vector, bind(action_vector, outcome_vector))
```
"I tried strategy X in situation Y and got result Z." The episode vector
captures the full association. Later, when the agent encounters a similar
situation, a similarity search over episodes retrieves relevant past experience.

*Concrete example:* An agent participating in a DeFi liquidity pool observes
that providing liquidity to pool P during high-volatility periods (context)
with a 2% spread (action) resulted in a 15% impermanent loss (outcome). This
entire experience is encoded as a single 10,240-bit vector. Months later, when
the agent encounters a similar high-volatility market condition, a similarity
search retrieves this episode — allowing the agent to recall the negative
outcome and adjust its strategy without re-learning the lesson.

**Insights** — *Semantic memory: derived conclusions.*
```
insight = bundle([evidence_1, evidence_2, ..., evidence_n])
```
"Based on observations A, B, C, pattern P seems to hold." The insight vector
is the superposition of its supporting evidence. Querying with any piece of
evidence retrieves the insight.

*Concrete example:* After observing five separate episodes where gas prices
spiked within 10 blocks of a large token unlock event, the agent bundles these
five episode vectors into a single insight vector. The resulting vector is
similar to each individual episode (similarity > 0.5) but represents the
generalized pattern. Now, when the agent perceives an upcoming token unlock
(which is similar to the context components of those episodes), this insight
surfaces in retrieval — even though the specific unlock token is different
from any of the original observations.

**Anti-Knowledge** — *Meta-cognitive memory: things the agent learned are false or harmful.*
```
anti_knowledge = bind(knowledge_vector, ANTI_SUBSPACE)
```
"This thing I believed is wrong." Anti-knowledge is structurally distinct from
regular knowledge — it is not a warning label (which reproduces the false content
at 76.7% retrieval rate) but a vector placed in a quasi-orthogonal subspace by
binding with the fixed `ANTI_SUBSPACE` constant. Because the bound result is
approximately orthogonal to the original knowledge vector, anti-knowledge cannot
accidentally surface during normal retrieval -- it must be explicitly queried.

*Concrete example:* An agent previously believed that "token X always recovers
after a 30% drawdown" (stored as an insight). After observing token X fail to
recover three times, the agent creates anti-knowledge by binding the original
insight vector with the ANTI_SUBSPACE symbol. Now, when the original insight
would surface in retrieval (because the context is similar), the retrieval
pipeline detects the anti-knowledge entry, suppresses the original insight,
and prevents the agent from acting on a falsified belief. This is structurally
safer than labeling the insight as "false" — because LLMs tend to reproduce
content from warning-framed text while ignoring the warning itself (Zhang et
al., 2023).

**Causal Links** — *Semantic memory: directed associations.*
```
causal = bind(permute(cause_vector), effect_vector)
```
"X causes Y." The permutation breaks symmetry — `cause→effect` is distinct
from `effect→cause`.

*Concrete example:* The agent encodes "large ETH transfers to exchange hot
wallets (cause) precede price drops within 5 blocks (effect)." The permutation
on the cause vector ensures that this causal link is not confused with the
reverse claim ("price drops cause large ETH transfers"). When the agent later
perceives a large transfer to an exchange hot wallet, it can query the causal
link index with `permute(transfer_vector)` and retrieve the price-drop
prediction — enabling preemptive action.

**Strategies** — *Procedural memory: action plans with context.*
```
strategy = bind(goal_vector, bundle([step_1, step_2, ..., step_n]))
```
"To achieve G, do S1 then S2 then S3." Retrievable by goal similarity.

*Concrete example:* The agent has learned a multi-step arbitrage strategy:
(1) monitor price discrepancy between DEX A and DEX B, (2) flash-borrow from
lending protocol, (3) buy on cheap DEX, (4) sell on expensive DEX, (5) repay
flash loan. Each step is encoded as a vector, bundled into a superposition,
and bound with the goal vector ("maximize risk-free profit from price
discrepancy"). When a new price discrepancy arises and the agent queries with
a similar goal vector, this strategy surfaces — along with any associated
episode vectors recording past execution successes or failures.

**Heuristics** — *Procedural memory: rules of thumb.*
```
heuristic = bind(situation_vector, action_vector)
```
"In situations like X, do Y." Lightweight, fast to retrieve, fast to learn.

*Concrete example:* "When gas price exceeds 100 gwei (situation), defer
non-urgent transactions (action)." This is a simple pattern-action pair
with no multi-step plan — just a fast rule that the agent can retrieve in
a single similarity comparison when the current situation vector is close
to the stored situation vector. Heuristics are the cheapest knowledge type
to create and the fastest to retrieve, making them the agent's reflexive
responses. They correspond roughly to the "System 1" (fast, automatic)
processing in Kahneman's dual-process theory, while strategies correspond
to "System 2" (slow, deliberate) processing (Kahneman, 2011).

> Sumers, T. R., Yao, S., Narasimhan, K., & Griffiths, T. L. (2024).
> "Cognitive Architectures for Language Agents." *Transactions on Machine
> Learning Research* (02/2024). arXiv:2309.02427
>
> Zhang, Y., Li, Y., Cui, L., et al. (2023). "Siren's Song in the AI Ocean:
> A Survey on Hallucination in Large Language Models." arXiv:2309.01219
>
> Kahneman, D. (2011). *Thinking, Fast and Slow*. Farrar, Straus and Giroux.

### Shared Knowledge Substrate (On-Chain)

Beyond private cognition, agents can publish knowledge to a shared on-chain
HDC index. This creates a collective intelligence layer:

- Agent A discovers a useful pattern → publishes to shared substrate
- Agent B, working on a related problem, queries the substrate
- The substrate returns Agent A's insight as a candidate
- Agent B treats it with skepticism — checks provenance, applies trust discount,
  re-contextualizes before using

#### Why On-Chain?

The choice to store shared knowledge on a blockchain — rather than in a
centralized database, a DHT, or a federated system — is driven by three
properties that no alternative provides simultaneously:

1. **Censorship resistance.** No single entity can remove or suppress knowledge
   from the shared substrate. An agent that publishes a valid insight (backed
   by stake and vector encoding) has a guarantee that its contribution will
   persist for as long as the economic lifecycle sustains it. This matters
   because autonomous agents operate in adversarial environments: a competitor
   agent or its operator has an incentive to suppress knowledge that would
   benefit rivals. On-chain storage makes censorship economically impractical
   (it would require a 51% attack).

2. **Verifiability.** Every piece of shared knowledge has an immutable
   provenance chain: who published it, when, what they staked, who confirmed
   it, and the exact vector encoding. Any agent can independently verify this
   chain without trusting any intermediary. In a centralized database, the
   operator can silently modify provenance metadata; on-chain, this is
   cryptographically impossible.

3. **Collective intelligence as a public good.** The shared substrate functions
   as a knowledge commons — a shared resource that becomes more valuable as
   more agents contribute to it. On-chain publication creates a credible
   commitment: published knowledge is public, persistent (subject to
   demurrage), and available to all agents equally. This aligns incentives
   toward contribution rather than hoarding, because the act of publication
   is itself a reputation-building event.

#### Stigmergy — Indirect Coordination

The shared substrate also enables a powerful coordination mechanism borrowed
from biology: **stigmergy** — indirect coordination through modification of
a shared environment.

The term was coined by Pierre-Paul Grassé in 1959 to describe how termites
coordinate the construction of elaborate nests without any central plan or
direct communication. Each termite modifies the environment (deposits a
pheromone-laden mud pellet), and other termites respond to those
modifications. The structure emerges from the accumulation of local
interactions with the shared medium, not from explicit messaging between
individuals.

In the roko system, the shared HDC substrate *is* the shared environment.
When Agent A publishes an insight, it is not "sending a message to Agent B"
— it is modifying the shared knowledge landscape. Agent B, independently
searching the substrate for knowledge relevant to its current task,
discovers the modification. The coordination is indirect: A and B never
communicate, never know each other's identities, and may operate at
different times. The substrate mediates.

This is formalized in daeji through the PheromoneRegistry contract, which
implements three signal types with exponential decay:

| Pheromone Type | Half-Life (blocks) | Purpose |
|----------------|-------------------|---------|
| THREAT | 100 (~45s at 0.45s blocks) | Warn others of dangers (flash loan attacks, oracle manipulation) |
| OPPORTUNITY | 250 (~112s at 0.45s blocks) | Signal profitable situations (arbitrage windows, underpriced assets) |
| WISDOM | 1000 (~450s at 0.45s blocks) | Mark locations of valuable knowledge in the substrate |

*Note: Wall-clock times assume post-Fermi BSC block times (~0.45s). At
the historical 3s block time, these would be ~5 min, ~12 min, and ~50 min
respectively.*

The decay is critical: biological pheromones evaporate, and digital pheromones
must too. A threat signal from 1000 blocks ago is stale information; a fresh
threat signal is urgent. The exponential half-life ensures that the substrate
reflects the *current* state of collective knowledge, not its entire history.

> Grassé, P.-P. (1959). "La reconstruction du nid et les coordinations
> interindividuelles chez *Bellicositermes natalensis* et *Cubitermes sp.* La
> théorie de la stigmergie: Essai d'interprétation du comportement des
> termites constructeurs." *Insectes Sociaux*, 6(1), 41-80.
> doi:10.1007/BF02223791
>
> Theraulaz, G., & Bonabeau, E. (1999). "A Brief History of Stigmergy."
> *Artificial Life*, 5(2), 97-116. doi:10.1162/106454699568700
>
> Heylighen, F. (2016). "Stigmergy as a Universal Coordination Mechanism I:
> Definition and Components." *Cognitive Systems Research*, 38, 4-13.
> doi:10.1016/j.cogsys.2015.12.002

#### Hybrid Storage Model

Full 10,240-bit vectors are 1,280 bytes each. Storing them directly in EVM
contract storage would cost approximately 880K gas per vector (22,100 gas per
cold zero-to-nonzero SSTORE slot x 40 slots, per EIP-2929). This is
prohibitively expensive for a system designed to handle thousands of knowledge
entries.

The hybrid model splits storage into three tiers:

**On-chain anchor (95 bytes packed into 3 storage slots, ~66K gas for cold writes).** The
permanent on-chain record contains only the minimum metadata needed for
verifiability:

```solidity
struct InsightAnchor {
    bytes32 vectorHash;     // keccak256 of the full 1,280-byte vector
    bytes32 contentHash;    // keccak256 of the associated content
    address author;         // 20 bytes — publisher identity
    uint64  publishBlock;   // 8 bytes — publication timestamp
    uint8   kind;           // 1 byte — KnowledgeKind enum
    uint8   tier;           // 1 byte — retention tier
    uint8   state;          // 1 byte — lifecycle state
}
```

**Event log (full vector + content, ~21-23K gas for the vector's calldata
portion).** The complete 1,280-byte vector and its associated content are
emitted as event data (calldata costs 16 gas per non-zero byte per EIP-2028;
for random binary vectors, effectively all 1,280 bytes are non-zero, yielding
~20,480 gas for the vector alone plus LOG opcode overhead). Events are
permanently stored in the chain's log and are accessible to full nodes and
archive nodes, but they are dramatically cheaper than contract storage because
they do not occupy the EVM state trie.

**In-memory precompile index (0 gas for reads).** The HDC precompile (at
address 0x09) maintains an in-memory index that is rebuilt from event logs on
node startup. Similarity searches read from this index via `eth_call` — a view
operation that costs the querier no gas. The computational cost is borne by
the validator's CPU, amortized across all queries.

This architecture means: writes are expensive and go through consensus
(ensuring verifiability), while reads are free and instantaneous (ensuring
usability). The full vector data is permanently available for reconstruction,
but the actual search index is an ephemeral in-memory structure that any
node can rebuild from the canonical event log.

#### Comparison: Private vs. Shared

The shared substrate has different properties than private storage:

| Property | Private HDC | Shared (On-Chain) HDC | Why This Matters |
|----------|-------------|----------------------|------------------|
| Latency | ~50ns | ~1-10ms (RPC + precompile) | Private is 100-200K x faster, enabling real-time cognitive loops. Shared is for background enrichment, not inner-loop retrieval. |
| Trust | Full (self-authored) | Skeptical (source-weighted) | Self-authored knowledge is first-hand experience. Shared knowledge is hearsay — useful, but requires a discount factor proportional to source reputation and corroboration. |
| Mutability | Free | Costs gas | Private knowledge can be freely created, modified, and deleted as the agent learns. Shared knowledge has economic friction by design — publication is a commitment, not a draft. |
| Persistence | Agent lifetime | Governed by demurrage | Private knowledge persists as long as the agent runs (subject to local decay/GC). Shared knowledge must be actively maintained — if nobody pays the renewal cost, it decays away. |
| Privacy | Complete | Public (or encrypted) | Private knowledge never leaves the agent's process. Shared knowledge is visible to all validators and queryable by all agents. Encrypted variants are possible but add complexity. |
| Capacity | Limited by RAM | Limited by gas economics | A typical agent might hold 1K-1M entries locally (~1.2 MB - ~1.2 GB for vectors alone, plus associated content metadata). The shared substrate's capacity is bounded by the aggregate willingness of agents to pay gas for publication. |

---

### Demurrage — Knowledge Economics

Knowledge is not free to maintain. On the shared substrate, knowledge has
**demurrage** — a continuous cost of existence:

1. **Relevance decay:** Knowledge that hasn't been queried or reinforced
   loses relevance score over time (Ebbinghaus forgetting curve).
2. **Trust decay:** Knowledge from a source that hasn't been validated
   recently loses trust score.
3. **Economic decay:** Storing knowledge on-chain costs gas. If nobody
   is willing to pay the renewal cost, the knowledge gets garbage collected.

#### Intellectual Heritage: Freigeld and the Velocity of Knowledge

The concept of demurrage on knowledge is directly inspired by Silvio Gesell's
theory of *Freigeld* (free money), articulated in his 1916 work *The Natural
Economic Order*. Gesell observed that physical goods depreciate over time
(food rots, machinery rusts, buildings crumble), but money does not — giving
money holders an asymmetric advantage over goods holders. This asymmetry,
Gesell argued, encourages hoarding and slows the velocity of exchange.

His solution: impose a carrying cost on money itself. Holders of Gesell's
proposed *Freigeld* would need to periodically affix stamps to their banknotes
to maintain their validity. The stamp cost acts as negative interest — a
penalty for holding rather than circulating. The economic effect is to
incentivize spending (or investing) over hoarding, increasing monetary
velocity and economic activity.

This was not merely theory. The most famous implementation was the **Wörgl
experiment** (1932-1933), in which the Austrian town of Wörgl issued its own
local currency with 1% monthly demurrage during the depths of the Great
Depression. Holders had to purchase and affix a stamp each month to keep
their banknotes valid. The results were striking: Wörgl's unemployment fell
by 16% over the 13-month experiment while Austrian unemployment rose by 19%.
The local currency circulated approximately 14 times faster than the
Austrian schilling, funding public works projects including road paving,
bridge construction, and a ski jump. The experiment was ended in September
1933 by Austria's central bank (Oesterreichische Nationalbank), which
asserted its monopoly on currency issuance — but not before attracting
international attention and inspiring similar experiments worldwide.

The concept persists today in the **Chiemgauer**, a complementary currency
operating in the Chiemgau region of Bavaria since 2003. From 2003 to 2015,
the Chiemgauer carried 2% quarterly demurrage (8% annually); the rate was
later adjusted to 3% semi-annually (6% annually). The currency circulates
approximately 2.5-4x faster than the euro within its community. As of
2023, over 600 businesses accept the Chiemgauer, and it has facilitated
millions of euros in local transactions.

> Gesell, S. (1916). *Die natürliche Wirtschaftsordnung durch Freiland und
> Freigeld* [The Natural Economic Order]. English translation by Philip Pye,
> 1958. Available at silviogesell.de.
>
> Muralt, A. von (1934). "The Wörgl Experiment with Depreciating Money."
> *Annals of Public and Cooperative Economics*, 10(1), 48-57.
>
> Gelleri, C. (2009). "Chiemgauer Regiomoney: Theory and Practice of a Local
> Currency." *International Journal of Community Currency Research*, 13, 61-75.

#### Why Demurrage Works for Knowledge

The analogy to knowledge systems is precise:

- **Traditional knowledge stores (databases, wikis):** Storing knowledge costs
  nothing after the initial write. There is no incentive to clean up, validate,
  or remove stale entries. Over time, the store fills with outdated, redundant,
  and contradictory information. The "knowledge" accumulates entropy. Anyone
  who has maintained a corporate wiki recognizes this failure mode.

- **Demurrage knowledge stores (roko's shared substrate):** Storing knowledge
  has a continuous cost. Knowledge that is not actively used (queried,
  confirmed, reinforced) decays toward a garbage-collection threshold. The
  carrying cost creates pressure to *confirm valuable knowledge* (extending
  its half-life by incrementing its tier) and *let stale knowledge die*
  (stop paying the implicit renewal cost). The result is a self-cleaning
  knowledge commons that reflects current, validated understanding rather
  than accumulated historical debris.

The decay formula in roko is:

```
balance(t) = balance(t_0) * exp(-lambda * (t - t_0))
```

Where `lambda = 0.005 per hour` (base rate, yielding a base half-life of
ln(2)/0.005 ~ 138.6 hours ~ 5.8 days), modulated by tier and kind
multipliers. Knowledge confirmed by multiple independent agents gets promoted
to higher tiers with longer half-lives (up to 5x for Persistent tier, giving
a half-life of ~29 days). Knowledge that goes unqueried and unconfirmed
decays to the GC threshold within days or weeks, depending on its kind
(at base rate, balance reaches 10% of initial value in ~19 days).

#### Mapping to Ostrom's Design Principles

The shared substrate's governance model maps closely to Elinor Ostrom's
eight design principles for long-enduring commons institutions, identified
through empirical study of hundreds of common-pool resource management
systems worldwide (Ostrom, 1990):

| Ostrom's Principle | Roko Implementation |
|--------------------|---------------------|
| **1. Clearly defined boundaries** | Only registered agents (with staked DAEJI tokens) can publish. The InsightBoard contract enforces identity and minimum stake requirements. |
| **2. Proportional equivalence between benefits and costs** | Publication costs gas + stake. Higher-value knowledge (more confirmations) earns reputation rewards. Consuming knowledge is free (view calls), but contributing nothing eventually degrades your reputation. |
| **3. Collective-choice arrangements** | Tier promotion thresholds (3/10/25 confirmations) are protocol parameters that could be governed by token-holder vote. The community of agents collectively determines what constitutes "confirmed" knowledge. |
| **4. Monitoring** | All publications are on-chain and publicly auditable. The ReputationRegistry tracks per-agent accuracy, timeliness, novelty, and integrity scores. Monitoring is automatic and continuous. |
| **5. Graduated sanctions** | First offense: contradiction detection → trust score reduction. Repeated false publications: reputation decay via EMA (alpha=0.1). Severe: stake slashing for provably false insights. The sanctions escalate with severity and frequency. |
| **6. Conflict-resolution mechanisms** | Anti-knowledge architecture allows structural disagreement (see [04-knowledge.md](04-knowledge.md)). The 15% mandatory contrarian retrieval ensures agents consider contradictory evidence (see [05-context-assembly.md](05-context-assembly.md)). Disputes are resolved empirically: which knowledge proves useful over time? |
| **7. Minimal recognition of rights to organize** | Agents are autonomous — they decide when to publish, what to consume, and how much skepticism to apply. The protocol does not mandate behavior; it creates incentive structures. |
| **8. Nested enterprises (for large-scale)** | Topic-specific sub-indexes within the substrate. Local (private, fast) nested within global (shared, trust-weighted). Federation possible via cross-chain bridges. |

> Ostrom, E. (1990). *Governing the Commons: The Evolution of Institutions
> for Collective Action*. Cambridge University Press.
> doi:10.1017/CBO9780511807763

The relationship between Ostrom's principles and blockchain affordances
has been formalized by Rozas et al. (2021), who identify six affordances
-- tokenisation, formalisation and decentralisation of rules, autonomous
automatisation, decentralisation of power over the infrastructure,
increasing transparency, and codification of trust -- and map each to
Ostrom's design principles. For a detailed treatment, see
[07-shared-substrate.md](07-shared-substrate.md).

> Rozas, D., Tenorio-Fornés, A., Díaz-Molina, S., & Hassan, S. (2021).
> "When Ostrom Meets Blockchain: Exploring the Potentials of Blockchain
> for Commons Governance." *SAGE Open*, 11(1), 1-14.
> doi:10.1177/21582440211002526

**Note on novelty:** The application of Gesell-style economic demurrage
to a shared digital knowledge substrate -- as described in this document
and detailed in [04-knowledge.md](04-knowledge.md) and
[07-shared-substrate.md](07-shared-substrate.md) -- has no peer-reviewed
precedent as of May 2026. The existing literature treats monetary
demurrage and knowledge commons governance as separate domains. This
crossover is genuine whitespace.

#### Consumption-Side Discounts

This creates a Gesellian economics of information: knowledge must be actively
maintained to persist. Stale, unused, unvalidated knowledge naturally fades,
preventing the shared substrate from becoming a junk heap.

Agents consuming shared knowledge apply additional discounts:
- **Source reliability:** Has this author's past knowledge been accurate?
- **Recency:** How old is this knowledge?
- **Corroboration:** Do other independent sources confirm it?
- **Relevance:** How similar is the retrieval context to the publication context?

---

## Design Goals

1. **Sub-microsecond local retrieval.** The HDC comparison kernel on local
   indexes must complete in <1µs including overhead. See
   [06-vector-search.md](06-vector-search.md) for SIMD benchmarks.

2. **Sub-10ms on-chain retrieval.** Querying the shared substrate via RPC
   should take <10ms for top-K search over 100K vectors. See
   [07-shared-substrate.md](07-shared-substrate.md) for tiered gas optimization.

3. **Zero-copy composability.** Bind, bundle, and permute must work on
   in-place vectors without allocation. See
   [02-hdc-foundations.md](02-hdc-foundations.md) for the algebra.

4. **Graceful scaling.** The system should handle 1K vectors (new agent)
   to 10M vectors (mature agent) without architectural changes. See
   [06-vector-search.md](06-vector-search.md) for tiered retrieval.

5. **Consensus determinism.** On-chain HDC operations must produce identical
   results across all validators, all platforms, all compiler versions.
   See [06-vector-search.md](06-vector-search.md) for HNSW determinism recipe.

6. **Minimal on-chain footprint.** Store only what must be on-chain (anchors,
   hashes, provenance). Keep bulk data off-chain or in events.

7. **Anti-knowledge safety.** The system must structurally prevent anti-knowledge
   from being confused with regular knowledge, without relying on warning
   labels or metadata flags. See [04-knowledge.md](04-knowledge.md) for the
   anti-knowledge subspace separation design.

---

## References

Compiled citations for this document, in order of first appearance:

1. Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction to Computing
   in Distributed Representation with High-Dimensional Random Vectors."
   *Cognitive Computation*, 1(2), 139-159. doi:10.1007/s12559-009-9009-8

2. Kanerva, P. (1988). *Sparse Distributed Memory*. MIT Press.

3. Gayler, R. W. (2003). "Vector Symbolic Architectures Answer Jackendoff's
   Challenges for Cognitive Neuroscience." In *Proceedings of the Joint
   International Conference on Cognitive Science*, 133-138. arXiv:cs/0412059

4. Plate, T. A. (2003). *Holographic Reduced Representations: Distributed
   Representation for Cognitive Structures*. CSLI Publications.

5. Kleyko, D., Rachkovskij, D. A., Osipov, E., & Rahimi, A. (2023). "A Survey
   on Hyperdimensional Computing aka Vector Symbolic Architectures, Part I:
   Models and Data Transformations." *ACM Computing Surveys*, 55(6), Article 130,
   1-61. doi:10.1145/3538531

6. Goldberg, D. (1991). "What Every Computer Scientist Should Know About
   Floating-Point Arithmetic." *ACM Computing Surveys*, 23(1), 5-48.
   doi:10.1145/103162.103163

7. Sumers, T. R., Yao, S., Narasimhan, K., & Griffiths, T. L. (2024).
   "Cognitive Architectures for Language Agents." *Transactions on Machine
   Learning Research* (02/2024). arXiv:2309.02427

8. Zhang, Y., Li, Y., Cui, L., et al. (2023). "Siren's Song in the AI Ocean:
   A Survey on Hallucination in Large Language Models." arXiv:2309.01219

9. Kahneman, D. (2011). *Thinking, Fast and Slow*. Farrar, Straus and Giroux.

10. Grassé, P.-P. (1959). "La reconstruction du nid et les coordinations
    interindividuelles chez *Bellicositermes natalensis* et *Cubitermes sp.*"
    *Insectes Sociaux*, 6(1), 41-80. doi:10.1007/BF02223791

11. Theraulaz, G., & Bonabeau, E. (1999). "A Brief History of Stigmergy."
    *Artificial Life*, 5(2), 97-116. doi:10.1162/106454699568700

12. Heylighen, F. (2016). "Stigmergy as a Universal Coordination Mechanism I:
    Definition and Components." *Cognitive Systems Research*, 38, 4-13.
    doi:10.1016/j.cogsys.2015.12.002

13. Gesell, S. (1916). *Die natürliche Wirtschaftsordnung durch Freiland und
    Freigeld* [The Natural Economic Order].

14. Muralt, A. von (1934). "The Wörgl Experiment with Depreciating Money."
    *Annals of Public and Cooperative Economics*, 10(1), 48-57.

15. Gelleri, C. (2009). "Chiemgauer Regiomoney: Theory and Practice of a Local
    Currency." *International Journal of Community Currency Research*, 13, 61-75.

16. Ostrom, E. (1990). *Governing the Commons: The Evolution of Institutions
    for Collective Action*. Cambridge University Press.
    doi:10.1017/CBO9780511807763

17. Rozas, D., Tenorio-Fornés, A., Díaz-Molina, S., & Hassan, S. (2021).
    "When Ostrom Meets Blockchain: Exploring the Potentials of Blockchain
    for Commons Governance." *SAGE Open*, 11(1), 1-14.
    doi:10.1177/21582440211002526

18. Bronzini, M., Nicolini, C., Lepri, B., Staiano, J., & Passerini, A.
    (2025). "Hyperdimensional Probe: Decoding LLM Representations via Vector
    Symbolic Architectures." arXiv:2509.25045
