# Issue 23: No Key Rotation -- Static Validator Set for Lifetime of Chain

**Severity**: High (blocks any production deployment)
**Component**: `crates/node/runner`, `crates/node/dkg`
**Labels**: `consensus`, `security`, `dkg`, `production-blocker`

---

## Summary

The validator set in Kora is permanently fixed at chain genesis. There is no mechanism to rotate threshold keys, add new validators, remove existing validators, or recover from validator key loss while the chain is running. The only way to change the validator set is to stop every node and restart the entire chain from scratch, losing all on-chain state. This makes the system unsuitable for any deployment where validators need to be maintained, replaced, or scaled over time.

---

## Problem Description

Kora uses a BLS12-381 threshold signature scheme for consensus, where a group of validators collectively sign blocks. The threshold keys are generated once during an initial Distributed Key Generation (DKG) ceremony that runs *before* the chain starts. Once the chain is running, those keys are used forever. The system has no concept of key expiration, epoch transitions, or validator set changes.

This creates several concrete problems:

1. **No validator replacement**: If a validator machine fails permanently, it cannot be replaced. The operator must provision a new machine with the exact same key material. If the key material is lost, that validator slot is gone forever.

2. **No key rotation**: Compromise of a validator's share key is permanent. There is no way to invalidate the compromised share and generate a new one. An attacker who obtains `threshold` shares can forge signatures for the lifetime of the chain.

3. **No scaling**: The validator set cannot grow or shrink. A 4-validator devnet cannot become a 7-validator testnet without a full chain restart.

4. **Permanent chain death**: If fewer than `threshold` validators remain operational (due to key loss, hardware failure, or operator departure), the chain halts permanently with no recovery path.

---

## Evidence in Code

### 1. Infinite Epoch Length

In `crates/node/runner/src/runner.rs`, line 55:

```rust
const EPOCH_LENGTH: u64 = u64::MAX;
```

This is used at line 521 to construct the epocher:

```rust
let epocher = FixedEpocher::new(NZU64!(EPOCH_LENGTH));
```

With `u64::MAX` as the epoch length, the first epoch boundary would be reached after 18,446,744,073,709,551,615 blocks. At 1 block per second, that is roughly 584 billion years. The epoch transition machinery in Commonware exists but will never fire.

### 2. ConstantSchemeProvider Ignores Epoch

In `crates/node/runner/src/runner.rs`, lines 89-103:

```rust
#[derive(Clone)]
struct ConstantSchemeProvider(Arc<ThresholdScheme>);

impl commonware_cryptography::certificate::Provider for ConstantSchemeProvider {
    type Scope = Epoch;
    type Scheme = ThresholdScheme;

    fn scoped(&self, _epoch: Epoch) -> Option<Arc<Self::Scheme>> {
        Some(self.0.clone())
    }

    fn all(&self) -> Option<Arc<Self::Scheme>> {
        Some(self.0.clone())
    }
}
```

The `scoped` method takes an `Epoch` parameter but discards it entirely (note the `_epoch` prefix). Every epoch -- if one were ever reached -- would use the identical threshold scheme. There is no storage for multiple schemes and no mechanism to register a new one.

### 3. DKG Hardcoded to Round 0

In `crates/node/dkg/src/protocol.rs`, lines 40-58, the `CeremonySession::new` constructor:

```rust
pub fn new(chain_id: u64, participants: &[ed25519::PublicKey], timestamp_nanos: u64) -> Self {
    // ... hash computation ...
    Self { ceremony_id, chain_id, round: 0 }
}
```

The `round` field is hardcoded to `0`. The `CeremonySession` struct *has* a `round` field (line 31), and the Commonware DKG library supports multiple rounds, but Kora never sets it to anything other than zero. There is no code path that would create a session with `round: 1` or higher.

Similarly, in `DkgParticipant::new` at line 367-375:

```rust
let info = Info::<MinSig, ed25519::PublicKey>::new::<N3f1>(
    format!("kora-dkg-{}", config.chain_id).as_bytes(),
    0,    // round 0 for initial DKG
    None, // no previous output
    Mode::default(),
    participants_set.clone(), // dealers
    participants_set,         // players
)
```

The `round` parameter is `0` and `previous output` is `None`. A key rotation DKG would need to reference the previous round's output to perform a resharing, but this is never done.

### 4. DKG Runs Only Before Chain Start

The `DkgCeremony` in `crates/node/dkg/src/ceremony.rs` is a standalone process that runs to completion before the chain starts. At line 58-61:

```rust
if DkgOutput::exists(&self.config.data_dir) {
    info!("DKG output already exists, loading from disk");
    return DkgOutput::load(&self.config.data_dir);
}
```

If DKG output already exists on disk, the ceremony short-circuits and returns the existing keys. There is no mechanism to trigger a new ceremony while consensus is running, no on-chain governance to propose a rekey, and no way for the running consensus engine to consume a new threshold scheme.

### 5. Single Scheme Loaded at Startup

In `crates/node/runner/src/scheme.rs`, `load_threshold_scheme` loads exactly one scheme from a single `output.json` + `share.key` pair:

```rust
pub fn load_threshold_scheme(data_dir: &Path) -> anyhow::Result<ThresholdScheme> {
    let output = DkgOutput::load(data_dir)?;
    // ... deserialize single scheme ...
    Ok(scheme)
}
```

There is no concept of loading multiple schemes, keyed by epoch or round number. The file format (`output.json`, `share.key`) has no versioning or epoch tagging.

### 6. Oracle Tracks Single Static Set

In `crates/node/runner/src/runner.rs`, line 337:

```rust
transport.oracle.track(0, TrackedPeers::new(validators, secondary)).await;
```

The P2P oracle is told about validators for epoch `0` only. If the validator set were to change, the oracle would need to be updated with the new set for the new epoch, but no code does this.

---

## What Commonware Already Provides

The underlying Commonware framework has infrastructure designed for exactly this use case, but Kora does not use it:

1. **Multi-round DKG**: The `round` field in `CeremonySession` and the `round` parameter in `Info::new` support multiple DKG rounds. The `previous output` parameter (currently `None`) is designed to accept the output of a prior round for resharing.

2. **Epoch-scoped Provider trait**: The `Provider` trait has `type Scope = Epoch` and a `scoped(&self, epoch: Epoch)` method, explicitly designed to return different schemes for different epochs.

3. **FixedEpocher with configurable length**: The `FixedEpocher` accepts any `NonZeroU64` for the epoch length. Setting this to a reasonable value (e.g., 100,000 blocks) would cause epoch transitions to actually occur.

4. **Oracle epoch tracking**: The `oracle.track(epoch, peers)` call accepts an epoch parameter, allowing different peer sets to be registered for different epochs.

These are not theoretical features; they are concrete API surfaces that exist in the dependency tree today but are bypassed by Kora's implementation choices.

---

## Impact Scenarios

### Scenario 1: Validator Key Compromise
A validator's `share.key` file is exfiltrated. The attacker now holds a valid share permanently. If the attacker obtains `threshold` shares total, they can forge block signatures. There is no revocation mechanism. The only remediation is to stop the chain entirely, run a new DKG, and restart -- losing the chain's history and state.

### Scenario 2: Validator Hardware Failure
A validator's disk fails and the `share.key` is unrecoverable (no backup). That validator can never participate again. The remaining validators continue operating only if their count still meets the threshold. If a second validator is lost, the chain may halt permanently.

### Scenario 3: Planned Validator Rotation
An operator wants to decommission an old validator and bring up a new one (routine infrastructure maintenance). This is impossible without a full chain restart. Every validator must coordinate simultaneously to stop, re-run DKG with the new participant set, and restart.

### Scenario 4: Network Growth
A devnet with 4 validators wants to add 3 more for increased decentralization. The only path is to stop the chain, re-run DKG with 7 participants, and restart from genesis.

---

## What Would Be Required

Implementing key rotation requires changes across several layers:

### 1. Define Real Epoch Boundaries

Replace the infinite epoch length with a meaningful value and implement epoch transition logic:

```rust
// Instead of:
const EPOCH_LENGTH: u64 = u64::MAX;

// Something like:
const EPOCH_LENGTH: u64 = 100_000; // ~1 day at 1 block/sec
```

The consensus engine needs to recognize epoch boundaries and coordinate the transition.

### 2. On-Chain DKG While Chain Is Running

The current DKG runs as a separate pre-chain process. For key rotation, a new DKG ceremony must execute while the current epoch's consensus is still active. This likely means:

- Running DKG messages over the existing P2P channels (rather than a separate network)
- Coordinating DKG start/completion relative to block heights
- Handling the case where the new DKG fails (the old epoch continues)

### 3. Epoch Handoff at Deterministic Block Height

All validators must agree on exactly which block starts the new epoch. The new threshold scheme must be activated at the same block height on every node, or the chain forks. This requires:

- Embedding the epoch transition decision in finalized blocks
- Ensuring the new scheme is available to all validators before the transition height
- Handling edge cases (what if a validator doesn't have the new scheme ready?)

### 4. Dynamic SchemeProvider

Replace `ConstantSchemeProvider` with a provider that stores multiple schemes keyed by epoch:

```rust
struct EpochSchemeProvider {
    schemes: BTreeMap<Epoch, Arc<ThresholdScheme>>,
}

impl Provider for EpochSchemeProvider {
    type Scope = Epoch;
    type Scheme = ThresholdScheme;

    fn scoped(&self, epoch: Epoch) -> Option<Arc<Self::Scheme>> {
        // Find the scheme for this epoch (or the most recent one before it)
        self.schemes.range(..=epoch).next_back().map(|(_, s)| s.clone())
    }
}
```

### 5. Validator Set Changes Between Epochs

Support adding and removing validators:

- New validators join the next DKG ceremony
- Departing validators are excluded from the next DKG
- The P2P oracle must be updated with the new validator set for the new epoch
- The `TrackedPeers` must reflect the new set

### 6. Multi-Round DKG Support

Update `CeremonySession::new` to accept a round parameter and pass the previous round's output:

```rust
let info = Info::new::<N3f1>(
    namespace,
    current_round,           // not hardcoded 0
    Some(previous_output),   // output from prior DKG
    Mode::default(),
    new_dealers,
    new_players,
);
```

### 7. Scheme Persistence with Epoch Tagging

The current `output.json` / `share.key` format stores a single scheme. This needs to support multiple schemes indexed by epoch, so a restarting node can load all schemes it needs to validate historical and current blocks.

---

## Files to Modify

| File | Change Required |
|------|----------------|
| `crates/node/runner/src/runner.rs` | Replace `EPOCH_LENGTH = u64::MAX` with a real value. Replace `ConstantSchemeProvider` with a dynamic provider. Update `oracle.track` to support multiple epochs. |
| `crates/node/runner/src/scheme.rs` | Extend `load_threshold_scheme` to support loading multiple schemes keyed by epoch. |
| `crates/node/dkg/src/protocol.rs` | Allow `CeremonySession::new` to accept a `round` parameter. Support passing previous DKG output for resharing. |
| `crates/node/dkg/src/ceremony.rs` | Support running a DKG ceremony while the chain is active, not just as a pre-chain bootstrap step. |
| `crates/node/dkg/src/config.rs` | Extend `DkgConfig` with epoch/round context and previous scheme reference. |
| `crates/node/dkg/src/output.rs` | Add epoch tagging to `DkgOutput`. Support storing and loading multiple outputs. |
| New: epoch manager | A new module to coordinate epoch transitions, trigger DKG ceremonies, and manage the handoff between schemes. |

---

## Cross-References

- **Issue 13** (DKG Silent Broadcast Failures): If the initial DKG fails, there is no recovery path. Key rotation would provide a secondary opportunity to establish keys, and a more robust DKG execution environment (running over the established P2P network rather than a bootstrap network).
- **Issue 15** (Docker Production Readiness): Production Docker deployments need the ability to rotate validators without downtime. The current architecture forces a full cluster restart for any validator change.

---

## Notes

This is arguably the single largest architectural gap between Kora's current state and production readiness. Every other issue -- crash recovery, transaction gossip, monitoring -- can be worked around or degraded gracefully. A static validator set with no key rotation is a hard blocker: it means any key compromise is permanent, any validator loss is irreversible, and any change to the validator set requires destroying and recreating the chain.

The good news is that Commonware has already designed the relevant abstractions (`Provider` trait scoped by `Epoch`, multi-round DKG support, epoch-aware oracle tracking). The work is in wiring these together within Kora's runner and building the coordination logic for epoch transitions.
