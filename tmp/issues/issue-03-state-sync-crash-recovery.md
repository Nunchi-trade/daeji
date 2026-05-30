# State Sync and Crash Recovery for Node Restarts

## Priority

**P0 -- Stability & Correctness (most critical issue in the set)**

## Summary

Kora has no mechanism for a restarted validator to rejoin an active chain. When a node restarts -- whether from a crash, OOM kill, maintenance, or rolling upgrade -- it starts consensus at view 1 with no execution state, while the rest of the network may be at view 80,000+. The restarted node becomes a "phantom voter" that tracks consensus views but can never propose or verify blocks. In a 4-validator cluster, restarting 2 nodes causes permanent chain death.

This issue documents the complete problem, including the tmpfs root cause, and proposes a phased solution covering persistent storage, local block replay, peer-to-peer state download, and shared block execution infrastructure. It is designed to be self-contained: an implementing agent should be able to build the entire state sync system from this document alone.

**Prerequisite**: Issue 23 (KORA_RUNTIME_DIR points to tmpfs) must be resolved first. Without persistent storage, there is no local state to recover from, and Phases 1-2 of this issue are ineffective.

## Problem Description

### The restart failure chain

When a validator container restarts, the following cascade occurs:

```
Container stops (SIGTERM, OOM, crash)
        |
        v
tmpfs /runtime is destroyed (all Commonware runtime data lost)
        |
        v
Container restarts via restart: unless-stopped
        |
        v
Commonware runtime opens empty storage directory
        |
        +--- Consensus journals: empty (starts at view 1)
        +--- Finalized block archive (kora-finalized-blocks): empty
        +--- Finalization cert archive (kora-finalizations-by-height): empty
        +--- QMDB partitions (accounts, storage, code): empty
        |
        v
recover_finalized_state() finds nothing to recover
        |
        v
Node starts consensus engine at view 1 with genesis-only state
        |
        v
View counter fast-forwards to ~80,000 via peer consensus messages
BUT: finalized count stays at 0, proposed count stays at 0
        |
        v
When elected leader: build_block() fails (parent snapshot missing)
When verifying peers: verify_block() returns false (parent snapshot missing)
        |
        v
Resolver interprets verify_block()=false as "peer sent invalid data"
NoOpBlocker (PR #131) prevents permanent ban, but resolver gives up
        |
        v
Node is permanently stuck as a phantom voter
```

### Why state is lost on restart

**File**: `docker/compose/devnet.yaml`, lines 42-43 (`x-validator-common`)

```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  tmpfs:
    - /runtime:size=1g,mode=1777    # ALL runtime state lives here
  environment:
    - KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
```

The `KORA_RUNTIME_DIR` environment variable directs the Commonware runtime to store all its data at `/runtime`, which is a tmpfs mount. tmpfs is an in-memory filesystem destroyed when the container stops.

**File**: `crates/node/runner/src/runner.rs`, lines 95-110

```rust
const RUNTIME_DIR_ENV: &str = "KORA_RUNTIME_DIR";

pub fn runtime_storage_directory(data_dir: &Path) -> PathBuf {
    runtime_storage_directory_from(data_dir, std::env::var_os(RUNTIME_DIR_ENV))
}

fn runtime_storage_directory_from(data_dir: &Path, override_dir: Option<OsString>) -> PathBuf {
    match override_dir {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => data_dir.join("runtime"),
    }
}
```

When `KORA_RUNTIME_DIR` is set, the runtime uses that path. When unset, it falls back to `data_dir/runtime` (under the persistent `/data` volume). The code comment even acknowledges the tmpfs trade-off:

> Local devnets can set `KORA_RUNTIME_DIR` to put consensus journals on tmpfs and avoid Docker-volume fsync latency.

Everything under the runtime directory is destroyed on restart:
- **Simplex consensus journals** (view state, proposals, votes, finalizations)
- **Finalized block archive** (`kora-finalized-blocks`) -- all block history
- **Finalization certificate archive** (`kora-finalizations-by-height`) -- all certificates
- **QMDB partitions** (accounts, storage, code) -- all EVM state
- **Marshal state** -- block delivery tracking

The persistent Docker volume (`data_nodeN` mounted at `/data`) only stores DKG keys, node configuration, and the commit marker file. The actual blockchain state is ephemeral.

### Why `recover_finalized_state()` cannot help

**File**: `crates/node/runner/src/runner.rs`, lines 162-223

```rust
async fn recover_finalized_state<FB, FC>(
    ledger: &LedgerService,
    block_index: &Arc<kora_indexer::BlockIndex>,
    finalized_blocks: &FB,
    finalizations_by_height: &FC,
    provider: &RevmContextProvider,
    data_dir: &Path,
) -> anyhow::Result<()>
```

This function iterates over the finalized block archive and the finalization certificate archive to restore VRF seeds, rebuild the block index, and create a single HEAD snapshot via `restore_persisted_snapshot()`. After a tmpfs restart, both archives are empty, so this function does nothing. Even if the archives survived (persistent storage), the function only restores a single HEAD snapshot -- not the chain of snapshots needed for block verification.

`restore_persisted_snapshot()` (in `crates/node/ledger/src/lib.rs`, line 347) creates a snapshot with an empty overlay pointing at the current QMDB state:

```rust
pub async fn restore_persisted_snapshot(&self, block: &Block) {
    let inner = self.inner.lock().await;
    let digest = block.commitment();
    let state = OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
    let snapshot = Snapshot::new(
        Some(block.parent()),
        state,
        block.state_root,
        QmdbChangeSet::default(),
        tx_ids(&block.txs),
    );
    inner.snapshots.insert(digest, snapshot);
    inner.snapshots.mark_persisted(&[digest]);
}
```

### Why the resolver cannot catch up

The Commonware resolver (initialized via `PeerInitializer` in `crates/network/marshal/src/peers.rs`) fetches missed blocks from peers and passes them to `verify_block()` in `crates/node/runner/src/app.rs`.

`verify_block()` (lines 194-274) requires the parent block's execution snapshot:

```rust
let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
    warn!(?digest, ?parent_digest, height = block.height, "missing parent snapshot");
    return false;
};
```

After restart, the parent snapshot does not exist. `verify_block()` returns `false`. The resolver interprets this as the peer sending invalid data. PR #131 introduced `NoOpBlocker` to prevent permanent peer bans, but the resolver still gives up after receiving "invalid data" -- it does not retry.

`verify()` (lines 379-434) in the `VerifyingApplication` trait collects unverified blocks from the ancestry stream and attempts to verify them oldest-first. But the oldest unverified block's parent snapshot does not exist, so the entire chain fails:

```rust
fn verify<A>(
    &mut self,
    _context: (Env, Self::Context),
    mut ancestry: AncestorStream<A, Self::Block>,
) -> impl std::future::Future<Output = bool> + Send
where
    A: BlockProvider<Block = Self::Block>,
{
    async move {
        let mut blocks_to_verify = Vec::new();
        while let Some(block) = ancestry.next().await {
            let digest = block.commitment();
            if self.ledger.query_state_root(digest).await.is_some() {
                break;
            }
            blocks_to_verify.push(block);
        }
        // Verify from oldest (parent) to newest (tip)
        for block in blocks_to_verify.into_iter().rev() {
            if !self.verify_block(&block).await {
                return false;
            }
        }
        true
    }
}
```

### Why `build_block()` also fails

When the restarted node is elected as leader, `build_block()` (lines 91-192) also fails because the parent snapshot is missing:

```rust
let parent_snapshot = match self.ledger.parent_snapshot(parent_digest).await {
    Some(snap) => snap,
    None => {
        warn!(parent_height = parent.height, ?parent_digest,
              "build_block: parent snapshot not found");
        return None;
    }
};
```

This triggers the 5-second `leader_timeout_secs` (line 31 in `crates/node/config/src/consensus.rs`) followed by nullification. At ~89 blocks/s baseline throughput, each wasted leader round significantly impacts overall throughput.

### Observed impact from testing

| Scenario | Expected Behavior | Actual Behavior |
|----------|-------------------|-----------------|
| Stop 1 of 4 validators | ~75% throughput | 99.9% throughput loss (0.08 blocks/s vs 89 baseline) |
| Restart stopped validator | Catch up in ~60s, resume | Stuck at genesis forever, phantom voter |
| Rolling restart of all 4 | Each catches up before next stops | All chain history permanently lost |
| 2-node simultaneous restart | Temporary stall, recover when both return | Permanent network deadlock (0 blocks/s) |
| OOM crash + auto-restart | Rejoin chain automatically | Fresh fork from genesis |

Log evidence from node2 after restart (network at height 58,271):

```
WARN kora_runner::app: build_block: parent snapshot not found
     parent_height=58271  parent_digest=7c27d3...

WARN kora_runner::app: propose failed: build_block returned None
     (likely missing parent snapshot -- node may still be catching up)

WARN commonware_resolver::p2p::engine: invalid data received
     peer=9ff6a9f5e681...

WARN kora_runner::runner: NoOpBlocker: ignoring block request for peer
     (catch-up safe)  peer=9ff6a9f5e681...
```

Node status comparison 30 seconds after restart:

| Field | Healthy Nodes (0,1,3) | Restarted Node (2) |
|-------|----------------------|-------------------|
| currentView | 79,378 | 79,378 (synchronized) |
| finalizedCount | 58,271 | 2 |
| proposedCount | ~13,000-16,000 | 0 |
| nullifiedCount | ~21,110 | 8 |
| peerCount | 3 | 3 |

## Root Cause Analysis

The crash recovery failure has three independent root causes:

### 1. All state lives on tmpfs (Issue 23)

The `KORA_RUNTIME_DIR=/runtime` environment variable combined with the `tmpfs: /runtime:size=1g` mount directive means every piece of state is destroyed on container restart. This is the foundational root cause. Without persistent storage, there is nothing to recover from.

### 2. EVM verification requires sequential parent state

The Commonware resolver assumes any block can be independently verified. EVM block verification is inherently sequential: verifying block N requires re-executing all its transactions against block N-1's state. This sequential dependency chains back to genesis (or the last persisted state). When the parent snapshot is missing, verification cannot proceed.

The resolver has no way to distinguish "peer sent genuinely invalid data" from "I cannot verify this right now because I am missing parent state." Both cases cause `verify_block()` to return `false`.

### 3. Recovery only restores HEAD, not the snapshot chain

Even if state survived restart (via persistent storage), `recover_finalized_state()` only restores a single HEAD snapshot. It does not replay blocks through the executor to rebuild the intermediate snapshot chain. The snapshot store after recovery contains exactly one entry (the HEAD). Since `verify_block(H)` requires `parent_snapshot(H-1)`, and only `parent_snapshot(HEAD)` exists, verification fails for any block where H-1 != HEAD.

### 4. No state sync protocol exists

There is no mechanism for downloading blocks or state from peers outside of the Commonware resolver's backfill mechanism. The resolver was designed for catching up within an active consensus session (missed a few blocks in normal operation), not for bootstrapping from scratch. There is no:
- Block range download RPC or P2P protocol
- State snapshot transfer mechanism
- Chain history replay procedure
- Checkpoint export/import system

## The Two Recovery Scenarios

Recovery behavior depends on how long the node was offline relative to snapshot retention:

### Scenario A: Short outage (< snapshot retention window)

The node was down for less time than the snapshot retention covers. With persistent storage, the local finalized block archive and QMDB state survive. The node can recover by replaying recent finalized blocks from its local archive to rebuild the snapshot cache, then resume consensus.

**Limitations of the current snapshot retention window**: The `InMemorySnapshotStore` retains at most 64 persisted snapshots (`DEFAULT_MAX_PERSISTED_RETAINED` in `crates/node/consensus/src/components/snapshot.rs`, line 24). At ~89 blocks/s observed throughput, 64 blocks represents less than 1 second of chain time. This means the replay window covers a sub-second outage -- impractical for any real restart scenario.

To make local replay useful, the snapshot retention or the block replay window must be substantially larger. However, snapshot retention affects memory usage (each snapshot holds an `OverlayState` with pending changes). A more practical approach is to keep the in-memory snapshot retention at 64 but replay from the finalized block archive directly, which is stored on disk and not bounded by the in-memory limit.

### Scenario B: Long outage (> snapshot retention window)

The node was down long enough that even the finalized block archive does not cover the gap (or the archive was lost, as in the tmpfs case). The node must download blocks or state from peers. This requires a new P2P protocol.

**The tmpfs case (current devnet) is always Scenario B**: since all state is lost, the node starts from genesis every time.

```
                       Restart
                         |
                         v
  +------------------+--------+------------------+
  |   Scenario A     |        |   Scenario B     |
  | Local state OK   |        | No local state   |
  |                  |        | (tmpfs/data loss) |
  +------------------+        +------------------+
  |                  |        |                  |
  | Replay last N    |        | Download blocks  |
  | finalized blocks |        | from peers       |
  | from local       |        | (new P2P proto)  |
  | archive          |        |                  |
  +--------+---------+        +--------+---------+
           |                           |
           v                           v
    Rebuild snapshot            Verify via
    cache via re-exec           certificates
           |                           |
           +----------+---------------+
                      |
                      v
           Resume consensus engine
```

## Proposed Solution

### Architecture Overview

```
+------------------------------------------------------------------+
|                    Node Startup Flow                              |
+------------------------------------------------------------------+
|                                                                   |
|  1. Load DKG keys + config from /data (persistent)               |
|  2. Open Commonware runtime (runtime_storage_directory)          |
|  3. Open QMDB partitions (accounts, storage, code)              |
|  4. recover_finalized_state()                                    |
|     - Restore VRF seeds from cert archive                        |
|     - Rebuild block index from block archive                     |
|     - Restore HEAD snapshot from QMDB                            |
|                                                                   |
|  5. [NEW] Determine recovery mode:                               |
|     +----------------------------------------------------------+ |
|     | local_height = finalized_blocks.last_index()             | |
|     | peer_height  = query_peers_for_chain_status()            | |
|     |                                                          | |
|     | if local_height == 0 && peer_height > 0:                 | |
|     |     --> Scenario B: Full state sync from peers           | |
|     | elif peer_height - local_height > SYNC_THRESHOLD:        | |
|     |     --> Scenario B: Partial state sync from peers        | |
|     | elif local_height > 0 && gap <= replay_window:           | |
|     |     --> Scenario A: Local block replay                   | |
|     | else:                                                    | |
|     |     --> No sync needed, proceed to consensus             | |
|     +----------------------------------------------------------+ |
|                                                                   |
|  6. [NEW] Execute recovery (Scenario A or B)                     |
|  7. Start Simplex consensus engine                               |
|  8. Node is operational                                          |
+------------------------------------------------------------------+
```

### Phase 0: Persistent storage (prerequisite -- covered by Issue 23)

Move consensus state off tmpfs to persistent storage. Without this, every restart loses all state and Phases 1-2 are useless.

**The fix is a 5-minute change to `docker/compose/devnet.yaml`**: either remove `KORA_RUNTIME_DIR` from the environment (so `runtime_storage_directory()` falls back to `/data/runtime` on the persistent volume) or add named runtime volumes per validator.

After this fix:
- Consensus journals survive restart
- Finalized block and cert archives survive restart
- QMDB partitions survive restart
- `recover_finalized_state()` finds data to recover from

See Issue 23 for the complete fix specification.

### Phase 1: Block replay from local archive (Scenario A recovery)

When a node restarts with persistent storage and the finalized block archive is intact, replay recent finalized blocks through the executor to rebuild the in-memory snapshot cache. This must happen after `recover_finalized_state()` and before starting the Simplex consensus engine.

#### Why this is needed even with persistent storage

`recover_finalized_state()` restores a single HEAD snapshot via `restore_persisted_snapshot()`. But `verify_block(H)` requires `parent_snapshot(H-1)`, and only `parent_snapshot(HEAD)` exists. During the time between restart and the first new finalization, the node cannot verify any blocks because no snapshot chain exists beyond HEAD. Replaying the last N blocks rebuilds the snapshot chain so the node can immediately participate in consensus.

#### Shared block execution utility

Both normal finalization (`finalize_block()` in `crates/node/reporters/src/lib.rs`, lines 169-279) and the replay function need to execute a block against parent state, compute the state root, and insert the resulting snapshot. This logic should be factored into a shared utility to avoid duplication.

**File**: `crates/node/consensus/src/execution.rs` (extend existing `BlockExecution`)

The existing `BlockExecution::execute()` handles the executor call. The shared utility should extend this to include the full pipeline:

```rust
/// Execute a block against parent state and produce a verified snapshot.
///
/// This performs the complete execution pipeline used by both
/// finalization (FinalizedReporter) and startup replay:
///   1. Execute transactions against parent snapshot
///   2. Compute deterministic state root
///   3. Validate state root matches the block's committed root
///   4. Build the next overlay state
///
/// The caller is responsible for inserting the resulting snapshot
/// into the snapshot store and persisting to QMDB.
pub struct VerifiedExecution {
    /// The execution outcome (changes, receipts, gas).
    pub outcome: ExecutionOutcome,
    /// The computed state root (verified to match block.state_root).
    pub state_root: StateRoot,
    /// The merged overlay state for the next snapshot.
    pub next_state: OverlayState<QmdbState>,
}

impl VerifiedExecution {
    pub async fn execute_and_verify<E>(
        parent_snapshot: &Snapshot<OverlayState<QmdbState>>,
        executor: &E,
        context: &BlockContext,
        block: &Block,
        ledger: &LedgerService,
        parent_digest: ConsensusDigest,
    ) -> Result<Self, ConsensusError>
    where
        E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    {
        let execution = BlockExecution::execute(
            parent_snapshot, executor, context, &block.txs
        ).await?;

        let state_root = ledger
            .compute_root_from_store(parent_digest, execution.outcome.changes.clone())
            .await
            .map_err(|e| ConsensusError::Execution(e.to_string()))?;

        if state_root != block.state_root {
            return Err(ConsensusError::Execution(format!(
                "state root mismatch: expected {:?}, computed {:?}",
                block.state_root, state_root
            )));
        }

        let merged_changes = parent_snapshot
            .state
            .merge_changes(execution.outcome.changes.clone());
        let next_state = OverlayState::new(
            parent_snapshot.state.base(),
            merged_changes,
        );

        Ok(Self {
            outcome: execution.outcome,
            state_root,
            next_state,
        })
    }
}
```

This utility is then used by:
- `replay_finalized_blocks()` (Phase 1, below)
- `finalize_block()` (existing reporter logic -- can be refactored to use it)
- `verify_block()` and `build_block()` in app.rs (longer term refactor)

#### The replay function

**New function in `crates/node/runner/src/runner.rs`:**

```rust
/// Replay the last `window_size` finalized blocks through the executor
/// to rebuild the in-memory snapshot cache, enabling the node to verify
/// and propose blocks for the current chain tip.
///
/// This function must be called after `recover_finalized_state()` and
/// before starting the Simplex consensus engine.
///
/// For each replayed block:
/// 1. Load the block from the finalized block archive
/// 2. Look up the parent snapshot (restored by recover_finalized_state
///    for the HEAD, or built by a previous iteration for subsequent blocks)
/// 3. Execute the block's transactions against the parent state
/// 4. Verify the computed state root matches the block's committed root
/// 5. Insert the resulting snapshot into the in-memory store
///
/// QMDB state root validation (step 4) is critical: it detects
/// corruption in the local archive. If a replayed block's state root
/// does not match, the function logs an error and returns Err,
/// signalling the caller to fall back to Scenario B (peer sync).
async fn replay_finalized_blocks<FB>(
    ledger: &LedgerService,
    finalized_blocks: &FB,
    executor: &RevmExecutor,
    context_provider: &RevmContextProvider,
    window_size: u64,
) -> anyhow::Result<()>
where
    FB: Archive<Key = ConsensusDigest, Value = Block>,
{
    let Some(last_index) = finalized_blocks.last_index() else {
        return Ok(()); // No history, nothing to replay
    };

    let start = last_index.saturating_sub(window_size);
    info!(start, end = last_index, "replaying finalized blocks");

    for height in start..=last_index {
        let Some(block) = finalized_blocks
            .get(ArchiveId::Index(height))
            .await?
        else {
            continue;
        };

        let digest = block.commitment();
        let parent_digest = block.parent();

        // Skip if we already have this snapshot (e.g., HEAD restored
        // by recover_finalized_state).
        if ledger.query_state_root(digest).await.is_some() {
            trace!(height, "replay: snapshot already exists, skipping");
            continue;
        }

        let Some(parent_snapshot) = ledger.parent_snapshot(parent_digest).await
        else {
            // If this is the first block in the window and the parent
            // is the HEAD snapshot restored by recover_finalized_state,
            // this should succeed. If it doesn't, the archive is
            // inconsistent with QMDB.
            warn!(
                height,
                ?parent_digest,
                "replay: parent snapshot not found; \
                 archive may be inconsistent with QMDB"
            );
            continue;
        };

        let block_context = context_provider.context(&block);
        let txs_bytes: Vec<Bytes> =
            block.txs.iter().map(|tx| tx.bytes.clone()).collect();
        let outcome = executor
            .execute(&parent_snapshot.state, &block_context, &txs_bytes)
            .map_err(|e| anyhow::anyhow!(
                "replay: execution failed at height {height}: {e}"
            ))?;

        // CRITICAL: Validate state root matches the committed block.
        // Without this check, replaying from a corrupted archive
        // silently produces wrong state.
        let state_root = ledger
            .compute_root_from_store(parent_digest, outcome.changes.clone())
            .await
            .map_err(|e| anyhow::anyhow!(
                "replay: compute root failed at height {height}: {e}"
            ))?;

        if state_root != block.state_root {
            return Err(anyhow::anyhow!(
                "replay: state root mismatch at height {height}: \
                 expected {:?}, computed {:?}. \
                 Local archive is corrupted; falling back to peer sync.",
                block.state_root, state_root
            ));
        }

        let merged_changes =
            parent_snapshot.state.merge_changes(outcome.changes.clone());
        let next_state = OverlayState::new(
            parent_snapshot.state.base(),
            merged_changes,
        );

        ledger
            .insert_snapshot(
                digest,
                parent_digest,
                next_state,
                state_root,
                outcome.changes,
                &block.txs,
            )
            .await;
    }

    let replayed = window_size.min(last_index + 1);
    info!(replayed, "snapshot cache rebuilt from finalized archive");
    Ok(())
}
```

#### Integration point in `ProductionRunner::run()`

In `crates/node/runner/src/runner.rs`, after the `recover_finalized_state()` call (around line 565) and before starting the Simplex engine (around line 755):

```rust
// After recover_finalized_state():
recover_finalized_state(
    &ledger, &block_index, &finalized_blocks,
    &finalizations_by_height, &context_provider, &config.data_dir,
).await.context("recover finalized state")?;

// NEW: Replay recent finalized blocks to rebuild snapshot cache.
if let Err(e) = replay_finalized_blocks(
    &ledger,
    &finalized_blocks,
    &RevmExecutor::new(self.chain_id),
    &context_provider,
    64, // replay window matches DEFAULT_MAX_PERSISTED_RETAINED
).await {
    warn!(
        error = %e,
        "local replay failed; node will attempt peer sync \
         when state sync protocol is available"
    );
    // In Phase 1, log and continue -- the node will operate as a
    // non-producing phantom voter until Phase 2 is implemented.
    // In Phase 2+, this triggers Scenario B (peer sync).
}
```

#### Replay window sizing

The replay `window_size` should match `DEFAULT_MAX_PERSISTED_RETAINED` (64) from the snapshot store. Replaying 64 blocks rebuilds exactly the number of snapshots the store retains during normal operation. The window is configurable:

```rust
// In crates/node/config/src/lib.rs or a new sync.rs:
pub struct SyncConfig {
    /// Number of recent finalized blocks to replay on startup.
    /// Default: 64 (matches snapshot store retention).
    pub replay_window: u64,
}
```

**Performance impact**: For 64 empty blocks at ~7ms each, replay adds ~450ms of startup time. For 64 blocks with 200 transactions each, it could add several seconds. This is acceptable for crash recovery.

#### Increasing replay effectiveness

64 blocks at 89 blocks/s covers less than 1 second of chain time. For local replay to handle outages longer than 1 second, the replay window must be much larger. However, the replay window is bounded by two factors:

1. **Startup latency**: Replaying 10,000 blocks at 7ms/block would add ~70 seconds to startup time. This may be acceptable for crash recovery but should be configurable.

2. **QMDB state correspondence**: The HEAD snapshot from `restore_persisted_snapshot()` points at the current QMDB base state. Replaying blocks that were already persisted to QMDB will produce incorrect state roots because the QMDB base state already includes those changes. The replay must start from the last persisted block and only replay blocks whose changes have not yet been committed to QMDB. The commit marker (`crates/node/runner/src/commit_marker.rs`) tracks the last committed digest and can be used to determine this boundary.

**Recommended approach**: Start replay from the block identified by the commit marker (last persisted to QMDB), not from `last_index - window_size`. This allows the replay to cover an arbitrary gap between the last QMDB commit and the archive head:

```rust
let replay_start = match read_commit_marker(&config.data_dir) {
    Some(marker_digest) => {
        // Find the height of the committed block in the archive.
        // Replay from the next block after the committed one.
        find_height_of_digest(&finalized_blocks, marker_digest)
            .map(|h| h + 1)
            .unwrap_or(start)
    }
    None => start,
};
```

### Phase 2: State sync protocol from peers (Scenario B recovery)

When a node starts with no finalized block archive (tmpfs restart, data loss, or new node joining), it must download blocks or state from peers. This requires a new P2P protocol.

#### New crate: `crates/node/sync/`

```
crates/node/sync/
  src/
    lib.rs          -- Public API and SyncService
    protocol.rs     -- P2P message definitions
    downloader.rs   -- Block download state machine
    verifier.rs     -- Certificate-based block trust
    server.rs       -- Serve sync requests from peers
```

#### P2P channel allocation

The sync protocol needs a dedicated channel separate from the existing 5 channels. Register channel 5 in the transport layer:

**File**: `crates/network/transport/src/channels.rs`

```rust
// Existing channels:
pub const CHANNEL_VOTES: u64 = 0;
pub const CHANNEL_CERTS: u64 = 1;
pub const CHANNEL_RESOLVER: u64 = 2;
pub const CHANNEL_BLOCKS: u64 = 3;
pub const CHANNEL_BACKFILL: u64 = 4;

// NEW:
/// Channel ID for state sync messages.
pub const CHANNEL_SYNC: u64 = 5;
```

**File**: `crates/network/transport/src/transport.rs`

```rust
pub struct NetworkTransport<P: PublicKey, E: Clock> {
    pub oracle: discovery::Oracle<P>,
    pub handle: Handle<()>,
    pub simplex: SimplexChannels<P, E>,
    pub marshal: MarshalChannels<P, E>,
    pub sync: SyncChannels<P, E>,  // NEW
}
```

#### Protocol messages

```rust
// crates/node/sync/src/protocol.rs

use kora_domain::{Block, ConsensusDigest};

/// Request the peer's current chain status.
#[derive(Debug, Encode, Decode)]
pub struct ChainStatusRequest;

/// Response with the peer's chain tip.
#[derive(Debug, Encode, Decode)]
pub struct ChainStatusResponse {
    /// Highest finalized block height.
    pub finalized_height: u64,
    /// Current consensus view.
    pub current_view: u64,
    /// Genesis block digest (to verify chain identity).
    pub genesis_digest: ConsensusDigest,
}

/// Request a range of finalized blocks from a peer.
#[derive(Debug, Encode, Decode)]
pub struct BlockRangeRequest {
    /// Start height (inclusive).
    pub start: u64,
    /// End height (inclusive). Capped at batch_size by server.
    pub end: u64,
}

/// Response containing finalized blocks and their certificates.
#[derive(Debug, Encode, Decode)]
pub struct BlockRangeResponse {
    /// Finalized blocks in height order.
    pub blocks: Vec<Block>,
    /// Finalization certificates proving each block was finalized.
    pub certificates: Vec<Finalization<ThresholdScheme, ConsensusDigest>>,
}
```

#### Certificate-based verification

During sync, blocks are NOT verified by re-executing and comparing state roots (the normal `verify_block()` path). Re-execution from genesis would take hours for a long chain. Instead, blocks are trusted based on their finalization certificates.

A valid finalization certificate contains a threshold BLS signature from 2f+1 validators, which cryptographically proves the block was finalized by the network. This is safe because:
- The threshold signature cannot be forged without compromising f+1 validators
- The finalization certificate binds to the specific block digest (content-addressed)
- The last batch of synced blocks is re-executed (Phase 1 replay) to verify the final state root against QMDB, catching any corruption

```rust
// crates/node/sync/src/verifier.rs

pub struct CertificateVerifier {
    scheme: ThresholdScheme,
}

impl CertificateVerifier {
    pub fn new(scheme: ThresholdScheme) -> Self {
        Self { scheme }
    }

    /// Verify a finalization certificate is valid for the given block.
    ///
    /// Checks the threshold BLS signature against the known validator set.
    /// Does NOT re-execute the block.
    pub fn verify(
        &self,
        block: &Block,
        cert: &Finalization<ThresholdScheme, ConsensusDigest>,
    ) -> bool {
        let digest = block.commitment();
        cert.proposal.payload == digest
            && self.scheme.verify_certificate(cert)
    }
}
```

#### Block download state machine

```rust
// crates/node/sync/src/downloader.rs

pub struct BlockDownloader {
    local_height: u64,
    target_height: u64,
    batch_size: u64,
    verifier: CertificateVerifier,
}

impl BlockDownloader {
    /// Execute the full sync protocol:
    ///
    /// 1. Query all connected peers for ChainStatusResponse
    /// 2. Select the peer with the highest finalized_height
    ///    (verify genesis_digest matches ours)
    /// 3. Download blocks in batches of batch_size
    /// 4. For each batch:
    ///    a. Verify each finalization certificate against threshold scheme
    ///    b. Apply blocks to QMDB (fast path: commit without re-execution)
    ///    c. Update the finalized block and cert archives
    ///    d. Write commit marker after each batch
    /// 5. After all batches: run Phase 1 replay on last replay_window
    ///    blocks to rebuild snapshot cache with full execution verification
    ///
    /// The post-sync replay in step 5 is critical: it re-executes the
    /// most recent blocks and validates state roots against QMDB,
    /// detecting any corruption in the downloaded data.
    pub async fn sync(
        &mut self,
        transport: &SyncChannels,
        ledger: &LedgerService,
        finalized_blocks: &impl Archive<Key = ConsensusDigest, Value = Block>,
        finalizations: &impl Archive<Key = ConsensusDigest, Value = CertArchive>,
    ) -> Result<(), SyncError> {
        // Query peers
        let peer_status = self.query_best_peer(transport).await?;
        if peer_status.finalized_height <= self.local_height {
            return Ok(()); // Already caught up
        }
        self.target_height = peer_status.finalized_height;

        info!(
            local = self.local_height,
            target = self.target_height,
            gap = self.target_height - self.local_height,
            "starting block download from peers"
        );

        // Download in batches
        let mut current = self.local_height + 1;
        while current <= self.target_height {
            let batch_end = (current + self.batch_size - 1)
                .min(self.target_height);

            let response = self.download_range(
                transport, current, batch_end
            ).await?;

            // Verify certificates
            for (block, cert) in response.blocks.iter()
                .zip(response.certificates.iter())
            {
                if !self.verifier.verify(block, cert) {
                    return Err(SyncError::InvalidCertificate {
                        height: block.height,
                    });
                }
            }

            // Apply to archives (without re-execution)
            for (block, cert) in response.blocks.iter()
                .zip(response.certificates.iter())
            {
                finalized_blocks.insert(block).await?;
                finalizations.insert(cert).await?;
            }

            // Write commit marker at batch boundary
            if let Some(last_block) = response.blocks.last() {
                write_commit_marker(data_dir, &last_block.commitment())?;
            }

            info!(
                from = current,
                to = batch_end,
                remaining = self.target_height - batch_end,
                "downloaded batch"
            );
            current = batch_end + 1;
        }

        Ok(())
    }
}
```

#### Integration with the runner

In `ProductionRunner::run()`, after `recover_finalized_state()`:

```rust
let local_height = finalized_blocks.last_index().unwrap_or(0);

// Phase 1: Try local replay first (cheaper than peer sync)
if local_height > 0 {
    match replay_finalized_blocks(
        &ledger, &finalized_blocks, &executor,
        &context_provider, config.sync.replay_window,
    ).await {
        Ok(()) => {
            info!(local_height, "local replay succeeded");
            // Proceed to consensus
        }
        Err(e) => {
            warn!(error = %e, "local replay failed, falling back to peer sync");
            // Fall through to Phase 2
        }
    }
}

// Phase 2: Peer sync if needed
let peer_status = query_peer_chain_status(&transport.sync).await;
if peer_status.finalized_height > local_height + config.sync.sync_threshold {
    info!(
        local = local_height,
        remote = peer_status.finalized_height,
        "node is behind; starting state sync from peers"
    );
    let mut downloader = BlockDownloader::new(
        local_height,
        config.sync.batch_size,
        CertificateVerifier::new(self.scheme.clone()),
    );
    downloader.sync(
        &transport.sync, &ledger, &finalized_blocks,
        &finalizations_by_height,
    ).await.context("state sync from peers")?;

    // Re-run Phase 1 replay on the newly downloaded blocks
    // to rebuild the snapshot cache with full execution verification
    replay_finalized_blocks(
        &ledger, &finalized_blocks, &executor,
        &context_provider, config.sync.replay_window,
    ).await.context("post-sync replay")?;

    info!("state sync complete");
}

// NOW start consensus engine
let engine = simplex::Engine::new(/* ... */);
```

#### Sync server (responding to peer sync requests)

Each node must also serve sync requests from peers. This runs as a background task:

```rust
// crates/node/sync/src/server.rs

pub struct SyncServer {
    finalized_blocks: Arc<dyn Archive<Key = ConsensusDigest, Value = Block>>,
    finalizations: Arc<dyn Archive<Key = ConsensusDigest, Value = CertArchive>>,
    max_batch_size: u64,
}

impl SyncServer {
    /// Handle an incoming BlockRangeRequest.
    ///
    /// Returns at most max_batch_size blocks. The response
    /// is bounded to prevent a single request from consuming
    /// excessive memory or bandwidth.
    pub async fn handle_range_request(
        &self,
        request: BlockRangeRequest,
    ) -> Result<BlockRangeResponse, SyncError> {
        let end = request.end.min(
            request.start + self.max_batch_size - 1
        );
        let mut blocks = Vec::new();
        let mut certificates = Vec::new();

        for height in request.start..=end {
            if let Some(block) = self.finalized_blocks
                .get(ArchiveId::Index(height)).await?
            {
                if let Some(cert) = self.finalizations
                    .get(ArchiveId::Index(height)).await?
                {
                    blocks.push(block);
                    certificates.push(cert);
                }
            }
        }

        Ok(BlockRangeResponse { blocks, certificates })
    }

    /// Handle a ChainStatusRequest.
    pub async fn handle_status_request(
        &self,
    ) -> ChainStatusResponse {
        let finalized_height = self.finalized_blocks
            .last_index()
            .unwrap_or(0);
        ChainStatusResponse {
            finalized_height,
            current_view: 0, // Updated by consensus reporter
            genesis_digest: /* loaded from config or archive index 0 */,
        }
    }
}
```

### Phase 3: View recovery / consensus fast-forward

After syncing blocks and rebuilding state, the node needs to fast-forward its Simplex consensus view. Today, a restarted node starts at view 1 but immediately receives consensus messages at the current network view (e.g., 80,000). The Simplex engine handles this via internal view synchronization -- the node advances views via received notarizations and nullifications. However, this creates a window during which the node votes on views it does not understand.

**Current behavior (workable)**: View sync via consensus messages occurs within seconds. The node catches up to the current view rapidly. During this window, the node may vote in rounds where its state is incomplete, but these votes are effectively ignored by peers (the node has no proposals to contribute). This is a performance nuisance, not a correctness issue.

**Ideal upstream fix**: The `simplex::Engine` should accept an optional starting view in its config:

```rust
simplex::Config {
    starting_view: Some(last_finalization_cert.proposal.round.view()),
    // ...existing fields...
}
```

This requires a Commonware framework change. The Kora-side preparation:

```rust
// In ProductionRunner::run(), after sync completes:
let starting_view = if let Some(last_cert) = last_finalization_cert {
    last_cert.proposal.round.view()
} else {
    View::new(1)
};
```

### Phase 4: Consensus checkpoint on persistent storage

As an additional durability measure, periodically checkpoint critical consensus state to the persistent `/data` volume. This allows faster recovery without full archive replay.

```rust
// crates/node/runner/src/consensus_checkpoint.rs

#[derive(Serialize, Deserialize)]
pub struct ConsensusCheckpoint {
    /// Last finalized block digest and height.
    pub finalized_head: (ConsensusDigest, u64),
    /// Current consensus view.
    pub current_view: u64,
    /// Timestamp.
    pub timestamp: u64,
}

/// Write checkpoint using atomic rename (same pattern as commit_marker.rs).
pub fn write_checkpoint(
    data_dir: &Path, checkpoint: &ConsensusCheckpoint,
) -> io::Result<()> {
    let tmp = data_dir.join("consensus_checkpoint.tmp");
    let final_path = data_dir.join("consensus_checkpoint");
    let encoded = bincode::serialize(checkpoint)?;
    let mut f = File::create(&tmp)?;
    f.write_all(&encoded)?;
    f.sync_all()?;
    fs::rename(&tmp, &final_path)?;
    Ok(())
}
```

**Trigger**: Every N finalized blocks from `FinalizedReporter` (default: 100).

**On startup**: Read checkpoint, use `current_view` as starting view hint, validate against commit marker.

## Interaction with Existing Components

### `NoOpBlocker` (PR #131)

PR #131 introduced `NoOpBlocker` in `crates/node/runner/src/runner.rs` (lines 55-89) to prevent the resolver from permanently banning peers when `verify_block()` returns false during catch-up. This fix remains necessary even with state sync:

```rust
struct NoOpBlocker<P> {
    _marker: std::marker::PhantomData<P>,
}

impl<P: commonware_cryptography::PublicKey> Blocker for NoOpBlocker<P> {
    type PublicKey = P;
    fn block(&mut self, peer: Self::PublicKey)
        -> impl std::future::Future<Output = ()> + Send
    {
        warn!(?peer, "NoOpBlocker: ignoring block request for peer");
        async {}
    }
}
```

During state sync, there will be a transient period where the resolver receives blocks that the node cannot yet verify (sync is still in progress). The `NoOpBlocker` ensures these transient failures do not permanently ban peers.

**Remaining gap**: The resolver itself still gives up after receiving "invalid data" responses. It does not retry with backoff. This means the resolver's built-in backfill is unreliable for any scenario where the node is behind by more than a trivial number of blocks. The state sync protocol (Phase 2) bypasses the resolver entirely with a purpose-built download protocol.

### `InMemorySnapshotStore` (PR #125)

The bounded eviction policy (max 64 persisted snapshots, `DEFAULT_MAX_PERSISTED_RETAINED` in `crates/node/consensus/src/components/snapshot.rs`) works correctly with the replay window. After sync and replay, the snapshot store will contain up to 64 snapshots, and the eviction policy maintains this bound during normal operation.

The eviction logic preserves the `persisted` marker even after evicting snapshot data, which is critical for the `merged_changes()` and `changes_for_persist()` chain-walking functions.

### `commit_marker.rs`

The commit marker (`crates/node/runner/src/commit_marker.rs`) tracks the last block digest persisted to QMDB. The `validate_commit_marker()` function (runner.rs lines 225-260) checks it against the archive head on startup:

```rust
fn validate_commit_marker(data_dir: &Path, archive_head: &Block) {
    let marker_digest = crate::commit_marker::read_commit_marker(data_dir);
    let head_digest = archive_head.commitment();
    match marker_digest {
        None => { info!("no commit marker found"); }
        Some(marker) if marker == head_digest => {
            info!("commit marker matches archive head; QMDB consistent");
        }
        Some(marker) => {
            warn!("commit marker does not match archive head; \
                   QMDB may be behind or inconsistent");
        }
    }
}
```

With persistent storage, this check becomes more important. A mismatch means QMDB was partially updated before a crash. The sync protocol should detect this and re-sync from the last known-good height. Currently the node only logs a warning and proceeds, which risks state divergence.

**Recommendation**: Upgrade the mismatch case from warn to error, and use the commit marker to determine the safe replay start point rather than proceeding with potentially inconsistent state.

### `FinalizedReporter`

The reporter's `handle_finalized_update()` function (`crates/node/reporters/src/lib.rs`, lines 112-161) already handles the case where a snapshot does not exist for a finalized block -- it re-executes using the parent snapshot:

```rust
if !snapshot_exists || block_index.is_some() {
    if let Some(parent_snapshot) = state.parent_snapshot(parent_digest).await {
        // Re-execute block
    }
}
```

This code path is exercised during sync when finalized blocks arrive from the consensus engine before the sync protocol has processed them. The reporter will attempt to re-execute, and if the parent snapshot is available (from replay), it will succeed.

### Transport layer

The existing transport (`crates/network/transport/src/transport.rs`) bundles 5 P2P channels:
- Channel 0: Simplex votes
- Channel 1: Simplex certificates
- Channel 2: Simplex resolver
- Channel 3: Marshal blocks
- Channel 4: Marshal backfill

The sync protocol adds Channel 5. This requires updates to:
- `crates/network/transport/src/channels.rs` (new `SyncChannels` struct)
- `crates/network/transport/src/transport.rs` (add `sync` field to `NetworkTransport`)
- `crates/network/transport/src/builder.rs` (register channel 5 in `build_local_transport`)
- `crates/network/transport-sim/src/provider.rs` (register in test transport)

### Marshal actor configuration

The marshal actor (`crates/network/marshal/src/actor.rs`) has a `DEFAULT_MAX_REPAIR` of 128, which limits how many blocks it will attempt to repair (fetch from peers) at once. This is separate from the state sync batch size and does not need modification. The marshal's repair mechanism uses the resolver/backfill channels, while state sync uses its own channel.

## Configuration

```rust
// crates/node/config/src/sync.rs (new file)

/// Configuration for state sync and crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Number of recent finalized blocks to replay on startup
    /// for snapshot cache warmup.
    #[serde(default = "default_replay_window")]
    pub replay_window: u64,

    /// Number of blocks to download per batch in peer sync.
    #[serde(default = "default_batch_size")]
    pub batch_size: u64,

    /// Minimum height gap to trigger block download from peers.
    /// If the node is fewer than this many blocks behind,
    /// rely on the resolver's built-in backfill instead.
    #[serde(default = "default_sync_threshold")]
    pub sync_threshold: u64,

    /// How often to write a consensus checkpoint (every N blocks).
    #[serde(default = "default_checkpoint_interval")]
    pub checkpoint_interval: u64,
}

const fn default_replay_window() -> u64 { 64 }
const fn default_batch_size() -> u64 { 256 }
const fn default_sync_threshold() -> u64 { 10 }
const fn default_checkpoint_interval() -> u64 { 100 }
```

## File Changes Summary

| Phase | File | Change |
|-------|------|--------|
| 0 | `docker/compose/devnet.yaml` | Remove tmpfs, add runtime volumes or remove `KORA_RUNTIME_DIR` (Issue 23) |
| 1 | `crates/node/runner/src/runner.rs` | Add `replay_finalized_blocks()`, call after `recover_finalized_state()` |
| 1 | `crates/node/consensus/src/execution.rs` | Add `VerifiedExecution` shared utility |
| 1 | `crates/node/reporters/src/lib.rs` | Refactor `finalize_block()` to use `VerifiedExecution` |
| 2 | `crates/node/sync/` (new crate) | Block download protocol, certificate verifier, sync server |
| 2 | `crates/network/transport/src/channels.rs` | Add `CHANNEL_SYNC` and `SyncChannels` |
| 2 | `crates/network/transport/src/transport.rs` | Add `sync` field to `NetworkTransport` |
| 2 | `crates/network/transport/src/builder.rs` | Register channel 5 |
| 2 | `crates/network/transport-sim/src/provider.rs` | Register channel 5 in test transport |
| 2 | `crates/node/runner/src/runner.rs` | Integrate sync before consensus start |
| 3 | `crates/node/runner/src/runner.rs` | Pass starting view to Simplex (when upstream supports it) |
| 4 | `crates/node/runner/src/consensus_checkpoint.rs` (new) | Checkpoint read/write |
| 4 | `crates/node/reporters/src/lib.rs` | Trigger periodic checkpointing from `FinalizedReporter` |
| All | `crates/node/config/src/sync.rs` (new) | `SyncConfig` with replay_window, batch_size, sync_threshold |
| All | `crates/node/config/src/lib.rs` | Add `sync` field to `NodeConfig` |

## Implementation Order

### Phase 0: Persistent storage (Issue 23) -- 30 minutes

Prerequisite. Fix the 5-minute Docker Compose change so all subsequent phases can be tested.

### Phase 1: Local block replay -- 1-2 days

1. Implement `VerifiedExecution` in `crates/node/consensus/src/execution.rs`
2. Implement `replay_finalized_blocks()` in `crates/node/runner/src/runner.rs`
3. Integrate into `ProductionRunner::run()` after `recover_finalized_state()`
4. Add `SyncConfig` with `replay_window` field
5. Upgrade `validate_commit_marker()` to use commit marker as replay start point
6. Test: restart a single validator, verify it catches up within 60 seconds

### Phase 2: Peer state sync -- 3-5 days

1. Create `crates/node/sync/` crate with protocol messages
2. Add `CHANNEL_SYNC` to transport layer
3. Implement `CertificateVerifier`
4. Implement `BlockDownloader` state machine
5. Implement `SyncServer` for serving peer requests
6. Integrate into runner: detect sync gap, download blocks, run Phase 1 replay
7. Test: start a fresh node (no prior state), verify it syncs from peers

### Phase 3: View recovery -- 0.5 days (Kora side) + upstream PR

1. Determine starting view from last finalization certificate
2. Pass to Simplex engine config (requires upstream Commonware change)
3. Fallback: current view sync via consensus messages (already works)

### Phase 4: Consensus checkpointing -- 1 day

1. Implement `consensus_checkpoint.rs` (atomic write/read)
2. Wire into `FinalizedReporter` on configurable interval
3. Read checkpoint on startup, use for faster recovery
4. Test: restart after checkpoint, verify recovery is faster

## Upstream Commonware Changes Required

1. **Simplex starting view hint**: The `simplex::Engine` should accept an optional starting view in its config so restarted nodes skip past already-finalized views. Without this, the node starts at view 1 and fast-forwards via consensus messages (workable but wastes time).

2. **Resolver retry semantics**: The resolver should distinguish between "peer sent garbage" (decode failure) and "application cannot verify right now" (verify returns false). The latter should trigger retry with exponential backoff. The `NoOpBlocker` is a Kora-side workaround; the resolver itself still gives up after receiving "invalid data."

3. **Archive range reads**: The `Archive` trait's `get(ArchiveId::Index(height))` interface works for single reads but is not optimized for sequential reads of thousands of blocks. A `get_range(start, end)` method would improve sync server performance.

## Testing Plan

### Unit tests

- [ ] `VerifiedExecution::execute_and_verify()` with matching state root (success)
- [ ] `VerifiedExecution::execute_and_verify()` with mismatching state root (error)
- [ ] `replay_finalized_blocks()` with empty archive (no-op)
- [ ] `replay_finalized_blocks()` with populated archive (rebuilds snapshot cache)
- [ ] `replay_finalized_blocks()` detects corrupted block (state root mismatch)
- [ ] `CertificateVerifier::verify()` with valid certificate (success)
- [ ] `CertificateVerifier::verify()` with invalid certificate (failure)
- [ ] `CertificateVerifier::verify()` with wrong block digest (failure)
- [ ] `BlockDownloader` batching logic (correct start/end per batch)
- [ ] `BlockDownloader` handles peer disconnect (retries with other peers)
- [ ] `ConsensusCheckpoint` serialization round-trip
- [ ] `SyncServer` bounds response to max_batch_size
- [ ] `SyncConfig` serde defaults applied correctly

### Integration tests (devnet)

- [ ] **Phase 0 prerequisite**: After Issue 23 fix, verify `recover_finalized_state()` finds data
- [ ] **Single node restart**: Stop node3, wait 60s, restart. Verify catch-up within 120s.
- [ ] **Rolling restart**: Stop/restart each of 4 validators sequentially. Verify chain continuity.
- [ ] **Two-node simultaneous restart**: Stop nodes 2+3, restart both. Verify recovery within 5 minutes.
- [ ] **OOM crash recovery**: Set 2GB memory limit, run loadgen until OOM. Verify auto-recovery.
- [ ] **Fresh node join**: Start a 5th validator with no state. Verify it syncs via block download.
- [ ] **State consistency after sync**: Query `eth_getBalance`, `eth_getBlockByNumber` on synced node vs healthy node. Verify identical results at same height.
- [ ] **Corrupted archive detection**: Tamper with a finalized block in the archive. Verify replay detects the mismatch and reports an error.

### Performance benchmarks

- [ ] Replay startup time with window sizes 16, 64, 256, 1024
- [ ] Block download throughput (blocks/sec) over local network
- [ ] QMDB commit latency: persistent storage vs tmpfs
- [ ] Memory usage during sync (peak RSS with batch_size 256 vs 1024)

## References

### Source files

| File | Relevance |
|------|-----------|
| `crates/node/runner/src/runner.rs` | Core runner, `NoOpBlocker`, `runtime_storage_directory`, `recover_finalized_state`, `ProductionRunner::run()` |
| `crates/node/runner/src/app.rs` | `verify_block()`, `build_block()`, `verify()` -- the functions that fail after restart |
| `crates/node/runner/src/commit_marker.rs` | Atomic commit marker for QMDB crash consistency validation |
| `crates/node/consensus/src/components/snapshot.rs` | `InMemorySnapshotStore`, bounded eviction (64 persisted max) |
| `crates/node/consensus/src/traits.rs` | `Snapshot`, `SnapshotStore` trait definitions |
| `crates/node/consensus/src/execution.rs` | `BlockExecution::execute()` -- shared execution utility |
| `crates/node/ledger/src/lib.rs` | `LedgerView`, `restore_persisted_snapshot()`, `compute_root_from_store()`, snapshot persistence |
| `crates/node/reporters/src/lib.rs` | `FinalizedReporter`, `finalize_block()`, `handle_finalized_update()` |
| `crates/node/config/src/consensus.rs` | `leader_timeout_secs` (5s), simplex tuning params |
| `crates/network/marshal/src/peers.rs` | `PeerInitializer` -- resolver with 200ms timeout |
| `crates/network/marshal/src/actor.rs` | `ActorInitializer` -- marshal defaults (max_repair=128) |
| `crates/network/transport/src/channels.rs` | Channel IDs 0-4, `SimplexChannels`, `MarshalChannels` |
| `crates/network/transport/src/transport.rs` | `NetworkTransport` bundle |
| `crates/storage/qmdb/src/root.rs` | `StateRoot::transition()` -- deterministic consensus root computation |
| `docker/compose/devnet.yaml` | tmpfs config, `KORA_RUNTIME_DIR`, named volumes, resource limits |

### Related issues and PRs

| Reference | Description | Relationship |
|-----------|-------------|--------------|
| Issue 23 | KORA_RUNTIME_DIR tmpfs persistence | **Prerequisite** -- fixes root cause of total state loss |
| PR #131 | `NoOpBlocker` -- prevents resolver from permanently blocking peers | Partial fix; resolver still gives up after "invalid data" |
| PR #125 | Snapshot store bounded eviction (max 64) | Sets the natural replay window size |
| PR #132 | Docker resource limits, RPC health checks, log rotation | Devnet infrastructure |

### Test reports

| Report | Key findings |
|--------|-------------|
| `tmp/devnet-test-summary-2026-05-22.md` | Full devnet test summary |
| `tmp/controlled-single-node-failure.md` | Restarted node at block 19, network at 58,271 |
| `tmp/resolver-catchup-failure.md` | Resolver blocks all peers within 50ms |
| `tmp/rolling-restart-cascading-failure.md` | Rolling restarts destroy all chain history |
