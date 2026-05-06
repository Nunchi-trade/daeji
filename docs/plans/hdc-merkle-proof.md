# HDC Search — Merkle Inclusion Proof (Follow-up to D-PR1-v2)

**Status:** Deferred from D-PR1-v2 (`jl/precompile-registry-spec-aligned`). This document is the design plan for the follow-up PR that lands the proof.

**Spec source:** `~/obsidian-vault/research/2026-05-04-wp-agent-chainv2/raw/02-daeji/02-precompiles-and-contracts.md`, lines 161-163:

> "`proof` (variable bytes): Merkle inclusion proof for the entry against the current block's state root, so third parties can verify results without trusting the responding node."

## Why deferred

D-PR1-v2 stubs the `proof` field on the `Hit` ABI to empty bytes. The full implementation requires:

1. **QMDB proof-extraction API** that doesn't currently exist as a public surface. The `commonware_storage::merkle::journaled` Merkle tree has the structure (per `crates/storage/backend/src/backend.rs`), but no `prove(key) -> MerkleProof` method is exposed.
2. **Side-channel access** from the precompile (running inside revm execution) to the QMDB instance owning the current state. revm precompiles only see the abstract `StateDb` trait; QMDB-specific Merkle proof generation needs a concrete dependency injection.
3. **Proof-encoding format** (RLP? SSZ? Custom?) — needs to match what off-chain verifiers can replay against the block header's state root.

Implementing all three in D-PR1-v2 would have added ~1-2 weeks; deferring keeps the consensus-correct event-replay rework on schedule for the May 11 announcement.

## Scope of this follow-up PR

### Goal

`HDCPrecompile::search()` returns a non-empty `proof: bytes` for each `Hit`. Off-chain verifiers can:
1. Take the `proof` + the `entry_id` + the `block.stateRoot` from the block header
2. Recompute the expected leaf hash
3. Verify the Merkle path from leaf to state root
4. Conclude: this entry was in the chain's HDC index at this block.

### Files to add/modify

#### `crates/storage/qmdb/src/lib.rs` — new public API

```rust
pub trait QmdbProvable {
    /// Generate an inclusion proof for `key` in the named partition
    /// against the current state root.
    ///
    /// Returns `None` if the key is absent. Use `prove_absence` for
    /// non-membership proofs.
    fn prove_inclusion(
        &self,
        partition: Partition,
        key: &[u8],
    ) -> Option<MerkleProof>;

    fn prove_absence(
        &self,
        partition: Partition,
        key: &[u8],
    ) -> MerkleProof;
}

pub struct MerkleProof {
    /// Leaf hash being proven.
    pub leaf: B256,
    /// Sibling hashes from leaf to root, bottom-up.
    pub siblings: Vec<B256>,
    /// Bitmap of left/right sibling positions, packed.
    pub directions: Bytes,
}
```

#### `crates/storage/backend/src/backend.rs` — implementation

Implement `QmdbProvable` for the existing `Backend` type. Internals: walk the `commonware_storage::merkle::journaled` tree from the leaf up, collecting sibling hashes at each level. The tree depth is `log2(N)` where N is the number of entries — should complete in microseconds for 100k entries.

#### `crates/precompiles/src/hdc.rs` — wire proof into output

`HDCPrecompiles` needs access to `QmdbProvable`. Two options:

**Option A — Inject the prover at construction:**
```rust
pub struct HDCPrecompiles {
    eth: EthPrecompiles,
    state: Arc<HDCState>,
    prover: Arc<dyn QmdbProvable>,
}
```
Cleaner. Requires threading `Arc<dyn QmdbProvable>` from `RevmExecutor::with_precompiles()` down to construction.

**Option B — Look up via thread-local or context handle:**
revm `ContextTr` exposes the state DB. If the concrete state type implements `QmdbProvable`, downcast at call time. Avoids new constructor parameters but couples the precompile to the QMDB-specific concrete type.

**Recommendation: Option A.** Cleaner trait boundary; testable with mock prover.

#### `crates/precompiles/src/hdc.rs` — `dispatch_search` change

After collecting `Hit { id, similarity, weight }` entries:
1. For each hit, derive the `InsightBoard` storage key for that entry's `Insight` struct
2. Call `prover.prove_inclusion(Partition::Storage, &storage_key)`
3. Encode the proof as `bytes` and append to the `Hit` ABI tuple

#### `crates/precompiles/src/hdc.rs` — ABI bump

The `Hit` output struct in Solidity (`packages/agents/src/IHDCPrecompile.sol`) is currently `(bytes16 id, uint32 similarity, uint32 weight, uint32 score)`. New shape: `(bytes16 id, uint32 similarity, uint32 weight, uint32 score, bytes proof)`.

This is an ABI-breaking change. Migration path: bump precompile-registry selector for `search`, keep old selector returning empty proof for one release, then drop. Or version the Solidity interface (`IHDCPrecompileV2`).

### Tests

#### `crates/storage/backend/tests/proofs.rs` (new)

- `test_prove_inclusion_round_trip`: insert key K with value V; `prove_inclusion(K)` returns proof; verify proof against state root recovers V.
- `test_prove_absence_round_trip`: prove K' absent against the same state root.
- `test_proof_size_is_log_n`: insert 1024 keys; assert proof has ~10 sibling hashes.

#### `crates/precompiles/tests/proof_integration.rs` (new)

- `test_search_returns_valid_proof`: post 10 InsightBoard insights; call `search`; verify each returned proof against `state_root`.
- `test_proof_verifies_offchain`: capture state_root + proof; replay verification in pure code (no chain access); assert verifier accepts.

### Acceptance criteria

- [ ] `QmdbProvable` trait merged with `prove_inclusion` + `prove_absence`
- [ ] `Backend` implements `QmdbProvable` against the journaled Merkle tree
- [ ] `HDCPrecompiles::new()` accepts `Arc<dyn QmdbProvable>`; `RevmExecutor` plumbs it through
- [ ] `dispatch_search` returns non-empty `proof: bytes` for each hit
- [ ] `IHDCPrecompile.sol` updated; off-chain verifier sample (TypeScript or Rust) demonstrates verification
- [ ] All existing D-PR1-v2 tests still pass with the new ABI
- [ ] New tests above pass

### Out of scope for this follow-up

- `eth_getProof` RPC handler — separate work; not strictly required for the precompile contract
- QMDB historical-state proofs (`0x0B` precompile per Will's spec) — different precompile, separate PR
- Cross-chain proof verification (e.g. relaying HDC search results to Hyperliquid) — application layer, not infra

### Estimated effort

- QMDB proof API + Backend impl: ~3 days
- Precompile wiring + ABI bump + tests: ~2 days
- Total: ~1 week including review cycles

### Dependencies

- D-PR1-v2 must merge first (introduces the `crates/precompiles/` crate the follow-up modifies)
- D-PR2 / `Nunchi-trade/daeji#39` must merge first (introduces wall-clock `block.timestamp`; without it, key derivation for proofs is unstable)

### Owner

Jae Lee (open). May reassign to Bhargav (storage / QMDB expertise) or Will (precompile owner) post-Berlin.
