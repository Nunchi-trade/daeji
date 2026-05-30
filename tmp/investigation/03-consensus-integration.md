# Consensus Integration Analysis

## Summary

Kora uses Commonware's Simplex BFT consensus engine with BLS12-381 threshold VRF for leader election. The consensus layer has well-tuned timeouts but several critical failure modes: the executor abort causing nullification cascades, an early return bug in the finalization reporter that skips mempool pruning, and a single-mutex design that serializes all state access.

---

## 1. Simplex Consensus Configuration

**File:** `crates/node/runner/src/runner.rs` (Lines 48-53)

### Timeout Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `CONSENSUS_LEADER_TIMEOUT` | 2 seconds | How long leader waits before proposing |
| `CONSENSUS_CERTIFICATION_TIMEOUT` | 4 seconds | How long to wait for 2/3+ certification |
| `CONSENSUS_TIMEOUT_RETRY` | 1 second | Retry delay on timeout |
| `CONSENSUS_FETCH_TIMEOUT` | 1 second | Block fetch timeout from peers |
| `CONSENSUS_ACTIVITY_TIMEOUT` | 256 views | Views until activity timeout (very long) |
| `CONSENSUS_SKIP_TIMEOUT` | 32 views | Views until skip advance |

**Note:** Runner overrides several Simplex defaults (simplex/config.rs):
- Leader timeout: 1s default → 2s in runner
- Certification timeout: 2s default → 4s in runner
- Activity timeout: 20 views default → 256 views in runner
- Skip timeout: 10 views default → 32 views in runner
- Fetch concurrent: 8 default → 32 in runner

### Engine Initialization (Lines 548-573)

```rust
let engine = simplex::Engine::new(
    context.with_label("engine"),
    simplex::Config {
        scheme: self.scheme.clone(),          // BLS12-381 threshold signatures
        elector: Random,                       // Random leader election via VRF
        blocker: transport.oracle.clone(),     // P2P connectivity tracking
        automaton: marshaled.clone(),          // Block proposal/verification
        relay: marshaled,                      // Block relay
        reporter,                              // Reporter chain (see below)
        strategy,                              // Signature strategy (2 threads)
        partition: self.partition_prefix.clone(),
        mailbox_size: MAILBOX_SIZE,
        epoch: Epoch::zero(),
        replay_buffer: NZUsize!(16 * 1024 * 1024),   // 16 MB
        write_buffer: NZUsize!(16 * 1024 * 1024),    // 16 MB
        leader_timeout: CONSENSUS_LEADER_TIMEOUT,
        certification_timeout: CONSENSUS_CERTIFICATION_TIMEOUT,
        timeout_retry: CONSENSUS_TIMEOUT_RETRY,
        fetch_timeout: CONSENSUS_FETCH_TIMEOUT,
        activity_timeout: CONSENSUS_ACTIVITY_TIMEOUT,
        skip_timeout: CONSENSUS_SKIP_TIMEOUT,
        fetch_concurrent: 32,
        page_cache,
        forwarding: simplex::ForwardingPolicy::SilentLeader,
    },
);
engine.start(transport.simplex.votes, transport.simplex.certs, transport.simplex.resolver);
```

Key design choices:
- **ForwardingPolicy::SilentLeader**: Leader doesn't gossip own proposals
- **16 MB replay/write buffers**: Large buffers for journal
- **Random elector**: VRF-based fair leader selection

---

## 2. Reporter Chain

Reporters process consensus activity events in a nested chain:

```
Simplex Engine
    → SeedReporter (caches VRF seeds)
        → NodeStateReporter (updates RPC-visible counters)
            → FinalizedReporter (persists blocks, prunes mempool)
                → MarshalMailbox (forwards tips to application)
```

### Activity Types (reporters/lib.rs:459-475)

| Activity | Handler | Effect |
|----------|---------|--------|
| `Notarization` | `set_view(view)` | Updates current view counter |
| `Finalization` | `set_view(view)`, `inc_finalized()` | Updates view + increments finalized count |
| `Nullification` | `inc_nullified()` | Increments nullified counter |

---

## 3. FinalizedReporter — The Finalization Pipeline

**File:** `crates/node/reporters/src/lib.rs` (Lines 105-232)

### Full Pipeline

```
Finalized Block Received (via Update::Block)
    │
    ├── PHASE 1: VERIFICATION & RE-EXECUTION (if needed)
    │   ├── If snapshot missing: re-execute block txs
    │   ├── Verify state root matches
    │   └── Cache snapshot for future use
    │
    ├── PHASE 2: PERSISTENCE
    │   ├── Spawn async persist task
    │   ├── persist_snapshot(digest) → QMDB commit
    │   └── If persist fails → EARLY RETURN (BUG)
    │
    ├── PHASE 3: INDEX & PRUNE
    │   ├── Index block for RPC queries (if block_index available)
    │   └── prune_mempool(&block.txs)  ← ONLY REACHED IF PERSIST SUCCEEDS
    │
    └── ack.acknowledge()  ← Always called (even on error)
```

### The Early Return Bug (Lines 216-219)

```rust
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    ack.acknowledge();
    return;  // BUG: Returns WITHOUT calling prune_mempool (line 226)
}
```

**Consequences:**
1. `persist_snapshot()` fails (disk full, I/O error, QMDB corruption)
2. Error is logged, ack is sent to Marshal
3. Function returns WITHOUT pruning mempool
4. Finalized transactions stay in mempool
5. Next proposal re-includes stale transactions → executor fails → nullification
6. Repeat forever → permanent stall

**All early return paths and their pruning behavior:**

| Phase | Failure | Line | Prune Called? |
|-------|---------|------|---------------|
| Execution fails | BlockExecution::execute error | 142-146 | NO |
| Root computation fails | compute_root error | 154-158 | NO |
| State root mismatch | Computed ≠ expected | 160-169 | NO |
| Parent snapshot missing | Non-cached block | 196-200 | NO |
| Persist task spawn fails | Tokio spawn error | 210-214 | NO |
| **Persist fails** | **QMDB commit error** | **216-220** | **NO (BUG)** |
| Success | All phases complete | 226 | YES |

The execution-phase early returns are arguably acceptable (block may not be truly final if execution disagrees). But the persistence-phase early return is a **confirmed bug** — the block IS consensus-final, and transactions must be pruned regardless of persistence outcome.

---

## 4. LedgerState — Single Mutex Design

**File:** `crates/node/ledger/src/lib.rs`

### Structure (Lines 56-68)

```rust
pub struct LedgerView {
    inner: Arc<Mutex<LedgerState>>,  // SINGLE MUTEX for all state
    genesis_block: Block,
}

struct LedgerState {
    mempool: InMemoryMempool,
    snapshots: InMemorySnapshotStore<OverlayState<QmdbState>>,
    seeds: InMemorySeedTracker,
    qmdb: QmdbLedger,
}
```

### Contention Points

| Operation | Lock Required | Duration |
|-----------|---------------|----------|
| `submit_tx()` | Yes (write) | Microseconds |
| `proposal_components()` | Yes (read) | Microseconds |
| `parent_snapshot()` | Yes (read) | Microseconds |
| `insert_snapshot()` | Yes (write) | Microseconds |
| `persist_snapshot()` | Yes (2 acquisitions) | Milliseconds-seconds |
| `prune_mempool()` | Yes (write) | Microseconds |
| `compute_root_from_store()` | Yes (read) | Microseconds |

The single mutex serializes all operations. Under high throughput, proposal building blocks on persistence.

### persist_snapshot() — Double Lock Pattern (Lines 289-333)

```
LOCK 1: Read snapshot chain, mark as persisting, extract changes → UNLOCK 1
    ↓
NO LOCK: qmdb.commit_changes(changes) → Can take ms-seconds
    ↓
LOCK 2: Clear persisting marker, compact snapshots → UNLOCK 2
```

This is safe from deadlock (lock released before async operation), but the double-lock pattern means other operations can interleave between the two phases.

**Danger:** If the task panics between LOCK 1 and LOCK 2, the "persisting" marker is never cleared, blocking all future persistence attempts until restart.

---

## 5. Consensus Failure Modes

### Mode A: Nullification (build_block returns None)

**Trigger:** Any transaction fails in executor, parent snapshot missing, or root computation fails.

**Recovery:**
1. Current view is nullified
2. Simplex advances to next view
3. Random elector picks new leader
4. New leader calls propose()

**Cost:** 2-4 seconds per nullified view (leader_timeout + certification_timeout)

**Problem:** If the same poisoned transactions are in every validator's mempool, ALL leaders will fail, creating an infinite nullification loop.

### Mode B: Verification Rejection (verify_block returns false)

**Trigger:** Re-execution produces different result or state root mismatch.

**Recovery:**
1. Rejecting validator doesn't participate in certification
2. If enough validators reject → insufficient 2/3+ → timeout → view change

**Worst case:** State divergence between validators. If one validator's state differs, it can never verify blocks from others. That validator is effectively isolated.

### Mode C: View Change

**Trigger:** Leader timeout (2s), certification timeout (4s), or activity timeout (256 views).

**Recovery:**
1. View advances
2. New leader elected
3. New proposal attempt

**Cost:** With 2s leader timeout and 256-view activity timeout, a persistent stall could last ~512 seconds (8.5 minutes) before activity timeout triggers.

### Mode D: Permanent Stall (observed)

**Trigger:** Poisoned transactions + executor abort + no pruning

**Sequence:**
```
Load test sends many transactions with rapidly incrementing nonces
    ↓
Some transactions become stale (nonce already consumed by pending blocks)
    ↓
Stale transactions accepted into mempool (no nonce validation in InMemoryMempool)
    ↓
Leader proposes block containing stale transactions
    ↓
Executor hits stale nonce → ? operator → ExecutionError → abort entire block
    ↓
build_block() returns None → view nullified
    ↓
prune_mempool() NOT called (only called on successful finalization)
    ↓
Same stale transactions proposed again → same failure → permanent loop
```

---

## 6. Consensus Constants Reference

**File:** `crates/node/runner/src/runner.rs`

```rust
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_secs(2);
const CONSENSUS_CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(4);
const CONSENSUS_TIMEOUT_RETRY: Duration = Duration::from_secs(1);
const CONSENSUS_FETCH_TIMEOUT: Duration = Duration::from_secs(1);
const CONSENSUS_ACTIVITY_TIMEOUT: ViewDelta = ViewDelta::new(256);
const CONSENSUS_SKIP_TIMEOUT: ViewDelta = ViewDelta::new(32);
const BLOCK_CODEC_MAX_TXS: usize = 10_000;
const BLOCK_CODEC_MAX_TX_BYTES: usize = 8 * 1024 * 1024;  // 8 MiB
const SIGNATURE_THREADS: usize = 2;
const EPOCH_LENGTH: u64 = u64::MAX;
const PARTITION_PREFIX: &str = "kora";
const MAILBOX_SIZE: usize = 1024;  // (from simplex defaults)
```

---

## 7. Runner Startup Sequence

**File:** `crates/node/runner/src/runner.rs` (Lines 329-578)

1. Initialize transport & register validators with P2P Oracle
2. Initialize storage archives (finalization certs, finalized blocks, QMDB)
3. Create LedgerService with LedgerView
4. Spawn ledger event observers
5. Recover finalized blocks from archive
6. Setup RPC server (optional) with transaction submission callback
7. Setup Prometheus metrics endpoint (optional)
8. Initialize FinalizedReporter → SeedReporter → NodeStateReporter chain
9. Initialize Marshal actor (block archiving, page cache)
10. Initialize Broadcast engine (block gossip)
11. Create RevmApplication (proposal/verification)
12. Wrap in Inline marshaler
13. Configure and start Simplex consensus engine
14. Log "Validator started successfully"
