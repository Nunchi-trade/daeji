//! HDC RPC extensions -- `hdc_*` namespace methods.

use std::sync::Arc;

use alloy_primitives::B256;
use kora_hdc::{
    BYTES, HdcVector, TrigramEncoder, bind, bundle, deserialize, hamming_distance, permute,
    serialize, similarity, vector_id,
};

use crate::index::OnChainHdcIndex;

/// Errors from HDC RPC methods.
#[derive(Debug, thiserror::Error)]
pub enum HdcRpcError {
    /// Invalid vector byte length.
    #[error("invalid vector length: expected {expected}, got {0}", expected = BYTES)]
    InvalidVectorLength(usize),
    /// A search operation failed.
    #[error("search failed: {0}")]
    SearchFailed(String),
    /// The on-chain index is not available.
    #[error("HDC index unavailable")]
    IndexUnavailable,
    /// Encoding or decoding error.
    #[error("encoding error: {0}")]
    EncodingError(String),
    /// The requested insight was not found.
    #[error("insight not found")]
    InsightNotFound,
    /// A generic operation failure.
    #[error("operation failed: {0}")]
    OperationFailed(String),
}

/// HDC API implementation for JSON-RPC.
#[derive(Debug, Clone)]
pub struct HdcApi {
    index: Arc<parking_lot::RwLock<OnChainHdcIndex>>,
}

impl HdcApi {
    /// Create a new HDC API with the given on-chain index.
    pub fn new(index: Arc<parking_lot::RwLock<OnChainHdcIndex>>) -> Self {
        Self { index }
    }

    /// `hdc_hammingDistance` -- compute Hamming distance between two vectors.
    pub fn hamming_distance_rpc(&self, a: &[u8], b: &[u8]) -> Result<u32, HdcRpcError> {
        let va = parse_vector(a)?;
        let vb = parse_vector(b)?;
        Ok(hamming_distance(&va, &vb))
    }

    /// `hdc_similarity` -- compute normalized similarity (OFF-CHAIN ONLY).
    pub fn similarity_rpc(&self, a: &[u8], b: &[u8]) -> Result<f64, HdcRpcError> {
        let va = parse_vector(a)?;
        let vb = parse_vector(b)?;
        Ok(similarity(&va, &vb))
    }

    /// `hdc_bind` -- XOR-bind two vectors.
    pub fn bind_rpc(&self, a: &[u8], b: &[u8]) -> Result<Vec<u8>, HdcRpcError> {
        let va = parse_vector(a)?;
        let vb = parse_vector(b)?;
        let result = bind(&va, &vb);
        Ok(serialize(&result).to_vec())
    }

    /// `hdc_bundle` -- majority-vote bundle of vectors.
    pub fn bundle_rpc(&self, vectors: &[Vec<u8>]) -> Result<Vec<u8>, HdcRpcError> {
        let vecs: Vec<HdcVector> =
            vectors.iter().map(|v| parse_vector(v)).collect::<Result<_, _>>()?;
        let refs: Vec<&HdcVector> = vecs.iter().collect();
        let result = bundle(&refs);
        Ok(serialize(&result).to_vec())
    }

    /// `hdc_search` -- search the on-chain index for similar vectors.
    pub fn search_rpc(
        &self,
        query: &[u8],
        top_k: usize,
    ) -> Result<Vec<SearchResultRpc>, HdcRpcError> {
        let q = parse_vector(query)?;
        let index = self.index.read();
        let results = index.search(&q, top_k);
        Ok(results
            .into_iter()
            .map(|r| SearchResultRpc { id: r.id, distance: r.distance })
            .collect())
    }

    /// `hdc_vectorId` -- compute the keccak-256 content address of a vector.
    pub fn vector_id_rpc(&self, v: &[u8]) -> Result<B256, HdcRpcError> {
        let vec = parse_vector(v)?;
        Ok(B256::from(vector_id(&vec)))
    }

    /// `hdc_encode` -- encode text into a hypervector using trigram encoding.
    pub fn encode_rpc(&self, text: &str) -> Result<Vec<u8>, HdcRpcError> {
        let vec = TrigramEncoder::encode(text);
        Ok(serialize(&vec).to_vec())
    }

    /// `hdc_permute` -- cyclic left-rotation of a vector by `n` positions.
    pub fn permute_rpc(&self, v: &[u8], n: usize) -> Result<Vec<u8>, HdcRpcError> {
        let vec = parse_vector(v)?;
        let result = permute(&vec, n);
        Ok(serialize(&result).to_vec())
    }

    /// `hdc_getInsight` -- retrieve a vector and its metadata by ID.
    pub fn get_insight_rpc(
        &self,
        id: &B256,
    ) -> Result<(Vec<u8>, crate::index::InsightMeta), HdcRpcError> {
        let index = self.index.read();
        let (vec, meta) = index.get(id).ok_or(HdcRpcError::InsightNotFound)?;
        Ok((serialize(vec).to_vec(), meta.clone()))
    }
}

/// Parse raw bytes into an `HdcVector`.
fn parse_vector(data: &[u8]) -> Result<HdcVector, HdcRpcError> {
    let bytes: &[u8; BYTES] =
        data.try_into().map_err(|_| HdcRpcError::InvalidVectorLength(data.len()))?;
    Ok(deserialize(bytes))
}

/// RPC search result.
#[derive(Debug, Clone)]
pub struct SearchResultRpc {
    /// Vector ID.
    pub id: B256,
    /// Hamming distance to query.
    pub distance: u32,
}
