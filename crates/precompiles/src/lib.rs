//! Chain-specific REVM precompiles for Kora (Daeji testnet).
//!
//! Registers two pages of chain-extension precompiles on top of the standard
//! Ethereum precompile set:
//!
//! - **HDC similarity** at `0xA0C` (10,240-bit hyperdimensional vectors,
//!   Hamming-distance similarity, 8 selectors: projectBytes, projectTokens,
//!   bind, bundle, similarity, search, insert, remove).
//! - **Agent namespace** at `0xA10–0xA1F` (passport / capability / tier /
//!   reputation lookups against on-chain ERC-8004 registries — currently
//!   reserved; concrete handlers land as the contract-side wiring matures).
//!
//! Use [`KoraPrecompiles::new`] to construct the provider and pass it to the
//! REVM builder via `.with_precompiles(...)` after `build_mainnet()`.
//!
//! Match the mirage-rs wiring pattern at
//! `~/roko/apps/mirage-rs/src/fork.rs:1646`:
//!
//! ```ignore
//! let mut evm = built.with_precompiles(KoraPrecompiles::new(spec, hdc_state));
//! ```

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod agent_ns;
pub mod hdc;
pub mod hdc_index;
pub mod hdc_vector;
pub mod insight_event;
pub mod insight_id;
pub mod projection;
pub mod stigmergy;

pub use agent_ns::AGENT_NS_RESERVED;
// Re-export the primary type the executor needs.
pub use hdc::HDCPrecompiles as KoraPrecompiles;
pub use hdc::{HDC_PRECOMPILE_ADDRESS, HDCPrecompiles, HDCState, InsightPostedEvent};
pub use hdc_index::{HdcIndex, Hit, IndexedVector};
pub use hdc_vector::HdcVector;
pub use insight_event::{
    InsightConfirmedEvent, InsightPromotedEvent, decode_insight_confirmed, decode_insight_posted,
    decode_insight_promoted, insight_confirmed_topic0, insight_posted_topic0,
    insight_promoted_topic0,
};
pub use insight_id::{InsightId, KnowledgeKind};
pub use stigmergy::{
    KnowledgeKindCode, STIGMERGY_PRECOMPILE_ADDRESS, StigmergyPrecompile, StigmergyState, TierCode,
};
