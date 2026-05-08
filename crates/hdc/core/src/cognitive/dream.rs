//! Dream cycle — offline consolidation (NREM) and creative recombination (REM).

/// Configuration for the dream cycle.
#[derive(Debug)]
pub struct DreamConfig {
    /// Maximum episodes to process in NREM phase.
    pub max_nrem_episodes: usize,
    /// Number of random cross-bindings in REM phase.
    pub rem_cross_bindings: usize,
    /// Similarity threshold for near-duplicate merging.
    pub merge_threshold: f64,
    /// Similarity threshold for REM resonance detection.
    pub resonance_threshold: f64,
    /// GC threshold for entry retention.
    pub gc_threshold: f64,
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            max_nrem_episodes: 500,
            rem_cross_bindings: 100,
            merge_threshold: 0.95,
            resonance_threshold: 0.65,
            gc_threshold: 0.01,
        }
    }
}

/// Report produced by a dream cycle, for logging and telemetry.
#[derive(Debug)]
pub struct DreamReport {
    /// Number of entries re-encoded during NREM.
    pub nrem_re_encoded: usize,
    /// Number of near-duplicates merged during NREM.
    pub nrem_merged: usize,
    /// Number of entries promoted to a higher tier during NREM.
    pub nrem_promoted: usize,
    /// Number of entries garbage-collected during NREM.
    pub nrem_garbage_collected: usize,
    /// Number of random cross-bindings attempted during REM.
    pub rem_cross_bindings_attempted: usize,
    /// Number of resonances detected during REM.
    pub rem_resonances_found: usize,
    /// Number of new insights created from REM resonances.
    pub rem_insights_created: usize,
    /// Wall-clock duration of the dream cycle in milliseconds.
    pub duration_ms: u64,
    /// true if alarming patterns were detected -> transition to CAUTIOUS.
    pub alarming: bool,
}

impl DreamReport {
    /// Create a new empty report.
    pub const fn empty() -> Self {
        Self {
            nrem_re_encoded: 0,
            nrem_merged: 0,
            nrem_promoted: 0,
            nrem_garbage_collected: 0,
            rem_cross_bindings_attempted: 0,
            rem_resonances_found: 0,
            rem_insights_created: 0,
            duration_ms: 0,
            alarming: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = DreamConfig::default();
        assert_eq!(config.max_nrem_episodes, 500);
        assert_eq!(config.rem_cross_bindings, 100);
        assert!((config.merge_threshold - 0.95).abs() < 1e-9);
        assert!((config.resonance_threshold - 0.65).abs() < 1e-9);
        assert!((config.gc_threshold - 0.01).abs() < 1e-9);
    }

    #[test]
    fn empty_report() {
        let report = DreamReport::empty();
        assert_eq!(report.nrem_re_encoded, 0);
        assert!(!report.alarming);
    }
}
