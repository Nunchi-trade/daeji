//! `kora-hdc-chain` -- On-chain HDC integration.
//!
//! This crate provides:
//! - REVM precompile at address `0x09` for HDC operations
//! - On-chain HDC index for consensus-critical vector storage
//! - Event sync handlers for block finalization
//! - RPC extensions for HDC queries
//! - WisdomGate submit/challenge/resolve lifecycle

pub mod event;
pub mod index;
pub mod precompile;
pub mod rpc;
pub mod wisdom;

pub use index::{IndexError, InsightMeta, InsightState, OnChainHdcIndex, OnChainSearchResult};
pub use precompile::{HDC_PRECOMPILE_ADDRESS, hdc_precompile};
pub use rpc::{HdcApi, HdcRpcError, SearchResultRpc};
pub use wisdom::{WisdomError, WisdomGate, WisdomState, WisdomSubmission};
