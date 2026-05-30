# NodeConfig::validate() Does Not Cross-Check Consensus Parameters

**Category**: configuration
**Severity**: medium
**Labels**: `config`, `reliability`, `enhancement`, `good first issue`

## Summary

The `NodeConfig::validate()` method performs only a single validation check (`worker_threads >= 1`). It does not validate any consensus parameters (threshold vs participant count), network addresses, RPC bind addresses, chain ID, or any other field. Invalid configuration values are not caught at startup time -- they only surface later as cryptic runtime errors, often deep in the consensus or networking stack, making them difficult to diagnose.

## Problem

In `crates/node/config/src/node.rs`, the `NodeConfig::validate()` method at lines 72-77 contains only one check:

```rust
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.worker_threads == 0 {
            return Err(ConfigError::InvalidValue("worker_threads must be >= 1".to_string()));
        }
        Ok(())
    }
```

This method is called from `NodeConfig::load()` at line 94:

```rust
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let config = path.map_or_else(
            || Ok(Self::default()),
            |p| { /* ... load from file ... */ },
        )?;
        config.validate()?;
        Ok(config)
    }
```

The following validations are missing:

1. **Consensus threshold vs participant count**: The `ConsensusConfig` at `crates/node/config/src/consensus.rs` has a `threshold` field (default: 2) and `participants` list. Setting `threshold > participants.len()` causes runtime failures in the consensus layer. Setting `threshold = 0` would cause division-by-zero panics. Note: In the current architecture, the threshold is recomputed from participant count via `N3f1::quorum()` at startup (see `cli.rs:217`), so the config `threshold` field is effectively dead code (see local file 057). However, if anyone reads this config value directly, it would be wrong.

2. **Participant key validity**: Invalid hex-encoded participant keys only fail when `ConsensusConfig::build_validator_set()` is called (at consensus startup), not at config load time. The error message from `build_validator_set()` ("InvalidParticipantKeyLength") is less helpful than a config-level "consensus.participants[2] has invalid key length" would be.

3. **Network listen address**: `network.listen_addr` is a string that is parsed later. An invalid address (e.g., "not-an-address") only fails when the P2P transport tries to bind.

4. **RPC bind address**: `rpc.http_addr` is parsed in `cli.rs:201` -- an invalid address fails there with a generic error rather than at config validation.

5. **Chain ID**: The default chain ID is `1` (Ethereum mainnet), set at `node.rs:11`:
   ```rust
   pub const DEFAULT_CHAIN_ID: u64 = 1;
   ```
   This collides with Ethereum mainnet. While this is a separate issue (local file 051), a validation check for `chain_id == 0` (which is invalid per EIP-155) would be valuable.

## Code Reference

`NodeConfig::validate()` in `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs` (lines 72-77):

```rust
    /// Validate configuration values.
    ///
    /// Returns an error if any value is out of range.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.worker_threads == 0 {
            return Err(ConfigError::InvalidValue("worker_threads must be >= 1".to_string()));
        }
        Ok(())
    }
```

`NodeConfig::load()` calling validate in `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs` (lines 83-96):

```rust
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let config = path.map_or_else(
            || Ok(Self::default()),
            |p| {
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("toml");
                match ext {
                    "json" => Self::from_json_file(p),
                    _ => Self::from_toml_file(p),
                }
            },
        )?;
        config.validate()?;
        Ok(config)
    }
```

`ConsensusConfig` definition in `/Users/will/dev/nunchi/daeji/crates/node/config/src/consensus.rs` (lines 142-178):

```rust
pub struct ConsensusConfig {
    pub validator_key: Option<PathBuf>,
    pub threshold: u32,
    pub participants: Vec<Vec<u8>>,
    pub block_codec: ConsensusBlockCodecConfig,
    pub simplex: ConsensusSimplexConfig,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            validator_key: None,
            threshold: DEFAULT_THRESHOLD,  // = 2
            participants: Vec::new(),
            block_codec: ConsensusBlockCodecConfig::default(),
            simplex: ConsensusSimplexConfig::default(),
        }
    }
}
```

## Impact

**Configuration errors surface as cryptic runtime failures instead of clear validation messages at startup.** Specific scenarios:

1. **threshold > participants**: An operator sets `threshold = 7` but only has 4 participants. The node starts and loads config successfully, then fails deep in the consensus engine with an opaque error about insufficient signatures.

2. **Invalid participant key**: A typo in a hex-encoded public key (`"abc123"` instead of a 32-byte hex string) passes config loading but fails at `build_validator_set()` later, with an error that does not identify which participant entry is wrong.

3. **Invalid listen address**: Setting `listen_addr = "foobar"` passes config loading but fails when the P2P transport attempts to bind, with a confusing "address parse error" that does not reference the config field name.

4. **Development footgun**: The default `chain_id = 1` means a fresh default config collides with Ethereum mainnet. While Kora is not an Ethereum mainnet client, tools and libraries that check chain ID may behave unexpectedly.

## Root Cause

The `validate()` method was written with minimal checks early in development and never extended as the configuration surface area grew. Each component validates its own config at runtime instead of having centralized validation at load time.

## Suggested Fix

Extend `NodeConfig::validate()` with comprehensive checks:

```rust
pub fn validate(&self) -> Result<(), ConfigError> {
    if self.worker_threads == 0 {
        return Err(ConfigError::InvalidValue("worker_threads must be >= 1".into()));
    }

    // Validate consensus config
    if !self.consensus.participants.is_empty() {
        if self.consensus.threshold == 0 {
            return Err(ConfigError::InvalidValue(
                "consensus.threshold must be >= 1".into()
            ));
        }
        if self.consensus.threshold > self.consensus.participants.len() as u32 {
            return Err(ConfigError::InvalidValue(format!(
                "consensus.threshold ({}) must not exceed participants count ({})",
                self.consensus.threshold, self.consensus.participants.len()
            )));
        }
        // Validate participant key lengths
        for (i, pk) in self.consensus.participants.iter().enumerate() {
            if pk.len() != 32 {
                return Err(ConfigError::InvalidValue(format!(
                    "consensus.participants[{}] has invalid key length {} (expected 32)",
                    i, pk.len()
                )));
            }
        }
    }

    // Validate chain_id
    if self.chain_id == 0 {
        return Err(ConfigError::InvalidValue("chain_id must not be 0".into()));
    }

    // Validate network listen address
    if self.network.listen_addr.parse::<std::net::SocketAddr>().is_err() {
        return Err(ConfigError::InvalidValue(format!(
            "network.listen_addr '{}' is not a valid socket address",
            self.network.listen_addr
        )));
    }

    // Validate RPC addresses
    if self.rpc.http_addr.parse::<std::net::SocketAddr>().is_err() {
        return Err(ConfigError::InvalidValue(format!(
            "rpc.http_addr '{}' is not a valid socket address",
            self.rpc.http_addr
        )));
    }

    Ok(())
}
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs` -- Extend `validate()` at line 72

## Related Issues

- Local file `057-config-consensus-threshold-dead-code.md` -- `consensus.threshold` config field is dead code (the threshold is always recomputed from participant count via N3f1)
- Local file `051-config-default-chain-id-collides-mainnet.md` -- Default chain_id = 1 collides with Ethereum mainnet
- Local file `050-config-validator-key-auto-generated-silently.md` -- Validator key is auto-generated silently if not found
