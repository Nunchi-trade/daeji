# PR #41 — docs(plans): HDC search Merkle inclusion proof — follow-up plan

**Branch:** `jl/hdc-merkle-proof-followup`
**State:** OPEN (DRAFT)
**Author:** Jae Lee (@JaeLeex)
**Base:** main
**Stats:** +141 / -0 (1 file added, 1 commit)

---

## What This PR Does

This is a **plan-only PR** — no code changes. It tracks the deferred Merkle inclusion proof work from PR #42 (D-PR1-v2). The HDC precompile's `search()` function currently returns stubbed empty bytes for the `proof` field on each `Hit`. This PR documents the design path for a follow-up implementation PR.

---

## Why Deferred

D-PR1-v2 stubs the `proof: bytes` field because full implementation requires three components that don't exist yet:

1. **QMDB proof-extraction API** — `commonware_storage::merkle::journaled` has the Merkle tree structure, but no `prove(key) -> MerkleProof` method is publicly exposed.

2. **Side-channel access from revm precompiles to QMDB** — revm precompiles only see the abstract `StateDb` trait; generating Merkle proofs needs a concrete dependency injection to the QMDB instance.

3. **Proof-encoding format** — needs to match what off-chain verifiers can replay against the block header's state root (RLP? SSZ? custom?).

Implementing all three would have added ~1-2 weeks, delaying the consensus-correct event-replay rework needed for the May 11 announcement.

---

## Design Plan Summary

### Goal

`HDCPrecompile::search()` returns a non-empty `proof: bytes` for each `Hit`. Off-chain verifiers can:
1. Take the `proof` + `entry_id` + `block.stateRoot` from the block header
2. Recompute the expected leaf hash
3. Verify the Merkle path from leaf to state root
4. Conclude: this entry was in the chain's HDC index at this block

### New API: `QmdbProvable` trait

```rust
pub trait QmdbProvable {
    fn prove_inclusion(&self, partition: Partition, key: &[u8]) -> Option<MerkleProof>;
    fn prove_absence(&self, partition: Partition, key: &[u8]) -> MerkleProof;
}

pub struct MerkleProof {
    pub leaf: B256,
    pub siblings: Vec<B256>,
    pub directions: Bytes,  // packed left/right bitmap
}
```

Implemented on the existing `Backend` type against the `commonware_storage::merkle::journaled` tree.

### Wiring into HDCPrecompiles

**Recommended: Option A — Inject prover at construction:**

```rust
pub struct HDCPrecompiles {
    eth: EthPrecompiles,
    state: Arc<HDCState>,
    prover: Arc<dyn QmdbProvable>,
}
```

Cleaner than Option B (thread-local/context downcast). Testable with mock prover.

### ABI Change

`Hit` struct changes from `(bytes16 id, uint32 similarity, uint32 weight, uint32 score)` to `(bytes16 id, uint32 similarity, uint32 weight, uint32 score, bytes proof)`. This is ABI-breaking — migration path: bump selector for `search`, keep old selector with empty proof for one release.

### Files to Add/Modify

| File | Change |
|------|--------|
| `crates/storage/qmdb/src/lib.rs` | New `QmdbProvable` trait + `MerkleProof` struct |
| `crates/storage/backend/src/backend.rs` | Implement `QmdbProvable` for `Backend` |
| `crates/precompiles/src/hdc.rs` | Accept `Arc<dyn QmdbProvable>`, generate proofs in `dispatch_search` |
| `IHDCPrecompile.sol` | Updated Hit struct |
| `crates/storage/backend/tests/proofs.rs` | New: round-trip, absence, proof-size tests |
| `crates/precompiles/tests/proof_integration.rs` | New: search returns valid proof, off-chain verification |

### Estimated Effort

- QMDB proof API + Backend impl: ~3 days
- Precompile wiring + ABI bump + tests: ~2 days
- Total: ~1 week including review cycles

### Dependencies

- PR #42 (D-PR1-v2) must merge first (introduces `crates/precompiles/`)
- D-PR2 / #39 must merge first (introduces wall-clock `block.timestamp`)

### Out of Scope

- `eth_getProof` RPC handler
- QMDB historical-state proofs (`0x0B` precompile)
- Cross-chain proof verification

---

## Why a PR Instead of an Issue

- Travels with the codebase as a versioned design artifact
- Reviewable line-by-line before implementation starts
- Forces explicit owner assignment
- Keeps `gh pr list` view of deferred work honest

---

## Owner

Jae Lee (default). May reassign to Bhargav (QMDB expertise) or Will (precompile ownership).
