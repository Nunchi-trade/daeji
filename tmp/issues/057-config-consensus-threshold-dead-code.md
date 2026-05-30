# ConsensusConfig.threshold Field Is Dead Code -- Never Used at Runtime

**Category:** config, correctness
**Severity:** low

## Summary

The `ConsensusConfig` struct has a `threshold` field (default value: `2`) that is serialized/deserialized in configuration files but never read by any runtime code path. The actual consensus quorum is always dynamically computed from the validator count using `N3f1::quorum(participant_count)`, which implements the standard BFT formula `floor(2n/3) + 1`. An operator who sets `threshold = 5` in their config file will have that value silently ignored, potentially giving a false sense of stronger finality guarantees.

## Problem

Kora uses commonware simplex BFT consensus, which requires a quorum of `n - f` validators where `f = floor((n-1)/3)`. This quorum is always computed dynamically, not read from configuration.

The `ConsensusConfig` struct (in `crates/node/config/src/consensus.rs`, lines 142-167) includes a `threshold` field:

```rust
pub struct ConsensusConfig {
    pub validator_key: Option<PathBuf>,
    #[serde(default = "default_threshold")]
    pub threshold: u32,
    pub participants: Vec<Vec<u8>>,
    pub block_codec: ConsensusBlockCodecConfig,
    pub simplex: ConsensusSimplexConfig,
}
```

The default value is `2` (line 15: `pub const DEFAULT_THRESHOLD: u32 = 2;`).

Searching the codebase for runtime uses of this field yields only:
1. **E2E test harness** (`crates/e2e/src/harness.rs`, line 142): used in a `tracing::info!` log message, not to configure consensus
2. **E2E test config** (`crates/e2e/src/setup.rs`, line 67): computed from validator count (same formula as N3f1), used only for the log message above
3. **DKG output** (`crates/node/dkg/src/output.rs`, line 55): the DKG output struct has its own `threshold` field that is always recomputed from `N3f1::quorum(output.participants)` on load (line 95-102)
4. **Serialization tests**: the config field is tested for serialization roundtrips but never for runtime effect

The `load_peers()` function in `bin/kora/src/cli.rs` (line 397) documents this directly:

```rust
/// Accepts peers.json files with either "quorum" (new format) or "threshold"
/// (legacy format) key -- both are ignored at runtime since the quorum is
/// always computed from the validator count via N3f1.
```

The actual quorum computation happens in `run_validator()` (line 217) and `run_dkg()` (line 119):

```rust
let quorum = N3f1::quorum(validator_count as usize);
```

## Code Reference

File: `crates/node/config/src/consensus.rs`, lines 148-150 (the unused field):

```rust
    /// Threshold for consensus (e.g., 2f+1 of 3f+1).
    #[serde(default = "default_threshold")]
    pub threshold: u32,
```

File: `crates/node/config/src/consensus.rs`, lines 15-16 and 200-202 (the default constant):

```rust
/// Default validator threshold.
pub const DEFAULT_THRESHOLD: u32 = 2;

// ...

const fn default_threshold() -> u32 {
    DEFAULT_THRESHOLD
}
```

File: `bin/kora/src/cli.rs`, lines 217-225 (where the actual quorum is computed from validator count, ignoring config.threshold):

```rust
let quorum = N3f1::quorum(validator_count as usize);
tracing::info!(
    validator_count = validator_count,
    quorum = quorum,
    max_faulty = validator_count - quorum,
    "Consensus requires {} of {} validators active (N3f1 BFT)",
    quorum,
    validator_count
);
```

File: `crates/node/dkg/src/output.rs`, lines 93-102 (DKG output also recomputes threshold, ignoring the persisted value):

```rust
// Always compute the correct quorum from N3f1 rather than trusting
// the persisted threshold value, which may be wrong in old output files.
let correct_threshold = N3f1::quorum(output.participants);

Ok(Self {
    // ...
    threshold: correct_threshold,
    // ...
})
```

## Impact

1. **False security guarantees**: An operator deploying a 10-validator chain might set `threshold = 8` in their config, believing they require 8-of-10 agreement for finality. In reality, the BFT quorum is `10 - floor(9/3) = 7`, and the config value is completely ignored. The operator's security assumptions about the chain are wrong.

2. **Configuration audit confusion**: During security audits or operational reviews, the presence of a `threshold` field in the config file (and its default of `2`) may raise false alarms. A reviewer might question why the threshold is set to "only 2" without realizing it is dead code.

3. **Dead code maintenance burden**: The constant, default function, serde annotation, and test assertions for this field all need to be maintained even though the field serves no runtime purpose.

## Root Cause

The `threshold` field was part of an earlier design where the consensus quorum was configurable. When the quorum calculation was standardized to `N3f1`, the runtime code was updated to always compute the quorum dynamically, but the config field was not removed -- likely to maintain backward compatibility with existing config files.

## Suggested Fix

**Option 1 (recommended)**: Remove the field entirely and add `#[serde(deny_unknown_fields)]` to prevent silent ignoring of old config files that still include it.

**Before** (`crates/node/config/src/consensus.rs`, lines 148-150):
```rust
    /// Threshold for consensus (e.g., 2f+1 of 3f+1).
    #[serde(default = "default_threshold")]
    pub threshold: u32,
```

**After**:
```rust
    // REMOVED: threshold field was dead code. Quorum is always computed
    // from N3f1::quorum(participant_count).
```

**Option 2**: If backward compatibility with existing config files is required, keep the field but log a warning at startup if it is set to a non-default value:

```rust
if config.consensus.threshold != DEFAULT_THRESHOLD {
    tracing::warn!(
        configured_threshold = config.consensus.threshold,
        actual_quorum = quorum,
        "consensus.threshold config value is ignored; quorum is always \
         computed from N3f1::quorum(validator_count)"
    );
}
```

## Files to Modify

- `crates/node/config/src/consensus.rs` -- remove `threshold` field, `DEFAULT_THRESHOLD` constant, and `default_threshold()` function
- `crates/e2e/src/setup.rs` -- remove `threshold` from `TestConfig` (or keep as computed-only)
- `crates/e2e/src/harness.rs` -- update log message
- `crates/node/dkg/src/output.rs` -- remove `threshold` field from `DkgOutput` struct (it is already recomputed on load)

## Related Issues

- `053-startup-no-peers-dkg-validation.md` -- another configuration validation gap at startup

## Labels

bug, config, good first issue
