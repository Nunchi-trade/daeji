//! State-sync and crash-recovery configuration.
//!
//! Controls the startup recovery pipeline: how many recent finalized blocks are
//! replayed or pre-populated into the in-memory snapshot cache, and how quickly
//! the node decides it needs to request blocks from peers (the sync threshold).
//!
//! The defaults are chosen to match the snapshot store's in-memory retention
//! window (`DEFAULT_MAX_PERSISTED_RETAINED = 64` in
//! `kora_consensus::components::snapshot`) so that a freshly restarted node
//! always has enough snapshots to participate in the current consensus round.

use serde::{Deserialize, Serialize};

/// Default number of recent finalized blocks to pre-populate into the
/// in-memory snapshot cache on startup.
///
/// 64 blocks matches the `DEFAULT_MAX_PERSISTED_RETAINED` constant in
/// `kora_consensus::components::snapshot`, ensuring the snapshot cache
/// mirrors what would normally be retained during live operation.
pub const DEFAULT_SNAPSHOT_PREPOPULATE_COUNT: u64 = 64;

/// Default number of blocks behind the network tip that triggers the
/// resolver's built-in backfill rather than a direct peer-sync download.
///
/// If the node is fewer than `sync_threshold` blocks behind, normal
/// consensus message propagation and the resolver's built-in backfill
/// are sufficient.  A larger gap indicates a crash recovery scenario
/// where the graduated-blocker catch-up path is needed.
pub const DEFAULT_SYNC_THRESHOLD: u64 = 10;

/// Configuration for state sync and crash recovery.
///
/// These settings control what happens between the archive-restore step
/// and the start of the Simplex consensus engine on every node restart.
///
/// **Phase 1 (local replay)** is active when the node restarts with an
/// intact finalized block archive on persistent storage. The node:
/// 1. Restores the QMDB checkpoint identified by the commit marker.
/// 2. Re-executes any blocks between the checkpoint and the archive head
///    (the "tail replay"), verifying state roots against the committed
///    block data.
/// 3. Pre-populates the in-memory snapshot cache with shallow snapshots
///    for the last `snapshot_prepopulate_count` blocks so that the first
///    consensus round after restart can find its parent snapshot without
///    entering catch-up mode.
///
/// **Phase 2 (peer sync)** -- downloading blocks from peers when no local
/// state is available -- is tracked by issue #3 and not yet implemented.
/// The `sync_threshold` field is reserved for that path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncConfig {
    /// Number of recent finalized blocks to pre-populate into the in-memory
    /// snapshot cache during startup recovery.
    ///
    /// After a restart the QMDB checkpoint is restored and the archive tail
    /// is re-executed.  This count controls how many *additional* shallow
    /// snapshots are inserted so that the consensus engine can immediately
    /// locate parent snapshots for blocks that arrived before finalization
    /// caught up.
    ///
    /// Setting this higher reduces the chance of entering catch-up mode
    /// after a brief outage at the cost of slightly longer startup time.
    /// The default of 64 matches the snapshot store's eviction window.
    ///
    /// Set to 0 to disable the pre-population step entirely (only the
    /// tail-replay snapshots will be available at startup).
    #[serde(default = "default_snapshot_prepopulate_count")]
    pub snapshot_prepopulate_count: u64,

    /// Minimum block gap that activates the graduated-blocker catch-up
    /// path for Phase 2 peer sync.
    ///
    /// If the local finalized height is more than `sync_threshold` blocks
    /// behind the network's finalized height, the node logs a warning that
    /// peer sync would be beneficial.  The threshold does *not* currently
    /// trigger an automatic download; it is intended as the trigger point
    /// for the future Phase 2 implementation.
    ///
    /// The default of 10 matches the resolver's built-in backfill capacity
    /// for short outages.
    #[serde(default = "default_sync_threshold")]
    pub sync_threshold: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            snapshot_prepopulate_count: DEFAULT_SNAPSHOT_PREPOPULATE_COUNT,
            sync_threshold: DEFAULT_SYNC_THRESHOLD,
        }
    }
}

const fn default_snapshot_prepopulate_count() -> u64 {
    DEFAULT_SNAPSHOT_PREPOPULATE_COUNT
}

const fn default_sync_threshold() -> u64 {
    DEFAULT_SYNC_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_are_correct() {
        let cfg = SyncConfig::default();
        assert_eq!(cfg.snapshot_prepopulate_count, DEFAULT_SNAPSHOT_PREPOPULATE_COUNT);
        assert_eq!(cfg.sync_threshold, DEFAULT_SYNC_THRESHOLD);
    }

    #[test]
    fn serde_roundtrip_defaults() {
        let cfg = SyncConfig::default();
        let serialized = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: SyncConfig = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn serde_roundtrip_custom() {
        let cfg = SyncConfig { snapshot_prepopulate_count: 128, sync_threshold: 25 };
        let serialized = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: SyncConfig = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn toml_roundtrip() {
        let cfg = SyncConfig { snapshot_prepopulate_count: 32, sync_threshold: 5 };
        let toml_str = toml::to_string_pretty(&cfg).expect("to_toml");
        let parsed: SyncConfig = toml::from_str(&toml_str).expect("from_toml");
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        // Only snapshot_prepopulate_count provided; sync_threshold should default.
        let toml_str = "snapshot_prepopulate_count = 256\n";
        let parsed: SyncConfig = toml::from_str(toml_str).expect("from_toml");
        assert_eq!(parsed.snapshot_prepopulate_count, 256);
        assert_eq!(parsed.sync_threshold, DEFAULT_SYNC_THRESHOLD);
    }

    #[test]
    fn zero_prepopulate_is_accepted() {
        let cfg = SyncConfig { snapshot_prepopulate_count: 0, ..Default::default() };
        let serialized = serde_json::to_string(&cfg).expect("serialize");
        let deserialized: SyncConfig = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.snapshot_prepopulate_count, 0);
    }
}
