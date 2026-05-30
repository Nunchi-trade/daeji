# P2P Oracle Tracks Epoch 0 Only -- No Mechanism to Update Validator Set at Runtime

**Category**: enhancement
**Severity**: medium

## Summary

The production runner calls `transport.oracle.track(0, ...)` exactly once at startup, registering all validators for epoch 0 only. Because the epoch length is set to `u64::MAX` (effectively infinite), this works today but means there is no runtime mechanism to add, remove, or rotate validators in the P2P layer. Compromised validator keys cannot be revoked without a full cluster restart.

## Problem

Kora uses commonware's `discovery::Oracle` for P2P peer management. The oracle's `track()` method registers a set of peers for a specific epoch number. In the production runner, this is called exactly once at startup with epoch 0:

**File**: `crates/node/runner/src/runner.rs`, line 955:
```rust
transport.oracle.track(0, TrackedPeers::new(validators, secondary));
```

The epoch length is configured as `u64::MAX` (line 73 of the same file):
```rust
const EPOCH_LENGTH: u64 = u64::MAX;
```

This means:
1. There is effectively only one epoch that never ends, so the single `track(0, ...)` call suffices for now.
2. There is no code path that calls `track()` with any other epoch number.
3. If the epoch length were ever changed to a finite value, the oracle would have no peers registered for epoch 1+, causing complete P2P connectivity loss at the epoch boundary.

The same pattern appears in `LegacyNodeService::run_with_context()` in `crates/node/service/src/service.rs`, line 123:
```rust
transport.oracle.track(0, validator_set);
```

## Code Reference

**Production runner** -- `crates/node/runner/src/runner.rs:952-960`:
```rust
let validators = self.scheme.participants().clone();
let secondary = Set::from_iter_dedup(self.secondary_peers.iter().cloned());
let secondary_count = secondary.len();
transport.oracle.track(0, TrackedPeers::new(validators, secondary));
info!(
    validators = self.scheme.participants().len(),
    secondary_peers = secondary_count,
    "Registered primary and secondary peers with oracle"
);
```

**Legacy service** -- `crates/node/service/src/service.rs:118-125`:
```rust
let validators = self.config.consensus.build_validator_set()?;
if !validators.is_empty() {
    let validator_set: commonware_utils::ordered::Set<_> = validators
        .try_into()
        .map_err(|_| eyre::eyre!("failed to convert validator set"))?;
    transport.oracle.track(0, validator_set);
    tracing::info!("registered validators with oracle");
}
```

**Epoch length constant** -- `crates/node/runner/src/runner.rs:73`:
```rust
const EPOCH_LENGTH: u64 = u64::MAX;
```

## Impact

1. **No validator rotation at P2P level**: Even if key rotation is implemented at the DKG/consensus level (tracked in issue #94), the P2P transport oracle has no mechanism to update the tracked peer set. A compromised validator key remains trusted by the P2P layer indefinitely.

2. **No dynamic membership**: Adding or removing validators requires a coordinated cluster restart of all nodes, which means downtime and consensus interruption.

3. **Future epoch support is broken**: If a developer changes `EPOCH_LENGTH` to a finite value to enable epoch-based features, P2P connectivity will silently break at the first epoch boundary because no peers are registered for epoch 1.

4. **No revocation path**: In a security incident where a validator's private key is compromised, operators cannot remove the attacker from the P2P layer without restarting every node in the network.

## Root Cause

The P2P oracle tracking was implemented for the single-epoch devnet case (infinite epoch length). No infrastructure exists to update the tracked peer set at runtime, and no epoch transition logic invokes `track()` for subsequent epochs.

## Suggested Fix

**Short term** (documentation): Add a comment at the `EPOCH_LENGTH` constant explicitly warning that changing it to a finite value will break P2P connectivity unless dynamic oracle tracking is implemented. Ensure the epoch length remains `u64::MAX` until the infrastructure is ready.

**Medium term** (dynamic tracking API): Implement a method that the consensus layer can call during epoch transitions to update the oracle:

```rust
// Called by consensus when a new epoch begins
fn on_epoch_transition(&self, new_epoch: u64, new_validators: Vec<ed25519::PublicKey>) {
    let peers = TrackedPeers::new(
        Set::from_iter_dedup(new_validators.iter().cloned()),
        Set::default(),
    );
    self.transport.oracle.track(new_epoch, peers);
}
```

**Long term** (validator rotation): Coordinate DKG key rotation (#94) with P2P oracle updates, ensuring the transport layer always has the current validator set registered for the active epoch.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- add epoch transition logic or at minimum document the limitation
- `crates/node/service/src/service.rs` -- same pattern, same limitation

## Related Issues

- `103-dkg-resharing-tracking.md` -- DKG key rotation tracking (feature request)

## Labels

`enhancement`, `p2p`, `consensus`, `reliability`
