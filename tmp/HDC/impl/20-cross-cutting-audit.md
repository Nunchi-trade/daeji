# 20 -- Cross-Cutting Audit of HDC Implementation

> **Status: AUDIT COMPLETE, SOME FINDINGS RESOLVED.**
> Original findings F01-F04 were identified during the first audit pass.
> Resolution status for each finding is documented inline below.
> New findings F07-F15 were added 2026-05-08 based on PR #42 review and
> deeper codebase analysis. See "New Findings (F07-F15)" section at end.

> **Scope.** This audit examines cross-cutting concerns across all HDC-related
> crates in the daeji/kora repository: `kora-hdc` (crates/hdc/core), `kora-hdc-chain`
> (crates/hdc/chain), the orphaned `crates/kora-hdc`, and HDC integration points in
> `kora-executor`, `kora-rpc`, and `kora-runner`.
>
> **Date:** 2026-05-08
>
> **Repository root:** `/Users/will/dev/nunchi/daeji/`

---

## Cross-Cutting Audit Findings

### F01: Orphaned Duplicate Crate (`crates/kora-hdc`)

**Severity: HIGH**
**Resolution: OPEN.** The orphaned crate at `crates/kora-hdc/` still exists in the
repository as an untracked directory. It has not been deleted. PR #42 introduces a
third crate (`kora-precompiles`) but does not address this orphan. The workspace
`Cargo.toml` still resolves `kora-hdc` to `crates/hdc/core`, so the orphan is
not compiled, but it remains a source of confusion.

There are two copies of the core HDC algebra code:

| Path | Package Name | In Workspace? | Line Count |
|------|-------------|---------------|------------|
| `crates/hdc/core/` | `kora-hdc` | YES (via `crates/hdc/*` glob) | 8,142 lines |
| `crates/kora-hdc/` | `kora-hdc` | NO (not matched by any glob) | 747 lines |

The files `vector.rs`, `bundle.rs`, `encode.rs`, and `constants.rs` are
**byte-identical** between the two locations (`diff` produces zero output).
The workspace `Cargo.toml` line 77 resolves `kora-hdc` to `crates/hdc/core`:

```toml
kora-hdc = { path = "crates/hdc/core" }
```

The `crates/kora-hdc/` directory is an orphan. It is not in the workspace
members glob and not referenced by any Cargo.toml. However, it is tracked as
an untracked file (`?? crates/kora-hdc/` would appear if listed independently).

**Key difference:** `crates/kora-hdc/src/search.rs` is a 9-line stub that
merely re-exports constants. `crates/hdc/core/src/search/` is a full module
directory (1,547 lines across 6 files: `mod.rs`, `brute.rs`, `hnsw.rs`,
`local.rs`, `simd.rs`, `tiered.rs`). The `crates/hdc/core` version also adds
4 additional modules not present in the orphan: `knowledge/` (7 files),
`trust.rs`, `context.rs`, and `cognitive/` (5 files).

**Risk:** A developer may edit the orphan crate believing it is the canonical
source, or a merge may accidentally swap the workspace path back to the orphan.

**Recommendation:** Delete `crates/kora-hdc/` entirely. It is dead code.

---

### F02: Event Processing is Completely Stubbed

**Severity: HIGH (consensus-safety impact)**
**Resolution: PARTIALLY RESOLVED.** PR #42 (`kora-precompiles`) includes event
decoders and an `on_finalize_extend` hook that processes finalized block events.
However, the `hdc` branch's `event.rs` remains entirely stubbed with `B256::ZERO`
topic hashes. The PR #42 event replay is more complete but still uses a different
architecture (precompile-internal vs. external event sink). Full resolution
requires merging the PR #42 event handling approach into the main codebase and
deleting the stubbed `event.rs`.

The event sync handler at `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs`
is entirely non-functional:

- **Lines 13-25:** All 5 event topic hashes are hardcoded to `B256::ZERO`:
  ```rust
  pub const INSIGHT_PUBLISHED: B256 = B256::ZERO; // TODO: compute actual hash
  pub const INSIGHT_ACCEPTED: B256 = B256::ZERO;   // TODO: compute actual hash
  pub const INSIGHT_REJECTED: B256 = B256::ZERO;   // TODO: compute actual hash
  pub const INSIGHT_CHALLENGED: B256 = B256::ZERO; // TODO: compute actual hash
  pub const PHEROMONE_DEPOSITED: B256 = B256::ZERO; // TODO: compute actual hash
  ```

- **Lines 29-53:** `process_log()` function body is all comments and `info!()` calls.
  It never actually decodes log data or calls any index mutation method.

- **Line 131:** `OnChainHdcIndex::record_pheromone()` is a no-op:
  ```rust
  pub fn record_pheromone(&mut self, _topic: B256, _region: B256, _strength: u64) {
      // TODO: implement pheromone tracking
  }
  ```

Since all topic hashes are `B256::ZERO`, multiple events would collide on the
same topic hash value, and none would actually match real on-chain events.

**Note:** PR #42 (on the `main` branch path) has a separate, more complete
implementation of event replay via `on_finalize_extend`. This branch's
implementation is behind.

---

### F03: Precompile Address Conflict: `0x09` vs `0xA0C`

**Severity: MEDIUM (architectural decision divergence)**
**Resolution: RESOLVED in favor of `0xA0C`.** PR #42 uses `0xA0C` for the HDC
precompile and `0xA0D` for the Stigmergy precompile. This avoids the BLAKE2F
collision at `0x09` (EIP-152). The `hdc` branch still uses `0x09` and must be
updated. All Solidity contracts (`HdcLib.sol`, `InsightBoard.sol`) and all
documentation must be updated to reference `0xA0C`.

| Crate | Address | Source |
|-------|---------|--------|
| `crates/hdc/chain/src/precompile.rs` (this branch) | `0x09` | Impl plans 07, 15 |
| PR #42 (`kora-precompiles`) | `0xA0C` | Spec alignment, avoids BLAKE2F collision |

The `0x09` address collides with the BLAKE2F precompile (EIP-152, Istanbul).
The impl plans acknowledge this at `15-wiring-guide.md` lines 916-924 but
proceed anyway. PR #42 uses `0xA0C` in unallocated space.

The current precompile.rs at line 14-17:
```rust
pub const PRECOMPILE_ADDRESS: Address = Address::new([
    0x00, 0x00, ..., 0x00, 0x09,
]);
```

**Risk:** Any EVM tooling or contract assuming standard BLAKE2F availability
at `0x09` will silently get HDC operations instead.

---

### F04: Gas Cost Mismatch

**Severity: MEDIUM**
**Resolution: RESOLVED in favor of 50,000 flat gas.** PR #42 uses a flat 50,000
gas cost for all HDC precompile operations. This is conservative but prevents the
DoS vector created by the `hdc` branch's 100-200 gas pricing. The impl plans'
original 500-1,500 gas values were based on early benchmarks and may be revisited
after production profiling, but the flat 50,000 is the current standard.

| Operation | This Branch (precompile.rs L20-35) | PR #42 (spec) | Impl Plans (07) |
|-----------|-----------------------------------|---------------|-----------------|
| Hamming Distance | 100 gas | 50,000 gas | 1,500 gas |
| Bind | 100 gas | 50,000 gas | 500 gas |
| Bundle (base) | 100 gas | 50,000 gas | 500 gas |
| Permute | 120 gas | 50,000 gas | 500 gas |
| Vector ID | 200 gas | 50,000 gas | N/A |
| Is Similar | 110 gas | 50,000 gas | N/A |

The gas costs in the current implementation are 2-3 orders of magnitude lower
than both the spec-aligned PR and the original impl plans. At 100 gas per
HDC operation, the precompile is essentially free, which creates a DoS vector
(attackers can execute millions of 10,240-bit vector operations per block).

---

## Error Handling Analysis

### Pattern Summary

| Crate | Primary Error Type | Pattern | Consistency |
|-------|-------------------|---------|-------------|
| `kora-hdc` (core) | None -- returns raw values | No error types at all | N/A |
| `kora-hdc-chain::precompile` | `PrecompileError` (thiserror) | `Result<(u64, Vec<u8>), PrecompileError>` | Good |
| `kora-hdc-chain::rpc` | `HdcRpcError` (thiserror) | `Result<T, HdcRpcError>` | Good but limited |
| `kora-hdc-chain::index` | None | Returns `bool` or `Option` | Inconsistent |
| `kora-rpc::hdc` | Maps to `RpcError` | `RpcResult<T>` via jsonrpsee | Good |
| `kora-executor` | `ExecutionError` (thiserror) | `Result<T, ExecutionError>` | Good |
| `kora-runner` | `RunnerError` (anyhow) | `Result<T, RunnerError>` | Good |

### Specific Issues

**E01: `HdcRpcError` has only one variant (rpc.rs L108-112)**
```rust
pub enum HdcRpcError {
    InvalidVectorLength(usize),
}
```
This means every possible error (search failure, index unavailable, encoding
error) must be shoehorned into `InvalidVectorLength`. Compare with
`kora-rpc::RpcError` which has 12 variants including HDC-specific ones
(`InvalidVectorLength`, `InsightNotFound`, `KnowledgeStoreUnavailable`,
`UnknownEncodingMethod`). The chain-level error type is too narrow.

**E02: Precompile unwrap in production code (precompile.rs L158)**
```rust
let n_bytes: [u8; 4] = data[BYTES..BYTES + 4].try_into().unwrap();
```
This is in `exec_permute()`, which is called during consensus block execution.
Although the length check at line 154 (`data.len() < BYTES + 4`) should
guarantee the slice is 4 bytes, the `unwrap()` is unnecessary -- this should
use `map_err` like `read_vector()` does (line 103-105). A panic in a
precompile during block execution would crash the validator.

**E03: Index operations silently discard errors (index.rs)**
- `insert_insight()` returns only the ID, no indication of capacity limits.
- `update_state()` returns `bool` instead of `Result`.
- `search()` returns empty vec on any issue.

**E04: WisdomGate uses `HashMap` without error propagation (wisdom.rs)**
- `submit()` returns `bool` (line 60-75) instead of a typed result.
- `challenge()` returns `bool` (line 79-88) with no reason for failure.

---

## Concurrency & Thread Safety

### C01: `unsafe impl Send + Sync` for `HdcVector` (vector.rs L23-24)

```rust
unsafe impl Send for HdcVector {}
unsafe impl Sync for HdcVector {}
```

`HdcVector` is `#[repr(C, align(64))] pub struct HdcVector(pub [u64; WORDS])`.
Since `[u64; 160]` is already `Send + Sync` by default (all primitives are),
these unsafe impls are **unnecessary**. They are harmless but suggest the
author may have been working around a compiler error that was actually caused
by something else, or copying from a version that used raw pointers.

**Recommendation:** Remove both unsafe impls. `[u64; WORDS]` is inherently
`Send + Sync`.

### C02: `OnChainHdcIndex` protected by `parking_lot::RwLock` (runner.rs L238)

```rust
Some(Arc::new(parking_lot::RwLock::new(
    kora_hdc_chain::OnChainHdcIndex::new(),
)))
```

This is the correct pattern. The `RwLock` allows concurrent reads from RPC
handlers while the finalized reporter holds a write lock during event
processing. However:

- The `OnChainHdcIndex` uses `HashMap` internally (index.rs L14-16), which
  means iteration order is non-deterministic. For search results this is
  acceptable (results are sorted by distance), but if any future consensus
  code iterates the index, this becomes a consensus safety bug.

- PR #42's comparison doc (comparison-prs-vs-impl-plans.md L186) explicitly
  flags this: "Does NOT fix HashMap ordering (uses RwLock<HdcIndex> with Vec
  storage)."

### C03: `HnswIndex` uses `BTreeMap` for deterministic iteration (hnsw.rs L73-75)

```rust
nodes: BTreeMap<u64, HnswNode>,
key_to_id: BTreeMap<H256, u64>,
```

This is the correct approach for consensus-safe code. The HNSW implementation
deliberately uses `BTreeMap` instead of `HashMap` to ensure deterministic
iteration order. The comment at line 4 confirms this is intentional.

### C04: RPC handler correctly uses `spawn_blocking` (hdc.rs L104, L114, etc.)

All RPC method implementations correctly offload HDC computation to
`tokio::task::spawn_blocking`, preventing the async runtime from being blocked
by CPU-intensive vector operations. This is the correct pattern for 10,240-bit
computations.

### C05: No `Mutex` contention analysis

The `parking_lot::RwLock<OnChainHdcIndex>` is held during `search()` which
does brute-force iteration over all vectors. With a large index, this read
lock could be held for significant time, potentially blocking event processing
writes. The brute-force search has O(n) complexity with n being the total
number of accepted insights.

---

## Code Duplication Report

### D01: Orphan Crate Duplication (CRITICAL)

| File | `crates/kora-hdc/src/` | `crates/hdc/core/src/` | Identical? |
|------|----------------------|---------------------|------------|
| `vector.rs` | 304 lines | 304 lines | YES (byte-identical) |
| `bundle.rs` | 126 lines | 126 lines | YES (byte-identical) |
| `encode.rs` | 251 lines | 251 lines | YES (byte-identical) |
| `constants.rs` | 22 lines | 22 lines | YES (byte-identical) |
| `lib.rs` | 35 lines | 42 lines | NO (hdc/core has more modules) |
| `search.rs` | 9 lines (stub) | N/A (directory) | NO (completely different) |

Total duplicated code: **703 lines** across 4 byte-identical files.

### D02: Hamming Distance Implemented Three Times

1. **`crates/hdc/core/src/vector.rs` L96-102** -- scalar `hamming_distance()`,
   the public API on `HdcVector`.
2. **`crates/hdc/core/src/search/simd.rs` L12-17** -- `hamming_scalar()`,
   identical logic, used as the oracle for SIMD testing.
3. **`crates/hdc/core/src/search/simd.rs` L199-217** -- `hamming_distance()`
   (in `search::simd` module), the SIMD-dispatching version.

The vector.rs `hamming_distance` is used by `kora-hdc-chain` (precompile and
index), while the search module has its own `hamming_distance` with SIMD
dispatch. These could diverge if one is updated and the other is not.

**Current call paths:**
- `kora-hdc-chain::precompile` -> `kora_hdc::hamming_distance` (vector.rs, scalar only)
- `kora-hdc-chain::index::search` -> `kora_hdc::hamming_distance` (vector.rs, scalar only)
- `kora-hdc::search::brute::BruteForceIndex::search` -> `search::simd::hamming_distance` (SIMD-dispatched)

The precompile and on-chain index do NOT use the SIMD path, even though it
exists and is consensus-safe (integer-only arithmetic).

### D03: Vector Parsing/Validation Duplicated

Vector deserialization from bytes happens in three places with different error
handling:

1. **`kora-hdc::vector::deserialize()`** (vector.rs L147-153) -- takes
   `&[u8; BYTES]`, cannot fail (unwrap on try_into is safe for known-size
   array).
2. **`kora-hdc-chain::precompile::read_vector()`** (precompile.rs L94-107) --
   takes `&[u8]` + offset, returns `Result<HdcVector, PrecompileError>`.
3. **`kora-hdc-chain::rpc::parse_vector()`** (rpc.rs L90-95) -- takes `&[u8]`,
   returns `Result<HdcVector, HdcRpcError>`.

### D04: EVM Construction Duplicated via Macros

The `revm.rs` file uses two macros (`simulate!` at L252-271 and
`execute_block!` at L413-444) that contain nearly identical EVM
construction + result handling logic. The pattern of "build EVM, set tx,
call replay(), handle result" is repeated.

---

## Dependency Graph Issues

### DEP01: `kora-hdc` (core) Has Correct Zero-Chain-Dep Property

Verified from `crates/hdc/core/Cargo.toml`:
```toml
[dependencies]
rand = { workspace = true }
rand_chacha = { workspace = true }
tiny-keccak = { workspace = true }
serde = { workspace = true }
```

No alloy, no revm, no kora chain dependencies. This matches the architectural
decision in `00-overview.md` and `15-wiring-guide.md`.

### DEP02: `kora-hdc-chain` Depends on `revm` Directly

From `crates/hdc/chain/Cargo.toml`:
```toml
revm.workspace = true
```

However, `kora-hdc-chain` does NOT actually use `revm` in any of its source
files. The precompile function signature is plain Rust:
```rust
pub fn hdc_precompile(input: &[u8], gas_limit: u64) -> Result<(u64, Vec<u8>), PrecompileError>
```

The `revm` dependency appears to be vestigial -- the actual REVM integration
is in `kora-executor/src/hdc_precompiles.rs` which wraps the chain crate's
precompile function into REVM's `PrecompileProvider` trait.

**Recommendation:** Remove `revm` from `kora-hdc-chain`'s dependencies. It
violates the layering principle: the chain crate should not depend on the
execution engine.

### DEP03: Orphan `crates/kora-hdc/` is Missing `serde` Dependency

The orphan `crates/kora-hdc/Cargo.toml` does not list `serde` in its
dependencies, while the canonical `crates/hdc/core/Cargo.toml` does. This
confirms the orphan is stale.

### DEP04: Dependency Flow is Clean (No Cycles)

```
kora-hdc (core, no chain deps)
  ^
  |
kora-hdc-chain (precompile, index, event, rpc, wisdom)
  ^
  |
kora-executor (HdcPrecompileProvider wraps chain precompile)
kora-rpc (HdcApiImpl wraps chain rpc::HdcApi)
  ^
  |
kora-runner (assembles everything)
```

No circular dependencies. The dependency direction matches the design.

### DEP05: `tracing` Dependency Only Used in `event.rs`

`kora-hdc-chain` has `tracing.workspace = true` but it is only used in one
file: `event.rs` line 4 (`use tracing::info;`). And that file is entirely
stubbed out. The `info!()` calls log messages like "Processing InsightPublished
event" but never actually process anything.

---

## Code Smell Census (unwrap, todo, unsafe, clone abuse counts)

### `unwrap()` Calls

| Location | Count | In Test Code? | Risk |
|----------|-------|---------------|------|
| `crates/hdc/chain/src/precompile.rs` | 1 | NO (L158) | **HIGH** -- consensus path |
| `crates/hdc/chain/src/precompile.rs` | 9 | YES (tests) | Low |
| `crates/hdc/chain/src/wisdom.rs` | 4 | YES (tests) | Low |
| `crates/hdc/core/src/vector.rs` | 1 | NO (L150, `deserialize`) | Low (infallible for fixed-size) |
| `crates/hdc/core/src/search/hnsw.rs` | 7 | NO (L107, 206, 222, 262, 267, 274, 356) | **HIGH** -- graph invariant panics |
| `crates/hdc/core/src/search/hnsw.rs` | 16 | YES (tests) | Low |
| `crates/hdc/core/src/search/brute.rs` | 11 | YES (tests) | Low |
| `crates/hdc/core/src/search/local.rs` | 5 | YES (tests) | Low |
| `crates/hdc/core/src/search/tiered.rs` | 3 | YES (tests) | Low |
| `crates/hdc/core/src/trust.rs` | 1 | YES (tests) | Low |
| `crates/hdc/core/src/knowledge/store.rs` | 8 | YES (tests) | Low |
| `crates/hdc/core/tests/` | 8 | YES | Low |
| `crates/node/rpc/src/hdc.rs` | 11 | YES (tests) | Low |

**Summary:** 2 production `unwrap()` calls of concern:
1. `precompile.rs:158` -- in consensus block execution path
2. `hnsw.rs:206,222,262,267,274,356` -- 6 unwraps in HNSW graph operations that
   assume invariants but would panic if violated

### `todo!()` / `unimplemented!()`

**Zero occurrences** in the entire `crates/hdc` tree or `crates/node` tree.
However, there are 7 `// TODO:` comments (not macro invocations):

| File | Line | Comment |
|------|------|---------|
| `crates/hdc/chain/src/event.rs` | 13 | `// TODO: compute actual hash` |
| `crates/hdc/chain/src/event.rs` | 16 | `// TODO: compute actual hash` |
| `crates/hdc/chain/src/event.rs` | 19 | `// TODO: compute actual hash` |
| `crates/hdc/chain/src/event.rs` | 22 | `// TODO: compute actual hash` |
| `crates/hdc/chain/src/event.rs` | 25 | `// TODO: compute actual hash` |
| `crates/hdc/chain/src/event.rs` | 35 | `// TODO: ABI-decode log data for each event type` |
| `crates/hdc/chain/src/index.rs` | 131 | `// TODO: implement pheromone tracking` |

All 7 are in `kora-hdc-chain`. The event system is entirely stub.

### `unsafe`

| File | Line | Usage | Justified? |
|------|------|-------|------------|
| `crates/hdc/core/src/vector.rs` | 23 | `unsafe impl Send for HdcVector` | **NO** -- `[u64; 160]` is already Send |
| `crates/hdc/core/src/vector.rs` | 24 | `unsafe impl Sync for HdcVector` | **NO** -- `[u64; 160]` is already Sync |
| `crates/hdc/core/src/search/hnsw.rs` | 97-101 | `std::slice::from_raw_parts` | **QUESTIONABLE** -- could use `bytemuck` or safe serialization |
| `crates/hdc/core/src/search/simd.rs` | 29 | `unsafe fn popcount_mm256` | YES -- intrinsics require unsafe |
| `crates/hdc/core/src/search/simd.rs` | 46 | `unsafe fn csa_256` | YES -- intrinsics require unsafe |
| `crates/hdc/core/src/search/simd.rs` | 60 | `pub unsafe fn hamming_avx2` | YES -- intrinsics require unsafe |
| `crates/hdc/core/src/search/simd.rs` | 130 | `pub unsafe fn hamming_avx512` | YES -- intrinsics require unsafe |
| `crates/hdc/core/src/search/simd.rs` | 153-154 | `pub(super) unsafe fn hamming_neon` | YES -- intrinsics require unsafe |
| `crates/hdc/core/src/search/simd.rs` | 203, 206, 212 | `unsafe { hamming_* }` | YES -- calling unsafe SIMD fns |

**Summary:** 11 `unsafe` occurrences. 2 are unnecessary (Send/Sync), 1 is
questionable (raw pointer cast in HNSW), 8 are justified (SIMD intrinsics).

### `HACK` / `FIXME` / `XXX`

**Zero occurrences** in the entire codebase.

### `clone()` Patterns

| Location | Pattern | Concern |
|----------|---------|---------|
| `knowledge/store.rs:40` | `entry.vector.clone()` | Clones 1,280-byte HdcVector on every insert into the flat index |
| `knowledge/store.rs:91` | `entry.clone()` | Full entry clone during scored retrieval |
| `knowledge/store.rs:139` | `entry.clone()` | Clone during search result construction |
| `cognitive/replay.rs:40` | `candidates.clone()` | Clones entire Vec of candidates |
| `cognitive/state_machine.rs:153` | `self.current.clone()` | Clones state on no-transition |
| `cognitive/state_machine.rs:264` | `next.clone()` | Clones next state before assignment |
| `search/hnsw.rs:248` | `candidates.clone()` | Clones candidate list |
| `search/hnsw.rs:312,341` | `node.vector.clone()` | Clones HdcVector during search |
| `context.rs:344` | `candidate.clone()` | Clones during context assembly |

Most clones are of `HdcVector` (1,280 bytes each). In hot paths like search,
this means allocating and copying 1,280 bytes per candidate per iteration.
Consider using `Arc<HdcVector>` for shared ownership in the index to avoid
copies during search.

---

## Crate Boundary Violations

### B01: `kora-rpc` Directly Constructs `kora-hdc-chain` Types

In `crates/node/rpc/src/hdc.rs` line 181, the test directly constructs
`kora_hdc_chain::OnChainHdcIndex::new()` and wraps it in
`Arc<parking_lot::RwLock<_>>`. This is fine for tests but reveals that the
RPC crate has deep knowledge of the chain crate's internal types.

The `HdcApi` constructor (rpc.rs L21) takes
`Arc<parking_lot::RwLock<OnChainHdcIndex>>`, which means the `parking_lot`
synchronization primitive is part of the public API contract. If the chain
crate ever changes its concurrency model, the RPC crate must also change.

**Recommendation:** Define a trait (e.g., `HdcQueryable`) in the chain crate
and have the RPC layer depend on the trait, not the concrete type + lock.

### B02: `kora-executor` Uses `#[path]` Directive

In `crates/node/executor/src/revm.rs` lines 32-34:
```rust
#[path = "hdc_precompiles.rs"]
mod hdc_precompiles;
use hdc_precompiles::HdcPrecompileProvider;
```

The `#[path]` directive is unusual and suggests the module was added alongside
`revm.rs` without modifying `lib.rs`. This works but is non-standard. The
module should be declared in `lib.rs` or placed in a subdirectory.

### B03: `kora-hdc-chain::rpc` Has Asymmetric Error Handling

The chain crate's `HdcRpcError` has only `InvalidVectorLength`, while the
node-level `kora-rpc::RpcError` has multiple HDC-specific variants
(`InsightNotFound`, `KnowledgeStoreUnavailable`, `UnknownEncodingMethod`).
The node-level errors cannot be produced by the chain-level API because the
chain API never returns those error types. These RPC error variants are dead
code.

### B04: Type Mismatch Between Crate Boundaries

The core crate uses `[u8; 32]` for vector IDs (vector.rs `vector_id` returns
`[u8; 32]`), while the chain crate uses `alloy_primitives::B256` (index.rs
L73-74 converts via `B256::from(id_bytes)`). This conversion happens at every
crate boundary crossing and is a source of friction.

The RPC layer adds another conversion: `kora-hdc-chain::rpc::SearchResultRpc`
uses `B256`, which is then mapped to `kora-rpc::hdc::HdcSearchResult` also
using `B256` -- these two types are structurally identical but separate,
requiring a manual field-by-field copy (hdc.rs L149-152).

---

## Configuration Propagation Analysis

### CFG01: `HdcConfig` Propagation Path

The configuration flows correctly through the system:

```
NodeConfig.hdc (kora-config/src/hdc.rs)
  -> ProductionRunner.hdc_config (runner.rs L134)
  -> run() checks hdc_enabled (runner.rs L232-243)
  -> Conditionally creates OnChainHdcIndex (L236-243)
  -> Conditionally registers precompile on 3 executors (L264-265, L329-332, L389-392)
  -> Conditionally wires HdcApi into RPC (L312-318)
```

However:

- The `KnowledgeStore` initialization described in `15-wiring-guide.md` sections
  3.2 (lines 360-380) is NOT implemented in `runner.rs`. The runner only creates
  `OnChainHdcIndex`, not `KnowledgeStore`.

- The `FinalizedReporter` HDC event handler wiring described in
  `15-wiring-guide.md` section 3.5 (lines 448-455) is NOT implemented. The
  runner does not attach any HDC event handler to the finalized reporter.

### CFG02: Executor Created 3 Times, HDC Flag Set Independently

Three separate `RevmExecutor` instances are created in `runner.rs`:

| Instance | Line | Purpose | HDC check |
|----------|------|---------|-----------|
| RPC executor | L263-266 | Serves `eth_call` | `if hdc_enabled` |
| Finalized reporter executor | L329-332 | Re-executes for state root | `if hdc_enabled` |
| Consensus app executor | L389-392 | Block execution | `if hdc_enabled` |

All three independently check `hdc_enabled` and call `.with_hdc_precompile()`.
This repetition is error-prone -- if someone adds a fourth executor and forgets
the HDC check, consensus could produce different results between the block
execution path and the RPC simulation path.

**Recommendation:** Create a factory method:
```rust
fn make_executor(&self) -> RevmExecutor {
    let mut e = RevmExecutor::new(self.chain_id);
    if self.hdc_enabled() { e = e.with_hdc_precompile(); }
    e
}
```

---

## Divergence from PR #42 and Design Docs

### DIV01: Interface Style

| Aspect | This Branch | PR #42 |
|--------|-------------|--------|
| Dispatch | Raw opcode byte (`input[0]`) | Solidity 4-byte function selectors |
| Gas model | Per-opcode (100-200 gas) | Flat 50,000 gas |
| Precompile address | `0x09` | `0xA0C` |
| Stigmergy | Solidity `PheromoneRegistry` exists; Rust replay/read path is stubbed | Full `StigmergyPrecompile` at `0xA0D` |
| Event replay | Stubbed (all `B256::ZERO`) | Fully implemented |
| Crate structure | `kora-hdc` + `kora-hdc-chain` (2 crates) | `kora-precompiles` (1 crate) |

### DIV02: Knowledge Kind Half-Lives

PR #42 tests use a 3-minute Warning half-life, while the design docs specify
48 hours. The current branch's `knowledge/kind.rs` values should be verified
against the canonical spec.

### DIV03: Merkle Proofs

PR #41 provides a concrete follow-up plan for Merkle proofs via
`QmdbProvable` trait injection. Neither this branch nor PR #42 implements
actual proofs. PR #42 stubs them; this branch does not address them at all.

---

## Recommended Changes Checklist (Prioritized)

### P0 -- Critical (consensus safety / correctness)

- [ ] **Delete `crates/kora-hdc/`** -- orphaned duplicate crate. Risk of
  confusion is high. No workspace references point to it.
  - Files: `crates/kora-hdc/` (entire directory)

- [ ] **Fix production `unwrap()` in `precompile.rs:158`** -- replace with
  `try_into().map_err(|_| PrecompileError::InvalidInput(...))`. This is on the
  consensus block execution path.
  - File: `crates/hdc/chain/src/precompile.rs` line 158

- [ ] **Implement event topic hashes in `event.rs`** -- all 5 hashes are
  `B256::ZERO`. Compute actual keccak256 of event signatures or adopt
  `alloy-sol-types` event decoding.
  - File: `crates/hdc/chain/src/event.rs` lines 13-25

- [ ] **Implement `process_log()` body in `event.rs`** -- currently a no-op.
  Without this, the on-chain index never receives updates from finalized blocks.
  - File: `crates/hdc/chain/src/event.rs` lines 29-53

- [ ] **Resolve precompile address** -- decide between `0x09` (this branch)
  and `0xA0C` (PR #42). Update all code and docs to match.
  - Files: `crates/hdc/chain/src/precompile.rs` line 14-17,
    `crates/node/executor/src/hdc_precompiles.rs`

### P1 -- High (correctness / maintainability)

- [ ] **Fix gas costs** -- current 100-200 gas values are far too low. Align
  with spec (50,000 flat) or impl plans (500-1,500).
  - File: `crates/hdc/chain/src/precompile.rs` lines 20-35

- [ ] **Remove unnecessary `unsafe impl Send/Sync`** for `HdcVector`.
  - File: `crates/hdc/core/src/vector.rs` lines 23-24

- [ ] **Remove vestigial `revm` dependency** from `kora-hdc-chain`.
  - File: `crates/hdc/chain/Cargo.toml` line 17

- [ ] **Wire `FinalizedReporter` HDC event handler** as specified in the
  wiring guide section 3.5. Currently missing from `runner.rs`.
  - File: `crates/node/runner/src/runner.rs`

- [ ] **Use SIMD `hamming_distance` in precompile** -- the precompile
  currently calls the scalar-only `kora_hdc::hamming_distance` (vector.rs)
  instead of the SIMD-dispatched version (search/simd.rs).
  - Files: `crates/hdc/chain/src/precompile.rs`,
    `crates/hdc/core/src/lib.rs` (re-export SIMD version)

- [ ] **Address HNSW `unwrap()` calls** -- 6 unwraps in production HNSW code
  (hnsw.rs L206, 222, 262, 267, 274, 356) that could panic on graph
  corruption.
  - File: `crates/hdc/core/src/search/hnsw.rs`

### P2 -- Medium (code quality / architecture)

- [ ] **Replace raw `#[path]` directive** in executor with standard module
  declaration.
  - File: `crates/node/executor/src/revm.rs` lines 32-34

- [ ] **Expand `HdcRpcError`** to cover more than just `InvalidVectorLength`.
  - File: `crates/hdc/chain/src/rpc.rs` lines 107-112

- [ ] **Remove dead `RpcError` variants** (`InsightNotFound`,
  `KnowledgeStoreUnavailable`, `UnknownEncodingMethod`) or implement the
  code paths that produce them.
  - File: `crates/node/rpc/src/error.rs` lines 78-88

- [ ] **Replace `HashMap` with `BTreeMap`** in `OnChainHdcIndex` for
  deterministic iteration order, matching the pattern used in `HnswIndex`.
  - File: `crates/hdc/chain/src/index.rs` lines 14-16

- [ ] **Replace `HashMap` with `BTreeMap`** in `WisdomGate` for deterministic
  iteration during `resolve()`.
  - File: `crates/hdc/chain/src/wisdom.rs` line 39

- [ ] **Replace `HashMap` with `BTreeMap`** in `KnowledgeStore::entries` and
  `TrustRegistry::agents` for deterministic behavior.
  - Files: `crates/hdc/core/src/knowledge/store.rs` line 13,
    `crates/hdc/core/src/trust.rs` line 73

- [ ] **Replace unsafe `slice::from_raw_parts`** in HNSW level assignment
  with safe serialization (e.g., `kora_hdc::serialize()`).
  - File: `crates/hdc/core/src/search/hnsw.rs` lines 97-101

- [ ] **Extract executor factory** to eliminate the 3x executor creation
  with independent HDC flag checks.
  - File: `crates/node/runner/src/runner.rs`

### P3 -- Low (optimization / polish)

- [ ] **Consider `Arc<HdcVector>`** for shared index entries to reduce
  1,280-byte clones during search.
  - Files: `crates/hdc/core/src/knowledge/store.rs`,
    `crates/hdc/core/src/search/hnsw.rs`

- [ ] **Unify `hamming_distance` implementations** -- have a single canonical
  entry point that auto-dispatches to SIMD when available, and use it
  everywhere (precompile, index search, RPC).
  - Files: `crates/hdc/core/src/vector.rs`,
    `crates/hdc/core/src/search/simd.rs`

- [ ] **Add structured logging** (`tracing::instrument`, span context) to HDC
  operations for observability. Currently only `event.rs` uses tracing, and
  it logs stubs.
  - Files: all `crates/hdc/chain/src/*.rs`

- [ ] **Implement `record_pheromone()`** in `OnChainHdcIndex` (currently a
  no-op at index.rs L130-132).

- [ ] **Define `HdcQueryable` trait** in `kora-hdc-chain` and use it in
  `kora-rpc` instead of depending on concrete `Arc<RwLock<OnChainHdcIndex>>`.
  - Files: `crates/hdc/chain/src/rpc.rs`, `crates/node/rpc/src/hdc.rs`

---

## Second-Pass Remediation Detail

### Current Implementation State

The implementation now has three partially overlapping HDC surfaces:

| Surface | Current State | Main Gap |
|---------|---------------|----------|
| Rust core (`crates/hdc/core`) | Algebra, SIMD search, HNSW/local search, knowledge scoring, trust, context, affect, state machine, and replay scoring exist. | Persistence, FSRS, dream execution, `KnowledgeStore` search integration, and canonical SIMD export are incomplete. |
| Rust chain/node integration (`crates/hdc/chain`, `crates/node/*`) | Raw Rust precompile, ephemeral index, RPC wrapper, REVM provider, and runner gating exist. | Event replay is stubbed, finalized reporter is not wired to the HDC index, and the index uses states/IDs that do not match Solidity. |
| Solidity contracts (`contracts/src`) | `InsightBoard.sol`, `PheromoneRegistry.sol`, interfaces, and tests exist. | `HdcPrecompile.sol` expects a different precompile ABI than Rust implements; contract tests bypass the real precompile path. |

The most important correction to the first-pass audit is that Solidity contracts
are present. The current risk is interface divergence, not absence: the Rust
precompile is stateless vector math, while the Solidity library assumes a
stateful vector index with store/search/delete operations.

### Ad-Hoc / Duct-Tape / Anti-Pattern Clarifications

**Rust/Solidity precompile ABI drift is the largest correctness risk.**
`crates/hdc/chain/src/precompile.rs` defines:

| Rust Opcode | Rust Meaning |
|-------------|--------------|
| `0x01` | `hamming_distance(a, b)` |
| `0x02` | `bind(a, b)` |
| `0x03` | `bundle(vectors)` with a 4-byte count |
| `0x04` | `permute(vector, n)` |
| `0x05` | `vector_id(vector)` |
| `0x06` | `is_similar(a, b)` |

`contracts/src/HdcPrecompile.sol` documents and uses a different surface:
`0x01` `storeVector`, `0x02` `searchSimilar`, `0x03` `deleteVector`, `0x04`
`bundle`, `0x05` `bind`, `0x06` `hamming`, and `0x07` `permute`. That means
`InsightBoard.submit()` calls `HdcLib.searchSimilar()` with opcode `0x02`, but
Rust interprets `0x02` as `bind` and rejects the payload length. `HdcLib.bind()`
uses opcode `0x05`, but Rust returns a 32-byte vector ID rather than a
1,280-byte vector. `HdcLib.permute()` uses opcode `0x07`, which Rust rejects as
invalid.

**The Solidity contract tests currently hide that mismatch.**
`contracts/test/InsightBoard.t.sol` defines `TestableInsightBoard`, copy-pastes
`submit()` and `purge()`, and removes `HdcLib.searchSimilar()`,
`HdcLib.storeVector()`, and `HdcLib.deleteVector()`. Those tests validate much of
the FSM logic, but not the production precompile integration.

**Event sync is still a placeholder, and it is also out of date.**
`crates/hdc/chain/src/event.rs` uses `B256::ZERO` topic constants and references
events such as `InsightAccepted(bytes32)` and `InsightRejected(bytes32)`. The
current Solidity interfaces emit `InsightPublished`, `InsightConfirmed`,
`InsightChallenged`, `InsightStateChanged`, `InsightRenewed`, `InsightPurged`,
`PheromoneDeposited`, `PheromoneConfirmed`, and `PheromonePruned`. The existing
`process_log(index, topic0, data, log_address)` signature cannot decode indexed
arguments because it does not receive `topics[1..]`.

**The on-chain index is not keyed or filtered the same way as the contract.**
`OnChainHdcIndex::insert_insight()` computes `id = vector_id(vector)`, while
`InsightBoard.submit()` uses `insightId = keccak256(vectorHash, author,
block.number)`. `OnChainHdcIndex::search()` filters on Rust
`InsightState::Accepted`, but Solidity uses `ACTIVE` for confirmed searchable
insights and has no `Accepted` enum value.

**HDC is enabled in executors but not in finalized indexing.**
`ProductionRunner::run()` creates the shared HDC index and passes it to RPC, and
it enables the precompile on all three `RevmExecutor` instances. However,
`FinalizedReporter` only updates `BlockIndex`; no HDC event observer is attached,
so finalized contract events never update `OnChainHdcIndex`.

### Better Target Architecture

The clean target is an event-replayed read model:

1. The Rust precompile remains deterministic and stateless. It provides only
   pure vector operations that are safe inside EVM execution.
2. Solidity contracts own lifecycle state and emit complete events. They should
   not assume the precompile has process-local mutable storage unless Rust
   explicitly implements such storage as consensus state.
3. `OnChainHdcIndex` is a node-local projection rebuilt from finalized logs. It
   is keyed by Solidity `insightId`, stores the full vector plus `vectorHash`,
   author, kind, tier, state, block numbers, and optional pheromone metadata.
4. RPC serves two categories: pure vector utility calls and read-only queries
   against the finalized projection. Missing backing stores should be omitted or
   return explicit unavailable errors, not empty placeholder data.

This architecture keeps consensus state in normal contract storage and uses the
precompile as acceleration, not as an undocumented storage engine.

### Concrete Implementation Steps

**Precompile ABI alignment**

- In `contracts/src/HdcPrecompile.sol`, replace the opcode table and payload
  layout with the Rust layout in `crates/hdc/chain/src/precompile.rs`.
- Update `HdcLib.hamming()` to call opcode `0x01` and decode the right-aligned
  32-byte EVM word produced by `exec_hamming_distance()`.
- Update `HdcLib.bind()` to call opcode `0x02`.
- Update `HdcLib.bundle()` to call opcode `0x03` and encode a 4-byte `uint32`
  count, not a 2-byte `uint16`.
- Update `HdcLib.permute()` to call opcode `0x04` and encode `vector || uint32 n`
  because `exec_permute()` reads the vector first and `n` second.
- Add `HdcLib.vectorId()` and `HdcLib.isSimilar()` wrappers only if Solidity
  needs them.
- Remove or gate `HdcLib.storeVector()`, `HdcLib.searchSimilar()`, and
  `HdcLib.deleteVector()` unless Rust adds matching stateful opcodes.

**Rust precompile hardening**

- In `crates/hdc/chain/src/precompile.rs::exec_hamming_distance()`, require
  `data.len() == 2 * BYTES`.
- In `exec_bind()`, require `data.len() == 2 * BYTES`.
- In `exec_bundle()`, reject `count == 0`, check
  `data.len() == 4 + count * BYTES`, and use saturating/checked gas arithmetic.
- In `exec_permute()`, require `data.len() == BYTES + 4` and replace
  `try_into().unwrap()` with an error path.
- In `exec_vector_id()`, require `data.len() == BYTES`.
- In `exec_is_similar()`, require `data.len() == 2 * BYTES`.
- Revisit `gas::{HAMMING_DISTANCE,BIND,BUNDLE_BASE,BUNDLE_PER_VECTOR,PERMUTE,VECTOR_ID,IS_SIMILAR}`
  before production, because 100-200 gas underprices 1,280-byte operations.

**Event replay and index projection**

- In `crates/hdc/chain/src/index.rs`, replace `InsightState` with a Solidity
  aligned enum: `Submitted`, `Verified`, `Active`, `Challenged`, `Decaying`,
  `Archived`, `Purged`, or add a conversion layer from `u8`.
- Change `OnChainHdcIndex::insert_insight()` to accept
  `insight_id: B256`, `vector_hash: B256`, `vector: HdcVector`, `publisher:
  Address`, `block_number: u64`, `kind: u8`, and `tier: u8`.
- Change `OnChainHdcIndex::update_state()` to validate transitions or at least
  preserve the source contract's state exactly. Return a typed `Result`, not
  `bool`, so event replay can report impossible updates.
- Change `OnChainHdcIndex::search()` to filter on `Active` and whatever other
  states are policy-approved for search. Do not filter on a Rust-only
  `Accepted` state.
- In `crates/hdc/chain/src/event.rs`, replace `process_log()` with a function
  that accepts `topics: &[B256]`, `data: &[u8]`, `address: Address`, and
  `block_number: u64`. Decode indexed fields from `topics[1..]` and dynamic
  fields from `data`.
- Compute topics for the actual signatures in `contracts/src/IInsightBoard.sol`
  and `contracts/src/IPheromoneRegistry.sol`, preferably from generated
  bindings or `alloy-sol-types` rather than handwritten constants.
- In `crates/node/reporters/src/lib.rs`, add an optional HDC log sink to
  `FinalizedReporter` and invoke it after finalized execution receipts are
  available. `index_finalized_block()` already demonstrates how to iterate logs
  from `ExecutionOutcome::receipts`.
- In `crates/node/runner/src/runner.rs`, pass the shared
  `Arc<RwLock<OnChainHdcIndex>>` into `FinalizedReporter` when HDC is enabled.

**Core cleanup**

- Delete the orphan `crates/kora-hdc/` crate after confirming no local path
  references remain.
- In `crates/hdc/core/src/lib.rs`, export `search::hamming_distance` as the
  canonical root `kora_hdc::hamming_distance`, or rename the scalar version so
  callers cannot accidentally bypass SIMD.
- In `crates/hdc/core/src/knowledge/store.rs`, replace
  `vectors: Vec<([u8; 32], HdcVector)>` and `index_search()` with
  `search::LocalIndex`.
- In `crates/hdc/core/src/knowledge/store.rs::open()`, implement durable load
  and save semantics or remove the persistence-shaped API until it is real.
- In `crates/hdc/core/src/cognitive/dream.rs`, add an executable dream-cycle
  function that consumes `DreamConfig`, calls `prioritize_replay()` from
  `replay.rs`, updates `KnowledgeStore`, and returns `DreamReport`.

### Prioritized Checklist

**P0: Must fix before HDC contracts are usable**

- [ ] Align `contracts/src/HdcPrecompile.sol` with
  `crates/hdc/chain/src/precompile.rs`, or add the stateful opcodes Solidity
  currently expects.
- [ ] Stop bypassing the production precompile path in `InsightBoard.t.sol`;
  replace `TestableInsightBoard` with mocked precompile responses or integration
  tests.
- [ ] Align `OnChainHdcIndex` IDs and states with `IInsightBoard.sol`.
- [ ] Implement finalized event replay into `OnChainHdcIndex`.
- [ ] Harden precompile input validation and remove the production unwrap in
  `exec_permute()`.

**P1: Required for reliable node operation**

- [ ] Replace all zero event topics with actual topic hashes.
- [ ] Decode all current HDC contract events, not stale accepted/rejected events.
- [ ] Fix gas pricing for vector operations.
- [ ] Delete `crates/kora-hdc/`.
- [ ] Make SIMD Hamming distance the canonical public API.

**P2: Required for feature completeness**

- [ ] Implement pheromone read indexing and `record_pheromone()`.
- [ ] Add typed error returns to chain index and WisdomGate operations.
- [ ] Implement `KnowledgeStore::open()` persistence and serde derives for core
  persisted types.
- [ ] Wire `KnowledgeStore` to `LocalIndex`.
- [ ] Implement dream-cycle execution and FSRS scheduling if they remain in
  scope for the current milestone.

### Tests / Verification

- Add Rust precompile ABI tests covering each opcode against Solidity-compatible
  byte payloads and return decoding.
- Add malformed-input tests for short, long, empty, zero-count, and gas-limited
  precompile inputs.
- Add Foundry tests that call the real `InsightBoard.submit()` and `purge()`
  paths with mocked precompile behavior. The duplicate-check and vector-delete
  branches must be covered.
- Add event replay unit tests in `crates/hdc/chain/src/event.rs` using logs that
  match the current Solidity event signatures.
- Add `OnChainHdcIndex` tests proving that an `InsightPublished` plus
  `InsightStateChanged(..., ACTIVE)` log makes `search()` return the expected
  `insightId`.
- Add a runner/reporters integration test that finalizes a block with an HDC
  event and verifies the shared RPC index changes without a second replay.
- Run `cargo nextest run --workspace --all-features`, `cargo clippy
  --all-targets --all-features -- -D warnings`, and `forge test` once code
  changes are made.

### Genuinely Unresolved Questions

- Is `0x09` non-negotiable for this chain, despite replacing the standard
  BLAKE2F precompile, or should the testnet move to a non-standard address such
  as `0xA0C` before external users integrate?
- Should duplicate detection happen synchronously in Solidity, or should it be
  an RPC/off-chain preflight backed by finalized event replay?
- Should the shared HDC search API return contract `insightId`, vector content
  hash, or both? Current Rust and Solidity disagree.
- Should `DECAYING` insights remain searchable, or should search be restricted
  to `ACTIVE` only?
- Are pheromone search and local `KnowledgeStore` persistence part of the
  Railway testnet acceptance criteria, or can the first launch expose only raw
  vector precompile operations and InsightBoard lifecycle events?

---

---

## New Findings (F07-F15)

> **Added 2026-05-08.** These findings were discovered during deep codebase
> analysis, PR #42 review, and reconciliation with the implementation plans.

---

### F07: Two Trust Implementations Competing

**Severity: HIGH**

Two separate trust-tracking implementations exist with overlapping functionality:

| File | Type | Storage | EMA Alpha |
|------|------|---------|-----------|
| `crates/hdc/core/src/trust.rs` | `TrustRegistry` | `HashMap<H256, AgentTrust>` | 0.1 |
| `crates/hdc/core/src/knowledge/trust.rs` | `TrustTracker` | Per-entry trust scores | 0.1 |

Both implement EMA-based trust scoring, penalty/reward methods, and agent lookup.
If both are wired into the same node, the same agent can have divergent trust
scores depending on which module queries trust.

**Risk:** Knowledge scoring uses one trust value while replay prioritization uses
another. Inconsistent trust scores produce inconsistent knowledge ranking.

**Recommendation:** Merge into a single canonical `TrustRegistry` in
`crates/hdc/core/src/trust.rs`. The `knowledge/trust.rs` module should be deleted
and its callers updated.

---

### F08: EMA Double-Application Bug

**Severity: MEDIUM**

The `TrustRegistry::record_outcome()` method applies an EMA update internally.
Some call sites in `knowledge/store.rs` also call `TrustRegistry::update()`
after `record_outcome()`, effectively applying the EMA twice per event. This
causes trust scores to converge to steady state ~2x faster than intended.

**Evidence:** In `knowledge/store.rs`, the `tick()` method iterates entries and
for each entry with a trust-affecting outcome, calls both `record_outcome()`
(which internally updates the EMA) and then a separate trust adjustment method.

**Impact:** Agents are over-penalized or over-rewarded. A single bad prediction
can effectively blacklist an agent within a few ticks instead of the intended
gradual decay.

**Recommendation:** Audit all call paths to ensure exactly one EMA update per
trust-affecting event. Add regression test comparing observed trust trajectory
against expected single-EMA curve.

---

### F09: JsonRpcServer Missing HDC Wiring

**Severity: MEDIUM**

The `JsonRpcServer` in `crates/node/rpc/src/lib.rs` constructs the RPC module
set, but the HDC RPC methods are only conditionally registered based on whether
the `hdc_index` is `Some`. The problem is that `ProductionRunner::run()` creates
the HDC index and passes it to the RPC server, but the `FinalizedReporter` does
not receive it. This means the HDC RPC methods work for queries but never
receive updates from finalized blocks, so search results go stale immediately
after node restart.

**Evidence:** `runner.rs` creates `Arc<RwLock<OnChainHdcIndex>>` and passes it
to the RPC server (lines ~312-318) but does not pass it to `FinalizedReporter`
(no HDC sink in the reporter construction).

**Impact:** HDC search results via RPC are always based on the initial empty
index. No finalized event ever populates it.

**Recommendation:** Wire the shared `Arc<RwLock<OnChainHdcIndex>>` into
`FinalizedReporter` as an event sink, following the pattern described in
`15-wiring-guide.md` section 3.5.

---

### F10: E2E Tests Don't Verify Precompile Output

**Severity: MEDIUM**

The e2e tests in `crates/e2e/src/tests/hdc.rs` call the raw Rust precompile
directly (bypassing Solidity) and verify that the precompile returns success, but
they do not decode or verify the return value content. For example, a hamming
distance test might verify `ok == true` and `ret.len() == 32` but not verify
that the decoded distance is the correct value.

**Evidence:** E2e test assertions check return length and success flag but do
not decode the ABI-encoded return value and compare against expected results.

**Impact:** The precompile could return arbitrary 32-byte values and the test
would pass. Correctness of the actual HDC computation is not verified at the
integration level.

**Recommendation:** Add decoded value assertions to all e2e precompile tests.
Compute expected values using the `kora-hdc` core library and compare.

---

### F11: TestApplication Doesn't Register HDC Precompile

**Severity: MEDIUM**

The `TestApplication` struct used in integration and e2e tests does not register
the HDC precompile on its REVM executor. Tests that exercise HDC operations must
manually construct a precompile-aware executor, which means test coverage for the
production wiring path (where `ProductionRunner` calls
`.with_hdc_precompile()`) is absent.

**Evidence:** The test application factory in `crates/e2e/` constructs a bare
`RevmExecutor` without calling `.with_hdc_precompile()`. HDC e2e tests work
around this by calling the precompile function directly.

**Impact:** The exact layer where production wiring happens (runner -> executor ->
precompile registration) is never tested end-to-end.

**Recommendation:** Add an `hdc_enabled` flag to `TestApplication` and register
the precompile when enabled. Add at least one test that exercises the full path:
submit transaction -> REVM execution -> precompile dispatch -> return value.

---

### F12: Solidity/Rust Opcode Mismatch

**Severity: CRITICAL**

`HdcLib.sol` opcode assignments do not match `precompile.rs` opcode assignments.
See the opcode comparison table in doc 19 section 9.1. Every `HdcLib` function
call triggers the wrong Rust operation.

**Evidence:** `HdcLib.hamming()` sends opcode `0x06`, Rust interprets `0x06` as
`is_similar`. `HdcLib.bind()` sends `0x05`, Rust interprets as `vector_id`.

**Impact:** `InsightBoard.submit()` is non-functional when connected to the real
Rust precompile. The Foundry test suite hides this because
`TestableInsightBoard` bypasses all precompile calls.

**Recommendation:** Adopt PR #42's 4-byte selector dispatch. Update both
`HdcLib.sol` and `precompile.rs` to use the same selector table. See doc 18
section 10.2 for the canonical selector table.

---

### F13: WisdomGate challenge/resolve Stubbed

**Severity: LOW**

`WisdomGate::challenge()` and `WisdomGate::resolve()` in
`crates/hdc/chain/src/wisdom.rs` exist but have minimal logic:

- `challenge()` sets a `challenged` flag and returns `bool`. No actual challenge
  verification (e.g., anti-knowledge resonance check) is performed.
- `resolve()` iterates submissions and auto-resolves any that have passed the
  challenge window. No voting or multi-party resolution is implemented.

**Evidence:** `wisdom.rs` lines 79-88 (`challenge`), lines 92-110 (`resolve`).

**Impact:** The WisdomGate cannot actually protect against incorrect knowledge
submissions. Any agent can challenge any submission, and challenges are
auto-resolved by timeout, not by evidence evaluation.

**Recommendation:** Either implement proper challenge resolution (anti-knowledge
similarity check, multi-agent voting) or document WisdomGate as a placeholder
module that will be replaced by InsightBoard's on-chain challenge mechanism.

---

### F14: State Machine Transitions Stubbed

**Severity: LOW**

The behavioral state machine in `crates/hdc/core/src/cognitive/state_machine.rs`
defines states (Exploring, Exploiting, Consolidating, Resting) and transition
logic, but several transitions are stubbed:

- `should_transition()` returns `false` for some state pairs, meaning those
  transitions can never fire.
- `execute_transition()` clones the current state without actually performing
  the transition actions.

**Evidence:** `state_machine.rs` lines 153, 264 -- `self.current.clone()` on
no-transition path.

**Impact:** The cognitive state machine cannot drive agent behavior changes. An
agent stuck in Exploring never transitions to Exploiting even when it should.

**Recommendation:** Low priority unless the cognitive module is in scope for the
current milestone. Document as deferred and add TODO markers.

---

### F15: Dream Cycle NREM/REM Logic Missing

**Severity: LOW**

The dream cycle module at `crates/hdc/core/src/cognitive/dream.rs` defines
`DreamConfig`, `DreamPhase` (NREM, REM), and `DreamReport` types, but the
actual dream execution logic is not implemented:

- No function consumes `DreamConfig` and produces `DreamReport`.
- NREM phase (memory consolidation) has no implementation.
- REM phase (creative recombination) has no implementation.
- The `replay.rs` module provides `prioritize_replay()` which could serve as
  the NREM backbone, but it is never called from `dream.rs`.

**Evidence:** `dream.rs` defines types only, no executable logic. No function
in the module takes a `KnowledgeStore` reference or produces side effects.

**Impact:** Dream-cycle-driven memory consolidation is entirely absent. Knowledge
entries are never replayed, consolidated, or pruned by the dream system.

**Recommendation:** Low priority unless NREM/REM consolidation is in scope for
the current milestone. If deferred, add a note to the overview doc marking it as
not-yet-implemented.

---

## Findings Summary Table

| Finding | Severity | Status | Resolution |
|---------|----------|--------|------------|
| F01: Orphaned crate | HIGH | OPEN | Still exists; delete `crates/kora-hdc/` |
| F02: Stubbed events | HIGH | PARTIALLY RESOLVED | PR #42 has event decoders; `hdc` branch still stubbed |
| F03: Address conflict | MEDIUM | RESOLVED | `0xA0C` adopted per PR #42 |
| F04: Gas mismatch | MEDIUM | RESOLVED | 50,000 flat gas adopted per PR #42 |
| F07: Two trust impls | HIGH | OPEN | Merge into single `TrustRegistry` |
| F08: EMA double-apply | MEDIUM | OPEN | Audit call sites, add regression test |
| F09: RPC missing HDC wiring | MEDIUM | OPEN | Wire index into FinalizedReporter |
| F10: E2E no output verification | MEDIUM | OPEN | Add decoded value assertions |
| F11: TestApp no precompile | MEDIUM | OPEN | Add hdc_enabled flag to TestApplication |
| F12: Solidity/Rust opcode mismatch | CRITICAL | OPEN | Adopt PR #42 selectors |
| F13: WisdomGate stubbed | LOW | OPEN | Implement or document as placeholder |
| F14: State machine stubbed | LOW | OPEN | Deferred unless in scope |
| F15: Dream cycle missing | LOW | OPEN | Deferred unless in scope |

---

*End of cross-cutting audit.*
