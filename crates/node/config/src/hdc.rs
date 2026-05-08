//! HDC (Hyperdimensional Computing) configuration.
//!
//! Controls whether the HDC precompile (address `0x09`) is active and
//! configures the on-chain HDC index that tracks InsightBoard events.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Configuration for the HDC subsystem.
///
/// When `enabled` is `true` (the default), the node installs the HDC precompile
/// at `precompile_address`, initialises an on-chain index with up to
/// `max_entries` vectors, and processes InsightBoard / PheromoneRegistry events
/// from finalized blocks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HdcConfig {
    /// Whether the HDC subsystem is active.
    ///
    /// Defaults to `true`. When disabled, no precompile is registered, no
    /// index is created, and finalized-block event processing is skipped.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// EVM address of the HDC precompile (hex string, e.g. `"0x09"`).
    ///
    /// Defaults to `"0x09"`.
    #[serde(default = "default_precompile_address")]
    pub precompile_address: String,

    /// Directory for the local knowledge store.
    ///
    /// Defaults to `{data_dir}/hdc/knowledge` at runtime.
    #[serde(default)]
    pub knowledge_store_path: Option<PathBuf>,

    /// Maximum number of insight vectors the on-chain index will hold.
    ///
    /// Once this limit is reached new `InsightPublished` events are ignored.
    /// Defaults to `100_000`.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
}

impl Default for HdcConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            precompile_address: default_precompile_address(),
            knowledge_store_path: None,
            max_entries: default_max_entries(),
        }
    }
}

const fn default_enabled() -> bool {
    true
}

fn default_precompile_address() -> String {
    "0x09".to_string()
}

const fn default_max_entries() -> usize {
    100_000
}
