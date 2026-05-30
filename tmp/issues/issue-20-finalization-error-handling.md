# Finalization error handling: silent failures, untyped errors, and missing retry logic

## Summary

The block finalization path -- the code responsible for persisting consensus-agreed blocks to the state database -- has several error handling gaps that can cause **silent state divergence** between a node and the rest of the network. Specifically:

1. **Five distinct failure modes in `finalize_block()` all return `Err(())`** with no structured error information, no retry, and no recovery path. A consensus-finalized block that fails to persist is permanently lost from that node's state.
2. **`build_block()` in `app.rs` returns `None` for three unrelated failure causes** (catch-up, execution failure, root computation failure), making it impossible to distinguish expected behavior from critical errors in logs, metrics, or calling code.
3. **No retry logic exists for transient failures** in finalization. A single I/O hiccup or momentary memory pressure during QMDB persistence causes permanent state loss.

These issues compound: when finalization silently drops a block, the node continues participating in consensus with stale state. It may vote on future blocks whose execution depends on state it never persisted, leading to cascading verification failures.

**Connection to Issue 19 (Application-Level Metrics):** Typed errors naturally provide labels for Prometheus error counters (e.g., `kora_finalization_error_total{cause="state_root_mismatch"}` vs `{cause="execution_failed"}`). These two issues should be implemented together so that the error enums introduced here become the metric labels in Issue 19.

## Affected Files

| File | Role |
|------|------|
| `crates/node/reporters/src/lib.rs` | `finalize_block()` -- the primary finalization code path |
| `crates/node/runner/src/app.rs` | `build_block()` and `verify_block()` -- block production and verification |
| `crates/node/runner/src/error.rs` | `RunnerError` -- existing error type in the runner crate; `BuildBlockError` should live here |
| `crates/node/ledger/src/lib.rs` | `persist_snapshot()` -- QMDB persistence with snapshot eviction |
| `crates/node/consensus/src/error.rs` | `ConsensusError` -- existing typed error infrastructure (NOT the right place for `BuildBlockError`) |
| `crates/node/runner/src/runner.rs` | `recover_finalized_state()` -- startup recovery (only restores HEAD) |

## Problem 1: Finalization failures are silent and unrecoverable

### Current behavior

The `finalize_block()` function in `crates/node/reporters/src/lib.rs` (lines 169-279) has five `Err(())` return sites, all returning the same untyped error:

```rust
// crates/node/reporters/src/lib.rs, lines 169-180
async fn finalize_block<E, P>(
    state: &LedgerService,
    context: &tokio::Context,
    executor: &E,
    provider: &P,
    block_index: Option<&Arc<BlockIndex>>,
    block: &Block,
) -> Result<(Option<ExecutionOutcome>, Option<BlockContext>), ()>
where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
```

### Enumeration of all 5 `Err(())` return paths

**Failure site 1 -- Execution failure (lines 204-207):**
The `BlockExecution::execute()` call fails. This can be caused by OOM, a poison in the transaction set, or an executor bug. When this happens during finalization, the block was already consensus-agreed but the local node cannot replay it. State divergence begins here.

```rust
Err(err) => {
    error!(?digest, error = ?err, "failed to execute finalized block");
    return Err(());
}
```

**Failure site 2 -- Root computation failure (lines 215-218):**
The QMDB `compute_root_from_store()` call fails. This means the ledger could not compute a Merkle root from the execution's changeset against the parent snapshot. Typically indicates a QMDB I/O error or a corrupted snapshot store state.

```rust
Err(err) => {
    error!(?digest, error = ?err, "failed to compute qmdb root");
    return Err(());
}
```

**Failure site 3 -- State root mismatch (lines 220-227):**
The computed state root does not match the block's declared `state_root`. This is a **deterministic, non-retryable** failure. It means the local node's state has diverged from the proposer's state (e.g., due to a previously missed finalized block, a different EVM execution result, or state corruption). Retrying will produce the same mismatch.

```rust
if state_root != block.state_root {
    warn!(
        ?digest,
        expected = ?block.state_root,
        computed = ?state_root,
        "state root mismatch for finalized block"
    );
    return Err(());
}
```

**Failure site 4 -- Missing parent snapshot (lines 254-256):**
The parent snapshot needed to re-execute the finalized block is not in the snapshot store. This only triggers when `snapshot_exists` is false (the block's own snapshot is also absent). There is an intermediate branch (`else if snapshot_exists`, lines 248-253) that warns and skips RPC indexing replay -- that path does NOT return an error. The error-returning path at lines 254-256 is the `else` branch where neither the block's snapshot nor the parent's snapshot is present.

This can happen for two distinct reasons with very different recovery paths:

- **Transient (catch-up race):** The finalization arrives before the node has verified the parent block. Retrying after a short delay may succeed if the parent is still being processed.
- **Permanent (eviction):** The parent snapshot was persisted and then evicted by `evict_persisted()` (see `crates/node/consensus/src/components/snapshot.rs:127-160`). Once evicted, the snapshot data is removed from memory. The `persisted` marker is retained so chain-walking terminates, but the `OverlayState` is gone. **Retrying is futile in this case.**

```rust
} else {
    error!(?digest, ?parent_digest, "missing parent snapshot for finalized block");
    return Err(());
}
```

**Failure site 5 -- Persist task failure (lines 268-275):**
Two sub-cases in the persistence path. First, the spawned persist task itself may panic or be cancelled (runtime error). Second, the `persist_snapshot()` call may return a `LedgerError` (QMDB I/O error, snapshot-not-found during the persist chain merge).

```rust
// Sub-case 5a: persist task runtime failure (lines 268-271)
Err(err) => {
    error!(?digest, error = ?err, "persist task failed");
    return Err(());
}

// Sub-case 5b: QMDB persistence error (lines 273-275)
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    return Err(());
}
```

### Why this matters

The caller (`handle_finalized_update`, starting at line 112) receives `Err(())` and continues execution -- pruning the mempool and acknowledging the update to the marshal. The `Update::Block` arm begins at line 127:

```rust
// crates/node/reporters/src/lib.rs, lines 127-159
Update::Block(block, ack) => {
    let result = finalize_block(
        &state,
        &context,
        &executor,
        &provider,
        block_index.as_ref(),
        &block,
    )
    .await;

    if let Ok((Some(outcome), Some(block_context))) = result.as_ref() {
        // ... indexing and GC only run on success
    }

    // These ALWAYS run, even on Err(())
    state.prune_mempool(&block.txs).await;
    publish_mempool_inclusions(mempool_broadcast.as_ref(), &block);
    ack.acknowledge();
}
```

The acknowledgment to the marshal is correct (the block IS finalized by consensus, so the marshal must advance). But the consequence is that the node's QMDB state is now missing a finalized block's state changes. Subsequent blocks that build on this state will either:
- Fail verification (state root mismatch) if re-executed
- Produce incorrect results if the missing state changes affect later transactions

There is no mechanism to detect this divergence, no retry, and no way to recover short of a full state resync.

### Which failures are transient vs. permanent

| # | Failure | Line | Transient? | Retryable? | Notes |
|---|---------|------|-----------|------------|-------|
| 1 | Execution failure | 204-207 | Possibly (OOM) | Yes, with backoff | |
| 2 | Root computation failure | 215-218 | Possibly (I/O) | Yes, with backoff | |
| 3 | State root mismatch | 220-227 | **No** (deterministic) | **No** | Indicates prior state divergence |
| 4 | Missing parent snapshot | 254-256 | **It depends** | **Conditionally** | See below |
| 5a | Persist task failure | 268-271 | Possibly (runtime) | Yes, with backoff | |
| 5b | Persist result error | 273-275 | Possibly (QMDB I/O) | Yes, with backoff | |

**Failure site 4 deserves special treatment.** The missing parent snapshot case has two fundamentally different root causes:

1. **Race with catch-up (transient):** The finalization arrived before the node verified the parent. A short retry may succeed. This is the simple case.

2. **Eviction (permanent):** The parent was persisted to QMDB and then evicted from the in-memory snapshot store (`evict_persisted()` in `crates/node/consensus/src/components/snapshot.rs:127-160` removes the `Snapshot` entry while retaining only the `persisted` marker in `InMemorySnapshotStore::persisted`). Once evicted, the `OverlayState` data is gone from memory. **Blind retry is futile** -- the snapshot will never reappear.

   Recovery from eviction requires a different path entirely: the node must identify the most recent snapshot that IS still available, then re-execute all blocks from that point forward to reconstruct the needed parent state. This is effectively a mini state-sync and is significantly more complex than a simple retry.

   To detect eviction, call `InMemorySnapshotStore::is_persisted(parent_digest)` directly. This is exposed from `LedgerService` indirectly via the snapshot store -- a new `LedgerService::is_snapshot_persisted(digest)` helper must be added, or the `finalize_block` function must be refactored to accept the snapshot store directly. The alternative approach of checking `query_state_root(parent_digest).await.is_some()` will NOT work: `query_state_root` reads from the in-memory snapshot map (via `snapshots.get()`), which is also cleared on eviction. After eviction, both `parent_snapshot(parent_digest)` and `query_state_root(parent_digest)` return `None`.

   The correct check is: if `is_persisted(parent_digest)` is true but `parent_snapshot(parent_digest)` returns `None`, the snapshot was evicted. If neither is true, it may still be in-flight.

## Problem 2: `build_block()` returns `None` for unrelated failure causes

### Current behavior

The `build_block()` method in `crates/node/runner/src/app.rs` (lines 91-192) returns `Option<Block>`, using `None` for three fundamentally different failure modes:

**Missing parent snapshot (lines 96-107):**
```rust
let parent_snapshot = match self.ledger.parent_snapshot(parent_digest).await {
    Some(snap) => snap,
    None => {
        warn!(
            parent_height = parent.height,
            ?parent_digest,
            "build_block: parent snapshot not found \
             -- node has not yet processed this parent block"
        );
        return None;
    }
};
```

**Execution failure (lines 143-155):**
```rust
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        warn!(
            parent = ?parent_digest,
            height,
            txs = txs.len(),
            error = ?err,
            "build_block: execution failed"
        );
        return None;
    }
};
```

**Root computation failure (lines 159-173):**
```rust
let state_root =
    match self.ledger.compute_root_from_store(parent_digest, outcome.changes.clone()).await
    {
        Ok(root) => root,
        Err(err) => {
            warn!(
                parent = ?parent_digest,
                height,
                error = %err,
                "build_block: compute root failed"
            );
            return None;
        }
    };
```

### Why this matters

The caller (`propose()`, defined at line 313) receives `None` and can only log a generic message. The `build_block` call and match block are at lines 340-366:

```rust
// crates/node/runner/src/app.rs, lines 340-366
let block = self.build_block(&parent, timestamp).await;
match block {
    Some(ref b) => { /* success logging */ }
    None => {
        warn!(
            parent_height = parent.height,
            parent_digest = ?parent.commitment(),
            build_ms = build_elapsed.as_millis(),
            "propose failed: build_block returned None \
             (likely missing parent snapshot -- node may still be catching up)"
        );
    }
}
```

The `propose()` log message guesses "likely missing parent snapshot" but has no way to confirm this or to take cause-specific action. A catch-up situation (expected, temporary) looks identical to state corruption (critical, permanent) from the perspective of any code outside `build_block()`.

This also makes metrics instrumentation impossible without typed errors. You cannot count `kora_proposal_failure_total{cause="catching_up"}` vs `{cause="execution_failed"}` when all failures look the same. This is the direct connection to Issue 19 (application-level metrics).

The same pattern exists in `verify_block()` (lines 194-274), which returns `bool` and maps four different failure modes (missing parent, execution failure, root computation failure, state root mismatch) to `false`.

## Proposed Fix

### Part 1: Typed errors for `build_block()`

Add a `BuildBlockError` enum to `crates/node/runner/src/error.rs`. This type belongs in the runner crate, NOT the consensus crate. The consensus crate (`kora_consensus`) defines domain-agnostic consensus primitives (snapshots, mempool traits, `ConsensusError`). Block building is an application-layer concern specific to the runner, and `BuildBlockError` references runner-specific types like `kora_executor::ExecutionError` and `LedgerError`. Placing it in the consensus crate would create a backwards dependency and violate the layering:

```
consensus (domain-agnostic) --> runner (application-specific)
                                  ^
                                  |
                            BuildBlockError belongs here
```

```rust
// crates/node/runner/src/error.rs (add to existing file)

/// Errors that can occur when building a block proposal.
#[derive(Debug, Error)]
pub enum BuildBlockError {
    /// The parent snapshot is not available in the in-memory store.
    ///
    /// This is expected during catch-up when the node has not yet processed
    /// the parent block. It is transient and should resolve as the node
    /// catches up to the network.
    #[error("parent snapshot not found (catching up): parent={parent_digest:?}")]
    CatchingUp {
        parent_digest: ConsensusDigest,
        parent_height: u64,
    },

    /// Block execution against the parent state failed.
    ///
    /// This could be transient (OOM) or indicate a problem with the
    /// transaction set (poisoned mempool).
    #[error("block execution failed: {source}")]
    ExecutionFailed {
        #[source]
        source: kora_executor::ExecutionError,
        parent_digest: ConsensusDigest,
        height: u64,
        tx_count: usize,
    },

    /// Computing the QMDB state root from the execution changes failed.
    ///
    /// This typically indicates a ledger or storage-layer issue.
    #[error("state root computation failed: {source}")]
    RootComputationFailed {
        #[source]
        source: LedgerError,
        parent_digest: ConsensusDigest,
        height: u64,
    },
}
```

Then change `build_block()` in `crates/node/runner/src/app.rs` from `Option<Block>` to `Result<Block, BuildBlockError>`:

```rust
async fn build_block(&self, parent: &Block, timestamp: u64) -> Result<Block, BuildBlockError> {
    let parent_digest = parent.commitment();
    let parent_snapshot = self.ledger.parent_snapshot(parent_digest).await
        .ok_or(BuildBlockError::CatchingUp {
            parent_digest,
            parent_height: parent.height,
        })?;

    // ... mempool drain (unchanged) ...

    let outcome = self.executor.execute(&parent_snapshot.state, &context, &txs_bytes)
        .map_err(|e| BuildBlockError::ExecutionFailed {
            source: e,
            parent_digest,
            height,
            tx_count: txs.len(),
        })?;

    let state_root = self.ledger
        .compute_root_from_store(parent_digest, outcome.changes.clone())
        .await
        .map_err(|e| BuildBlockError::RootComputationFailed {
            source: e,
            parent_digest,
            height,
        })?;

    // ... construct and return block ...
    Ok(block)
}
```

Update the `propose()` caller to match on the error:

```rust
async move {
    match self.build_block(&parent, timestamp).await {
        Ok(block) => {
            if let Some(ref state) = node_state {
                state.inc_proposed();
            }
            Some(block)
        }
        Err(BuildBlockError::CatchingUp { parent_height, .. }) => {
            debug!(parent_height, "skipping proposal: still catching up");
            None
        }
        Err(ref e) => {
            warn!(error = %e, "proposal failed");
            None
        }
    }
}
```

### Part 2: Typed errors and retry for `finalize_block()`

Add a `FinalizationError` enum:

```rust
// crates/node/reporters/src/lib.rs (or a shared error module)

/// Errors that can occur during block finalization.
#[derive(Debug, Error)]
enum FinalizationError {
    /// Block execution failed during finalization replay.
    #[error("execution failed: {0}")]
    ExecutionFailed(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// QMDB root computation failed.
    #[error("root computation failed: {0}")]
    RootComputationFailed(#[source] LedgerError),

    /// Computed state root does not match the block's declared root.
    /// This is a deterministic mismatch and is NOT retryable.
    #[error("state root mismatch: expected {expected:?}, computed {computed:?}")]
    StateRootMismatch {
        expected: StateRoot,
        computed: StateRoot,
    },

    /// The parent snapshot needed for re-execution was not found and
    /// may still be in-flight (catch-up race). Retryable with a short delay.
    #[error("missing parent snapshot (transient): digest={digest:?} parent={parent_digest:?}")]
    MissingParentSnapshot {
        digest: ConsensusDigest,
        parent_digest: ConsensusDigest,
    },

    /// The parent snapshot was persisted and then evicted from memory.
    /// The snapshot data is gone; retrying will not help. The node must
    /// re-execute from an earlier available snapshot to reconstruct the
    /// needed parent state.
    #[error("parent snapshot evicted: digest={digest:?} parent={parent_digest:?}")]
    MissingParentSnapshotEvicted {
        digest: ConsensusDigest,
        parent_digest: ConsensusDigest,
    },

    /// The persistence task itself failed (runtime error).
    #[error("persist task failed: {0}")]
    PersistTaskFailed(String),

    /// QMDB persistence returned an error.
    #[error("persist failed: {0}")]
    PersistFailed(#[source] LedgerError),
}

impl FinalizationError {
    /// Returns true if this error is potentially transient and the operation
    /// should be retried.
    const fn is_retryable(&self) -> bool {
        match self {
            Self::StateRootMismatch { .. } => false,
            Self::MissingParentSnapshotEvicted { .. } => false,
            Self::ExecutionFailed(_)
            | Self::RootComputationFailed(_)
            | Self::MissingParentSnapshot { .. }
            | Self::PersistTaskFailed(_)
            | Self::PersistFailed(_) => true,
        }
    }

    /// Returns a static label suitable for use as a Prometheus metric label.
    /// See Issue 19 (application-level metrics).
    const fn metric_label(&self) -> &'static str {
        match self {
            Self::ExecutionFailed(_) => "execution_failed",
            Self::RootComputationFailed(_) => "root_computation_failed",
            Self::StateRootMismatch { .. } => "state_root_mismatch",
            Self::MissingParentSnapshot { .. } => "missing_parent_snapshot",
            Self::MissingParentSnapshotEvicted { .. } => "parent_snapshot_evicted",
            Self::PersistTaskFailed(_) => "persist_task_failed",
            Self::PersistFailed(_) => "persist_failed",
        }
    }
}
```

### Part 3: Retry wrapper with eviction awareness

The retry logic must distinguish between a transiently missing snapshot (might appear shortly) and a permanently evicted one (will never appear).

**Important:** `LedgerService::query_state_root(parent_digest)` reads from the in-memory snapshot map and returns `None` for evicted snapshots (the snapshot data is removed, so `snapshots.get()` returns nothing). It cannot be used to detect eviction. To detect eviction correctly, you need `InMemorySnapshotStore::is_persisted()`. Expose this from `LedgerService` via a new method:

```rust
// crates/node/ledger/src/lib.rs -- add to LedgerService
/// Returns true if the snapshot for `digest` has been persisted to QMDB
/// (even if the in-memory snapshot data has since been evicted).
pub async fn is_snapshot_persisted(&self, digest: ConsensusDigest) -> bool {
    let inner = self.view.inner.lock().await;
    inner.snapshots.is_persisted(&digest)
}
```

Then in `finalize_block`, replace the bare `error!` + `return Err(())` at failure site 4 with eviction-aware classification:

```rust
// Replaces lines 254-256 in reporters/src/lib.rs
} else {
    // Distinguish: was the parent snapshot persisted-then-evicted, or never present?
    if state.is_snapshot_persisted(parent_digest).await {
        // Persisted then evicted -- snapshot data is gone, blind retry is futile
        return Err(FinalizationError::MissingParentSnapshotEvicted { digest, parent_digest });
    } else {
        // Never seen -- may still be arriving (catch-up race)
        return Err(FinalizationError::MissingParentSnapshot { digest, parent_digest });
    }
}
```

Retry wrapper:

```rust
/// Maximum number of retry attempts for transient finalization failures.
const MAX_FINALIZATION_RETRIES: u32 = 3;

/// Base delay between retry attempts (doubles each attempt).
const FINALIZATION_RETRY_BASE_MS: u64 = 50;

async fn finalize_with_retry<E, P>(
    state: &LedgerService,
    context: &tokio::Context,
    executor: &E,
    provider: &P,
    block_index: Option<&Arc<BlockIndex>>,
    block: &Block,
) -> Result<(Option<ExecutionOutcome>, Option<BlockContext>), FinalizationError>
where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    let digest = block.commitment();
    let mut last_err = None;

    for attempt in 0..MAX_FINALIZATION_RETRIES {
        match finalize_block(state, context, executor, provider, block_index, block).await {
            Ok(result) => {
                if attempt > 0 {
                    info!(?digest, attempt, "finalization succeeded after retry");
                }
                return Ok(result);
            }
            Err(e) if e.is_retryable() && attempt < MAX_FINALIZATION_RETRIES - 1 => {
                let delay_ms = FINALIZATION_RETRY_BASE_MS * 2u64.pow(attempt);
                warn!(
                    ?digest,
                    attempt,
                    delay_ms,
                    error = %e,
                    label = e.metric_label(),
                    "finalization failed with transient error, retrying"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                last_err = Some(e);
            }
            Err(e) => {
                error!(
                    ?digest,
                    attempt,
                    error = %e,
                    retryable = e.is_retryable(),
                    label = e.metric_label(),
                    "finalization failed permanently"
                );
                return Err(e);
            }
        }
    }

    Err(last_err.expect("at least one attempt was made"))
}
```

### Part 4: `finalize_block()` signature change

Change the existing `finalize_block()` return type from `Result<..., ()>` to `Result<..., FinalizationError>`:

```rust
async fn finalize_block<E, P>(
    // ... same parameters ...
) -> Result<(Option<ExecutionOutcome>, Option<BlockContext>), FinalizationError>
```

Each error site gets a specific variant instead of `Err(())`:

```rust
// Lines 204-207: Execution failure (site 1)
Err(err) => {
    return Err(FinalizationError::ExecutionFailed(Box::new(err)));
}

// Lines 215-218: Root computation failure (site 2)
Err(err) => {
    return Err(FinalizationError::RootComputationFailed(err));
}

// Lines 220-227: State root mismatch (site 3)
if state_root != block.state_root {
    return Err(FinalizationError::StateRootMismatch {
        expected: block.state_root,
        computed: state_root,
    });
}

// Lines 254-256: Missing parent snapshot (site 4)
// Distinguish transient vs. evicted (see Part 3 for the is_snapshot_persisted helper):
} else {
    if state.is_snapshot_persisted(parent_digest).await {
        return Err(FinalizationError::MissingParentSnapshotEvicted { digest, parent_digest });
    } else {
        return Err(FinalizationError::MissingParentSnapshot { digest, parent_digest });
    }
}

// Lines 268-271: Persist task failure (site 5a)
Err(err) => {
    return Err(FinalizationError::PersistTaskFailed(format!("{err}")));
}

// Lines 273-275: Persist result error (site 5b)
if let Err(err) = persist_result {
    return Err(FinalizationError::PersistFailed(err));
}
```

Then update `handle_finalized_update` to call `finalize_with_retry` instead of `finalize_block` directly.

## Impact Analysis

### Risk: Low-Medium

The changes are confined to error types and control flow. No state machine logic, consensus protocol, or QMDB persistence semantics change. The retry wrapper adds at most ~350ms of delay (50ms + 100ms + 200ms) for the three retry attempts, which is negligible compared to block intervals.

### Breaking changes

- `build_block()` return type changes from `Option<Block>` to `Result<Block, BuildBlockError>`. The only caller is `propose()` in the same file (`crates/node/runner/src/app.rs`).
- `finalize_block()` return type changes from `Result<..., ()>` to `Result<..., FinalizationError>`. The only caller is `handle_finalized_update()` in the same file (`crates/node/reporters/src/lib.rs`).
- Existing tests in `reporters/src/lib.rs` (`finalize_error_tests` at line 331, `finalize_success_tests` at line 451) will need minor updates to match on `FinalizationError` instead of `()`.

### Why NOT the consensus crate

The existing `ConsensusError` in `crates/node/consensus/src/error.rs` is domain-agnostic: it deals with digests, state roots, and generic validation strings. `BuildBlockError` references application-specific types (`kora_executor::ExecutionError`, `LedgerError`) and encodes block-building semantics. Placing it in `kora_consensus` would:

1. Create a dependency from the consensus crate on `kora_executor` and `kora_ledger`, which it currently does not depend on.
2. Mix domain-agnostic consensus errors with application-specific block production errors.
3. Make the consensus crate harder to reuse in different application contexts.

The runner crate (`crates/node/runner/src/error.rs`) already has `RunnerError` and is the natural home for `BuildBlockError`.

### What this does NOT fix

- The snapshot store TOCTOU race itself. This issue adds the eviction-aware check so that the consequence is properly classified rather than blindly retried.
- The startup recovery problem where only HEAD is restored (one snapshot). That requires snapshot cache pre-population logic.
- The `verify_block()` returning `false` during catch-up (requires Commonware framework changes to support a three-valued verification result).
- Re-execution recovery path for evicted snapshots. This issue classifies the `MissingParentSnapshotEvicted` case as non-retryable and logs it clearly. Actually implementing the walk-back-and-re-execute recovery is a separate, more complex piece of work.

## Testing

- Modify the existing `prune_and_ack_still_run_when_finalization_fails` test (line 393) to verify the error variant is `FinalizationError::ExecutionFailed`.
- Add a test that verifies `FinalizationError::StateRootMismatch` is not retried (non-retryable).
- Add a test that verifies `FinalizationError::MissingParentSnapshotEvicted` is not retried.
- Add a test with a mock executor that fails once then succeeds, verifying the retry wrapper recovers.
- Add a test that `BuildBlockError::CatchingUp` is logged at `debug` level (not `warn`), since it is expected during normal catch-up.
- Add a test that `FinalizationError::metric_label()` returns distinct labels for each variant (ensures Prometheus label coverage).

## Estimated Effort

| Task | Estimate |
|------|----------|
| Define `BuildBlockError` enum in `runner/src/error.rs` and update `build_block()` + `propose()` | 1-2 hours |
| Define `FinalizationError` enum in `reporters/src/lib.rs` and update all 5 error sites | 1-2 hours |
| Add `LedgerService::is_snapshot_persisted()` helper and eviction detection logic to failure site 4 | 1 hour |
| Implement `finalize_with_retry()` wrapper | 1 hour |
| Update existing tests | 1 hour |
| Add new retry, eviction, and error-variant tests | 1-2 hours |
| **Total** | **6-9 hours** |
