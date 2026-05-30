# Kora / Commonware Integration Boundaries

## Overview

**Kora** is an EVM-compatible blockchain built as a replicated state machine. It
executes Ethereum transactions (via REVM) and reaches agreement on block ordering
through BFT consensus.

**Commonware** is a modular Rust framework for building replicated state machines.
It provides consensus protocols, peer-to-peer networking, cryptographic primitives,
persistent storage, and an async runtime -- but explicitly does _not_ dictate what
the state machine computes. Kora plugs its EVM execution layer into Commonware's
trait boundaries to get a production-grade consensus stack without reimplementing
BFT internals.

---

## What Commonware Provides to Kora

All Commonware crates are pinned to version **`2026.4.0`** in the workspace
`Cargo.toml` at `/Users/will/dev/nunchi/daeji/Cargo.toml` (lines 77-88).

| Commonware Crate | Role in Kora |
|---|---|
| `commonware-consensus` | Simplex BFT engine (proposer election, voting rounds, notarization, finalization) and the Marshal layer (block dissemination, certificate archival, ancestry resolution) |
| `commonware-p2p` | Authenticated P2P networking with channel multiplexing, peer discovery, and Byzantine blocking |
| `commonware-broadcast` | Buffered reliable broadcast for block propagation |
| `commonware-resolver` | Request/response protocol for fetching missing blocks from peers (backfill) |
| `commonware-cryptography` | BLS12-381 threshold signatures (MinSig variant), Ed25519 identity keys, SHA-256 digests, `Committable`/`Digestible` traits |
| `commonware-storage` | Journaled write-ahead log (WAL) for consensus messages, immutable archive for finalized blocks/certificates, and merkle-journaled QMDB backend |
| `commonware-runtime` | Tokio-based async runtime with labeled contexts, spawner, clock, metrics (OpenMetrics encoding), buffer pools, and process lifecycle |
| `commonware-codec` | Binary serialization framework (`Read`/`Write`/`Encode`/`Decode`) used for all wire and storage formats |
| `commonware-parallel` | Parallel/sequential signature verification strategy |
| `commonware-stream` | (Available but usage is minimal; reserved for streaming protocols) |
| `commonware-macros` | Proc macros for test utilities |
| `commonware-utils` | Ordered sets, acknowledgement primitives, channels, NonZero wrappers |

---

## What Kora Implements On Top

| Kora Layer | Responsibility |
|---|---|
| **EVM Execution** (`kora-executor`) | REVM-based block executor; processes signed Ethereum transactions against state |
| **State Management** (`kora-qmdb`, `kora-qmdb-ledger`, `kora-overlay`, `kora-backend`) | Merkleized key-value database (QMDB) with account/storage/code partitions; overlays for speculative execution |
| **Ledger Service** (`kora-ledger`) | Coordinates snapshots, mempool, seed cache, and persistence lifecycle |
| **Transaction Pool** (`kora-txpool`) | Validates and orders pending transactions; enforces admission limits |
| **RPC Server** (`kora-rpc`, `kora-indexer`) | Ethereum-compatible JSON-RPC with block/transaction indexing |
| **DKG Ceremony** (`kora-dkg`) | Interactive distributed key generation to bootstrap BLS threshold shares |
| **Consensus Glue** (`kora-runner`, `kora-consensus`, `kora-reporters`, `kora-simplex`, `kora-marshal`) | Trait implementations, configuration wiring, and activity reporters that bridge Commonware traits to Kora's execution layer |

---

## Integration Boundary: Trait Implementations

Kora satisfies several Commonware traits to plug into the Simplex consensus engine
and the Marshal block-dissemination layer.

### 1. Block Domain Type

**File:** `/Users/will/dev/nunchi/daeji/crates/node/domain/src/block.rs`

Kora's `Block` struct implements:

```rust
impl Digestible for Block {
    type Digest = ConsensusDigest; // sha256::Digest
}

impl Committable for Block {
    type Commitment = ConsensusDigest;
}

impl commonware_consensus::Heightable for Block {
    fn height(&self) -> Height { ... }
}

impl commonware_consensus::Block for Block {
    fn parent(&self) -> Self::Digest { ... }
}
```

The `ConsensusDigest` is `commonware_cryptography::sha256::Digest` (32 bytes).
Block identity is computed as `keccak256(encode(block))` then hashed through
SHA-256 for Commonware compatibility.

### 2. Application Trait (Block Proposal)

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (line 271)

```rust
impl<Env, S, E> Application<Env> for RevmApplication<S, E>
where
    Env: Rng + Spawner + Metrics + Clock,
    S: CertScheme + Send + Sync + 'static,
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes> + Clone + Send + Sync + 'static,
{
    type SigningScheme = S;
    type Context = Context<ConsensusDigest, S::PublicKey>;
    type Block = Block;

    fn genesis(&mut self) -> impl Future<Output = Self::Block> + Send { ... }

    fn propose<A>(
        &mut self,
        _context: (Env, Self::Context),
        mut ancestry: AncestorStream<A, Self::Block>,
    ) -> impl Future<Output = Option<Self::Block>> + Send
    where A: BlockProvider<Block = Self::Block>
    { ... }
}
```

**How blocks are proposed:**
1. Simplex elects a leader (random elector based on VRF).
2. The engine calls `propose()` on the `Application` impl.
3. `RevmApplication::propose` fetches the parent from the ancestry stream,
   drains the mempool (excluding transactions in pending ancestors), executes
   transactions via REVM, computes a deterministic state root, and returns the
   constructed `Block`.

### 3. VerifyingApplication Trait (Block Verification)

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (line 321)

```rust
impl<Env, S, E> VerifyingApplication<Env> for RevmApplication<S, E>
where ...
{
    fn verify<A>(
        &mut self,
        _context: (Env, Self::Context),
        mut ancestry: AncestorStream<A, Self::Block>,
    ) -> impl Future<Output = bool> + Send
    where A: BlockProvider<Block = Self::Block>
    { ... }
}
```

**How blocks are verified:**
1. Simplex receives a proposed block from the leader.
2. The engine calls `verify()` on the `VerifyingApplication` impl.
3. `RevmApplication::verify` iterates the ancestry stream from tip to the last
   known-good block, then verifies from oldest to newest: re-executes transactions,
   recomputes the state root, and rejects the block if the roots diverge.

### 4. Reporter Trait (Finalization Callbacks)

**File:** `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs`

Three reporters are composed together via `Reporters::from((...))`:

| Reporter | Activity Type | Purpose |
|---|---|---|
| `SeedReporter<V>` (line 91) | `Activity<Scheme, ConsensusDigest>` | Extracts VRF seeds from notarizations/finalizations and stores them for future prevrandao |
| `FinalizedReporter<E, P>` (line 407) | `Update<Block>` | On finalized block delivery: re-executes if snapshot is missing, persists to QMDB, indexes for RPC, prunes mempool, and acknowledges the marshal |
| `NodeStateReporter<S>` (line 453) | `Activity<S, ConsensusDigest>` | Updates RPC-visible metrics (view number, finalized count, nullified count) |

**How finalization works:**
1. Simplex collects threshold finalization signatures.
2. The Marshal layer archives the finalization certificate and delivers
   `Update::Block(block, ack)` to `FinalizedReporter`.
3. The reporter persists the block's state changes to QMDB (the durable store),
   indexes it for RPC queries, prunes included transactions from the mempool,
   then calls `ack.acknowledge()` to unblock the marshal floor advancement.

### 5. CertifiableAutomaton and Relay (via Marshal Inline Adapter)

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` (line 532)

Kora does not implement `CertifiableAutomaton` or `Relay` directly. Instead it
uses Commonware's `marshal::standard::Inline` adapter:

```rust
let marshaled = Inline::new(context, app, marshal_mailbox, epocher);
```

`Inline` wraps the `Application`/`VerifyingApplication` and exposes the
`CertifiableAutomaton` + `Relay` traits that Simplex requires. This adapter
handles block serialization, ancestry resolution via the marshal mailbox, and
certificate forwarding -- Kora only needs to implement the higher-level
`Application` and `VerifyingApplication` traits.

### 6. Certificate Scheme (ThresholdScheme)

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs`

```rust
pub type ThresholdScheme = bls12381_threshold::vrf::Scheme<ed25519::PublicKey, MinSig>;
```

This is Commonware's BLS12-381 threshold VRF scheme parameterized with:
- `ed25519::PublicKey` as the participant identity type
- `MinSig` variant (smaller signatures, larger public keys)

The scheme is loaded from DKG output files containing the group polynomial and
the validator's secret share.

### 7. P2P Oracle and Transport

**File:** `/Users/will/dev/nunchi/daeji/crates/network/transport/src/transport.rs`

Kora wraps Commonware's authenticated P2P layer into a `NetworkTransport` struct
that provides:
- 5 multiplexed channels: votes (0), certs (1), resolver (2), blocks (3), backfill (4)
- An oracle for peer tracking and Byzantine blocking

The transport is created from `commonware_p2p::authenticated::discovery` and
wired to both Simplex (votes, certs, resolver) and Marshal (blocks, backfill).

---

## Consensus Engine Configuration

**File:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` (lines 48-56)

The production runner configures Simplex with:

| Parameter | Value | Purpose |
|---|---|---|
| `leader_timeout` | 2s | Time to wait for a proposal before voting to skip |
| `certification_timeout` | 4s | Time to wait for notarization quorum |
| `timeout_retry` | 1s | Interval for retransmitting nullification votes |
| `fetch_timeout` | 1s | Timeout for resolver fetch requests |
| `activity_timeout` | 256 views | Inactivity threshold before blocking a peer |
| `skip_timeout` | 32 views | Views without progress before escalated skip |
| `fetch_concurrent` | 32 | Maximum concurrent fetch requests |
| `epoch` | `Epoch::zero()` | Starting epoch |
| `EPOCH_LENGTH` | `u64::MAX` | Effectively infinite -- single epoch, no rotation |
| `forwarding` | `SilentLeader` | Non-leaders forward proposals silently |
| `replay_buffer` | 16 MiB | WAL replay buffer for consensus messages |
| `write_buffer` | 16 MiB | WAL write buffer |
| `elector` | `Random` | VRF-based random leader election |

---

## Version Pinning

All 12 Commonware crates are pinned to **`2026.4.0`** via workspace dependencies.
There is no version range -- exact pinning ensures deterministic builds and
prevents accidental breakage from upstream updates.

The version scheme follows `YYYY.M.patch` (CalVer-like), suggesting this is the
April 2026 release.

---

## Limitations Imposed by Commonware

### 1. Static Validator Set

Kora sets `EPOCH_LENGTH = u64::MAX` and uses a `ConstantSchemeProvider` that
returns the same threshold scheme regardless of epoch:

```rust
// runner.rs line 92-103
impl commonware_cryptography::certificate::Provider for ConstantSchemeProvider {
    type Scope = Epoch;
    type Scheme = ThresholdScheme;

    fn scoped(&self, _epoch: Epoch) -> Option<Arc<Self::Scheme>> {
        Some(self.0.clone())
    }
}
```

This means the validator set is fixed at startup (determined by the DKG output).
There is no mechanism for dynamic membership changes or validator rotation within
a running chain. Adding/removing validators requires a new DKG ceremony and chain
restart.

### 2. Epoch Model

Commonware's Simplex uses an epoch-based model where the scheme provider can
return different signing configurations per epoch. Kora deliberately bypasses
this by making epochs infinite, losing the ability to do planned key rotations
without a protocol upgrade.

### 3. Metrics Format

Commonware's runtime emits metrics in **OpenMetrics** format
(`application/openmetrics-text; version=1.0.0`). Kora exposes these directly via
an HTTP `/metrics` endpoint (runner.rs lines 444-477):

```rust
let body = metrics_context.encode();
// Content-Type: "application/openmetrics-text; version=1.0.0; charset=utf-8"
```

The metric names and labels are controlled by Commonware (prefixed by subsystem
labels like `engine`, `resolver`, etc.). Kora has limited ability to customize
metric naming, aggregation, or format without wrapping the metrics output.

### 4. Resolver Sequencing

The Marshal layer requires the application to `acknowledge()` each finalized block
before advancing the delivery floor. If the application (Kora's `FinalizedReporter`)
blocks during persistence (e.g., slow fsync to QMDB), subsequent finalized blocks
queue up and all peers waiting for acknowledgement are stalled. This is by design
for consistency but creates back-pressure on the consensus pipeline.

### 5. Block Codec Constraints

Commonware's codec framework requires static configuration for maximum sizes
at decode time. Kora sets:
- `BLOCK_CODEC_MAX_TXS = 10,000`
- `BLOCK_CODEC_MAX_TX_BYTES = 8 MiB`

These are compile-time constants. Changing them requires recompilation and
coordinated deployment across all validators.

---

## Key Code References

| Component | File Path |
|---|---|
| Workspace dependencies | `/Users/will/dev/nunchi/daeji/Cargo.toml` (lines 77-88) |
| Application trait impl | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` |
| Production runner (wiring) | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` |
| Threshold scheme loading | `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs` |
| Block type + Commonware traits | `/Users/will/dev/nunchi/daeji/crates/node/domain/src/block.rs` |
| Consensus application trait (Kora's own) | `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/application.rs` |
| Consensus support traits | `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/traits.rs` |
| Reporters (seed, finalized, node state) | `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` |
| Simplex config defaults | `/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs` |
| Simplex engine helper | `/Users/will/dev/nunchi/daeji/crates/node/simplex/src/engine.rs` |
| Marshal actor init | `/Users/will/dev/nunchi/daeji/crates/network/marshal/src/actor.rs` |
| Broadcast init | `/Users/will/dev/nunchi/daeji/crates/network/marshal/src/broadcast.rs` |
| Peer resolver init | `/Users/will/dev/nunchi/daeji/crates/network/marshal/src/peers.rs` |
| Archive init | `/Users/will/dev/nunchi/daeji/crates/network/marshal/src/archive.rs` |
| Network transport | `/Users/will/dev/nunchi/daeji/crates/network/transport/src/transport.rs` |
| Channel definitions | `/Users/will/dev/nunchi/daeji/crates/network/transport/src/channels.rs` |
| QMDB backend (storage) | `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/backend.rs` |
| DKG ceremony | `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` |
| Ledger service | `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` |
| Block execution helper | `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/execution.rs` |
| Stub implementations (dev) | `/Users/will/dev/nunchi/daeji/crates/node/service/src/stubs.rs` |

---

## Architecture Diagram (Textual)

```
+----------------------------------------------------------+
|                     Kora Application                      |
|                                                          |
|  +------------+  +------------+  +----------+  +-------+ |
|  | RevmExecutor|  | TxPool     |  | RPC Server|  | DKG  | |
|  +------+-----+  +-----+------+  +----+-----+  +---+---+ |
|         |               |              |            |      |
|  +------v---------------v--------------v------------+---+  |
|  |              LedgerService                           |  |
|  |  (mempool, snapshots, seeds, QMDB persistence)      |  |
|  +---+----------------------------------------------+---+  |
|      |                                              |      |
+------|-----------Commonware Trait Boundary-----------|------+
       |                                              |
  +----v----+                                   +-----v-----+
  | Application/VerifyingApplication            | Reporter   |
  | (propose, verify, genesis)                  | (finalize) |
  +----+----+                                   +-----+-----+
       |                                              |
+------v----------------------------------------------v------+
|                   Commonware Framework                      |
|                                                            |
|  +------------------+  +------------------+  +-----------+ |
|  | Simplex Engine   |  | Marshal (Actor)  |  | Archive   | |
|  | (BFT consensus)  |  | (dissemination)  |  | (storage) | |
|  +--------+---------+  +--------+---------+  +-----+-----+ |
|           |                      |                  |       |
|  +--------v----------------------v------------------v-----+ |
|  |          commonware-p2p (authenticated channels)       | |
|  |  votes | certs | resolver | blocks | backfill          | |
|  +--------------------------------------------------------+ |
|                                                            |
|  +------------------+  +------------------+  +-----------+ |
|  | commonware-      |  | commonware-      |  | commonware| |
|  | cryptography     |  | storage          |  | -runtime  | |
|  | (BLS12-381, Ed25519) | (WAL, merkle)  |  | (tokio)   | |
|  +------------------+  +------------------+  +-----------+ |
+------------------------------------------------------------+
```

---

## Summary of the Integration Contract

Kora implements **two primary traits** from Commonware to participate in consensus:

1. **`Application<Env>`** -- called by Simplex when this validator is elected
   leader. Kora builds a block by draining the mempool and executing via REVM.

2. **`VerifyingApplication<Env>`** -- called by Simplex when receiving a block
   from another leader. Kora re-executes and validates the state root.

Everything else (voting, signature aggregation, certificate management, block
archival, peer communication) is handled by Commonware internally. Kora also
implements the `Reporter` trait for post-consensus callbacks (seed tracking,
state persistence, metric updates) but these are optional extension points
rather than required consensus participants.

The integration is clean: Kora owns execution semantics and state, Commonware
owns ordering and Byzantine agreement.
