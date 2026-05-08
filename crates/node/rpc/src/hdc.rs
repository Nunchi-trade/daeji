//! HDC (Hyperdimensional Computing) JSON-RPC API implementation.
//!
//! Wraps [`kora_hdc_chain::rpc::HdcApi`] as a jsonrpsee `hdc_*` namespace.

use alloy_primitives::{B256, Bytes};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use serde::{Deserialize, Serialize};

use crate::error::RpcError;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// HDC JSON-RPC API trait.
///
/// Exposes hyperdimensional computing operations: Hamming distance, similarity,
/// bind, bundle, search, vector ID, and encoding.
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

    /// Cyclic left-rotation of an HDC vector by `n` bit positions.
    #[method(name = "permute")]
    async fn permute(&self, vector: Bytes, n: u32) -> RpcResult<Bytes>;
}

// ---------------------------------------------------------------------------
// RPC search result (serde-enabled)
// ---------------------------------------------------------------------------

/// Search result returned by `hdc_search`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HdcSearchResult {
    /// Vector ID (keccak-256 hash).
    pub id: B256,
    /// Hamming distance to query.
    pub distance: u32,
}

// ---------------------------------------------------------------------------
// Impl
// ---------------------------------------------------------------------------

/// Maximum number of results for search queries.
const MAX_SEARCH_K: u32 = 1000;

/// Concrete implementation of the HDC RPC API.
///
/// Wraps [`kora_hdc_chain::rpc::HdcApi`] and exposes it as a jsonrpsee server.
pub struct HdcApiImpl {
    inner: kora_hdc_chain::rpc::HdcApi,
}

impl std::fmt::Debug for HdcApiImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HdcApiImpl").finish_non_exhaustive()
    }
}

impl HdcApiImpl {
    /// Create a new HDC API implementation wrapping the chain-level HDC API.
    pub const fn new(inner: kora_hdc_chain::rpc::HdcApi) -> Self {
        Self { inner }
    }
}

/// Convert a chain-level HdcRpcError into our RpcError.
fn map_hdc_err(e: kora_hdc_chain::rpc::HdcRpcError) -> RpcError {
    use kora_hdc_chain::rpc::HdcRpcError;
    match e {
        HdcRpcError::InvalidVectorLength(len) => RpcError::InvalidVectorLength(len),
        HdcRpcError::SearchFailed(msg) => RpcError::Internal(msg),
        HdcRpcError::IndexUnavailable => {
            RpcError::KnowledgeStoreUnavailable("HDC index unavailable".into())
        }
        HdcRpcError::EncodingError(msg) => RpcError::Internal(format!("encoding error: {msg}")),
        HdcRpcError::InsightNotFound => RpcError::InsightNotFound("not found".into()),
        HdcRpcError::OperationFailed(msg) => RpcError::Internal(msg),
    }
}

#[jsonrpsee::core::async_trait]
impl HdcRpcApiServer for HdcApiImpl {
    async fn hamming_distance(&self, a: Bytes, b: Bytes) -> RpcResult<u32> {
        let a_vec = a.to_vec();
        let b_vec = b.to_vec();
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.hamming_distance_rpc(&a_vec, &b_vec))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(|e| map_hdc_err(e).into())
    }

    async fn similarity(&self, a: Bytes, b: Bytes) -> RpcResult<f64> {
        let a_vec = a.to_vec();
        let b_vec = b.to_vec();
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.similarity_rpc(&a_vec, &b_vec))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(|e| map_hdc_err(e).into())
    }

    async fn bind(&self, a: Bytes, b: Bytes) -> RpcResult<Bytes> {
        let a_vec = a.to_vec();
        let b_vec = b.to_vec();
        let inner = self.inner.clone();
        let result = tokio::task::spawn_blocking(move || inner.bind_rpc(&a_vec, &b_vec))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(map_hdc_err)?;
        Ok(Bytes::from(result))
    }

    async fn bundle(&self, vectors: Vec<Bytes>) -> RpcResult<Bytes> {
        let vecs: Vec<Vec<u8>> = vectors.iter().map(|b| b.to_vec()).collect();
        let inner = self.inner.clone();
        let result = tokio::task::spawn_blocking(move || inner.bundle_rpc(&vecs))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(map_hdc_err)?;
        Ok(Bytes::from(result))
    }

    async fn search(&self, query: Bytes, top_k: u32) -> RpcResult<Vec<HdcSearchResult>> {
        let q = query.to_vec();
        let k = top_k.min(MAX_SEARCH_K) as usize;
        let inner = self.inner.clone();
        let results = tokio::task::spawn_blocking(move || inner.search_rpc(&q, k))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(map_hdc_err)?;
        Ok(results
            .into_iter()
            .map(|r| HdcSearchResult { id: r.id, distance: r.distance })
            .collect())
    }

    async fn vector_id(&self, vector: Bytes) -> RpcResult<B256> {
        let v = vector.to_vec();
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || inner.vector_id_rpc(&v))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(|e| map_hdc_err(e).into())
    }

    async fn encode(&self, text: String) -> RpcResult<Bytes> {
        let inner = self.inner.clone();
        let result = tokio::task::spawn_blocking(move || inner.encode_rpc(&text))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(map_hdc_err)?;
        Ok(Bytes::from(result))
    }

    async fn permute(&self, vector: Bytes, n: u32) -> RpcResult<Bytes> {
        let v = vector.to_vec();
        let inner = self.inner.clone();
        let result = tokio::task::spawn_blocking(move || inner.permute_rpc(&v, n as usize))
            .await
            .map_err(|e| RpcError::Internal(format!("spawn_blocking failed: {e}")))?
            .map_err(map_hdc_err)?;
        Ok(Bytes::from(result))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn make_api() -> HdcApiImpl {
        let index = kora_hdc_chain::OnChainHdcIndex::new();
        let inner = kora_hdc_chain::rpc::HdcApi::new(Arc::new(parking_lot::RwLock::new(index)));
        HdcApiImpl::new(inner)
    }

    #[tokio::test]
    async fn hamming_distance_identical_vectors() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let dist = api.hamming_distance(v.clone(), v).await.unwrap();
        assert_eq!(dist, 0);
    }

    #[tokio::test]
    async fn hamming_distance_opposite_vectors() {
        let api = make_api();
        let a = Bytes::from(vec![0x00; kora_hdc::BYTES]);
        let b = Bytes::from(vec![0xFF; kora_hdc::BYTES]);
        let dist = api.hamming_distance(a, b).await.unwrap();
        assert_eq!(dist, kora_hdc::BYTES as u32 * 8);
    }

    #[tokio::test]
    async fn hamming_distance_rejects_wrong_length() {
        let api = make_api();
        let short = Bytes::from(vec![0u8; 100]);
        let ok = Bytes::from(vec![0u8; kora_hdc::BYTES]);
        let err = api.hamming_distance(short, ok).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn similarity_identical_vectors() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let sim = api.similarity(v.clone(), v).await.unwrap();
        assert!((sim - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn bind_produces_correct_length() {
        let api = make_api();
        let a = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let b = Bytes::from(vec![0x55; kora_hdc::BYTES]);
        let result = api.bind(a, b).await.unwrap();
        assert_eq!(result.len(), kora_hdc::BYTES);
    }

    #[tokio::test]
    async fn bundle_produces_correct_length() {
        let api = make_api();
        let v1 = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let v2 = Bytes::from(vec![0x55; kora_hdc::BYTES]);
        let v3 = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let result = api.bundle(vec![v1, v2, v3]).await.unwrap();
        assert_eq!(result.len(), kora_hdc::BYTES);
    }

    #[tokio::test]
    async fn vector_id_deterministic() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let id1 = api.vector_id(v.clone()).await.unwrap();
        let id2 = api.vector_id(v).await.unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn encode_produces_correct_length() {
        let api = make_api();
        let result = api.encode("hello world".into()).await.unwrap();
        assert_eq!(result.len(), kora_hdc::BYTES);
    }

    #[tokio::test]
    async fn search_empty_index() {
        let api = make_api();
        let q = Bytes::from(vec![0u8; kora_hdc::BYTES]);
        let results = api.search(q, 5).await.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_result_serde_roundtrip() {
        let sr = HdcSearchResult { id: B256::ZERO, distance: 42 };
        let json = serde_json::to_string(&sr).unwrap();
        let parsed: HdcSearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.distance, 42);
    }

    #[tokio::test]
    async fn permute_zero_is_identity() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let result = api.permute(v.clone(), 0).await.unwrap();
        assert_eq!(result, v);
    }

    #[tokio::test]
    async fn permute_produces_correct_length() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let result = api.permute(v, 7).await.unwrap();
        assert_eq!(result.len(), kora_hdc::BYTES);
    }

    #[tokio::test]
    async fn permute_changes_vector() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        let result = api.permute(v.clone(), 1).await.unwrap();
        assert_ne!(result, v);
    }

    #[tokio::test]
    async fn permute_rejects_wrong_length() {
        let api = make_api();
        let short = Bytes::from(vec![0u8; 100]);
        assert!(api.permute(short, 1).await.is_err());
    }

    #[tokio::test]
    async fn permute_full_rotation_is_identity() {
        let api = make_api();
        let v = Bytes::from(vec![0xAA; kora_hdc::BYTES]);
        // Full rotation by D (10240) bits should return original
        let result = api.permute(v.clone(), kora_hdc::D as u32).await.unwrap();
        assert_eq!(result, v);
    }
}
