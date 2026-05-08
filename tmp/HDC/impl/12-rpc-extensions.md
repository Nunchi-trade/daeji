# 12 -- HDC RPC Extensions

> **Status: PARTIALLY IMPLEMENTED** -- `HdcApi` trait with method signatures
> and `spawn_blocking` pattern exists. 4 of 7 spec methods are missing, 4
> extra algebra methods were added. Only wired to `RpcServer`, not
> `JsonRpcServer`. See [Audit Findings](#audit-findings) and
> [Recommended Changes Checklist](#recommended-changes-checklist) below.

Add an `hdc_*` JSON-RPC namespace to the kora node exposing hyperdimensional
computing operations: vector similarity, knowledge search, insight queries,
encoding, pheromone listing, and trust scoring.

### Source files

| Component | File |
|-----------|------|
| RPC trait + impl | `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs` |
| Chain-level HDC API | `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs` |
| Server wiring (RpcServer) | `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` |
| Error variants | `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/error.rs` |
| Module registration | `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/lib.rs` |
| Event decoders | `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/event.rs` |
| On-chain index | `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs` |

### What exists (DONE)

- [x] `HdcRpcApi` trait with 7 method signatures (different from spec -- algebra-oriented)
- [x] `HdcApiImpl` wrapping concrete `kora_hdc_chain::rpc::HdcApi`
- [x] `spawn_blocking` pattern on all RPC methods
- [x] 4 HDC error variants in `error.rs` with correct JSON-RPC codes
- [x] `hdc_api: Option<HdcApiImpl>` field + `with_hdc_api()` builder on `RpcServer`
- [x] Conditional merge in `RpcServer::start()`
- [x] 10 unit tests covering existing methods
- [x] `MAX_SEARCH_K` server-side cap on search results

### What is missing or broken

- **R1: Only wired to `RpcServer`, not `JsonRpcServer`** -- `JsonRpcServer` at `server.rs:344-437` has no `hdc_api` field, no builder, no merge. HDC methods silently unavailable via `JsonRpcServer` path.
- **R3: Vector validation inside `spawn_blocking`** -- Validation happens in `kora_hdc_chain::rpc::HdcApi::parse_vector()` inside the blocking closure, wasting a pool thread on invalid requests. Should validate before spawning.
- **R5: `hdc_getPheromones` returns empty** -- Backing `record_pheromone()` in `index.rs` is a TODO stub. The method cannot return real data.

---

## 0. Orientation

| What | Where |
|------|-------|
| RPC crate root | `crates/node/rpc/` |
| Crate name (Cargo) | `kora-rpc` |
| Server setup | `crates/node/rpc/src/server.rs` |
| Module index | `crates/node/rpc/src/lib.rs` |
| Existing custom namespace | `crates/node/rpc/src/kora.rs` (`kora_*`, 1 method) |
| Eth namespace | `crates/node/rpc/src/eth.rs` (`eth_*`, 25+ methods) |
| Error types | `crates/node/rpc/src/error.rs` |
| Shared RPC types | `crates/node/rpc/src/types.rs` |

The RPC server uses **jsonrpsee 0.24** with the `server` and `macros` features.
Types are from `alloy-primitives` (with serde). The pattern for adding a new
namespace is demonstrated by `kora.rs`: define a trait with `#[rpc(server,
namespace = "...")]`, implement it on a struct, then merge the generated
`into_rpc()` module into the `jsonrpsee::RpcModule` inside `server.rs`.

---

## 1. Return-type definitions

Create `crates/node/rpc/src/hdc_types.rs`. Every type returned from an HDC
method lives here.

```rust
//! Types returned by the `hdc_*` JSON-RPC namespace.

use alloy_primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

/// Single search result from the knowledge store.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Storage key of the matching entry.
    pub key: Bytes,
    /// Hamming distance from the query vector.
    pub distance: u32,
    /// Entry kind (e.g. "text", "code", "image-embedding").
    pub kind: String,
    /// Quality tier (0 = unverified, 1 = confirmed, 2 = canonical).
    pub tier: u8,
    /// Truncated content preview (first 256 bytes).
    pub content_preview: Bytes,
}

/// On-chain insight metadata from the InsightBoard contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsightInfo {
    /// Keccak-256 hash of the HDC vector.
    pub vector_hash: U256,
    /// Keccak-256 hash of the content payload.
    pub content_hash: U256,
    /// Address that submitted the insight.
    pub author: Address,
    /// Lifecycle state: 0=Pending, 1=Confirmed, 2=Disputed, 3=Rejected.
    pub state: u8,
    /// Quality tier.
    pub tier: u8,
    /// Number of validator confirmations.
    pub confirmations: u32,
}

/// Aggregate statistics for the local knowledge store.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeStats {
    /// Total number of stored vectors.
    pub total_vectors: u64,
    /// Entry counts keyed by kind (e.g. {"text": 120, "code": 45}).
    pub by_kind: std::collections::HashMap<String, u64>,
    /// Entry counts keyed by tier (e.g. {"0": 80, "1": 60, "2": 25}).
    pub by_tier: std::collections::HashMap<String, u64>,
}

/// A single active pheromone.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PheromoneInfo {
    /// On-chain pheromone ID.
    pub id: U256,
    /// Hash of the associated HDC vector.
    pub vector_hash: U256,
    /// Current signal intensity (decays over time).
    pub intensity: u64,
    /// Address of the agent that deposited this pheromone.
    pub depositor: Address,
    /// Age in seconds since deposit.
    pub age: u64,
}

/// Trust breakdown for an insight.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustInfo {
    /// Composite trust score (0-10000, basis points).
    pub overall: u32,
    /// Per-stage scores: confirmation, diversity, longevity, etc.
    pub stages: Vec<StageScore>,
    /// Taint level from disputed neighbours (0 = clean, higher = worse).
    pub taint: u32,
}

/// Score contribution from a single trust-evaluation stage.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageScore {
    /// Stage name (e.g. "confirmation", "diversity", "longevity").
    pub name: String,
    /// Weight of this stage in the composite (basis points, sums to 10000).
    pub weight: u32,
    /// Raw score for this stage (0-10000).
    pub score: u32,
}
```

All fields use `camelCase` serialization to match the existing `types.rs`
convention. Use `alloy_primitives` types (`U256`, `Address`, `Bytes`) -- they
already have serde support via the workspace feature flag in `Cargo.toml`.

---

## 2. HDC error variants

Open `crates/node/rpc/src/error.rs` and add variants for HDC-specific failures.

Add these variants inside the existing `RpcError` enum:

```rust
    /// Vector has wrong length (expected 1280 bytes).
    #[error("invalid vector length: expected 1280, got {0}")]
    InvalidVectorLength(usize),

    /// Insight not found on-chain.
    #[error("insight not found: {0}")]
    InsightNotFound(String),

    /// Knowledge store is unavailable.
    #[error("knowledge store unavailable: {0}")]
    KnowledgeStoreUnavailable(String),

    /// Unknown encoding method.
    #[error("unknown encoding method: {0}")]
    UnknownEncodingMethod(String),
```

Then extend the `From<RpcError> for ErrorObjectOwned` impl:

```rust
RpcError::InvalidVectorLength(_) => (codes::INVALID_PARAMS, err.to_string()),
RpcError::InsightNotFound(_) => (codes::RESOURCE_NOT_FOUND, err.to_string()),
RpcError::KnowledgeStoreUnavailable(_) => (codes::RESOURCE_UNAVAILABLE, err.to_string()),
RpcError::UnknownEncodingMethod(_) => (codes::INVALID_PARAMS, err.to_string()),
```

The error codes follow the same conventions already used: `INVALID_PARAMS`
(-32602) for bad input, `RESOURCE_NOT_FOUND` (-32001) for missing data,
`RESOURCE_UNAVAILABLE` (-32002) for backend unavailability.

---

## 3. Trait definition

Create `crates/node/rpc/src/hdc.rs`.

```rust
//! HDC (Hyperdimensional Computing) JSON-RPC API implementation.

use std::sync::Arc;

use alloy_primitives::{Bytes, U256};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};

use crate::error::RpcError;
use crate::hdc_types::{
    InsightInfo, KnowledgeStats, PheromoneInfo, SearchResult, TrustInfo,
};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// HDC JSON-RPC API trait.
///
/// Exposes hyperdimensional computing operations: vector similarity,
/// knowledge search, insight queries, encoding, pheromone listing, and
/// trust scoring.
#[rpc(server, namespace = "hdc")]
pub trait HdcApi {
    /// Compute the Hamming distance between two 1280-byte HDC vectors.
    #[method(name = "similarity")]
    async fn similarity(
        &self,
        vector_a: Bytes,
        vector_b: Bytes,
    ) -> RpcResult<u32>;

    /// Search the knowledge store for the `k` nearest vectors.
    ///
    /// `source` must be `"local"`, `"shared"`, or `"both"`.
    #[method(name = "search")]
    async fn search(
        &self,
        query: Bytes,
        k: u32,
        source: String,
    ) -> RpcResult<Vec<SearchResult>>;

    /// Return on-chain insight metadata for the given insight ID.
    #[method(name = "getInsight")]
    async fn get_insight(
        &self,
        insight_id: U256,
    ) -> RpcResult<InsightInfo>;

    /// Return aggregate statistics for the local knowledge store.
    #[method(name = "getKnowledgeStats")]
    async fn get_knowledge_stats(&self) -> RpcResult<KnowledgeStats>;

    /// Encode text into a 1280-byte HDC vector.
    ///
    /// `method` must be `"trigram"` or `"projection"`.
    #[method(name = "encode")]
    async fn encode(
        &self,
        text: String,
        method: String,
    ) -> RpcResult<Bytes>;

    /// List active pheromones of the given type.
    #[method(name = "getPheromones")]
    async fn get_pheromones(
        &self,
        pheromone_type: u8,
        limit: u32,
    ) -> RpcResult<Vec<PheromoneInfo>>;

    /// Compute the trust score for an on-chain insight.
    #[method(name = "trustScore")]
    async fn trust_score(
        &self,
        insight_id: U256,
    ) -> RpcResult<TrustInfo>;
}
```

The `#[rpc(server, namespace = "hdc")]` macro generates a companion trait
`HdcApiServer` with an `into_rpc()` method. You implement `HdcApiServer`
(not `HdcApi`). This mirrors how `KoraApiServer` is generated from
`KoraApi` in `kora.rs`.

---

## 4. Implementation struct

Still inside `crates/node/rpc/src/hdc.rs`, below the trait:

```rust
// ---------------------------------------------------------------------------
// Backing-service trait objects
// ---------------------------------------------------------------------------

/// Read-only handle to the local + shared knowledge vector store.
///
/// You must implement this trait on whatever struct wraps your actual
/// storage backend (e.g. an HNSW index backed by LMDB).
pub trait KnowledgeStore: Send + Sync + 'static {
    fn search(
        &self,
        query: &[u8],
        k: u32,
        source: &str,
    ) -> Result<Vec<SearchResult>, String>;

    fn stats(&self) -> Result<KnowledgeStats, String>;
}

/// Read-only handle to on-chain HDC state (InsightBoard, PheromoneTrail).
///
/// Backed by contract calls through the state provider or a dedicated
/// indexer.
pub trait OnChainHdcIndex: Send + Sync + 'static {
    fn get_insight(&self, id: U256) -> Result<Option<InsightInfo>, String>;
    fn get_pheromones(
        &self,
        pheromone_type: u8,
        limit: u32,
    ) -> Result<Vec<PheromoneInfo>, String>;
    fn trust_score(&self, id: U256) -> Result<TrustInfo, String>;
}

/// Encoder from text to HDC vector.
pub trait HdcEncoder: Send + Sync + 'static {
    fn encode(&self, text: &str, method: &str) -> Result<Vec<u8>, String>;
}

// ---------------------------------------------------------------------------
// Impl
// ---------------------------------------------------------------------------

/// Concrete implementation of the HDC RPC API.
pub struct HdcApiImpl {
    knowledge_store: Arc<dyn KnowledgeStore>,
    on_chain: Arc<dyn OnChainHdcIndex>,
    encoder: Arc<dyn HdcEncoder>,
}

impl std::fmt::Debug for HdcApiImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HdcApiImpl").finish_non_exhaustive()
    }
}

impl HdcApiImpl {
    /// Create a new HDC API implementation.
    pub fn new(
        knowledge_store: Arc<dyn KnowledgeStore>,
        on_chain: Arc<dyn OnChainHdcIndex>,
        encoder: Arc<dyn HdcEncoder>,
    ) -> Self {
        Self {
            knowledge_store,
            on_chain,
            encoder,
        }
    }
}

/// Expected byte-length of an HDC vector.
const HDC_VECTOR_LEN: usize = 1280;

/// Validate that `v` is exactly 1280 bytes.
fn validate_vector(v: &[u8]) -> Result<(), RpcError> {
    if v.len() != HDC_VECTOR_LEN {
        return Err(RpcError::InvalidVectorLength(v.len()));
    }
    Ok(())
}

#[jsonrpsee::core::async_trait]
impl HdcApiServer for HdcApiImpl {
    async fn similarity(
        &self,
        vector_a: Bytes,
        vector_b: Bytes,
    ) -> RpcResult<u32> {
        validate_vector(&vector_a)?;
        validate_vector(&vector_b)?;

        // Hamming distance is pure computation on two 1280-byte slices.
        // Use spawn_blocking to keep the RPC reactor free.
        let a = vector_a.to_vec();
        let b = vector_b.to_vec();
        let dist = tokio::task::spawn_blocking(move || {
            a.iter()
                .zip(b.iter())
                .map(|(x, y)| (x ^ y).count_ones())
                .sum::<u32>()
        })
        .await
        .map_err(|e| {
            RpcError::Internal(format!("spawn_blocking failed: {e}"))
        })?;

        Ok(dist)
    }

    async fn search(
        &self,
        query: Bytes,
        k: u32,
        source: String,
    ) -> RpcResult<Vec<SearchResult>> {
        validate_vector(&query)?;

        match source.as_str() {
            "local" | "shared" | "both" => {}
            other => {
                return Err(RpcError::InvalidTransaction(format!(
                    "invalid source: {other}, expected local|shared|both"
                ))
                .into())
            }
        }

        let store = Arc::clone(&self.knowledge_store);
        let q = query.to_vec();
        let results = tokio::task::spawn_blocking(move || {
            store.search(&q, k, &source)
        })
        .await
        .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
        .map_err(|e| RpcError::KnowledgeStoreUnavailable(e))?;

        Ok(results)
    }

    async fn get_insight(
        &self,
        insight_id: U256,
    ) -> RpcResult<InsightInfo> {
        let chain = Arc::clone(&self.on_chain);
        let info = tokio::task::spawn_blocking(move || {
            chain.get_insight(insight_id)
        })
        .await
        .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
        .map_err(|e| RpcError::Internal(e))?;

        info.ok_or_else(|| {
            RpcError::InsightNotFound(format!("{insight_id}")).into()
        })
    }

    async fn get_knowledge_stats(&self) -> RpcResult<KnowledgeStats> {
        let store = Arc::clone(&self.knowledge_store);
        let stats = tokio::task::spawn_blocking(move || store.stats())
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(|e| RpcError::KnowledgeStoreUnavailable(e))?;

        Ok(stats)
    }

    async fn encode(
        &self,
        text: String,
        method: String,
    ) -> RpcResult<Bytes> {
        match method.as_str() {
            "trigram" | "projection" => {}
            other => {
                return Err(
                    RpcError::UnknownEncodingMethod(other.to_string()).into()
                )
            }
        }

        let enc = Arc::clone(&self.encoder);
        let vec = tokio::task::spawn_blocking(move || {
            enc.encode(&text, &method)
        })
        .await
        .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
        .map_err(|e| RpcError::Internal(e))?;

        Ok(Bytes::from(vec))
    }

    async fn get_pheromones(
        &self,
        pheromone_type: u8,
        limit: u32,
    ) -> RpcResult<Vec<PheromoneInfo>> {
        let chain = Arc::clone(&self.on_chain);
        let pheromones = tokio::task::spawn_blocking(move || {
            chain.get_pheromones(pheromone_type, limit)
        })
        .await
        .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
        .map_err(|e| RpcError::Internal(e))?;

        Ok(pheromones)
    }

    async fn trust_score(
        &self,
        insight_id: U256,
    ) -> RpcResult<TrustInfo> {
        let chain = Arc::clone(&self.on_chain);
        let info = tokio::task::spawn_blocking(move || {
            chain.trust_score(insight_id)
        })
        .await
        .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
        .map_err(|e| RpcError::Internal(e))?;

        Ok(info)
    }
}
```

Key points about the implementation:

- **Every method uses `spawn_blocking`.** All seven methods offload work to the
  blocking threadpool. Even Hamming distance (which is fast) gets this treatment
  so the pattern is uniform and you never accidentally block the jsonrpsee
  reactor if vector sizes or computation change later.
- **Validation happens before the blocking call.** Input checks (`validate_vector`,
  `source` enum match, `method` enum match) run on the async task so bad
  requests fail fast without consuming a blocking thread.
- **Backing services are behind trait objects.** `KnowledgeStore`,
  `OnChainHdcIndex`, and `HdcEncoder` are defined as traits so the RPC layer
  does not depend on concrete storage implementations. Pass `Arc<dyn T>`
  at construction time.

---

## 5. Wire into `lib.rs`

Open `crates/node/rpc/src/lib.rs`. Add the new modules and re-exports after the
existing `kora` block:

```rust
mod hdc;
pub use hdc::{
    HdcApiImpl, HdcApiServer, HdcEncoder, KnowledgeStore, OnChainHdcIndex,
};

mod hdc_types;
pub use hdc_types::{
    InsightInfo, KnowledgeStats, PheromoneInfo, SearchResult, StageScore,
    TrustInfo,
};
```

Place these after line 21 (the `kora` re-export block):

```
mod kora;
pub use kora::{KoraApiImpl, KoraApiServer};

mod hdc;                       // <-- new
pub use hdc::{...};            // <-- new

mod hdc_types;                 // <-- new
pub use hdc_types::{...};      // <-- new
```

---

## 6. Wire into `server.rs`

### 6a. Add imports

At the top of `server.rs`, extend the `use crate::{...}` block:

```rust
use crate::{
    config::{CorsConfig, RpcServerConfig},
    eth::{
        EthApiImpl, EthApiServer, NetApiImpl, NetApiServer, TxSubmitCallback,
        Web3ApiImpl, Web3ApiServer,
    },
    hdc::{HdcApiImpl, HdcApiServer},          // <-- new
    kora::{KoraApiImpl, KoraApiServer},
    state::NodeState,
    state_provider::{NoopStateProvider, StateProvider},
};
```

### 6b. Extend `RpcServer` struct

Add the HDC dependencies to `RpcServer`:

```rust
pub struct RpcServer<S: StateProvider = NoopStateProvider> {
    state: NodeState,
    http_addr: SocketAddr,
    jsonrpc_addr: SocketAddr,
    chain_id: u64,
    tx_submit: Option<TxSubmitCallback>,
    state_provider: S,
    cors_config: CorsConfig,
    max_connections: u32,
    peer_count: u64,
    hdc_api: Option<HdcApiImpl>,              // <-- new
}
```

Add a builder method:

```rust
    /// Set the HDC API implementation.
    #[must_use]
    pub fn with_hdc_api(mut self, hdc: HdcApiImpl) -> Self {
        self.hdc_api = Some(hdc);
        self
    }
```

Initialize `hdc_api: None` in every constructor (`new`, `with_chain_id`,
`with_state_provider`, `from_config`).

### 6c. Merge into module

Inside the `start()` method of `RpcServer`, after the `kora_api` merge block,
add:

```rust
            // ---- HDC namespace (optional) ----
            if let Some(hdc_api) = self.hdc_api {
                if let Err(e) = module.merge(hdc_api.into_rpc()) {
                    error!(error = %e, "Failed to merge hdc API");
                    return None;
                }
            }
```

The HDC API is gated behind `Option` so that nodes without a knowledge store
can still start (the RPC server simply omits the `hdc_*` methods). This
matches how you might conditionally enable other namespaces.

---

## 7. Anti-patterns

Things to avoid when implementing these methods:

1. **Blocking the RPC reactor.** Never call `knowledge_store.search()` or
   `encoder.encode()` directly inside an `async fn` body. The jsonrpsee server
   runs on a tokio runtime; blocking it stalls all concurrent requests. Always
   use `tokio::task::spawn_blocking` for anything that touches the knowledge
   store, does vector math, or makes contract calls. The code in section 4
   shows the correct pattern.

2. **Leaking internal state.** The `KnowledgeStore` and `OnChainHdcIndex` traits
   return high-level result types, not raw database handles or mutable
   references. Never expose `&mut` access to the store through the RPC layer.
   Clone `Arc`s, move them into the blocking closure, and return owned values.

3. **Unbounded result sets.** `hdc_search` accepts a `k` parameter and
   `hdc_getPheromones` accepts a `limit`. Enforce a server-side cap
   (e.g. `k.min(1000)`) to prevent a single request from dumping the entire
   store into a JSON response. Add a constant like `const MAX_SEARCH_K: u32 =
   1000;` at the top of the impl and clamp before querying.

4. **Panicking in blocking tasks.** If the closure passed to `spawn_blocking`
   panics, the `JoinHandle` returns `Err(JoinError)`. The code already converts
   this to `RpcError::Internal`. Do not `unwrap()` inside the closure; propagate
   `Result` instead.

5. **Forgetting serde rename.** Every public return struct must use
   `#[serde(rename_all = "camelCase")]` to match Ethereum JSON-RPC conventions.
   Omitting it produces `snake_case` field names that break client
   expectations.

6. **Validating after the blocking spawn.** Input validation (vector length,
   enum checks) must happen before `spawn_blocking`. Validating inside the
   closure wastes a thread from the blocking pool on a request that will fail
   anyway.

---

## 8. Cargo.toml changes

No new external dependencies are required. The crate already depends on:

- `jsonrpsee` 0.24 with `server` + `macros`
- `alloy-primitives` with `serde`
- `tokio` with `net` + `sync`
- `serde`

`spawn_blocking` is available through the existing `tokio` dependency (it
requires the `rt` feature, which is already enabled via workspace). The
`std::collections::HashMap` used in `KnowledgeStats` is from stdlib.

If your concrete `KnowledgeStore` or `OnChainHdcIndex` implementations live in
separate crates, add those as path dependencies in `[dependencies]`:

```toml
# HDC backing stores (example -- use your actual crate paths)
hdc-knowledge = { path = "../../hdc/knowledge" }
hdc-onchain   = { path = "../../hdc/onchain" }
```

---

## 9. Checklist

- [ ] Create `crates/node/rpc/src/hdc_types.rs` with all six return-type
      structs (`SearchResult`, `InsightInfo`, `KnowledgeStats`, `PheromoneInfo`,
      `TrustInfo`, `StageScore`).
- [ ] Add HDC error variants to `crates/node/rpc/src/error.rs` and extend
      the `From<RpcError> for ErrorObjectOwned` match arms.
- [ ] Create `crates/node/rpc/src/hdc.rs` with the `HdcApi` trait (7 methods),
      the three backing-service traits, `HdcApiImpl`, and the
      `HdcApiServer` impl.
- [ ] Register the new modules in `crates/node/rpc/src/lib.rs`.
- [ ] Wire `HdcApiImpl` into `RpcServer` and `JsonRpcServer` in
      `crates/node/rpc/src/server.rs` (struct field, builder, module merge).
- [ ] Enforce server-side caps on `k` and `limit` parameters.
- [ ] Verify `cargo check -p kora-rpc` compiles cleanly.
- [ ] Verify `cargo test -p kora-rpc` passes (existing tests must not break).
- [ ] Write new tests (see section 10).

---

## 10. Test plan

### Unit tests (in `crates/node/rpc/src/hdc.rs`)

Add a `#[cfg(test)] mod tests` block at the bottom of `hdc.rs`. Use mock
implementations of the three backing traits.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // -- Mock backing services -----------------------------------------------

    struct MockStore;

    impl KnowledgeStore for MockStore {
        fn search(
            &self,
            _query: &[u8],
            k: u32,
            _source: &str,
        ) -> Result<Vec<SearchResult>, String> {
            Ok((0..k)
                .map(|i| SearchResult {
                    key: Bytes::from(vec![i as u8]),
                    distance: i,
                    kind: "text".into(),
                    tier: 1,
                    content_preview: Bytes::from_static(b"hello"),
                })
                .collect())
        }

        fn stats(&self) -> Result<KnowledgeStats, String> {
            Ok(KnowledgeStats {
                total_vectors: 100,
                by_kind: [("text".into(), 100)].into_iter().collect(),
                by_tier: [("1".into(), 100)].into_iter().collect(),
            })
        }
    }

    struct MockChain;

    impl OnChainHdcIndex for MockChain {
        fn get_insight(&self, id: U256) -> Result<Option<InsightInfo>, String> {
            if id == U256::from(1) {
                Ok(Some(InsightInfo {
                    vector_hash: U256::from(0xdead),
                    content_hash: U256::from(0xbeef),
                    author: Address::ZERO,
                    state: 1,
                    tier: 1,
                    confirmations: 3,
                }))
            } else {
                Ok(None)
            }
        }

        fn get_pheromones(
            &self,
            _pheromone_type: u8,
            limit: u32,
        ) -> Result<Vec<PheromoneInfo>, String> {
            Ok((0..limit.min(5))
                .map(|i| PheromoneInfo {
                    id: U256::from(i),
                    vector_hash: U256::from(i),
                    intensity: 1000 - (i as u64 * 100),
                    depositor: Address::ZERO,
                    age: i as u64 * 60,
                })
                .collect())
        }

        fn trust_score(&self, _id: U256) -> Result<TrustInfo, String> {
            Ok(TrustInfo {
                overall: 8500,
                stages: vec![
                    StageScore {
                        name: "confirmation".into(),
                        weight: 5000,
                        score: 9000,
                    },
                    StageScore {
                        name: "diversity".into(),
                        weight: 3000,
                        score: 8000,
                    },
                    StageScore {
                        name: "longevity".into(),
                        weight: 2000,
                        score: 7500,
                    },
                ],
                taint: 0,
            })
        }
    }

    struct MockEncoder;

    impl HdcEncoder for MockEncoder {
        fn encode(&self, _text: &str, _method: &str) -> Result<Vec<u8>, String> {
            Ok(vec![0u8; HDC_VECTOR_LEN])
        }
    }

    fn make_api() -> HdcApiImpl {
        HdcApiImpl::new(
            Arc::new(MockStore),
            Arc::new(MockChain),
            Arc::new(MockEncoder),
        )
    }

    // -- Tests ---------------------------------------------------------------

    #[tokio::test]
    async fn similarity_identical_vectors() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; HDC_VECTOR_LEN]);
        let dist = api.similarity(v.clone(), v).await.unwrap();
        assert_eq!(dist, 0);
    }

    #[tokio::test]
    async fn similarity_opposite_vectors() {
        let api = make_api();
        let a = Bytes::from(vec![0x00; HDC_VECTOR_LEN]);
        let b = Bytes::from(vec![0xFF; HDC_VECTOR_LEN]);
        let dist = api.similarity(a, b).await.unwrap();
        // Each byte differs in 8 bits => 1280 * 8 = 10240
        assert_eq!(dist, HDC_VECTOR_LEN as u32 * 8);
    }

    #[tokio::test]
    async fn similarity_rejects_wrong_length() {
        let api = make_api();
        let short = Bytes::from(vec![0u8; 100]);
        let ok = Bytes::from(vec![0u8; HDC_VECTOR_LEN]);
        let err = api.similarity(short, ok).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn search_returns_k_results() {
        let api = make_api();
        let q = Bytes::from(vec![0u8; HDC_VECTOR_LEN]);
        let results = api.search(q, 5, "local".into()).await.unwrap();
        assert_eq!(results.len(), 5);
    }

    #[tokio::test]
    async fn search_rejects_invalid_source() {
        let api = make_api();
        let q = Bytes::from(vec![0u8; HDC_VECTOR_LEN]);
        let err = api.search(q, 5, "invalid".into()).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn get_insight_found() {
        let api = make_api();
        let info = api.get_insight(U256::from(1)).await.unwrap();
        assert_eq!(info.confirmations, 3);
    }

    #[tokio::test]
    async fn get_insight_not_found() {
        let api = make_api();
        let err = api.get_insight(U256::from(999)).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn get_knowledge_stats_returns_counts() {
        let api = make_api();
        let stats = api.get_knowledge_stats().await.unwrap();
        assert_eq!(stats.total_vectors, 100);
    }

    #[tokio::test]
    async fn encode_trigram() {
        let api = make_api();
        let vec = api
            .encode("hello world".into(), "trigram".into())
            .await
            .unwrap();
        assert_eq!(vec.len(), HDC_VECTOR_LEN);
    }

    #[tokio::test]
    async fn encode_rejects_unknown_method() {
        let api = make_api();
        let err = api
            .encode("hello".into(), "unknown".into())
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn get_pheromones_respects_limit() {
        let api = make_api();
        let pheromones = api.get_pheromones(0, 3).await.unwrap();
        assert_eq!(pheromones.len(), 3);
    }

    #[tokio::test]
    async fn trust_score_returns_stages() {
        let api = make_api();
        let info = api.trust_score(U256::from(1)).await.unwrap();
        assert_eq!(info.overall, 8500);
        assert_eq!(info.stages.len(), 3);
        assert_eq!(info.taint, 0);
    }

    #[test]
    fn hdc_types_serde_roundtrip_search_result() {
        let sr = SearchResult {
            key: Bytes::from(vec![1, 2, 3]),
            distance: 42,
            kind: "text".into(),
            tier: 1,
            content_preview: Bytes::from_static(b"preview"),
        };
        let json = serde_json::to_string(&sr).unwrap();
        assert!(json.contains("contentPreview")); // camelCase check
        let parsed: SearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.distance, 42);
    }

    #[test]
    fn hdc_types_serde_roundtrip_insight_info() {
        let info = InsightInfo {
            vector_hash: U256::from(1),
            content_hash: U256::from(2),
            author: Address::ZERO,
            state: 1,
            tier: 2,
            confirmations: 5,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("vectorHash"));
        let parsed: InsightInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.confirmations, 5);
    }

    #[test]
    fn hdc_types_serde_roundtrip_trust_info() {
        let info = TrustInfo {
            overall: 9000,
            stages: vec![StageScore {
                name: "test".into(),
                weight: 10000,
                score: 9000,
            }],
            taint: 100,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"taint\":100"));
        let parsed: TrustInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.overall, 9000);
    }
}
```

### Integration tests

After the unit tests pass, add a full JSON-RPC integration test in
`crates/node/rpc/tests/` (or extend an existing one):

1. Stand up a `JsonRpcServer` with the mock-backed `HdcApiImpl` merged in.
2. Connect a `jsonrpsee::http_client::HttpClient`.
3. Call each `hdc_*` method via `client.request(...)`.
4. Assert HTTP 200 + correct JSON shapes.
5. Call with bad params (wrong vector length, unknown source) and assert
   JSON-RPC error codes (-32602, -32001, etc.).

### Manual smoke test

```bash
# Start a local kora node with HDC enabled
cargo run -p kora-node -- --hdc-enabled

# Test similarity
curl -X POST http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc": "2.0",
    "method": "hdc_similarity",
    "params": ["0x<1280-byte-hex-a>", "0x<1280-byte-hex-b>"],
    "id": 1
  }'

# Test encode
curl -X POST http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc": "2.0",
    "method": "hdc_encode",
    "params": ["hello world", "trigram"],
    "id": 2
  }'

# Test getKnowledgeStats (no params)
curl -X POST http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{
    "jsonrpc": "2.0",
    "method": "hdc_getKnowledgeStats",
    "params": [],
    "id": 3
  }'
```

---

## 11. File summary

| File | Action |
|------|--------|
| `crates/node/rpc/src/hdc_types.rs` | **Create.** All return-type structs. |
| `crates/node/rpc/src/hdc.rs` | **Create.** Trait, backing-service traits, impl, tests. |
| `crates/node/rpc/src/error.rs` | **Edit.** Add 4 error variants + match arms. |
| `crates/node/rpc/src/lib.rs` | **Edit.** Add `mod hdc; mod hdc_types;` + re-exports. |
| `crates/node/rpc/src/server.rs` | **Edit.** Add `hdc_api` field, builder method, module merge. |
| `crates/node/rpc/Cargo.toml` | **Edit only if** concrete HDC crate deps are needed. |

---

## Audit Findings

Audit performed 2026-05-08 comparing spec (sections 1-11 above) against the
actual implementation in the `hdc` branch.

### A1. API surface divergence -- the implementation is a different API

The spec defines a 7-method `hdc_*` namespace oriented around a knowledge-store
/ insight / pheromone / trust model with trait-object backing services. The
implementation instead exposes a 7-method `hdc_*` namespace oriented around raw
HDC vector operations. They are not the same API.

| Spec method | Spec signature | Impl method | Match? |
|---|---|---|---|
| `hdc_similarity` | `(Bytes, Bytes) -> u32` (Hamming distance) | `hdc_similarity` | **Partial.** Impl returns `f64` (normalized similarity), not `u32`. Spec's Hamming-distance concept is split into a separate `hdc_hammingDistance`. |
| `hdc_search` | `(Bytes, u32, String) -> Vec<SearchResult>` | `hdc_search` | **Partial.** Impl has 2 params `(Bytes, u32)` -- missing `source` param. Return type is `HdcSearchResult {id, distance}` vs spec's `SearchResult {key, distance, kind, tier, content_preview}`. |
| `hdc_getInsight` | `(U256) -> InsightInfo` | -- | **Missing.** Not implemented. |
| `hdc_getKnowledgeStats` | `() -> KnowledgeStats` | -- | **Missing.** Not implemented. |
| `hdc_encode` | `(String, String) -> Bytes` | `hdc_encode` | **Partial.** Impl takes 1 param `(String)` -- missing `method` param. Always uses trigram. No projection support. |
| `hdc_getPheromones` | `(u8, u32) -> Vec<PheromoneInfo>` | -- | **Missing.** Not implemented. Pheromone tracking is a TODO stub (`index.rs:130`). |
| `hdc_trustScore` | `(U256) -> TrustInfo` | -- | **Missing.** Not implemented. |
| -- | -- | `hdc_hammingDistance` | **Extra.** Not in spec. `(Bytes, Bytes) -> u32`. |
| -- | -- | `hdc_bind` | **Extra.** Not in spec. `(Bytes, Bytes) -> Bytes` (XOR bind). |
| -- | -- | `hdc_bundle` | **Extra.** Not in spec. `(Vec<Bytes>) -> Bytes` (majority vote). |
| -- | -- | `hdc_vectorId` | **Extra.** Not in spec. `(Bytes) -> B256` (keccak hash). |

**Summary:** 4 of 7 spec methods are missing entirely. 3 spec methods have
partial implementations with different signatures. 4 extra methods exist that
the spec did not call for.

### A2. Architectural divergence -- no trait-object backing services

The spec prescribes three trait objects (`KnowledgeStore`, `OnChainHdcIndex`,
`HdcEncoder`) injected into `HdcApiImpl` via `Arc<dyn T>`, keeping the RPC
layer decoupled from storage. The implementation instead wraps a concrete
`kora_hdc_chain::rpc::HdcApi` struct directly:

- File: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs`, line 74-76
- `HdcApiImpl { inner: kora_hdc_chain::rpc::HdcApi }` -- hardcoded concrete type

The chain-level `HdcApi` (`crates/hdc/chain/src/rpc.rs`, line 15-17) holds
`Arc<parking_lot::RwLock<OnChainHdcIndex>>` directly. There is no trait
abstraction. This makes it impossible to swap backends, test with mocks at the
RPC level without bringing in the real `kora_hdc_chain` crate, or add a
`KnowledgeStore` with local/shared sources.

### A3. Missing `hdc_types.rs` file

The spec calls for `crates/node/rpc/src/hdc_types.rs` containing 6 return-type
structs: `SearchResult`, `InsightInfo`, `KnowledgeStats`, `PheromoneInfo`,
`TrustInfo`, `StageScore`. This file does not exist. The only return type
defined is `HdcSearchResult` (2 fields) inline in `hdc.rs` at lines 57-62,
which is a stripped-down version of the spec's `SearchResult` (5 fields).

### A4. `hdc_search` source parameter missing

Spec says `hdc_search` takes a `source: String` parameter (`"local"`,
`"shared"`, or `"both"`) to choose between knowledge store backends. The
implementation at `hdc.rs:39` has no `source` parameter. It only searches the
on-chain index, which has no concept of local vs shared stores.

### A5. `hdc_encode` method parameter missing

Spec says `hdc_encode` takes `method: String` (`"trigram"` or `"projection"`).
Implementation at `hdc.rs:47` takes only `text: String` and always uses
`TrigramEncoder` (see `crates/hdc/chain/src/rpc.rs:83-86`). No projection
encoding path exists.

### A6. Error variants added but partially unused

All 4 HDC error variants from the spec are present in `error.rs` (lines 74-88):
`InvalidVectorLength`, `InsightNotFound`, `KnowledgeStoreUnavailable`,
`UnknownEncodingMethod`. The `From<RpcError>` match arms are also present
(lines 103-108). However:

- `InsightNotFound` -- never raised (no `hdc_getInsight` method exists)
- `KnowledgeStoreUnavailable` -- never raised (no knowledge store exists)
- `UnknownEncodingMethod` -- never raised (no `method` parameter on `encode`)

Only `InvalidVectorLength` is actually used, via `map_hdc_err()` at `hdc.rs:92-96`.

### A7. `map_hdc_err` is a lossy error-mapping shim

File: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs`, lines 92-96.

```rust
fn map_hdc_err(e: kora_hdc_chain::rpc::HdcRpcError) -> RpcError {
    RpcError::InvalidVectorLength(match e {
        kora_hdc_chain::rpc::HdcRpcError::InvalidVectorLength(len) => len,
    })
}
```

This function destructures the single-variant `HdcRpcError` enum. If the
chain-level crate adds more error variants, this will silently force them all
into `InvalidVectorLength` at the match level, or fail to compile if the match
is exhaustive. This is fragile coupling between two crate boundaries.

### A8. Input validation happens inside the blocking closure

The spec (section 7, anti-pattern 6) explicitly says: "Input validation (vector
length, enum checks) must happen before `spawn_blocking`. Validating inside the
closure wastes a thread from the blocking pool."

The implementation does the opposite. Vector length validation is performed by
`parse_vector()` inside `kora_hdc_chain::rpc::HdcApi` methods (e.g.,
`hamming_distance_rpc` at `chain/src/rpc.rs:27`), which are called inside the
`spawn_blocking` closures at `hdc.rs:104`, `hdc.rs:114`, etc. Every invalid
request consumes a blocking thread before failing.

### A9. `JsonRpcServer` does not wire HDC API

File: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines
344-437. The `JsonRpcServer` struct (a standalone JSON-RPC server without HTTP
status endpoints) has no `hdc_api` field, no `with_hdc_api()` builder, and its
`start()` method does not merge the HDC namespace. Only `RpcServer` (line 83)
has the HDC wiring. This means the standalone `JsonRpcServer` path cannot serve
HDC methods.

### A10. Trait naming mismatch

Spec says the macro generates `HdcApiServer` from a trait named `HdcApi`.
Implementation names the trait `HdcRpcApi` (line 20), generating
`HdcRpcApiServer`. The re-export in `lib.rs` (line 24) exports
`HdcRpcApiServer` instead of `HdcApiServer`. This is cosmetic but diverges from
the spec and from the pattern used by `KoraApi` / `KoraApiServer`.

### A11. Pheromone tracking is a stub

File: `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs`, line 130-132.

```rust
pub fn record_pheromone(&mut self, _topic: B256, _region: B256, _strength: u64) {
    // TODO: implement pheromone tracking
}
```

The `hdc_getPheromones` RPC method cannot be implemented until the backing
storage actually tracks pheromones.

### A12. Test coverage gaps

The implementation has 10 tests in `hdc.rs` (lines 173-269) covering the
methods that exist. However:

- No tests for missing methods (getInsight, getKnowledgeStats, getPheromones,
  trustScore) -- because they don't exist.
- No test for the `source` validation path -- because the parameter doesn't
  exist.
- No test for the `method` validation path -- because the parameter doesn't
  exist.
- No serde roundtrip tests for `InsightInfo`, `KnowledgeStats`, `PheromoneInfo`,
  `TrustInfo`, `StageScore` -- because those types don't exist.
- The test mock at `hdc.rs:178-182` constructs a real `OnChainHdcIndex` and
  real `HdcApi`, so these are integration tests not unit tests. The spec's
  mock-backed approach was not followed.

---

## Implementation Status

| Spec item | Status | Notes |
|---|---|---|
| 1. Return-type definitions (`hdc_types.rs`) | **Not implemented** | File does not exist. Only `HdcSearchResult` (2 fields) exists inline. |
| 2. HDC error variants (`error.rs`) | **Implemented** | All 4 variants present with correct codes. 3 of 4 are unused dead code. |
| 3. Trait definition (`HdcApi` / `HdcRpcApi`) | **Partially implemented** | 7 methods defined but different from spec: 4 spec methods missing, 4 extra methods added. Trait name differs (`HdcRpcApi` vs `HdcApi`). |
| 4. Implementation struct + backing-service traits | **Partially implemented** | No backing-service traits. `HdcApiImpl` wraps concrete `kora_hdc_chain::rpc::HdcApi`. Validation inside blocking closure (anti-pattern). |
| 5. Wire into `lib.rs` | **Implemented** | Module registered, types re-exported. Exports `HdcRpcApiServer` not `HdcApiServer`. |
| 6a. Server imports | **Implemented** | `hdc::{HdcApiImpl, HdcRpcApiServer}` imported. |
| 6b. `RpcServer` struct field + builder | **Implemented** | `hdc_api: Option<HdcApiImpl>` field, `with_hdc_api()` builder, `None` in all constructors. |
| 6c. Module merge | **Implemented** | Conditional merge in `RpcServer::start()`. Not done in `JsonRpcServer::start()`. |
| 7. Anti-pattern avoidance | **3 of 6 violated** | `spawn_blocking` used (good). `Arc` cloning used (good). `MAX_SEARCH_K` cap present (good). But: validation inside closure (A8), no `source`/`method` enum checks (A4/A5), serde rename only on 1 type. |
| 8. Cargo.toml changes | **Implemented** | `kora-hdc-chain.workspace = true` added. Dev-dep `kora-hdc.workspace = true` added. |
| 9. Checklist items | **4 of 9 done** | See table above. |
| 10. Test plan | **Partially implemented** | 10 tests exist but use real backing types not mocks. No coverage for missing methods. |

---

## Anti-Patterns & Duct Tape

### D1. Concrete coupling instead of trait objects (architecture)

The RPC layer directly depends on `kora_hdc_chain::rpc::HdcApi`, a concrete
struct holding `Arc<parking_lot::RwLock<OnChainHdcIndex>>`. This violates the
spec's dependency-inversion design and makes the RPC crate untestable in
isolation. The `kora-hdc-chain` dependency in `Cargo.toml` (line 45) would not
be needed if trait objects were used as specified.

**File:** `crates/node/rpc/src/hdc.rs:74-76`
**File:** `crates/node/rpc/Cargo.toml:45`

### D2. `map_hdc_err` exhaustive-match on a single-variant enum

The adapter function `map_hdc_err` at `hdc.rs:92-96` pattern-matches on
`HdcRpcError`, which has exactly one variant. If the chain crate ever adds
error variants, this becomes either a compile error (good, if exhaustive) or a
silent bug (bad, if a wildcard is added). The function's existence is duct tape
to bridge two error types that should share a common trait boundary.

**File:** `crates/node/rpc/src/hdc.rs:92-96`

### D3. Validation deferred to backing crate inside `spawn_blocking`

Every RPC method clones `self.inner`, moves it into a `spawn_blocking` closure,
and calls the inner method which then calls `parse_vector()`. Input validation
(vector length check) happens inside the blocking thread, wasting a pool thread
on requests that will fail immediately.

**Files:**
- `crates/node/rpc/src/hdc.rs:100-108` (`hamming_distance`)
- `crates/node/rpc/src/hdc.rs:110-118` (`similarity`)
- `crates/node/rpc/src/hdc.rs:120-129` (`bind`)
- `crates/node/rpc/src/hdc.rs:131-139` (`bundle`)
- `crates/node/rpc/src/hdc.rs:141-153` (`search`)
- `crates/node/rpc/src/hdc.rs:155-162` (`vector_id`)
- `crates/hdc/chain/src/rpc.rs:26-29`, `33-35`, `40-42`, `48-52`, `60-64`, `77-79`

### D4. Dead error variants

Three of the four HDC error variants in `error.rs` are never constructed
anywhere in the codebase: `InsightNotFound`, `KnowledgeStoreUnavailable`,
`UnknownEncodingMethod`. They were added to satisfy the spec but the
corresponding methods/code paths don't exist. With `#[warn(dead_code)]` or
`unused` lints these may trigger warnings.

**File:** `crates/node/rpc/src/error.rs:79-88`

### D5. `hdc_search` source abuse

The spec's `search` method misuse path (`RpcError::InvalidTransaction`) is
dead code because `hdc_search` in the implementation has no `source` parameter.
The spec code at section 4 (line 390) uses `InvalidTransaction` for a bad
`source` value, which is semantically wrong -- it should be `InvalidParams`.
This was never ported to the implementation but the wrong error variant choice
in the spec should be noted.

### D6. Missing `hdc_types.rs` -- types inlined and incomplete

The spec calls for a dedicated types module with 6 structs. The implementation
has only `HdcSearchResult` with 2 fields inlined in `hdc.rs`. The other 5
structs (`InsightInfo`, `KnowledgeStats`, `PheromoneInfo`, `TrustInfo`,
`StageScore`) do not exist anywhere.

**File:** `crates/node/rpc/src/hdc.rs:55-62`

### D7. `JsonRpcServer` HDC gap

The standalone `JsonRpcServer` struct has no HDC integration. If any code path
uses `JsonRpcServer` instead of `RpcServer`, HDC methods are silently
unavailable with no warning or configuration error.

**File:** `crates/node/rpc/src/server.rs:344-437`

### D8. `encode` always uses trigram, no method dispatch

`hdc_encode` calls `TrigramEncoder::encode()` unconditionally
(`chain/src/rpc.rs:84`). There is no `ProjectionEncoder` or any dispatch on an
encoding method parameter. The method signature in the RPC trait (`hdc.rs:47`)
accepts only `text: String`, not `(text, method)`.

**File:** `crates/hdc/chain/src/rpc.rs:83-86`
**File:** `crates/node/rpc/src/hdc.rs:47`

### D9. `inner.clone()` on every request

Every RPC method calls `self.inner.clone()` to move the `HdcApi` into the
blocking closure. `HdcApi` contains `Arc<RwLock<OnChainHdcIndex>>`, so the
clone is cheap (Arc bump), but this pattern is repeated 7 times without a
helper. The spec's approach of cloning `Arc<dyn T>` is cleaner.

**File:** `crates/node/rpc/src/hdc.rs:103,113,123,133,144,157,165`

---

## Recommended Changes Checklist

### High priority (spec compliance)

- [ ] **Implement `hdc_getInsight`** -- Add method to `HdcRpcApi` trait and
      `HdcApiImpl`. Requires extending `OnChainHdcIndex` or introducing a
      trait-object backing service per the spec.
      Files: `crates/node/rpc/src/hdc.rs`, `crates/hdc/chain/src/rpc.rs`

- [ ] **Implement `hdc_getKnowledgeStats`** -- Add method returning
      `KnowledgeStats`. Requires a `KnowledgeStore` trait or extending the
      existing index with stats methods.
      Files: `crates/node/rpc/src/hdc.rs`, `crates/hdc/chain/src/rpc.rs`

- [ ] **Implement `hdc_getPheromones`** -- Replace the TODO stub in
      `OnChainHdcIndex::record_pheromone` with real tracking, then add the RPC
      method.
      Files: `crates/hdc/chain/src/index.rs:130`, `crates/node/rpc/src/hdc.rs`

- [ ] **Implement `hdc_trustScore`** -- Add trust scoring logic and expose via
      RPC. Requires defining the trust evaluation pipeline.
      Files: `crates/node/rpc/src/hdc.rs`, `crates/hdc/chain/src/rpc.rs`

- [ ] **Add `source` parameter to `hdc_search`** -- Change signature from
      `(Bytes, u32)` to `(Bytes, u32, String)` with validation for
      `"local"|"shared"|"both"`.
      File: `crates/node/rpc/src/hdc.rs:39`

- [ ] **Add `method` parameter to `hdc_encode`** -- Change signature from
      `(String)` to `(String, String)` with validation for
      `"trigram"|"projection"`. Implement or stub `ProjectionEncoder`.
      Files: `crates/node/rpc/src/hdc.rs:47`, `crates/hdc/chain/src/rpc.rs:83`

- [ ] **Create `crates/node/rpc/src/hdc_types.rs`** -- Add the 6 return-type
      structs from spec section 1: `SearchResult`, `InsightInfo`,
      `KnowledgeStats`, `PheromoneInfo`, `TrustInfo`, `StageScore`. Add
      `mod hdc_types` and re-exports to `lib.rs`.

### Medium priority (architecture / correctness)

- [ ] **Introduce trait-object backing services** -- Define `KnowledgeStore`,
      `OnChainHdcIndex` (as a trait), `HdcEncoder` traits in the RPC crate.
      Inject as `Arc<dyn T>` into `HdcApiImpl`. Remove direct dependency on
      `kora-hdc-chain` from `kora-rpc`.
      Files: `crates/node/rpc/src/hdc.rs`, `crates/node/rpc/Cargo.toml:45`

- [ ] **Move input validation before `spawn_blocking`** -- Validate vector
      length and enum parameters in the async method body before spawning.
      This requires either duplicating the length check in the RPC layer or
      exposing a `validate()` function from the HDC crate.
      File: `crates/node/rpc/src/hdc.rs` (all 7 method impls)

- [ ] **Wire HDC into `JsonRpcServer`** -- Add `hdc_api: Option<HdcApiImpl>`
      field, `with_hdc_api()` builder, and conditional merge in
      `JsonRpcServer::start()`.
      File: `crates/node/rpc/src/server.rs:344-437`

- [ ] **Rename trait `HdcRpcApi` to `HdcApi`** -- Match the spec's naming
      convention and the pattern established by `KoraApi` / `KoraApiServer`.
      Update re-export in `lib.rs` from `HdcRpcApiServer` to `HdcApiServer`.
      Files: `crates/node/rpc/src/hdc.rs:20`, `crates/node/rpc/src/lib.rs:24`,
      `crates/node/rpc/src/server.rs:17`

### Low priority (cleanup / hardening)

- [ ] **Remove or use dead error variants** -- Either implement the methods
      that would raise `InsightNotFound`, `KnowledgeStoreUnavailable`,
      `UnknownEncodingMethod`, or remove the variants until they are needed.
      File: `crates/node/rpc/src/error.rs:79-88`

- [ ] **Replace `map_hdc_err` with a proper `From` impl** -- Implement
      `From<HdcRpcError> for RpcError` instead of a standalone function.
      File: `crates/node/rpc/src/hdc.rs:92-96`

- [ ] **Add serde roundtrip tests for all HDC types** -- Once `hdc_types.rs`
      exists, add tests verifying `camelCase` serialization for each struct.

- [ ] **Add mock-backed unit tests** -- Replace real-`HdcApi` tests with
      mock-trait-object tests per the spec's test plan (section 10).
      File: `crates/node/rpc/src/hdc.rs:173-269`

- [ ] **Enforce server-side cap on `hdc_search` `k` at the RPC boundary** --
      Currently `MAX_SEARCH_K` (line 69) is applied inside the impl at
      `hdc.rs:143` (`top_k.min(MAX_SEARCH_K)`). This is correct but should be
      documented. Also enforce a cap on `hdc_getPheromones` `limit` once that
      method exists.

- [ ] **Fix spec section 4, line 390** -- The spec uses
      `RpcError::InvalidTransaction` for a bad `source` value, which is
      semantically wrong. This should be a custom validation error or
      `RpcError::InvalidParams` equivalent. Not an implementation bug but a
      spec bug to correct.

---

## Second-Pass Remediation Detail

Second-pass review date: 2026-05-08. This section treats the current files as
the remediation baseline:

- `crates/node/rpc/src/hdc.rs` exposes an algebra-first `HdcRpcApi` wrapper.
- `crates/node/rpc/src/server.rs` wires HDC only through `RpcServer`, not
  `JsonRpcServer`.
- `crates/node/rpc/src/state.rs` only holds node status counters; it is not an
  HDC state carrier.
- `crates/node/rpc/src/state_provider.rs` abstracts EVM-style account/block/log
  reads; it does not expose local HDC knowledge, insight metadata, pheromones,
  or trust state.
- `crates/hdc/chain/src/rpc.rs` is a concrete adapter over
  `Arc<parking_lot::RwLock<OnChainHdcIndex>>`.

### R1. Concrete RPC API Decision

Decision: keep the current algebra RPC surface as the HDC v0 API, and make the
knowledge/insight/pheromone/trust API additive. Do not reinterpret
`hdc_similarity` as Hamming distance. The node already has the clearer split:

| Method | Canonical v0 signature | Semantics |
|---|---|---|
| `hdc_hammingDistance` | `(Bytes, Bytes) -> u32` | Raw Hamming distance. |
| `hdc_similarity` | `(Bytes, Bytes) -> f64` | Normalized similarity. |
| `hdc_bind` | `(Bytes, Bytes) -> Bytes` | XOR bind. |
| `hdc_bundle` | `(Vec<Bytes>) -> Bytes` | Majority-vote bundle. Empty input currently returns the zero vector through `kora_hdc::bundle`; document that if kept. |
| `hdc_search` | `(Bytes, u32) -> Vec<HdcSearchResult>` | Search accepted vectors in the on-chain index. This is not local/shared knowledge search. |
| `hdc_vectorId` | `(Bytes) -> B256` | Keccak content address of the serialized vector. |
| `hdc_encode` | `(String) -> Bytes` | Trigram text encoding only. |

The first-pass domain methods should be added only when their backing services
exist:

| Additive method | Signature | Backing requirement |
|---|---|---|
| `hdc_getInsight` | `(B256) -> InsightInfo` | Read-only insight metadata index. Prefer `B256` because current insight/vector IDs are `B256`, not `U256`. |
| `hdc_getKnowledgeStats` | `() -> KnowledgeStats` | Local knowledge-store reader. |
| `hdc_searchKnowledge` | `(Bytes, u32, String) -> Vec<SearchResult>` | Local/shared/both knowledge-store reader. Use this name instead of overloading or changing current `hdc_search`. |
| `hdc_getPheromones` | `(u8, u32) -> Vec<PheromoneInfo>` | Pheromone event storage and decay/query logic. |
| `hdc_trustScore` | `(B256) -> TrustInfo` | Trust pipeline/index over insight state. |
| `hdc_encodeWithMethod` | `(String, String) -> Bytes` | Encoder dispatch for `"trigram"` and any future method such as `"projection"`. |

This preserves the currently wired API while making the spec-level domain
surface explicit. If there are no external clients yet, the alternative is a
breaking rename before release, but the remediation recommendation is additive
because JSON-RPC has method names, not overloads.

### R2. Trait-Object Backing Services

The RPC layer should stop owning concrete HDC chain adapters directly once
domain methods are added. `NodeState` and `StateProvider` are not the right
extension points: `NodeState` is status-only, and `StateProvider` is
EVM/account/block oriented. Add HDC-specific read-only service traits in
`crates/node/rpc/src/hdc.rs` or a sibling module, then adapt the concrete chain
index and knowledge store behind `Arc<dyn Trait>`.

Recommended trait split:

```rust
pub trait HdcVectorOps: Send + Sync + 'static {
    fn hamming_distance(&self, a: &[u8], b: &[u8]) -> Result<u32, HdcServiceError>;
    fn similarity(&self, a: &[u8], b: &[u8]) -> Result<f64, HdcServiceError>;
    fn bind(&self, a: &[u8], b: &[u8]) -> Result<Vec<u8>, HdcServiceError>;
    fn bundle(&self, vectors: &[Vec<u8>]) -> Result<Vec<u8>, HdcServiceError>;
    fn vector_id(&self, vector: &[u8]) -> Result<B256, HdcServiceError>;
    fn encode_trigram(&self, text: &str) -> Result<Vec<u8>, HdcServiceError>;
}

pub trait HdcIndexReader: Send + Sync + 'static {
    fn search_on_chain(&self, query: &[u8], top_k: usize)
        -> Result<Vec<HdcSearchResult>, HdcServiceError>;
    fn get_insight(&self, id: B256) -> Result<Option<InsightInfo>, HdcServiceError>;
    fn trust_score(&self, id: B256) -> Result<TrustInfo, HdcServiceError>;
}

pub trait HdcKnowledgeReader: Send + Sync + 'static {
    fn search_knowledge(&self, query: &[u8], top_k: usize, source: KnowledgeSource)
        -> Result<Vec<SearchResult>, HdcServiceError>;
    fn stats(&self) -> Result<KnowledgeStats, HdcServiceError>;
}

pub trait HdcPheromoneReader: Send + Sync + 'static {
    fn pheromones(&self, pheromone_type: u8, limit: usize)
        -> Result<Vec<PheromoneInfo>, HdcServiceError>;
}
```

`HdcApiImpl` can then hold optional services:

```rust
pub struct HdcApiImpl {
    vector_ops: Arc<dyn HdcVectorOps>,
    index: Option<Arc<dyn HdcIndexReader>>,
    knowledge: Option<Arc<dyn HdcKnowledgeReader>>,
    pheromones: Option<Arc<dyn HdcPheromoneReader>>,
}
```

Current `kora_hdc_chain::rpc::HdcApi` can be wrapped by an adapter implementing
`HdcVectorOps` plus `HdcIndexReader::search_on_chain`. The future local
`kora_hdc::knowledge::KnowledgeStore` should not be exposed directly because it
is mutable local memory, not an RPC-safe read interface.

### R3. `spawn_blocking` Validation Boundary

Keep `spawn_blocking` for vector math, index scans, knowledge-store reads, and
trust/pheromone computations. Move cheap request validation before the spawn:

- Validate every vector is exactly `kora_hdc::BYTES` bytes in
  `crates/node/rpc/src/hdc.rs` before cloning inputs into the blocking closure.
- For `hdc_bundle`, validate every element before spawning. Decide and test
  empty-input behavior; current core behavior returns the zero vector.
- Clamp `top_k` before spawning with `MAX_SEARCH_K`. Keep `top_k = 0` as an
  immediate empty response or make it `INVALID_PARAMS`; choose one and test it.
- Validate domain enums before spawning: knowledge `source` must parse to
  `local`, `shared`, or `both`; encoding method must parse to `trigram` or a
  supported future encoder; `limit` must be capped before pheromone queries.

Current boundary issue: `hdc.rs` calls `spawn_blocking` first, then
`crates/hdc/chain/src/rpc.rs` calls `parse_vector()` inside the blocking
closure. Bad vector lengths therefore consume blocking pool capacity. The
next code pass should add a local helper:

```rust
fn validate_vector_len(bytes: &Bytes) -> Result<(), RpcError> {
    if bytes.len() != kora_hdc::BYTES {
        return Err(RpcError::InvalidVectorLength(bytes.len()));
    }
    Ok(())
}
```

Use this helper before `spawn_blocking` even if the backing service also parses
and validates defensively.

### R4. Errors and Error Codes

Keep the existing HDC-specific `RpcError` variants, but make them reachable
through typed service errors instead of ad hoc strings:

| Condition | RPC error | JSON-RPC code |
|---|---|---|
| Wrong vector length | `InvalidVectorLength(len)` | `-32602` |
| Bad HDC enum/range param | Add `InvalidHdcParams(String)` or equivalent | `-32602` |
| Missing insight | `InsightNotFound(id)` | `-32001` |
| Knowledge/pheromone/index backend unavailable | `KnowledgeStoreUnavailable(reason)` or a renamed `HdcBackendUnavailable` | `-32002` |
| Unsupported encoding method | `UnknownEncodingMethod(method)` | `-32602` |
| Blocking task join failure/panic | `Internal("spawn_blocking failed: ...")` | `-32603` |

Do not use `InvalidTransaction` for HDC parameter validation. It maps to the
same `INVALID_PARAMS` code today, but it is semantically wrong and makes logs
misleading.

Replace `map_hdc_err()` with one of these:

```rust
impl From<kora_hdc_chain::rpc::HdcRpcError> for RpcError {
    fn from(err: kora_hdc_chain::rpc::HdcRpcError) -> Self {
        match err {
            kora_hdc_chain::rpc::HdcRpcError::InvalidVectorLength(len) => {
                Self::InvalidVectorLength(len)
            }
        }
    }
}
```

or, preferably after trait-object extraction, map from a local
`HdcServiceError` enum. Avoid wildcard arms that collapse future chain/index
errors into `InvalidVectorLength`.

### R5. Pheromone Gaps

Do not expose `hdc_getPheromones` until the index actually records pheromones.
Returning `[]` would hide an indexing gap as a valid empty state.

Current blockers:

- `crates/hdc/chain/src/index.rs` has `record_pheromone(...)` as a TODO stub.
- `crates/hdc/chain/src/event.rs` uses zero-valued placeholder topics.
- `process_log()` does not ABI-decode `PheromoneDeposited`.
- There is no `PheromoneInfo`/record type with depositor, deposit block/time,
  type, topic/region/vector ID, strength, or decayed intensity.
- There is no query path with deterministic ordering and a server-side limit.

Minimum backing model before RPC:

```rust
pub struct PheromoneRecord {
    pub id: B256,
    pub pheromone_type: u8,
    pub topic: B256,
    pub region: B256,
    pub depositor: Address,
    pub strength: u64,
    pub deposited_at_block: u64,
}
```

`HdcPheromoneReader::pheromones()` should compute intensity at query time from
stored strength plus block/time context, sort by active intensity descending,
apply `limit.min(MAX_PHEROMONE_LIMIT)`, and return owned `PheromoneInfo`
values.

### R6. `JsonRpcServer` Wiring

Mirror the existing `RpcServer` HDC path in the standalone `JsonRpcServer`.
Today only `RpcServer` has `hdc_api: Option<HdcApiImpl>` and a
`with_hdc_api()` builder. `JsonRpcServer::start()` merges only eth/net/web3.

Required code change in the next implementation pass:

- Add `hdc_api: Option<HdcApiImpl>` to `JsonRpcServer`.
- Initialize it to `None` in `JsonRpcServer::new()` and
  `JsonRpcServer::with_state_provider()`.
- Add `pub fn with_hdc_api(mut self, hdc: HdcApiImpl) -> Self`.
- In `JsonRpcServer::start()`, move `self.hdc_api` into a local before module
  assembly and merge it after web3:

```rust
if let Some(hdc_api) = hdc_api {
    module.merge(hdc_api.into_rpc())?;
}
```

This keeps behavior consistent: if HDC is not configured, HDC methods are
absent; if HDC is configured, both server entry points expose the same
namespace.

### R7. Tests and Manual Smoke Steps

Add tests in layers, because each layer catches a different class of mistake:

- `crates/node/rpc/src/hdc.rs` unit tests with fake trait-object services.
  Include a fake service with an `AtomicUsize` call counter to prove wrong
  vector lengths fail before `spawn_blocking` and never call the backend.
- Algebra API tests for all v0 methods:
  `hdc_hammingDistance`, `hdc_similarity`, `hdc_bind`, `hdc_bundle`,
  `hdc_search`, `hdc_vectorId`, and `hdc_encode`.
- Error-code tests that convert `RpcError` into `ErrorObjectOwned` and assert
  `-32602`, `-32001`, `-32002`, and `-32603` where applicable.
- `JsonRpcServer` integration tests with HDC disabled and enabled. Disabled
  should return method-not-found for `hdc_hammingDistance`; enabled should
  serve the same HDC methods as `RpcServer`.
- Serde roundtrip tests for every future domain type:
  `InsightInfo`, `KnowledgeStats`, `SearchResult`, `PheromoneInfo`,
  `TrustInfo`, and `StageScore`, with explicit `camelCase` assertions.
- Pheromone tests once storage exists: event record insertion, deterministic
  ordering by active intensity, limit capping, and empty state only when no
  records have been indexed.

Target automated commands:

```sh
cargo test -p kora-rpc hdc
cargo test -p kora-rpc json_rpc
cargo nextest run -p kora-rpc --all-features
cargo clippy -p kora-rpc --all-targets --all-features -- -D warnings
```

Manual smoke against an HDC-enabled local node:

```sh
# Start the local node/devnet with HDC enabled by configuration.
just trusted-devnet

# Build 1280-byte vectors.
ZERO=0x$(printf '00%.0s' {1..1280})
ONES=0x$(printf 'ff%.0s' {1..1280})

# Hamming distance should be 10240 for all-zero vs all-one vectors.
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_hammingDistance\",\"params\":[\"$ZERO\",\"$ONES\"],\"id\":1}"

# Similarity should be 1.0 for identical vectors.
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_similarity\",\"params\":[\"$ZERO\",\"$ZERO\"],\"id\":2}"

# Encode should return a 1280-byte hex string.
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"hdc_encode","params":["hello world"],"id":3}'

# Search on an empty or unaccepted on-chain index should return [].
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_search\",\"params\":[\"$ZERO\",5],\"id\":4}"

# Wrong vector length should return INVALID_PARAMS (-32602) without backend work.
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"hdc_hammingDistance","params":["0x00","0x00"],"id":5}'
```

When `JsonRpcServer` wiring is added, repeat the same curl calls against a
standalone `JsonRpcServer` test binary or integration-test harness, not only
the full `RpcServer` path.

---

## Actual RPC Method Signatures (from implementation)

The implementation at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs`
exposes an algebra-oriented API that diverges from the spec. Here are the
actual method signatures currently wired:

```rust
// File: /Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs

#[rpc(server, namespace = "hdc")]
pub trait HdcRpcApi {
    /// Compute the Hamming distance between two HDC vectors.
    #[method(name = "hammingDistance")]
    async fn hamming_distance(&self, a: Bytes, b: Bytes) -> RpcResult<u32>;

    /// Compute normalized similarity between two HDC vectors (off-chain only).
    #[method(name = "similarity")]
    async fn similarity(&self, a: Bytes, b: Bytes) -> RpcResult<f64>;

    /// XOR-bind two HDC vectors.
    #[method(name = "bind")]
    async fn bind(&self, a: Bytes, b: Bytes) -> RpcResult<Bytes>;

    /// Majority-vote bundle of multiple HDC vectors.
    #[method(name = "bundle")]
    async fn bundle(&self, vectors: Vec<Bytes>) -> RpcResult<Bytes>;

    /// Search the on-chain index for similar vectors.
    #[method(name = "search")]
    async fn search(&self, query: Bytes, top_k: u32) -> RpcResult<Vec<HdcSearchResult>>;

    /// Compute the keccak-256 content address of a vector.
    #[method(name = "vectorId")]
    async fn vector_id(&self, vector: Bytes) -> RpcResult<B256>;

    /// Encode text into a hypervector using trigram encoding.
    #[method(name = "encode")]
    async fn encode(&self, text: String) -> RpcResult<Bytes>;
}
```

The chain-level backing API at `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs`
also has `get_insight_rpc()` which is NOT exposed via the RPC trait but exists as
a callable method on `HdcApi`:

```rust
// File: /Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs

pub fn get_insight_rpc(&self, id: &B256)
    -> Result<(Vec<u8>, crate::index::InsightMeta), HdcRpcError>;
```

---

## Consolidated Fix Checklist

### High priority (spec compliance + correctness)

- [ ] **Wire `HdcApi` to `JsonRpcServer`** -- Add `hdc_api: Option<HdcApiImpl>` field + `with_hdc_api()` builder to `JsonRpcServer` struct at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` lines 344-437. Add conditional merge in `JsonRpcServer::start()`:
  ```rust
  if let Some(hdc_api) = hdc_api {
      module.merge(hdc_api.into_rpc())?;
  }
  ```

- [ ] **Move vector length validation before `spawn_blocking`** -- Add a validation helper in `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs` and call it before spawning:
  ```rust
  fn validate_vector_len(bytes: &Bytes) -> Result<(), RpcError> {
      if bytes.len() != kora_hdc::BYTES {
          return Err(RpcError::InvalidVectorLength(bytes.len()));
      }
      Ok(())
  }
  ```
  Apply to: `hamming_distance`, `similarity`, `bind`, `bundle` (each element), `search`, `vector_id`. Currently validation happens inside `parse_vector()` at `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs` line 120-124, inside the blocking closure.

- [ ] **Implement `From<HdcServiceError> for RpcError`** -- Replace the standalone `map_hdc_err()` function at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs` lines 92-104 with a proper `From` impl:
  ```rust
  impl From<kora_hdc_chain::rpc::HdcRpcError> for RpcError {
      fn from(err: kora_hdc_chain::rpc::HdcRpcError) -> Self {
          match err {
              HdcRpcError::InvalidVectorLength(len) => Self::InvalidVectorLength(len),
              HdcRpcError::SearchFailed(msg) => Self::Internal(msg),
              HdcRpcError::IndexUnavailable => Self::KnowledgeStoreUnavailable("HDC index unavailable".into()),
              HdcRpcError::EncodingError(msg) => Self::Internal(format!("encoding error: {msg}")),
              HdcRpcError::InsightNotFound => Self::InsightNotFound("not found".into()),
              HdcRpcError::OperationFailed(msg) => Self::Internal(msg),
          }
      }
  }
  ```

- [ ] **Complete pheromone backing storage** -- Implement `record_pheromone()` at `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/index.rs` line ~130 (currently a TODO stub). Until this is done, `hdc_getPheromones` cannot return real data.

### Medium priority (API completeness)

- [ ] **Expose `hdc_getInsight` via RPC trait** -- The backing method `get_insight_rpc()` already exists in `/Users/will/dev/nunchi/daeji/crates/hdc/chain/src/rpc.rs` line 112-116. Add the corresponding method to the `HdcRpcApi` trait in `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs` and implement it.

- [ ] **Add `method` parameter to `hdc_encode`** -- Change from `(String) -> Bytes` to `(String, String) -> Bytes` with validation for `"trigram"` or `"projection"`. File: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc.rs` line 47.

- [ ] **Add `source` parameter to `hdc_search`** -- Or add a separate `hdc_searchKnowledge` method per the R1 remediation plan. The current `hdc_search` only searches on-chain index.

- [ ] **Create `hdc_types.rs`** -- Add the 6 return-type structs from spec section 1 (`SearchResult`, `InsightInfo`, `KnowledgeStats`, `PheromoneInfo`, `TrustInfo`, `StageScore`) at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/hdc_types.rs`. Register in `lib.rs`.

### Low priority (cleanup + hardening)

- [ ] **Add serde roundtrip tests for all HDC types** -- Once `hdc_types.rs` exists, verify `camelCase` serialization for each struct. Currently only `HdcSearchResult` has a roundtrip test.

- [ ] **Rename `HdcRpcApi` to `HdcApi`** -- Match spec naming and `KoraApi`/`KoraApiServer` pattern. Update re-export in `lib.rs` from `HdcRpcApiServer` to `HdcApiServer`.

- [ ] **Add mock-backed unit tests** -- Replace real-`HdcApi` tests with trait-object mock tests per the spec's test plan.

- [ ] **Add curl smoke test examples** for the actual v0 algebra API:

```bash
# Build 1280-byte vectors
ZERO=0x$(printf '00%.0s' {1..1280})
ONES=0x$(printf 'ff%.0s' {1..1280})

# hdc_hammingDistance -- expect 10240 for all-zero vs all-one
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_hammingDistance\",\"params\":[\"$ZERO\",\"$ONES\"],\"id\":1}"

# hdc_similarity -- expect 1.0 for identical vectors
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_similarity\",\"params\":[\"$ZERO\",\"$ZERO\"],\"id\":2}"

# hdc_encode -- expect 1280-byte hex string
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"hdc_encode","params":["hello world"],"id":3}'

# hdc_search -- expect [] on empty index
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_search\",\"params\":[\"$ZERO\",5],\"id\":4}"

# hdc_vectorId -- expect deterministic B256
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_vectorId\",\"params\":[\"$ZERO\"],\"id\":5}"

# hdc_bind -- XOR two vectors
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"hdc_bind\",\"params\":[\"$ZERO\",\"$ONES\"],\"id\":6}"

# Error case: wrong vector length, expect -32602
curl -s http://localhost:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"hdc_hammingDistance","params":["0x00","0x00"],"id":7}'
```

### Verification commands

```bash
# Check RPC crate compiles
cargo check -p kora-rpc

# Run RPC tests (including HDC)
cargo test -p kora-rpc

# Run only HDC-related tests
cargo test -p kora-rpc hdc

# Check for JsonRpcServer wiring
grep -n "hdc_api\|HdcApiImpl\|with_hdc_api" crates/node/rpc/src/server.rs

# Verify error variant usage
grep -rn "InvalidVectorLength\|InsightNotFound\|KnowledgeStoreUnavailable\|UnknownEncodingMethod" crates/node/rpc/src/

# Check spawn_blocking validation order
grep -B5 "spawn_blocking" crates/node/rpc/src/hdc.rs | grep -E "validate|parse_vector|len"
```
