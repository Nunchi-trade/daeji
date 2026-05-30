# No Validation That peers.json Participants Match DKG Output Participants

**Category:** config, correctness
**Severity:** medium

## Summary

During validator startup, the node loads DKG output (threshold signing shares and participant keys) from `output.json` and optionally loads bootstrap peer information from `peers.json`. There is no cross-validation between these two files. If an operator provides a `peers.json` from a different DKG ceremony or cluster, the node starts successfully but consensus will fail with cryptographic verification errors that are very difficult to diagnose.

## Problem

Kora is an EVM execution client that uses BLS threshold signatures for BFT consensus. Before starting consensus, each validator loads its DKG output (containing the group public key, share secret, and participant list) and a peers file (containing bootstrap addresses and participant public keys). These two files must refer to the same validator set for consensus to function.

In `bin/kora/src/cli.rs`, the `run_validator()` function (lines 154-239) loads both independently:

1. Lines 163-170: Load DKG output from the data directory
2. Lines 177-186: If `--peers` is provided, load bootstrap peers from the peers file

There is no check that the participant keys in the DKG output match the participant keys in the peers file. The node proceeds to join the network and attempt consensus with potentially mismatched configurations.

## Code Reference

File: `bin/kora/src/cli.rs`, lines 154-186 (relevant excerpt):

```rust
fn run_validator(&self, args: &ValidatorArgs) -> eyre::Result<()> {
    let mut config = self.load_config()?;

    // ...

    if !kora_dkg::DkgOutput::exists(&config.data_dir) {
        return Err(eyre::eyre!(
            "DKG output not found. Run 'kora dkg' first to generate threshold shares."
        ));
    }

    let dkg_output = kora_dkg::DkgOutput::load(&config.data_dir)?;
    tracing::info!(share_index = dkg_output.share_index, "Loaded DKG output");

    let scheme = load_threshold_scheme(&config.data_dir)
        .map_err(|e| eyre::eyre!("Failed to load threshold scheme: {}", e))?;
    tracing::info!("Loaded threshold signing scheme");

    let mut secondary_participants = Vec::new();
    if let Some(ref peers_path) = args.peers {
        let peers = load_peers(peers_path)?;
        config.network.bootstrap_peers = format_bootstrappers(&peers.bootstrappers);
        tracing::info!(
            bootstrap_peers = config.network.bootstrap_peers.len(),
            "Loaded bootstrap peers from peers.json"
        );

        secondary_participants = peers.secondary_participants;
    }
    // ...
    // NO CROSS-VALIDATION between dkg_output.participant_keys and peers.participants
```

The `DkgOutput` struct (in `crates/node/dkg/src/output.rs`, line 28) contains a `participant_keys: Vec<Vec<u8>>` field that lists all participant public keys from the DKG ceremony. The `load_peers()` function (in `bin/kora/src/cli.rs`, line 400) returns a `PeersInfo` struct with a `participants` field containing ed25519 public keys. These two sets of keys should be compared.

Note: The DKG output contains BLS public keys (used for threshold signing) while peers.json contains ed25519 public keys (used for P2P identity). A direct key comparison is not possible, but the participant count and the order/mapping should be validated.

## Impact

1. **Silent consensus failure**: The node starts, connects to peers, but every block it receives will fail BLS threshold signature verification. The error manifests deep in the commonware simplex consensus layer as "signature verification failed" or similar cryptographic errors -- not as a configuration mismatch, making root cause analysis extremely difficult.

2. **Resource waste**: The misconfigured node consumes CPU, memory, and network bandwidth while contributing nothing to consensus. It may even degrade the network by occupying peer connection slots.

3. **Operational trap**: This scenario is easy to trigger in practice. An operator who runs multiple clusters (e.g., staging and production) might copy the wrong `peers.json` into a node's data directory. Since both files are individually valid, no error is raised until consensus starts failing.

## Root Cause

The startup code loads the DKG output and the peers file as completely independent data sources. Each file is validated in isolation (DKG output checks share structure, peers file parses public keys), but no cross-validation ensures they describe the same validator set.

## Suggested Fix

After loading both files, validate that participant counts match (since direct key type comparison between BLS and ed25519 is not possible):

**Before** (`bin/kora/src/cli.rs`, after line 186):
```rust
// (no validation)
```

**After**:
```rust
if let Some(ref peers_path) = args.peers {
    let peers = load_peers(peers_path)?;
    config.network.bootstrap_peers = format_bootstrappers(&peers.bootstrappers);

    // Cross-validate: DKG participant count must match peers.json participant count
    if dkg_output.participants != peers.participants.len() {
        return Err(eyre::eyre!(
            "DKG output has {} participants but peers.json has {} participants. \
             Ensure both files are from the same DKG ceremony.",
            dkg_output.participants,
            peers.participants.len()
        ));
    }

    secondary_participants = peers.secondary_participants;
}
```

For a stronger check, the DKG output could also store the ed25519 identity keys alongside the BLS keys, enabling a full key-set comparison.

## Files to Modify

- `bin/kora/src/cli.rs` -- add participant count cross-validation in `run_validator()` after loading both files

## Related Issues

- `055-docker-no-config-toml-in-entrypoint.md` -- the Docker entrypoint always passes `--peers`, making this scenario possible if shared config volumes are mixed up between clusters

## Labels

bug, correctness, config, good first issue
