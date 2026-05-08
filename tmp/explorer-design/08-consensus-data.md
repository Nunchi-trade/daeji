# Consensus Data — What to Expose and How

Everything the explorer needs to visualize consensus as a live, breathing system.
The node already sees all of this internally — the gap is getting it out over the wire.

---

## What the Node Sees Today (and Discards)

The Simplex consensus engine emits **10 distinct Activity types** through the `Reporter` trait. Today, the `NodeStateReporter` handles only 3 of them and reduces them to counter increments:

| Activity | Currently Handled | What's Lost |
|----------|------------------|-------------|
| `Notarize` | Ignored | Individual vote events — who voted, when |
| `Notarization` | `set_view()` | Quorum details, participant set, VRF seed, payload digest |
| `Certification` | Ignored | Individual finalization vote events |
| `Finalize` | Ignored | Individual finalization votes |
| `Finalization` | `set_view() + inc_finalized()` | Same as Notarization — quorum details, seed |
| `Nullify` | Ignored | Individual nullification vote |
| `Nullification` | `inc_nullified()` | Round details, which view was nullified |
| `ConflictingNotarize` | **Ignored entirely** | Fork detection — critical safety event |
| `ConflictingFinalize` | **Ignored entirely** | Finalization fork — critical safety event |
| `NullifyFinalize` | **Ignored entirely** | Nullify/finalize conflict — critical safety event |

The explorer should see **all 10**. The consensus ring visualization needs the full lifecycle, not just the endpoints.

---

## New RPC Infrastructure

### 1. ConsensusEvent — The Wire Format

Every consensus activity gets serialized into a structured JSON event and pushed to WebSocket subscribers:

```rust
// New file: crates/node/rpc/src/consensus_events.rs

use serde::{Deserialize, Serialize};

/// A consensus event pushed to WebSocket subscribers.
/// Each variant maps 1:1 to a Simplex Activity type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ConsensusEvent {
    /// Individual validator voted to notarize a proposal.
    #[serde(rename_all = "camelCase")]
    Notarize {
        view: u64,
        payload: String,        // hex-encoded block digest
        timestamp_ms: u64,      // local timestamp when event was received
    },

    /// Quorum achieved — enough validators notarized the proposal.
    #[serde(rename_all = "camelCase")]
    Notarization {
        view: u64,
        payload: String,        // hex-encoded block digest
        seed: String,           // hex-encoded VRF seed
        timestamp_ms: u64,
    },

    /// Individual validator voted to finalize (certification step).
    #[serde(rename_all = "camelCase")]
    Certification {
        view: u64,
        payload: String,
        timestamp_ms: u64,
    },

    /// Individual finalization vote.
    #[serde(rename_all = "camelCase")]
    Finalize {
        view: u64,
        payload: String,
        timestamp_ms: u64,
    },

    /// Block finalized — threshold finalization signatures collected.
    #[serde(rename_all = "camelCase")]
    Finalization {
        view: u64,
        payload: String,        // hex-encoded block digest
        seed: String,           // hex-encoded VRF seed
        block_height: Option<u64>, // resolved from payload if available
        timestamp_ms: u64,
    },

    /// Individual nullification vote (timeout on current view).
    #[serde(rename_all = "camelCase")]
    Nullify {
        timestamp_ms: u64,
    },

    /// Quorum nullification — view skipped, no block produced.
    #[serde(rename_all = "camelCase")]
    Nullification {
        view: u64,              // the view that was nullified
        timestamp_ms: u64,
    },

    // ── Safety Violations ──
    // These should NEVER happen in normal operation.
    // When they do, the explorer should render them prominently.

    /// Two conflicting notarization votes from the same validator.
    #[serde(rename_all = "camelCase")]
    ConflictingNotarize {
        timestamp_ms: u64,
        severity: &'static str, // always "critical"
    },

    /// Two conflicting finalization votes from the same validator.
    #[serde(rename_all = "camelCase")]
    ConflictingFinalize {
        timestamp_ms: u64,
        severity: &'static str, // always "critical"
    },

    /// A validator sent both nullify and finalize for the same view.
    #[serde(rename_all = "camelCase")]
    NullifyFinalize {
        timestamp_ms: u64,
        severity: &'static str, // always "critical"
    },
}
```

### 2. ConsensusReporter — The New Reporter

A new reporter that intercepts ALL Activity events and broadcasts them to WebSocket subscribers:

```rust
// New file: crates/node/rpc/src/consensus_reporter.rs

use std::{marker::PhantomData, time::{SystemTime, UNIX_EPOCH}};
use tokio::sync::broadcast;
use commonware_consensus::{Reporter, simplex::types::Activity};
use commonware_cryptography::certificate::Scheme;
use super::consensus_events::ConsensusEvent;

/// Reporter that converts ALL consensus Activity events into ConsensusEvents
/// and broadcasts them to WebSocket subscribers via a tokio broadcast channel.
#[derive(Clone, Debug)]
pub struct ConsensusReporter<S> {
    tx: broadcast::Sender<ConsensusEvent>,
    _scheme: PhantomData<S>,
}

impl<S> ConsensusReporter<S> {
    pub fn new(capacity: usize) -> (Self, broadcast::Receiver<ConsensusEvent>) {
        let (tx, rx) = broadcast::channel(capacity);
        (Self { tx, _scheme: PhantomData }, rx)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ConsensusEvent> {
        self.tx.subscribe()
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

impl<S> Reporter for ConsensusReporter<S>
where
    S: Scheme + Clone + Send + 'static,
{
    type Activity = Activity<S, ConsensusDigest>;

    fn report(&mut self, activity: Self::Activity) -> impl Future<Output = ()> + Send {
        let event = match &activity {
            Activity::Notarize(n) => ConsensusEvent::Notarize {
                view: n.proposal.round.view().get(),
                payload: hex::encode(n.proposal.payload),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Notarization(n) => ConsensusEvent::Notarization {
                view: n.proposal.round.view().get(),
                payload: hex::encode(n.proposal.payload),
                seed: hex::encode(n.seed().encode()),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Certification(c) => ConsensusEvent::Certification {
                view: c.proposal.round.view().get(),
                payload: hex::encode(c.proposal.payload),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Finalize(f) => ConsensusEvent::Finalize {
                view: f.proposal.round.view().get(),
                payload: hex::encode(f.proposal.payload),
                timestamp_ms: Self::now_ms(),
            },
            Activity::Finalization(f) => ConsensusEvent::Finalization {
                view: f.proposal.round.view().get(),
                payload: hex::encode(f.proposal.payload),
                seed: hex::encode(f.seed().encode()),
                block_height: None, // resolved later by correlation with BlockIndex
                timestamp_ms: Self::now_ms(),
            },
            Activity::Nullify(_) => ConsensusEvent::Nullify {
                timestamp_ms: Self::now_ms(),
            },
            Activity::Nullification(n) => ConsensusEvent::Nullification {
                view: n.round.view().get(),
                timestamp_ms: Self::now_ms(),
            },
            Activity::ConflictingNotarize(_) => ConsensusEvent::ConflictingNotarize {
                timestamp_ms: Self::now_ms(),
                severity: "critical",
            },
            Activity::ConflictingFinalize(_) => ConsensusEvent::ConflictingFinalize {
                timestamp_ms: Self::now_ms(),
                severity: "critical",
            },
            Activity::NullifyFinalize(_) => ConsensusEvent::NullifyFinalize {
                timestamp_ms: Self::now_ms(),
                severity: "critical",
            },
        };

        let _ = self.tx.send(event);
        async {}
    }
}
```

### 3. Wiring Into the Reporter Chain

In `crates/node/runner/src/runner.rs`, the existing reporter composition:

```rust
// BEFORE (current):
let seed_reporter = SeedReporter::<MinSig>::new(ledger.clone());
let node_state_reporter = /* ... */;
let inner_reporters = Reporters::from((marshal_mailbox.clone(), node_state_reporter));
let reporter = Reporters::from((seed_reporter, inner_reporters));

// AFTER (with consensus streaming):
let seed_reporter = SeedReporter::<MinSig>::new(ledger.clone());
let (consensus_reporter, _consensus_rx) = ConsensusReporter::<ThresholdScheme>::new(1024);
let node_state_reporter = /* ... */;
let inner_reporters = Reporters::from((marshal_mailbox.clone(), node_state_reporter));
let with_consensus = Reporters::from((consensus_reporter.clone(), inner_reporters));
let reporter = Reporters::from((seed_reporter, with_consensus));

// Pass consensus_reporter to RPC server for subscription routing
```

The `Reporters::from()` tuple composition means the new reporter gets ALL Activity events without modifying existing reporters. Zero-impact addition.

### 4. kora_subscribe Subscription Method

Extend the `KoraApi` trait in `crates/node/rpc/src/kora.rs`:

```rust
#[rpc(server, namespace = "kora")]
pub trait KoraApi {
    #[method(name = "nodeStatus")]
    async fn node_status(&self) -> RpcResult<NodeStatus>;

    /// Subscribe to consensus activity events.
    /// Returns ALL consensus lifecycle events in real-time.
    #[subscription(
        name = "subscribe" => "subscription",
        unsubscribe = "unsubscribe",
        item = ConsensusEvent
    )]
    async fn subscribe(
        &self,
        kind: String,
    ) -> jsonrpsee::core::SubscriptionResult;
}
```

Subscription types:
- `"consensus"` — all consensus activity events (the full firehose)
- `"consensus.safety"` — only safety violation events (ConflictingNotarize, ConflictingFinalize, NullifyFinalize)
- `"consensus.rounds"` — only round-level events (Notarization, Finalization, Nullification) — no individual votes
- `"consensus.votes"` — only individual vote events (Notarize, Certification, Finalize, Nullify)

### 5. Enhanced kora_consensusState (Poll-Based Snapshot)

For clients that can't use WebSocket, add a richer poll endpoint:

```rust
#[method(name = "consensusState")]
async fn consensus_state(&self) -> RpcResult<ConsensusState>;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusState {
    // ── Identity ──
    pub chain_id: u64,
    pub validator_index: u32,
    pub validator_count: u32,       // total validators in the set (e.g., 4)
    pub threshold: u32,             // votes needed for quorum (e.g., 3)

    // ── Current Round ──
    pub current_view: u64,
    pub current_leader: u32,        // validator_index of current leader
    pub round_phase: RoundPhase,    // what phase the current round is in

    // ── Cumulative Metrics ──
    pub finalized_count: u64,
    pub proposed_count: u64,
    pub nullified_count: u64,

    // ── Health ──
    pub peer_count: u64,
    pub uptime_secs: u64,
    pub is_leader: bool,

    // ── Timing ──
    pub last_finalized_at_ms: Option<u64>,   // timestamp of last finalization
    pub avg_finalization_ms: Option<u64>,     // rolling average finalization latency
    pub last_nullified_at_ms: Option<u64>,    // timestamp of last nullification

    // ── Safety ──
    pub safety_violations: u64,              // total lifetime safety events
    pub last_violation_at_ms: Option<u64>,   // timestamp of most recent violation
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoundPhase {
    Proposing,      // leader is building a block
    Notarizing,     // validators are voting on the proposal
    Certifying,     // validators are finalizing
    Finalized,      // block committed
    Nullifying,     // round timing out
    Idle,           // between rounds
}
```

---

## What the Explorer Receives (Event Stream)

A typical healthy round produces this event sequence on the WS subscription:

```jsonc
// View 100: Validator 0 is leader (100 % 4 == 0)

// 1. Leader proposes, this node votes to notarize
{"type":"notarize","view":100,"payload":"0xabcd...","timestampMs":1715200000100}

// 2. Quorum notarization achieved (3/4 validators voted)
{"type":"notarization","view":100,"payload":"0xabcd...","seed":"0x1234...","timestampMs":1715200000250}

// 3. This node votes to finalize
{"type":"finalize","view":100,"payload":"0xabcd...","timestampMs":1715200000300}

// 4. Quorum finalization achieved — block committed
{"type":"finalization","view":100,"payload":"0xabcd...","seed":"0x5678...","blockHeight":286401,"timestampMs":1715200000450}
```

A nullified round (leader timed out):

```jsonc
// View 101: Validator 1 is leader but didn't propose in time

// 1. This node votes to nullify
{"type":"nullify","timestampMs":1715200001100}

// 2. Quorum nullification — view skipped
{"type":"nullification","view":101,"timestampMs":1715200001500}

// View 102 begins with new leader...
```

A safety violation (should never happen):

```jsonc
{"type":"conflictingNotarize","timestampMs":1715200005000,"severity":"critical"}
```

---

## Explorer State Machine for Consensus

The explorer maintains a local consensus state machine, updated by the WS event stream:

```typescript
// apps/explorer/src/data/consensus.ts

interface ConsensusRound {
  view: number;
  leader: number;           // view % validatorCount
  phase: RoundPhase;
  payload: string | null;   // block digest being proposed
  startedAt: number;        // timestamp when view began

  // Phase progression timestamps
  notarizeAt: number | null;
  notarizationAt: number | null;
  certificationAt: number | null;
  finalizeAt: number | null;
  finalizationAt: number | null;
  nullifyAt: number | null;
  nullificationAt: number | null;
}

type RoundPhase =
  | 'proposing'     // waiting for leader's proposal
  | 'notarizing'    // this node voted, waiting for quorum
  | 'certifying'    // notarization achieved, voting to finalize
  | 'finalizing'    // this node voted to finalize, waiting for quorum
  | 'finalized'     // block committed
  | 'nullifying'    // timeout, voting to skip
  | 'nullified';    // view skipped

// Phase transitions driven by events:
// (start)        -> proposing
// Notarize       -> notarizing
// Notarization   -> certifying
// Certification  -> finalizing
// Finalize       -> finalizing  (redundant but harmless)
// Finalization   -> finalized   -> (next view starts at proposing)
// Nullify        -> nullifying
// Nullification  -> nullified   -> (next view starts at proposing)
```

### Consensus History Ring

```typescript
interface ConsensusHistory {
  rounds: RingBuffer<ConsensusRound>;  // last 200 rounds

  // Aggregate metrics (computed from history)
  avgFinalizationMs: number;     // rolling average: finalizationAt - startedAt
  nullificationRate: number;     // nullified / total rounds (last 200)
  safetyViolations: SafetyEvent[];
}

interface SafetyEvent {
  type: 'conflictingNotarize' | 'conflictingFinalize' | 'nullifyFinalize';
  timestampMs: number;
  view: number | null;
}
```

---

## Visual Mapping — Consensus Ring (Scene 4)

The consensus ring visualization now has concrete data sources:

### Validator Nodes on Ring

```
Validators: 4 nodes equally spaced on a circle (90 degrees apart)
Position: validator_index * (2*pi / 4) = 0, 90, 180, 270 degrees

Node states (driven by ConsensusRound.phase):
  * proposing:    leader node glows bright rose, pulses
  o notarizing:   voted nodes show half-fill (rose-dim)
  * notarization: all voted nodes glow rose
  O certifying:   nodes show double ring (inner rose, outer bone)
  # finalized:    all nodes flash bright, center crystallizes
  o nullifying:   nodes dim to warning amber, pulse slowly
  . nullified:    nodes go dark (text-ghost), gap in pulse
```

### Center Block Visualization

```
Center of ring shows the block being formed:

  proposing:    empty center, faint wireframe outline
  notarizing:   wireframe fills partially (votes/threshold progress)
  notarization: wireframe solid, dream-colored (not yet final)
  certifying:   block solidifies, color shifts dream -> rose
  finalized:    block FLASHES (200ms white overlay), then drops
                into waterfall scene below
  nullified:    wireframe SHATTERS (8 fragments, outward, fade to ghost)
```

### Threshold Arc

```
Circular progress arc around the ring:
  - Arc fills as votes arrive (Notarize events)
  - Threshold line at 3/4 (75%) of arc
  - Color: rose-dim during voting -> rose-bright when threshold crossed
  - On nullification: arc turns warning amber, then resets
```

### Timing Visualization

```
Each round's duration is visible:
  - Inner ring shows elapsed time as a sweep hand (1 revolution = leader_timeout)
  - When sweep reaches the end without finalization -> turns amber (timeout approaching)
  - Historical rounds shown as thin arcs below the ring:
    |-- fast (< 500ms) --|-- normal --|-- slow --|-- nullified --|
    ####################  #########   #######    ............
    green                 rose        amber      ghost
```

### Safety Violation Rendering

When a `ConflictingNotarize`, `ConflictingFinalize`, or `NullifyFinalize` event arrives:

1. **Full-screen flash**: danger red (#cc5555) at 30% opacity, 200ms
2. **Ring disruption**: all node connections turn red, ring border pulses danger
3. **Alert panel**: glass panel appears center-screen:
   ```
   +-- SAFETY VIOLATION --------------------------+
   |                                               |
   |  CONFLICTING NOTARIZE DETECTED                |
   |  View 100 - 2025-05-08 14:23:01.500          |
   |                                               |
   |  A validator produced two conflicting          |
   |  notarization votes for the same view.        |
   |  This indicates Byzantine behavior.           |
   |                                               |
   +-----------------------------------------------+
   ```
4. **Persistent indicator**: red LED dot in status panel, incrementing violation counter
5. **Audio** (if enabled): sharp dissonant chord, sustained 2 seconds

Safety violations persist in the UI — they don't auto-dismiss. User must acknowledge.

---

## Extended NodeState — Backend Changes

The existing `NodeState` needs additional fields to support the richer `kora_consensusState` endpoint:

```rust
// Changes to crates/node/rpc/src/state.rs

struct NodeStateInner {
    // ... existing fields ...

    // NEW: consensus timing
    last_finalized_at: RwLock<Option<Instant>>,
    last_nullified_at: RwLock<Option<Instant>>,
    finalization_times: RwLock<VecDeque<Duration>>,  // last 100 finalization durations

    // NEW: round tracking
    current_round_start: RwLock<Instant>,
    round_phase: RwLock<RoundPhase>,

    // NEW: safety
    safety_violation_count: AtomicU64,
    last_violation_at: RwLock<Option<Instant>>,

    // NEW: validator set info (set once at startup)
    validator_count: u32,
    threshold: u32,
}
```

These get updated by the `ConsensusReporter` (same events, dual-purpose: both streaming and state tracking).

---

## Mempool Subscription

Beyond consensus, the mempool state is also valuable for visualization. Add a mempool snapshot endpoint:

```rust
#[method(name = "mempoolStatus")]
async fn mempool_status(&self) -> RpcResult<MempoolStatus>;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MempoolStatus {
    pub pending_count: usize,
    pub queued_count: usize,
    pub pending_limit: usize,    // from PoolConfig.max_pending_txs (4096)
    pub queued_limit: usize,     // from PoolConfig.max_queued_txs (1024)
}
```

And a mempool subscription via `kora_subscribe("mempool")`:

```jsonc
// Transaction enters mempool
{"type":"txAdded","hash":"0x1234...","from":"0xabcd...","to":"0xef01...","value":"1000000000000000000","gasPrice":"1000000000","timestampMs":1715200000100}

// Transaction included in finalized block (pruned from mempool)
{"type":"txIncluded","hash":"0x1234...","blockHeight":286401,"timestampMs":1715200001000}

// Transaction evicted (replaced by higher-fee tx)
{"type":"txEvicted","hash":"0x1234...","reason":"replaced","timestampMs":1715200002000}
```

This feeds the Particle Constellation's pending transaction lifecycle (spawn, orbit, snap/shatter).

---

## Implementation Sequence

```
Phase 1 — Consensus Streaming (enables Scene 4):
  1. Create ConsensusEvent enum                       [2 hours]
  2. Create ConsensusReporter (Reporter impl)         [3 hours]
  3. Wire into reporter chain in runner.rs            [1 hour]
  4. Add kora_subscribe("consensus") WS endpoint      [3 hours]
  5. Add kora_consensusState poll endpoint            [2 hours]
  6. Extend NodeState with timing/phase/safety fields [2 hours]

  Total: ~13 hours. Zero impact on existing reporters.

Phase 2 — Mempool Streaming:
  1. Add kora_mempoolStatus poll endpoint             [1 hour]
  2. Extend LedgerEvent with eviction events          [2 hours]
  3. Add kora_subscribe("mempool") WS endpoint        [2 hours]

  Total: ~5 hours.

Phase 3 — Explorer Integration:
  1. ConsensusRound state machine in TypeScript        [3 hours]
  2. Consensus Ring scene updates (Scene 4)            [6 hours]
  3. Safety violation rendering + alert panel          [3 hours]
  4. Status panel consensus section                    [2 hours]
  5. Mempool pressure gauge visualization              [3 hours]

  Total: ~17 hours.
```

---

## Event Volume Estimate

At 1-second block time with 4 validators, healthy operation produces:

| Event | Frequency | Per Second |
|-------|-----------|------------|
| Notarize | 1 per round (this node's vote) | ~1/s |
| Notarization | 1 per round | ~1/s |
| Certification | 1 per round | ~1/s |
| Finalize | 1 per round | ~1/s |
| Finalization | 1 per round | ~1/s |
| **Total** | | **~5 events/sec** |

With occasional nullifications (leader timeouts), add ~2 more events per missed round.
Safety violations: 0 in normal operation.

This is very low volume — a single broadcast channel with capacity 1024 is more than sufficient. No batching or throttling needed.
