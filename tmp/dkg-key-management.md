# DKG and Key Management

## What is Kora?

Kora is a blockchain node built on the Commonware consensus framework. It uses the Simplex BFT protocol where validators collaboratively produce blocks. A critical component of Simplex is leader election: for each consensus view, exactly one validator is selected to propose a block. Kora uses a threshold Verifiable Random Function (VRF) for this purpose, which requires all validators to jointly hold shares of a distributed secret key.

The process by which validators create this shared key -- without any single party ever learning the full secret -- is called Distributed Key Generation (DKG). This document covers how Kora implements DKG, how the resulting keys are stored and used, and what the current limitations are.

---

## Why DKG Matters

In Kora's consensus:

1. **Leader election must be unpredictable** -- no validator should be able to predict or manipulate who proposes the next block.
2. **Leader election must be verifiable** -- all validators must agree on who was selected.
3. **No single point of failure** -- the randomness source must survive even if some validators are offline or malicious.

A threshold VRF satisfies all three requirements. It produces deterministic, unpredictable randomness that requires a quorum (t-of-n) of validators to generate. The DKG ceremony creates the threshold key shares that make this possible.

---

## Key Types in Kora

| Key | Algorithm | File on Disk | Purpose |
|-----|-----------|--------------|---------|
| Identity key | Ed25519 | `{data_dir}/validator.key` (32-byte raw seed) | P2P authentication, DKG participation, message signing |
| Group public key | BLS12-381 (MinSig variant) | `{data_dir}/output.json` (hex-encoded) | Verifying aggregated threshold signatures |
| Public polynomial | BLS12-381 commitments | `{data_dir}/output.json` (hex-encoded) | Verifying individual share correctness |
| Secret share | BLS12-381 | `{data_dir}/share.key` (JSON with hex-encoded secret) | Producing partial VRF signatures |

The Ed25519 identity key is the validator's network identity -- used for P2P communication, DKG message authentication, and peer identification. The BLS12-381 keys are exclusively for consensus (threshold VRF signatures).

---

## What is DKG: Distributed Key Generation

DKG is a multi-party protocol where `n` participants jointly create:
- A **group public key** that can verify signatures produced by any `t` participants cooperating.
- Individual **secret shares** such that no single participant knows the full secret, but any `t` participants can reconstruct it.

Kora uses **Joint-Feldman DKG** over the BLS12-381 elliptic curve (specifically the MinSig variant from the `commonware-cryptography` crate). In Joint-Feldman, every participant simultaneously acts as both:
- A **dealer**: generating a random polynomial and distributing encrypted shares to all other participants.
- A **player**: receiving and verifying shares from all other dealers.

The result is that the group key is the sum of all individual dealer contributions, making it impossible for any single dealer to bias the output.

---

## Two Modes of Key Generation

### Mode 1: Interactive DKG Ceremony (Production)

This is the full Joint-Feldman DKG protocol implemented in `crates/node/dkg/src/`. All validators participate over the network, and no single party has access to the complete secret.

### Mode 2: Trusted Dealer (Development Only)

A single node generates all shares locally using `dkg::deal()` and distributes them to validators via the filesystem. This is fast (sub-second) but fundamentally insecure -- the dealer knows the full secret and could forge threshold signatures alone.

Implemented in `bin/keygen/src/dkg_deal.rs`.

---

## The Interactive DKG Protocol

### Protocol Overview

The interactive DKG is orchestrated by `DkgCeremony` (`crates/node/dkg/src/ceremony.rs`) and uses `DkgParticipant` (`crates/node/dkg/src/protocol.rs`) for the cryptographic state machine.

### Phase 1: Dealer Start

Each validator generates a random polynomial of degree `t-1` using `OsRng` (operating system entropy):

```rust
// crates/node/dkg/src/protocol.rs, line 434
let (dealer, pub_msg, priv_msgs) = Dealer::<MinSig, ed25519::PrivateKey>::start::<N3f1>(
    &mut rng,
    self.info.clone(),
    self.config.identity_key.clone(),
    None, // no previous share (initial DKG)
)
```

This produces:
- A `DealerPubMsg` containing the polynomial commitments (broadcast to all participants).
- A `DealerPrivMsg` for each participant containing their encrypted share (sent privately to each).

The public message is broadcast. Private messages are sent individually.

### Phase 2: Collecting Messages and Sending Acknowledgments

Each validator waits to receive both the public commitment and private share from every other validator. Upon receiving both from a given dealer, the player verifies the share against the public commitment:

```rust
// crates/node/dkg/src/protocol.rs, line 685
if let Some(ack) = player.dealer_message::<N3f1>(dealer.clone(), pub_msg, priv_msg) {
    // Send ack back to dealer
}
```

If verification succeeds, a `PlayerAck` is sent back to the dealer. If verification fails, no ack is sent (the player silently rejects that dealer).

**Timeout**: 120 seconds (`PHASE2_MAX_TIMEOUT_SECS`). If not all dealer messages are received within this window, the ceremony fails with `DkgError::Timeout`.

### Phase 2.5: Ready Synchronization

After sending all acks, each validator broadcasts a `Ready` message. The ceremony waits until all `n` participants have signaled ready before proceeding. This prevents a race condition where a dealer finalizes before receiving all acks.

**Timeout**: 120 seconds (same constant as Phase 2).

### Phase 3: Dealer Finalization

Once all nodes are ready, each validator finalizes its own dealer instance:

```rust
// crates/node/dkg/src/protocol.rs, line 711
let signed_log = dealer.finalize::<N3f1>();
```

This creates a `SignedDealerLog` -- a cryptographic transcript of which players acknowledged the dealer's shares. The signed log is broadcast to all participants.

### Phase 4: Collecting Dealer Logs

All validators collect signed dealer logs. The leader (validator at index 0) acts as a coordinator: non-leaders can request all collected logs from the leader via `RequestLogs` / `AllLogs` messages.

**Timeout**: 120 seconds (`PHASE4_MAX_TIMEOUT_SECS`). If insufficient logs are collected, the ceremony fails.

**Required logs**: For the initial DKG, all `n` dealer logs are required (`required_dealer_logs()` returns `config.n()`).

### Phase 5: Finalization

With all dealer logs collected, each validator finalizes:

```rust
// crates/node/dkg/src/protocol.rs, line 842
let (output, share) = player
    .finalize::<N3f1, ed25519::Batch>(&mut rng, logs, &Sequential)
```

This produces:
- `output`: The group public key and public polynomial (same for all validators).
- `share`: This validator's secret share (unique per validator, includes a 1-indexed `share.index`).

The output is saved to disk as `output.json` and `share.key`.

---

## The Trusted Dealer Shortcut

For development environments (Docker devnet), a single command generates all key material:

```bash
cargo run --release -p keygen -- dkg-deal --validators 4 --threshold 3 --output-dir ./testnet-artifacts
```

Implementation in `bin/keygen/src/dkg_deal.rs`:

```rust
// line 91-93
let (public_output, shares) =
    dkg::deal::<MinSig, _, N3f1>(&mut rng, Mode::default(), participants_set)
```

`dkg::deal()` is a Commonware utility that simulates the entire DKG protocol locally. It:
1. Generates a single random polynomial.
2. Evaluates the polynomial at each participant's index to produce shares.
3. Returns the public output (group key + polynomial) and all shares.

The tool then writes `output.json` and `share.key` into each node's directory.

### Security Implications of Trusted Dealer Mode

- The dealer machine holds all secret shares simultaneously. Anyone with access to that machine (or its memory/disk during generation) can reconstruct the full group secret.
- A compromised dealer key allows forging any threshold signature, including VRF outputs, which would allow manipulating leader election.
- The generated shares are written to disk unencrypted. If the output directory is network-accessible, all shares could be exfiltrated.
- This mode should NEVER be used in production or on untrusted infrastructure.

---

## How Shares Are Stored

### output.json (Per-Validator, Identical Content)

```json
{
  "group_public_key": "hex-encoded BLS12-381 public key",
  "public_polynomial": "hex-encoded commitment polynomial",
  "threshold": 3,
  "participants": 4,
  "participant_keys": ["hex-encoded ed25519 pubkey", ...]
}
```

### share.key (Per-Validator, Unique Content)

```json
{
  "index": 1,
  "secret": "hex-encoded BLS12-381 secret share"
}
```

The share index is 1-indexed (derived from `share.index` which is a `NonZeroU32` in the Commonware library).

These files are stored in the validator's `data_dir` (the same directory as `validator.key`).

---

## The Startup Sequence

When a Kora validator starts (`crates/node/runner/src/scheme.rs`):

1. **Load DKG output**: `DkgOutput::load(data_dir)` reads `output.json` and `share.key`.
2. **Decode participant keys**: Each hex-encoded Ed25519 public key is deserialized into the participant set.
3. **Decode public polynomial**: The BLS12-381 `Sharing` (polynomial commitments) is deserialized.
4. **Decode secret share**: The BLS12-381 `Share` is deserialized.
5. **Create threshold scheme**: `bls12381_threshold::Scheme::signer()` constructs the signing scheme.

```rust
// crates/node/runner/src/scheme.rs, line 49-51
let scheme = bls12381_threshold::Scheme::signer(
    SIMPLEX_NAMESPACE,     // b"_COMMONWARE_KORA_SIMPLEX"
    participants_set,       // ordered set of ed25519 public keys
    group_poly,            // public polynomial from output.json
    share,                 // secret share from share.key
)
```

If `output.json` or `share.key` do not exist, the validator cannot start consensus.

### DKG Must Complete Before Consensus

The `ThresholdScheme` is a required input to the `ProductionRunner`. Without it, the consensus engine cannot produce partial VRF signatures for leader election. The startup sequence is:

```
keygen setup (generates Ed25519 keys + peers.json)
    |
    v
DKG ceremony (interactive) OR keygen dkg-deal (trusted dealer)
    |
    v
load_threshold_scheme() -- creates ThresholdScheme from output files
    |
    v
ProductionRunner::run() -- starts Simplex consensus
```

---

## The BLS12-381 Threshold VRF

### How Leader Election Works

During each consensus finalization:

1. Each validator produces a **partial VRF signature** over the current round's seed using their secret share.
2. Once `t` partial signatures are collected, they are **aggregated** into a full threshold signature.
3. The threshold signature is deterministic for any `t`-subset (a property of Shamir secret sharing over BLS12-381).
4. The VRF output is hashed with keccak256 to produce the block's **seed**.
5. The seed is stored as the block's `prevrandao` field and used by the `Random` elector to select the leader for the next view.

```rust
// crates/node/runner/src/runner.rs, line 149
fn seed_hash(seed: impl commonware_codec::Encode) -> B256 {
    keccak256(seed.encode())
}
```

### Properties

- **Unpredictable**: No coalition of fewer than `t` validators can predict the VRF output before `t` partial signatures are revealed.
- **Verifiable**: Anyone with the group public key can verify the threshold signature.
- **Deterministic**: The same input always produces the same VRF output, regardless of which `t` validators participated.
- **Bias-resistant**: No single validator can manipulate the output (they can only withhold their partial signature, which stalls consensus rather than biasing randomness).

### Namespace

The signing namespace is `b"_COMMONWARE_KORA_SIMPLEX"` (defined in `crates/node/runner/src/scheme.rs`, line 19). This provides domain separation so that VRF signatures cannot be replayed across different Commonware-based protocols.

---

## Key Rotation: Not Currently Supported

**Current state**: The validator set is fixed for the lifetime of the chain.

Evidence:
- `EPOCH_LENGTH = u64::MAX` in `crates/node/runner/src/runner.rs` (line 55) -- effectively infinite; no epoch boundary is ever reached.
- `ConstantSchemeProvider` (runner.rs, lines 90-103) returns the same `ThresholdScheme` for all epochs, ignoring the epoch parameter entirely.
- The DKG ceremony uses `round: 0` exclusively (`CeremonySession::new()` hardcodes round 0).
- There is no mechanism to trigger a new DKG ceremony after the chain is running.

### What Would Be Required for Key Rotation

1. Define epoch boundaries (e.g., at specific block heights).
2. Implement a new DKG ceremony that runs while the current epoch is still active.
3. Handle the handoff: the new scheme must take effect at a deterministic block height agreed by all validators.
4. Support validator set changes (adding/removing validators between epochs).
5. Update `ConstantSchemeProvider` to return epoch-appropriate schemes.

### Infrastructure That Exists

- The `commonware-cryptography` library supports multiple DKG rounds (the `round` field in `CeremonySession`).
- The `Provider` trait in Commonware's certificate system is scoped by `Epoch`, allowing different schemes per epoch.
- The `FixedEpocher` used in the runner could be replaced with a dynamic epocher.

---

## What Happens if a Key is Lost

If a validator loses its `share.key` file:

- That validator can **never rejoin the current validator set**. There is no way to reconstruct an individual share without re-running the entire DKG ceremony with all participants.
- If fewer than `t` validators remain operational (because others also lost keys or went offline permanently), the chain **halts permanently** -- consensus can never finalize another block.
- The only recovery path is to coordinate all remaining validators to run a new DKG ceremony and update their threshold schemes. Since key rotation is not implemented, this currently requires restarting the chain from scratch.

Similarly, if `output.json` is lost but `share.key` is retained, the validator cannot reconstruct the group polynomial needed to verify other validators' signatures. However, `output.json` contains only public information and can be copied from any other validator.

---

## DKG Ceremony Crash Recovery

The DKG ceremony persists its state to `{data_dir}/dkg_state.json` after each phase transition. This is handled by `PersistedDkgState` in `crates/node/dkg/src/state.rs`.

### Persisted State

```rust
pub struct PersistedDkgState {
    pub phase: DkgPhase,                         // Current phase
    session: SerializedSession,                  // Ceremony ID + chain ID + round
    pub dealer_started: bool,                    // Has our dealer been started?
    pub dealer_finalized: bool,                  // Has our dealer been finalized?
    pub our_signed_log: Option<String>,          // Our signed dealer log (hex)
    pub received_logs: BTreeMap<String, String>, // Collected dealer logs (pk_hex -> log_hex)
    pub timestamp: u64,                          // Ceremony timestamp (nanos)
}
```

### Recovery Logic

On restart, `DkgParticipant::try_restore()` (protocol.rs, line 990):

1. Checks if `dkg_state.json` exists. If not, starts fresh.
2. Loads the persisted state and recomputes the expected `CeremonySession` from the current config + timestamp.
3. If the ceremony ID matches, restores the participant to its last phase.
4. If the ceremony ID does NOT match (e.g., different participants or different timestamp window), clears the old state and starts fresh.
5. Restores collected dealer logs from persisted hex data.

### Phase Skipping

When restored to an advanced phase, the ceremony skips already-completed phases:
- Restored to `DealerStarted` or later: skips Phase 1 (dealer generation).
- Restored to `DealerFinalized` or later: skips Phase 3 (dealer finalization).
- Phase 2 (collecting messages) and Phase 4 (collecting logs) are always re-entered because they involve network communication that may not have completed.

### Completion Cleanup

On successful completion, `dkg_state.json` is deleted and `output.json` + `share.key` are written. On the next startup, `DkgOutput::exists()` detects the output files and skips the ceremony entirely.

---

## DKG Ceremony Timeout and Retry

### Timeouts

| Phase | Timeout | Constant |
|-------|---------|----------|
| Phase 2 (collecting messages) | 120 seconds | `PHASE2_MAX_TIMEOUT_SECS` |
| Phase 2.5 (ready synchronization) | 120 seconds | `PHASE2_MAX_TIMEOUT_SECS` |
| Phase 4 (collecting logs) | 120 seconds | `PHASE4_MAX_TIMEOUT_SECS` |

### Backoff Strategy

Within each phase, the ceremony uses exponential backoff for polling:
- Initial delay: 100ms (`INITIAL_BACKOFF_MS`)
- Maximum delay: 5000ms (`MAX_BACKOFF_MS`)
- Doubles on each iteration until max is reached.

### Retry Behavior

The DKG ceremony does NOT automatically retry on failure. If any phase times out, the ceremony returns `DkgError::Timeout` and the process exits. The orchestration layer (Docker Compose, Kubernetes, etc.) is responsible for restarting the DKG container, which will then attempt to restore from `dkg_state.json`.

### Network Failure Handling

Network send failures are logged at debug level and silently dropped:

```rust
// crates/node/dkg/src/ceremony.rs, line 388
if let Err(e) = network.send_to(&pk, &msg) {
    debug!(?pk, ?e, "Failed to send to peer");
}
```

This means transient network partitions during DKG can cause the ceremony to stall until timeout, with no explicit retry of individual messages.

---

## Anti-Replay and Session Binding

### Ceremony Session ID

Each DKG ceremony is uniquely identified by a deterministic session ID:

```rust
// crates/node/dkg/src/protocol.rs, line 40-57
pub fn new(chain_id: u64, participants: &[ed25519::PublicKey], timestamp_nanos: u64) -> Self {
    let mut hasher = Sha256::default();
    hasher.update(b"kora-dkg-ceremony-v1");
    hasher.update(&chain_id.to_le_bytes());
    hasher.update(&(participants.len() as u64).to_le_bytes());
    // sorted participants
    for pk in &sorted_participants { hasher.update(pk.as_ref()); }
    hasher.update(&timestamp_nanos.to_le_bytes());
    // ...
}
```

The timestamp is rounded to 5-minute intervals (`(now / interval) * interval`) so that all participants independently compute the same ceremony ID without explicit coordination.

### Message Format

Version 2 messages include the session ID:
```
[0xFF][version=2][session_id: 32 bytes][tag: 1 byte][payload]
```

Legacy (version 1) messages omit the session ID:
```
[tag: 1 byte][payload]
```

Messages with mismatched session IDs are rejected. Legacy messages are accepted with a warning for backward compatibility.

### Deduplication

Every incoming message is hashed (SHA256) and checked against a `HashSet<[u8; 32]>`. Duplicate messages are silently dropped.

---

## DKG Message Types

| Tag | Message | Direction | Purpose |
|-----|---------|-----------|---------|
| 0 | `DealerPublic` | Broadcast | Polynomial commitments from a dealer |
| 1 | `DealerPrivate` | Unicast (to specific player) | Encrypted share for one participant |
| 2 | `PlayerAck` | Unicast (to dealer) | Verification acknowledgment |
| 3 | `DealerLog` | Broadcast | Signed dealer transcript |
| 4 | `RequestLogs` | Unicast (to leader) | Request for all collected logs |
| 5 | `AllLogs` | Unicast (from leader) | All collected dealer logs |
| 6 | `Ready` | Broadcast | Phase synchronization signal |

---

## DKG Network Transport

The DKG ceremony uses a simple TCP-based network (`crates/node/dkg/src/network.rs`), separate from the main P2P overlay used for consensus. Each message is framed as:

```
[sender ed25519 public key][payload length: 4 bytes LE][payload bytes]
```

The listener is set to non-blocking mode and polled during each ceremony iteration.

Note from the source: "For production, this should be replaced with proper authenticated channels." The current TCP transport does not authenticate message senders cryptographically -- it relies on the sender including their public key in the envelope, which could be spoofed.

---

## Docker Initialization Flow

### With Interactive DKG

```
init-setup:          keygen setup --validators 4 --threshold 3
                     (generates validator.key + setup.json + peers.json for each node)
    |
    v
dkg-node0..3:        kora dkg --data-dir /data --chain-id 1337
                     (runs interactive Joint-Feldman DKG ceremony over TCP)
                     (produces output.json + share.key per node)
    |
    v
validator-node0..3:  kora validator --data-dir /data
                     (loads threshold scheme, starts Simplex consensus)
```

### With Trusted Dealer

```
init-config:         keygen setup --validators 4 --threshold 3
                     keygen dkg-deal --validators 4 --threshold 3
                     (single step: generates everything including DKG shares)
    |
    v
validator-node0..3:  kora validator --data-dir /data
                     (loads threshold scheme, starts Simplex consensus)
```

---

## Known Issues

### 1. Leader Dependency in DKG

Validator 0 is hardcoded as the DKG "leader" (coordinator). The leader role matters during Phase 4: non-leaders send `RequestLogs` to the leader, and the leader responds with `AllLogs`. If validator 0 fails or is unreachable during DKG, other validators will repeatedly request logs from it and eventually time out.

There is no leader election or failover within the DKG protocol itself.

### 2. No Overall Ceremony Timeout

While individual phases have 120-second timeouts, there is no overall timeout for the ceremony as a whole. The `wait_for_peers()` function adds a fixed 10-second delay at the start (to let containers join the P2P overlay), but if a peer joins after the ceremony has already advanced past Phase 1, it will never catch up.

### 3. Silent Network Failures

Broadcast and unicast failures during DKG are handled with:
```rust
if let Err(e) = network.send_to(&pk, &msg) {
    debug!(?pk, ?e, "Failed to send to peer");
}
```

There is no retry mechanism for individual messages. A transient TCP failure causes a permanent message loss for that round.

### 4. No Key Rotation

As documented above, the validator set is fixed forever once the initial DKG completes. There is no mechanism to rotate keys, add validators, or remove validators without restarting the entire chain.

### 5. Unauthenticated DKG Transport

The TCP transport for DKG does not verify sender identity. A network attacker could inject messages with spoofed public keys. The session binding provides replay protection but not authentication of the transport layer.

---

## File Reference

| File | Purpose |
|------|---------|
| `crates/node/dkg/src/lib.rs` | Module entry point, re-exports |
| `crates/node/dkg/src/protocol.rs` | `DkgParticipant` state machine, message handling, finalization (1055 lines) |
| `crates/node/dkg/src/ceremony.rs` | `DkgCeremony` orchestration: phases, timeouts, backoff (433 lines) |
| `crates/node/dkg/src/config.rs` | `DkgConfig`: participants, threshold, identity key, data dir |
| `crates/node/dkg/src/output.rs` | `DkgOutput`: serialization to/from `output.json` + `share.key` |
| `crates/node/dkg/src/state.rs` | `PersistedDkgState`: crash recovery state machine |
| `crates/node/dkg/src/error.rs` | `DkgError` enum: all possible failure modes |
| `crates/node/dkg/src/network.rs` | TCP-based DKG networking |
| `crates/node/dkg/src/transport.rs` | Transport configuration constants |
| `bin/keygen/src/main.rs` | CLI entry point for `keygen` tool |
| `bin/keygen/src/setup.rs` | Generates Ed25519 identity keys and peers.json |
| `bin/keygen/src/dkg_deal.rs` | Trusted dealer: generates all BLS shares from one node |
| `crates/node/runner/src/scheme.rs` | `load_threshold_scheme()`: loads DKG output into consensus scheme |
| `crates/utilities/crypto/src/test_utils.rs` | `threshold_schemes()`: test-only deterministic scheme generation |
| `keygen.bash` | Shell script shortcut for running keygen setup |
