# HDC Precompile — Requirements & Implementation

Precompile address: `0x09`

Hyperdimensional Computing gives agents nanosecond-scale similarity search over
high-dimensional binary vectors. This is the cognitive backbone — every insight,
memory, and knowledge fragment is a 10,240-bit vector that can be compared,
combined, and searched in constant time.

---

## What HDC Does

HDC encodes arbitrary data (text, embeddings, structured records) into fixed-size
binary hypervectors. Operations on these vectors are:

- **Bind (XOR):** Associate two concepts → `A ⊕ B`
- **Bundle (majority vote):** Merge multiple concepts → `[A, B, C]`
- **Permute (rotate):** Create sequence-aware representations → `ρ(A)`
- **Similarity (Hamming distance):** Compare two vectors → 0.0 to 1.0

All operations are bitwise on 10,240-bit vectors (160 × u64 words = 1,280 bytes).
Hamming distance uses `popcnt` instructions — hardware-accelerated on all modern CPUs.

---

## Precompile Interface

### Operations

| Selector | Method | Gas | Description |
|----------|--------|-----|-------------|
| `0x01` | `storeVector(bytes32 key, bytes vector)` | 25,000 | Store a 1,280-byte vector keyed by hash |
| `0x02` | `searchSimilar(bytes vector, uint8 topK)` | 50,000 + 5,000/result | Find K nearest vectors by Hamming distance |
| `0x03` | `getMerkleProof(bytes32 key)` | 10,000 | Get inclusion proof for stored vector |
| `0x04` | `deleteVector(bytes32 key)` | 5,000 | Remove vector from index |
| `0x05` | `bundleVectors(bytes32[] keys)` | 15,000 + 2,000/key | Majority-vote bundle of stored vectors |
| `0x06` | `bindVectors(bytes32 keyA, bytes32 keyB)` | 8,000 | XOR bind two vectors, return result |

### Gas Rationale

Comparison with Solidity implementation (from roko v2-depth analysis):
- Solidity Hamming distance over 10,240 bits: ~2,220 gas (160 MLOAD + XOR + POPCNT simulation)
- Precompile Hamming distance: ~50 gas (native popcnt over 160 u64s)
- **44x cheaper** for single comparison
- Top-K search: Solidity O(n) loop = catastrophic gas; precompile = flat 50K + 5K/result
- **5,000x+ speedup** for similarity search at scale

### ABI Encoding

```solidity
// Store
abi.encodePacked(uint8(0x01), bytes32(key), bytes(vector))
// vector MUST be exactly 1,280 bytes (10,240 bits)

// Search
abi.encodePacked(uint8(0x02), bytes(queryVector), uint8(topK))
// Returns: abi.encode(bytes32[] keys, uint16[] distances)

// Proof
abi.encodePacked(uint8(0x03), bytes32(key))
// Returns: bytes (Merkle proof path)
```

---

## Storage Architecture

### In-Memory Index

The precompile maintains an in-memory index of all stored vectors for fast search.
Two tiers based on collection size:

| Size | Algorithm | Latency | Memory |
|------|-----------|---------|--------|
| < 100K vectors | Brute-force scan | < 1ms | ~128 MB |
| ≥ 100K vectors | HNSW (hierarchical navigable small world) | < 0.1ms | ~200 MB |

Auto-switches at the 100K threshold (matches mirage-rs behavior).

### On-Chain Storage

Vectors are persisted in QMDB under a dedicated partition or under a reserved
contract address's storage. Each vector occupies 20 storage slots (1,280 bytes / 32 bytes per slot + key metadata).

```
Slot layout for vector at key K:
  slot(K, 0)    = metadata (owner address, timestamp, flags)
  slot(K, 1..20) = vector data (160 u64s packed into 20 × 32-byte slots)
```

### Merkle Proofs

`getMerkleProof` returns a QMDB inclusion proof for the vector's storage slots.
This enables:
- Cross-chain verification of vector existence
- Light client proofs that a similarity search result is genuine
- Audit trails for knowledge provenance

---

## Genesis Configuration

### Projection Matrix

The HDC encoding pipeline uses a fixed ProjectionMatrix to convert raw embeddings
into binary hypervectors. This matrix must be identical across all validators.

```
Size: 10,240 × 768 × f16 = ~15,073,280 bytes (~14.4 MB at f16)
      or 10,240 × 192 × f32 = ~7,864,320 bytes (~7.5 MB at f32)
```

The agent-chainv2 spec quotes ~1.95 MB, implying a compressed or sparse representation.

**Options:**
1. Embed in genesis blob (simplest, but large genesis)
2. Content-addressed storage with hash in genesis config
3. Deterministic generation from a seed (eliminates storage, but constrains matrix choice)

Recommendation: Option 3 — use a seeded random matrix generated from a genesis-specified
seed. All validators derive the same matrix deterministically. No storage overhead.

---

## Implementation in daeji

### REVM Integration Point

REVM supports custom precompiles via the `PrecompileSet` trait. daeji's executor
currently uses the standard Cancun precompile set.

```rust
// Current: crates/executor/src/lib.rs
// Uses REVM's default precompiles

// Required: Add custom precompile registry
use revm::precompile::{Precompile, PrecompileResult};

struct HdcPrecompile {
    index: Arc<RwLock<HdcIndex>>,
    projection: Arc<ProjectionMatrix>,
}

impl Precompile for HdcPrecompile {
    fn run(&self, input: &Bytes, gas_limit: u64) -> PrecompileResult {
        let selector = input[0];
        match selector {
            0x01 => self.store_vector(&input[1..], gas_limit),
            0x02 => self.search_similar(&input[1..], gas_limit),
            0x03 => self.get_merkle_proof(&input[1..], gas_limit),
            // ...
        }
    }
}
```

### State Sync

The in-memory HDC index must be reconstructed on node startup by replaying all
`storeVector` / `deleteVector` operations from genesis. For fast startup:

1. Snapshot the index periodically (every N blocks)
2. On startup, load latest snapshot + replay subsequent blocks
3. Index snapshots stored alongside QMDB snapshots

### Consensus Safety

All precompile operations must be **deterministic**. The HDC operations (XOR, popcount,
majority vote) are inherently deterministic. HNSW search is also deterministic given
the same insertion order and parameters.

Critical: The HNSW index transition (brute-force → HNSW at 100K) must happen at a
deterministic block height, not based on wall-clock time.

---

## Mirage Parity

mirage-rs implements HDC via feature-gated `chain` module:

| mirage feature | daeji equivalent |
|---------------|-----------------|
| `HdcIndex::insert(key, vector)` | `storeVector` precompile call |
| `HdcIndex::search(query, k)` | `searchSimilar` precompile call |
| `HdcIndex::remove(key)` | `deleteVector` precompile call |
| Auto brute-force→HNSW switch | Same, at 100K threshold |
| In-process, single-node | Consensus-validated, multi-validator |

The key difference: mirage runs in-process (single node, mutable state). daeji runs
across validators (consensus on every write, deterministic reads). Reads are fast
(precompile, no consensus needed). Writes go through transactions.

---

## InsightBoard Integration

The InsightBoard contract uses HDC for duplicate detection and similarity search
across knowledge entries:

```
Agent stores insight → InsightBoard.submit(insight)
  → HDC.storeVector(hash(insight), encode(insight))
  → HDC.searchSimilar(encode(insight), 5) → check for near-duplicates
  → If novel: accept and store
  → If duplicate: reject or merge
```

InsightEntry types (from mirage-rs):
1. `OBSERVATION` — raw sensory data
2. `INFERENCE` — derived conclusion
3. `PREDICTION` — future state estimate
4. `STRATEGY` — action plan
5. `MEMORY` — episodic recall
6. `AXIOM` — foundational belief

Each type maps to different HDC encoding parameters (different projection subspaces).

---

## Testing Strategy

1. **Unit tests:** HDC algebra (bind, bundle, permute, similarity) correctness
2. **Precompile tests:** Gas metering, ABI encoding/decoding, edge cases (empty vectors, max topK)
3. **Consensus tests:** Two validators process same HDC transactions → identical state
4. **Performance tests:** Search latency at 1K, 10K, 100K, 1M vectors
5. **Integration tests:** InsightBoard contract calling HDC precompile end-to-end
