# Missing Backpressure Between Consensus and Execution

## Summary

The Commonware Simplex BFT consensus engine advances views as fast as the network allows, with no mechanism to wait for the execution layer to finish processing the previous block. On the Kora devnet, this results in approximately 100-150 empty blocks per second, causing unbounded growth in consensus metadata (approximately 2 KB per block in heap, plus approximately 4.6 KB per block in tmpfs freezer tables), snapshot eviction before finalization can consume them, and tmpfs exhaustion within approximately 24 minutes.

This issue covers only the backpressure problem (consensus outpacing execution during normal operation). Node catch-up after restart is a separate problem tracked in Issue 03 / PR #131.

## Problem

### Consensus produces blocks faster than execution can consume them

The Simplex consensus engine (configured in `crates/node/runner/src/runner.rs`, lines 755-782) advances to the next view immediately after the current view is notarized. There is no minimum block interval and no requirement that block N be finalized (or even executed) before block N+1 is proposed.

The consensus-to-execution data flow is:

```
Simplex Engine                  Application (app.rs)              Reporter (reporters/src/lib.rs)
     |                                |                                    |
     | propose(view N)                |                                    |
     |-----> build_block()            |                                    |
     |       (needs parent snapshot)  |                                    |
     |                                |                                    |
     | verify(view N)                 |                                    |
     |-----> verify_block()           |                                    |
     |       (needs parent snapshot)  |                                    |
     |                                |                                    |
     | notarize(view N)               |                                    |
     | IMMEDIATELY advance to N+1     |                                    |
     |                                |                                    |
     |                                |    finalize(view N)                |
     |                                |    -----> handle_finalized_update() |
     |                                |           execute block             |
     |                                |           persist to QMDB           |
     |                                |           prune mempool             |
     |                                |           ack.acknowledge()         |
```

The critical gap: consensus does not wait for finalization to complete before advancing. The `propose` and `verify` calls in `RevmApplication` (`crates/node/runner/src/app.rs`) return futures that the engine awaits, but finalization (the `FinalizedReporter::report` path in `crates/node/reporters/src/lib.rs`) runs asynchronously via the marshal. There is no feedback channel from finalization back to the engine.

### Measured block production rate

From devnet measurements (resource-exhaustion-timeline.md, 2026-05-22):

| Metric | Value | Source |
|--------|-------|--------|
| Block production rate | approximately 150 blocks/sec | resource-exhaustion-timeline.md (750 blocks per 5s sample) |
| Heap memory growth rate | approximately 0.25-0.34 MB/sec | resource-exhaustion-timeline.md (controlled 85s window) |
| Memory growth per block | approximately 1.7-2.0 KB | 24 MiB across 12,184 blocks |
| tmpfs freezer growth | approximately 4.6 KB/block (2 tables) | memory-growth-analysis.md (232 MB across 50,000 blocks) |
| Baseline memory | approximately 1,062 MiB (fresh start) | resource-exhaustion-timeline.md |
| Memory at time of crashes | 1.1-1.13 GiB (28% of 4 GiB limit) | resource-exhaustion-timeline.md |

### Devnet crashes are NOT OOM

The original long-running stability test reported exit code 137 crashes within 1-5 minutes and attributed them to OOM. The controlled resource-exhaustion-timeline investigation (same date) disproved this:

- `OOMKilled=false` on all containers at every observed crash.
- No kernel OOM messages in `dmesg` or `journalctl`.
- Memory was at 28% of the 4 GiB limit at every crash point.
- Docker events showed `kill -> stop -> die -> destroy -> create` sequences (Docker Compose redeploy, not cgroup OOM).

The actual failure mode is consensus liveness loss (nodes getting stuck after restart, quorum loss cascading), not memory exhaustion. Memory growth at approximately 0.3 MB/sec would take approximately 2.8 hours to reach the 4 GiB limit from a 1.06 GiB baseline. However, tmpfs exhaustion is a real risk (see Impact section).

### Where the per-block overhead accumulates

Each block produces overhead that accumulates:

1. **Heap consensus state** (approximately 1.7-2.0 KB/block): Notarizations, votes, view metadata, and block data accumulate in the Simplex engine's internal data structures and are not directly managed by Kora. At 150 bps, this produces approximately 0.3 MB/sec of heap growth.

2. **tmpfs freezer tables** (approximately 4.6 KB/block): Two append-only archives on the `/runtime` tmpfs:
   - `kora-finalizations-by-height-freezer-table`: 116 MB after 50,000 blocks
   - `kora-finalized-blocks-freezer-table`: 116 MB after 50,000 blocks
   - At 150 bps, this produces approximately 42 MB/min (approximately 0.7 MB/sec) of tmpfs growth.
   - Note: tmpfs is NOT counted toward the container cgroup memory limit -- it is a host-kernel mount.

3. **Snapshot store**: Each proposed/verified block creates a snapshot entry. The 64-snapshot retention cap bounds this but means the window covers only 0.4 seconds at 150 bps.

4. **Marshal delivery buffer**: Finalized blocks queue in the marshal's delivery channel waiting for the `FinalizedReporter` to process and acknowledge them.

### The acknowledge mechanism is not backpressure

The finalization path in `crates/node/reporters/src/lib.rs` (lines 127-158) calls `ack.acknowledge()` after processing each finalized block:

```rust
// crates/node/reporters/src/lib.rs, lines 127-158
Update::Block(block, ack) => {
    let result = finalize_block(/* ... */).await;
    // ... index, prune mempool ...
    // Marshal waits for the application to acknowledge processing before advancing the
    // delivery floor. Without this, the node can stall on finalized block delivery.
    ack.acknowledge();  // Line 158
}
```

The marshal waits for this acknowledgement before advancing its delivery floor (comment on lines 156-157). However, this only gates delivery of more finalized blocks to the reporter -- it does not gate consensus from proposing new views. The Simplex engine and the marshal operate independently: the engine proposes and notarizes blocks while the marshal delivers finalized blocks to the reporter. If finalization is slow, the marshal's delivery queue grows, but the engine keeps advancing.

## Impact

### 1. tmpfs exhaustion (primary risk)

The `/runtime` tmpfs is 1 GiB. The freezer tables grow at approximately 42 MB/min at 150 bps. With 256 MB already used:

- tmpfs fills in approximately 18-24 minutes.
- When tmpfs is full, consensus fails to write archive data and the node crashes or halts.
- At a normal block rate (1 block/sec), freezer growth drops to approximately 0.28 MB/min, giving approximately 46 hours before exhaustion -- effectively a non-issue.

### 2. Snapshot eviction cascade

At 150 blocks/sec, the 64-snapshot retention window covers only 0.4 seconds. If finalization falls behind by even 1 second, the snapshot it needs has been evicted, causing the "missing parent snapshot" error in `finalize_block` (`crates/node/reporters/src/lib.rs`, line 255). This forces re-execution from a potentially stale parent, or fails entirely with `Err(())`.

### 3. Slow heap growth

Heap memory grows at approximately 0.3 MB/sec during 150 bps empty-block consensus. This projects to an OOM at the 4 GiB container limit after approximately 2.8 hours from a fresh start. While not the acute failure mode (consensus liveness fails first), it represents a slow leak that would eventually cause problems in any long-running deployment.

## Root Cause

There is no backpressure mechanism between the Simplex consensus engine and the Kora execution/finalization layer. The engine's view advancement is gated only by network latency (vote/notarize round-trips), not by execution readiness.

In the `RevmApplication::propose()` method (`crates/node/runner/src/app.rs`, lines 313-370), the only check before building a block is whether the parent snapshot exists:

```rust
// crates/node/runner/src/app.rs, lines 91-107
async fn build_block(&self, parent: &Block, timestamp: u64) -> Option<Block> {
    use kora_consensus::Mempool as _;

    let start = Instant::now();
    let parent_digest = parent.commitment();
    let parent_snapshot = match self.ledger.parent_snapshot(parent_digest).await {
        Some(snap) => snap,
        None => {
            warn!(
                parent_height = parent.height,
                ?parent_digest,
                "build_block: parent snapshot not found — \
                 node has not yet processed this parent block"
            );
            return None;
        }
    };
    // ... build the block ...
}
```

When `build_block` returns `None`, the `propose()` method (lines 358-364) logs a warning. The consensus engine treats this as an absent proposal and the view times out after `leader_timeout_secs` (default 5 seconds, configured at `crates/node/config/src/consensus.rs`, line 31). This wastes 5 seconds per failed proposal but does not slow down the overall view advancement rate for successful proposals.

## Proposed Fix

### Option A: Execution-gated proposal via finalization tracking (recommended)

Add a finalization readiness check to `RevmApplication::build_block()`. Before building a block, verify that the most recently finalized block has been fully persisted. This approach returns `None` from `build_block` when execution is behind, which causes the proposal to be skipped.

#### Implementation

1. Add a `last_persisted_height` `AtomicU64` to `LedgerService` (or a shared state accessible from both `FinalizedReporter` and `RevmApplication`).

2. In `handle_finalized_update` (`crates/node/reporters/src/lib.rs`, line 158), after `ack.acknowledge()`, update the persisted height:

```rust
// crates/node/reporters/src/lib.rs, after line 158
state.set_last_persisted_height(block.height);
ack.acknowledge();
```

3. In `RevmApplication::build_block` (`crates/node/runner/src/app.rs`, before line 96), check the gap:

```rust
// crates/node/runner/src/app.rs - in build_block(), before parent_snapshot lookup
let parent_height = parent.height;
let persisted_height = self.ledger.last_persisted_height();
let gap = parent_height.saturating_sub(persisted_height);
if gap > MAX_FINALIZATION_GAP {
    warn!(
        parent_height,
        persisted_height,
        gap,
        max = MAX_FINALIZATION_GAP,
        "build_block: execution is behind finalization, deferring proposal"
    );
    return None;
}
```

4. Set `MAX_FINALIZATION_GAP` to a reasonable value (e.g., 16 blocks). This allows some pipelining while preventing unbounded divergence.

#### Trade-offs

- Returning `None` from `propose()` triggers a view timeout (default 5 seconds via `leader_timeout_secs`). This means the chain pauses for 5 seconds whenever execution falls behind by more than 16 blocks. This is acceptable for a devnet but might need tuning for production.
- The `propose()` method in the Commonware `Application` trait returns `impl Future<Output = Option<Self::Block>> + Send` (see `crates/node/runner/src/app.rs`, line 317). It is awaited by the engine, so the future can take time (e.g., it already does I/O to fetch snapshots and execute transactions). However, intentionally sleeping inside `propose()` would block the entire consensus engine loop for that duration, preventing vote processing and other consensus work. Returning `None` is safer because it lets the engine proceed with its timeout and nullification logic.

### Option B: Channel-based backpressure between consensus and execution

Instead of checking a shared atomic, use a bounded channel between finalization and proposal to create natural backpressure.

#### Implementation

1. Create a bounded `tokio::sync::Semaphore` (or `tokio::sync::mpsc::channel` with a small capacity) shared between `FinalizedReporter` and `RevmApplication`.

2. In `RevmApplication::propose()`, acquire a permit before building. If finalization is keeping up, permits are available immediately. If finalization falls behind, `propose()` blocks until a permit is released.

3. In `handle_finalized_update`, release a permit after `ack.acknowledge()`.

```rust
// Shared between RevmApplication and FinalizedReporter
let backpressure = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT_BLOCKS));

// In propose() -- acquire before building
let _permit = self.backpressure.acquire().await.ok()?;
let block = self.build_block(&parent, timestamp).await;

// In handle_finalized_update -- release after persisting
ack.acknowledge();
backpressure.add_permits(1);
```

#### Trade-offs

- This blocks the `propose()` future, which blocks the consensus engine loop. While the engine awaits `propose()`, it cannot process incoming votes or notarizations. This is functionally similar to sleeping in `propose()`.
- The semaphore count (`MAX_INFLIGHT_BLOCKS`) controls the pipeline depth. A value of 1 means fully synchronous (propose one, finalize one, propose next). A value of 16 allows pipelining while bounding the gap.
- This is more complex than Option A but provides tighter feedback control -- it blocks only as long as needed, rather than triggering a fixed 5-second timeout.

### Option C: Minimum block interval (simplest, Kora-side)

Add a configurable minimum block interval to `ConsensusSimplexConfig`:

```rust
// crates/node/config/src/consensus.rs
pub struct ConsensusSimplexConfig {
    // ... existing fields ...

    /// Minimum interval between block proposals in milliseconds.
    /// Set to 0 for no minimum (current behavior).
    #[serde(default = "default_min_block_interval_ms")]
    pub min_block_interval_ms: u64,
}

const fn default_min_block_interval_ms() -> u64 {
    100  // 100ms = max 10 blocks/sec
}
```

Enforce this in `RevmApplication::propose()` by tracking the last proposal timestamp and sleeping if the interval has not elapsed.

#### Trade-offs

- Sleeping in `propose()` blocks the consensus engine loop. The `propose()` method returns `impl Future<Output = Option<Self::Block>> + Send` (`crates/node/runner/src/app.rs`, line 317) and the engine awaits it. During the sleep, the engine cannot process votes, notarizations, or other consensus messages. This may cause timeouts or missed votes on other validators.
- This approach does not provide true backpressure -- it sets a fixed rate ceiling regardless of whether execution is keeping up. If execution takes 200ms per block and the interval is 100ms, execution still falls behind.
- At 100ms intervals, block production drops from approximately 150 bps to 10 bps. This reduces tmpfs growth from approximately 42 MB/min to approximately 2.8 MB/min (tmpfs lasts approximately 4.5 hours instead of approximately 24 minutes) and heap growth from approximately 0.3 MB/sec to approximately 0.02 MB/sec.
- This is the simplest option to implement (under an hour of work) and immediately stabilizes the devnet. It can serve as a stopgap while Option A or Option B is developed.

### Option D: Upstream Commonware SDK change (ideal, longer-term)

The ideal fix is for the Simplex engine to support an explicit "execution readiness" signal. The engine would not advance to view N+1 until the application confirms readiness. This would require a new method on the `Application` trait in the Commonware SDK:

```rust
// Upstream in commonware-consensus
trait Application {
    // Existing
    fn propose(&mut self, ...) -> impl Future<Output = Option<Block>> + Send;

    // New: engine calls this before starting a new view
    fn ready(&self) -> impl Future<Output = ()> + Send;
}
```

This is the cleanest solution but requires coordination with the Commonware SDK maintainers.

## Files Involved

| File | Lines | Role |
|------|-------|------|
| `crates/node/runner/src/app.rs` | 91-107 | `build_block()` -- no finalization check before proposing |
| `crates/node/runner/src/app.rs` | 313-370 | `propose()` -- returns `impl Future<Output = Option<Block>>`, awaited by engine |
| `crates/node/reporters/src/lib.rs` | 112-161 | `handle_finalized_update()` -- ack does not gate consensus |
| `crates/node/reporters/src/lib.rs` | 169-279 | `finalize_block()` -- execution, root computation, persistence |
| `crates/node/runner/src/runner.rs` | 755-782 | Simplex engine configuration -- no block interval setting |
| `crates/node/config/src/consensus.rs` | 74-126 | `ConsensusSimplexConfig` -- no min block interval field |
| `crates/node/consensus/src/components/snapshot.rs` | 24 | `DEFAULT_MAX_PERSISTED_RETAINED = 64` (covers only 0.4s at 150 bps) |

## Relationship to Other Issues

- **Issue 03 (Catch-Up After Restart)**: Separate problem. Issue 03 covers the scenario where a restarted node cannot rejoin because the resolver's `verify_block()` fails on missing parent snapshots and blocks the peer. The backpressure problem described here occurs during normal operation when all nodes are online. The two issues compound (high block rate means the catch-up gap grows faster), but fixing backpressure alone does not fix catch-up, and fixing catch-up alone does not fix backpressure.
- **Issue 09 (Snapshot Store Race Condition)**: The snapshot eviction problem is dramatically amplified by the high block rate. At 10 bps instead of 150 bps, the 64-snapshot window covers 6.4 seconds instead of 0.4 seconds, making eviction-before-use far less likely.
- **PR #125 (Snapshot Store Bounded Eviction)**: The 64-snapshot cap introduced by this PR is correct in principle but insufficient without backpressure to limit the rate at which snapshots are produced and consumed.

## Priority

**High** -- This is a root cause of devnet resource exhaustion (tmpfs, not heap OOM) and snapshot eviction cascades during normal operation. Option C (minimum block interval) can be implemented in under an hour and immediately stabilizes the devnet by reducing block rate to approximately 10 bps. Option A (finalization-gated proposal) provides a more robust long-term solution with actual backpressure semantics.
