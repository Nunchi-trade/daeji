# DKG Resharing: Static Validator Set Cannot Be Changed After Genesis

**Category**: dkg, consensus
**Severity**: high

**Labels**: `enhancement`, `dkg`, `consensus`, `security`

## Summary

Kora's DKG (Distributed Key Generation) is currently a one-time pre-genesis setup with no resharing capability. This means the validator set is permanently fixed at genesis time -- validators cannot be added, removed, or rotated after the network starts. The threshold signing key shares are frozen forever, increasing exposure to long-term key compromise. Additionally, without resharing as a Commonware actor, the DKG ceremony cannot use authenticated transport, forcing the use of raw TCP for key share transmission (audit issue #251, a P0 security vulnerability).

## Problem

### One-time ceremony architecture

The DKG crate (`crates/node/dkg/`) is designed as a standalone pre-genesis tool. It performs a one-shot ceremony and produces static key shares:

- **Trusted dealer mode** (`bin/keygen/src/dkg_deal.rs`): A single trusted party generates all shares offline. Used for fast devnet setup.
- **Interactive mode** (`crates/node/dkg/src/ceremony.rs`): Validators run an interactive Feldman-DeSmedt DKG ceremony via the network. Used for production.

Both modes produce output that is consumed once at genesis and never updated:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs:868-876
Ok(DkgOutput {
    group_public_key: group_key_bytes,
    public_polynomial: polynomial_bytes,
    threshold: self.config.t(),
    participants: self.config.n(),
    share_index: usize::from(share.index) as u32,
    share_secret: share_bytes,
    participant_keys,
})
```

### No mechanism for validator set changes

There is no onchain validator registry, no epoch-based resharing protocol, and no mechanism to trigger a new ceremony after genesis. The consensus engine uses a fixed validator set loaded at startup.

### Transport uses raw TCP for DKG

The DKG transport layer (`crates/node/dkg/src/transport.rs`) uses Commonware's authenticated discovery network, which provides encryption:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/dkg/src/transport.rs:1-5
//! Production DKG transport using commonware-p2p authenticated discovery.
//!
//! This module provides authenticated, encrypted channels for DKG ceremony messages
//! using the commonware-p2p authenticated discovery network.
```

However, the DKG is run as a standalone binary (`keygen`) rather than as a Commonware actor within the main node process. This means it must establish its own network connections, and the P2P overlay may not be fully established before ceremony messages are sent (hence the hardcoded 10-second wait in `ceremony.rs:404`).

### Fixed epoch length in consensus

The consensus engine uses `FixedEpocher(u64::MAX)`, meaning the entire network lifetime is a single epoch:

```rust
// /Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs:44
const EPOCH_LENGTH: u64 = u64::MAX;
```

Without epoch boundaries, there is no natural point at which the validator set could be rotated.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` -- One-shot ceremony runner
**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs`, lines 868-876 -- Static `DkgOutput` produced once
**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/transport.rs`, lines 1-5 -- Transport description
**File**: `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs`, line 44 -- `EPOCH_LENGTH = u64::MAX`

## Impact

- **No validator rotation**: Validators cannot be added or removed after genesis. If a validator's hardware fails permanently, the network must operate with a reduced validator set indefinitely. If enough validators fail, the network loses liveness.
- **No key rotation**: The threshold signing key shares are fixed forever. Long-running networks face increasing risk of key compromise through side-channel attacks, operational errors, or insider threats. This is audit issue #94 (no key rotation) and #283 (static validator set).
- **Security vulnerability**: Without resharing as a Commonware actor, the DKG ceremony must use standalone networking. While the transport uses authenticated discovery, the ceremony is a one-shot operation with no retry or rotation capability. Audit issue #251 (DKG cleartext TCP) identifies this as P0 severity.
- **Blocked production readiness**: Dynamic validator sets are a prerequisite for any production deployment. No production blockchain operates with a fixed validator set.

## Root Cause

The DKG crate was written as a standalone pre-genesis tool to get the network bootstrapped quickly, rather than as a Commonware `Actor` integrated into the node lifecycle. The existing code performs a one-shot ceremony and produces static key shares. There is no mechanism to:
1. Trigger a new ceremony at epoch boundaries
2. Coordinate share redistribution to a new validator set
3. Update the consensus engine's validator set at runtime
4. Register or deregister validators onchain

## Suggested Fix

### Architecture proposal

**1. Onchain validator registry via PoA system contract**: A `ValidatorManager` EVM system contract tracks validators added/removed per epoch. For each validator, it stores a moniker, IP address, public key, and fee recipient. This keeps validator-set decisions onchain, avoiding the need to split state or block data between offchain and onchain sources, and preserves block fee distribution logic.

**2. Resharing via actor pattern**: Refactor the `dkg` crate into two actors:
- **DKG Actor**: Responsible for coordinating the resharing protocol and networking. Uses Commonware's authenticated transport (solving #251). Fits the upstream reshare examples from Commonware.
- **Orchestrator Actor**: Manages the lifecycle of consensus engines across epochs. Handles transitions between validator sets, including starting/stopping simplex instances.

**3. Trusted initial setup (recommended)**: Keep the initial DKG ceremony as an offchain trusted setup rather than running it onchain. An onchain initial ceremony would mean the first epoch does not use threshold signing, causing downstream complexity (the storage engine would need different certificate types for initial blocks). The marginal trust reduction does not justify the complexity.

### Key parameters to configure

- Epoch length (blocks per epoch before resharing triggers)
- Resharing threshold and quorum requirements
- Validator registration/deregistration cooldown periods
- Fee recipient update mechanism

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/` -- Refactor into actor pattern with resharing support
- `/Users/will/dev/nunchi/daeji/bin/keygen/` -- Update to work with new actor-based DKG, may become a thin CLI wrapper
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- Integrate Orchestrator actor for epoch transitions; change `FixedEpocher(u64::MAX)` to a configurable epoch length
- New: `ValidatorManager` system contract (Solidity + deployment mechanism)
- New: Orchestrator actor crate for consensus engine lifecycle management across epochs

## Related Issues

- `091-dkg-timestamp-coordination-fragile.md` -- Timestamp coordination (would be addressed in resharing actor design)
- `102-audit-tracking-49-issues.md` -- Audit tracking (#94 no key rotation, #251 DKG cleartext TCP, #283 static validator set)
