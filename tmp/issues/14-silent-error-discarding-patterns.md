# Error Handling: Silent error discarding via `.ok()?` patterns hides critical failures

**Severity:** Medium-High
**Component:** Multiple crates (`runner`, `reporters`, `ledger`, `dkg`)
**Labels:** `bug`, `error-handling`, `reliability`, `state-integrity`

## Summary

Multiple production code paths use `.ok()?`, `.ok()`, or `let _ =` to silently discard errors, hiding critical failures from operators. The most dangerous instances are:

1. **A silent state root computation failure** in the block proposal path (`app.rs`) that causes the proposer to silently skip block production with no diagnostics.
2. **A finalization handler that acknowledges blocks to consensus even when persistence fails** (`reporters/src/lib.rs`), allowing the chain to advance past state that was never durably written.

In a blockchain node, silent error discarding is uniquely dangerous because it can lead to state divergence between validators, undetected data loss, and chain splits. Every error in the block production and finalization path must be surfaced to operators.

## Background

### Why Silent Failures Are Dangerous in Blockchain Nodes

Blockchain nodes have a core invariant: **all validators must agree on the same state transitions.** When errors are silently discarded, several failure modes emerge:

- **Missed blocks**: If a proposer silently fails to build a block, the consensus round is wasted. With repeated failures, the chain stalls.
- **State divergence**: If a validator acknowledges a finalized block but fails to persist its state, it will restart with stale state. On restart, it may not be able to reconstruct the finalized state, leading to a fork.
- **Phantom data**: If balance queries silently return `None` on database errors (instead of propagating the error), RPC clients receive incorrect responses and may make financial decisions based on stale data.
- **Silent chain stalls**: If the block proposer silently returns `None` from `propose()`, consensus moves to the next round with a nullification. Repeated silent failures look identical to "no transactions available" from the outside.

## Pattern 1 -- P1: Silent State Root Failure in Block Proposals

**File:** `crates/node/runner/src/app.rs`, line 145
**Risk:** Critical -- proposer silently drops blocks

### The Problem

When the block proposer builds a new block, it must compute a state root (a Merkle commitment over the post-execution state). This computation can fail if the parent snapshot is missing, QMDB encounters a storage error, or the system is out of memory. The current code discards the error:

```rust
// crates/node/runner/src/app.rs, lines 140-145
let root_start = Instant::now();
let state_root = self
    .ledger
    .compute_root_from_store(parent_digest, outcome.changes.clone())
    .await
    .ok()?;  // <-- Discards the actual error silently
```

The `.ok()?` converts the `Result<StateRoot, LedgerError>` to `Option<StateRoot>`, throwing away the error variant. The `?` then propagates `None` up to the `build_block` method, which returns `None` from `propose()`. Consensus treats this as "proposer has nothing to propose" and moves on.

**What can go wrong with `compute_root_from_store`?** Looking at the implementation in `crates/node/ledger/src/lib.rs` line 277:

```rust
pub async fn compute_root_from_store(
    &self,
    parent: ConsensusDigest,
    changes: QmdbChangeSet,
) -> LedgerResult<StateRoot> {
    let parent_root = {
        let inner = self.inner.lock().await;
        inner.snapshots.get(&parent)
            .ok_or(ConsensusError::SnapshotNotFound(parent))?
            .state_root
    };
    Ok(StateRoot(QmdbStateRoot::transition(parent_root.0, &changes)))
}
```

The primary failure mode is `SnapshotNotFound` -- the parent snapshot was pruned or never inserted. This is a serious state management bug that should be investigated, not silently swallowed.

Note that the `verify_block` path (lines 197-207) handles this correctly with an explicit `match`:

```rust
// crates/node/runner/src/app.rs, lines 197-207 (the CORRECT pattern)
let state_root = match self
    .ledger
    .compute_root_from_store(parent_digest, execution.outcome.changes.clone())
    .await
{
    Ok(root) => root,
    Err(err) => {
        warn!(?digest, error = ?err, "compute root failed");
        return false;
    }
};
```

### The Fix

Replace `.ok()?` with explicit error handling, matching the pattern already used in `verify_block`:

```rust
let state_root = match self
    .ledger
    .compute_root_from_store(parent_digest, outcome.changes.clone())
    .await
{
    Ok(root) => root,
    Err(err) => {
        warn!(
            parent = ?parent_digest,
            height,
            error = ?err,
            "build_block: state root computation failed"
        );
        return None;
    }
};
```

## Pattern 2 -- P1: Finalization Persists Ack Without Persisting State

**File:** `crates/node/reporters/src/lib.rs`, lines 208-220
**Risk:** Critical -- consensus advances past unpersisted state

### The Problem

When a block is finalized, `FinalizedReporter` (via `handle_finalized_update`) is responsible for persisting the block's state to QMDB and then acknowledging the block to the marshal. The marshal uses this acknowledgement to advance the delivery floor -- meaning it will not redeliver this block.

The problem is that when persistence fails, the code still calls `ack.acknowledge()`:

```rust
// crates/node/reporters/src/lib.rs, lines 208-220
let persist_result = match persist_handle.await {
    Ok(result) => result,
    Err(err) => {
        error!(?digest, error = ?err, "persist task failed");
        ack.acknowledge();   // <-- BUG: acking despite task failure
        return;
    }
};
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    ack.acknowledge();       // <-- BUG: acking despite persist failure
    return;
}
```

This also occurs earlier in the same function when execution fails (line 144) or the state root mismatches (line 167) -- in all error paths, the block is acknowledged.

**Why this is dangerous:** The marshal tracks which blocks have been acknowledged and advances its internal floor. Once acknowledged, a block will not be redelivered. If the node restarts after a persist failure, the finalized block's state is lost. The node will start from the last successfully persisted state, but consensus has already moved past this point. The node may be unable to catch up because it cannot reconstruct the missing state.

### The Fix

Do NOT acknowledge blocks that fail to persist. Let the marshal redeliver them on the next attempt:

```rust
let persist_result = match persist_handle.await {
    Ok(result) => result,
    Err(err) => {
        error!(?digest, error = ?err, "persist task failed; NOT acknowledging block");
        // Do NOT call ack.acknowledge() -- marshal will redeliver
        return;
    }
};
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block; NOT acknowledging block");
    // Do NOT call ack.acknowledge() -- marshal will redeliver
    return;
}
```

**Note on execution/state-root error paths:** The ack-on-error at lines 144 and 167 (execution failure, state root mismatch) is more nuanced. These indicate a consensus-level disagreement, and re-executing the block will likely produce the same error. These paths may warrant a different strategy (e.g., halting the node) but should at minimum be reviewed.

## Pattern 3 -- Medium: DKG Transport `.ok()` on recv

**File:** `crates/node/dkg/src/transport.rs`, line 250

```rust
pub async fn recv(&mut self) -> Option<(ed25519::PublicKey, Bytes)> {
    self.receiver.recv().await.ok().map(|(sender, message)| (sender, Bytes::from(message)))
}
```

The `recv()` method on the p2p receiver returns a `Result`. By calling `.ok()`, channel errors (receiver closed, network layer crashed) are converted to `None`, which is indistinguishable from "no messages available." The DKG ceremony will spin in its polling loop thinking it's simply waiting for messages, when in reality the transport is permanently broken.

**Fix:** Return `Result<Option<...>, DkgError>` to let callers distinguish "no message" from "transport failed":

```rust
pub async fn recv(&mut self) -> Result<Option<(ed25519::PublicKey, Bytes)>, DkgError> {
    match self.receiver.recv().await {
        Ok((sender, message)) => Ok(Some((sender, Bytes::from(message)))),
        Err(e) => {
            warn!(?e, "DKG transport recv failed");
            Err(DkgError::Network(format!("Transport recv failed: {}", e)))
        }
    }
}
```

## Pattern 4 -- Medium: Ledger Balance Query `.ok()`

**File:** `crates/node/ledger/src/lib.rs`, line 184

```rust
pub async fn query_balance(&self, digest: ConsensusDigest, address: Address) -> Option<U256> {
    let snapshot = {
        let inner = self.inner.lock().await;
        inner.snapshots.get(&digest)
    }?;
    snapshot.state.balance(&address).await.ok()   // <-- discards state read errors
}
```

If `balance()` returns an error (e.g., QMDB read failure, corrupted account trie), the RPC layer receives `None` and returns a zero balance or "account not found" to the caller. This is incorrect -- a database error is not the same as "account does not exist." A user or dapp querying a balance could receive stale or zero data during a storage issue.

**Fix:**

```rust
pub async fn query_balance(
    &self,
    digest: ConsensusDigest,
    address: Address,
) -> Result<Option<U256>, LedgerError> {
    let snapshot = {
        let inner = self.inner.lock().await;
        inner.snapshots.get(&digest)
    };
    match snapshot {
        Some(s) => s.state.balance(&address).await.map(Some).map_err(Into::into),
        None => Ok(None),
    }
}
```

This is a breaking API change and requires updating callers, so it may be better handled as a separate PR. At minimum, add a `warn!` log:

```rust
snapshot.state.balance(&address).await.map_err(|e| {
    warn!(?address, ?digest, error = ?e, "balance query failed");
    e
}).ok()
```

## Pattern 5 -- Low: DKG `set_read_timeout` `.ok()`

**File:** `crates/node/dkg/src/network.rs`, line 124

```rust
stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
```

If `set_read_timeout` fails, the subsequent `read_exact` calls on this stream will block indefinitely (no timeout). Since `poll_incoming` is called in the main DKG loop, this could stall the entire ceremony on a single connection.

**Fix:**

```rust
if let Err(e) = stream.set_read_timeout(Some(Duration::from_secs(5))) {
    warn!(%addr, ?e, "Failed to set read timeout, skipping connection");
    continue;
}
```

## Complete Inventory of Silent Error Patterns in Production Code

| # | File | Line | Pattern | Context | Risk |
|---|------|------|---------|---------|------|
| 1 | `crates/node/runner/src/app.rs` | 145 | `.ok()?` | State root computation during block proposal | **P1** -- silently drops blocks |
| 2 | `crates/node/reporters/src/lib.rs` | 212 | `ack.acknowledge()` after persist task error | Finalized block acknowledged despite failed persistence | **P1** -- state loss on restart |
| 3 | `crates/node/reporters/src/lib.rs` | 218 | `ack.acknowledge()` after persist error | Finalized block acknowledged despite failed persistence | **P1** -- state loss on restart |
| 4 | `crates/node/reporters/src/lib.rs` | 144 | `ack.acknowledge()` after execution error | Finalized block acknowledged despite execution failure | **P1** -- potential state divergence |
| 5 | `crates/node/reporters/src/lib.rs` | 167 | `ack.acknowledge()` after state root mismatch | Finalized block acknowledged despite state root mismatch | **P1** -- potential state divergence |
| 6 | `crates/node/dkg/src/transport.rs` | 250 | `.ok().map(...)` | Transport recv discards channel errors | Medium -- DKG hangs |
| 7 | `crates/node/ledger/src/lib.rs` | 184 | `.ok()` | Balance query discards state read errors | Medium -- stale RPC data |
| 8 | `crates/node/dkg/src/network.rs` | 124 | `.ok()` | `set_read_timeout` failure ignored | Medium -- indefinite blocking |
| 9 | `crates/node/dkg/src/ceremony.rs` | 346 | `let _ =` | Send to leader in Phase 4 | Medium -- log request lost |
| 10 | `crates/node/dkg/src/ceremony.rs` | 389 | `debug!` only | Send failure in `send_outgoing` | Medium -- silent send failures |
| 11 | `crates/node/dkg/src/ceremony.rs` | 394 | `debug!` only | Broadcast failure in `send_outgoing` | Medium -- silent broadcast failures |
| 12 | `crates/node/dkg/src/ceremony.rs` | 79 | `.unwrap()` | `SystemTime::now().duration_since(UNIX_EPOCH)` | Low -- panic on clock misconfiguration |
| 13 | `crates/node/dkg/src/protocol.rs` | 76 | `.unwrap()` | `bytes[32..40].try_into().unwrap()` on fixed-size slice | Low -- safe but fragile |
| 14 | `crates/node/dkg/src/protocol.rs` | 77 | `.unwrap()` | `bytes[40..44].try_into().unwrap()` on fixed-size slice | Low -- safe but fragile |
| 15 | `crates/node/dkg/src/state.rs` | 158 | `.ok()` | `hex::decode` on our signed log deserialization | Low -- corrupted state file |
| 16 | `crates/node/dkg/src/state.rs` | 170 | `.ok()` | `hex::decode` on received log deserialization | Low -- corrupted state file |
| 17 | `crates/node/dkg/src/protocol.rs` | 1029 | `.unwrap()` | `NonZeroU32::new(max_degree).unwrap()` in `try_restore()` | Low -- panics if threshold is 0 |
| 18 | `crates/node/dkg/src/protocol.rs` | 1043 | `.unwrap()` | `NonZeroU32::new(max_degree).unwrap()` in `try_restore()` | Low -- panics if threshold is 0 |
| 19 | `crates/storage/handlers/src/adapter.rs` | 243 | `let _ =` | `block_on(Self::commit(...))` in `DatabaseCommit` trait impl | Low -- matches REVM's interface (documented) |
| 20 | `crates/node/runner/src/runner.rs` | 545 | `let _ =` | `ledger.submit_tx(tx)` return value ignored | Low -- tx submission is best-effort |

**Note:** Patterns in `crates/e2e/` and test modules (e.g., `crates/node/dkg/src/tests.rs`) are excluded from this table as they are not production code. The `crates/node/txpool/src/pool.rs` lines 278-279 use `.ok()?` in `extract_sender` which is intentional -- malformed transactions should be silently skipped during mempool operations.

## Proposed Fix Approach

### Phase 1: Critical Fixes (P1 items)

1. **`app.rs` line 145**: Replace `.ok()?` with explicit `match` + `warn!` logging (see Pattern 1 fix above). This is a one-line change with no API impact.

2. **`reporters/src/lib.rs` persist error paths**: Remove `ack.acknowledge()` from the persist-failure branches (lines 212 and 218). The marshal will redeliver the block on the next iteration. This requires verifying that the marshal handles non-acknowledged blocks correctly (it should, by design -- the acknowledgement mechanism exists precisely for this purpose).

### Phase 2: Medium-Priority Fixes

3. **DKG transport recv**: Change return type to `Result<Option<...>>` and propagate errors. Update ceremony.rs callers.

4. **DKG network `set_read_timeout`**: Add `warn!` + `continue` on failure.

5. **DKG ceremony `send_outgoing`**: Elevate `debug!` to `warn!` for all send failures.

6. **Ledger `query_balance`**: At minimum, add `warn!` logging before `.ok()`. Ideally, change return type to `Result<Option<U256>>` and propagate to RPC layer.

### Phase 3: Defensive Improvements

7. **DKG `SystemTime` unwrap**: Replace with `map_err` returning `DkgError::CeremonyFailed`.

8. **Protocol byte slice unwraps**: Replace with `map_err` for defense-in-depth.

9. **DKG state hex decode `.ok()`**: Add `warn!` logging for corrupted state files.

10. **Protocol `NonZeroU32::new().unwrap()` in `try_restore()`** (lines 1029, 1043): Replace with `.ok_or_else(|| DkgError::CeremonyFailed(...))` to avoid panics during crash recovery if the config has an invalid threshold.

## Files to Modify

| File | Changes |
|------|---------|
| `crates/node/runner/src/app.rs` | Line 145: Replace `.ok()?` with explicit `match` + `warn!` logging |
| `crates/node/reporters/src/lib.rs` | Lines 212, 218: Remove `ack.acknowledge()` from persist-failure error paths. Lines 144, 167: Review and document ack-on-error strategy for execution/state-root failures |
| `crates/node/dkg/src/transport.rs` | Line 250: Change `recv()` return type to `Result<Option<...>, DkgError>` |
| `crates/node/dkg/src/network.rs` | Line 124: Replace `.ok()` with `if let Err` + `warn!` + `continue` |
| `crates/node/dkg/src/ceremony.rs` | Lines 389, 394: Elevate `debug!` to `warn!` for send failures |
| `crates/node/ledger/src/lib.rs` | Line 184: Add `warn!` logging before `.ok()` (minimum); ideally change return type to `Result<Option<U256>, LedgerError>` |
| `crates/node/dkg/src/ceremony.rs` | Line 79: Replace `.unwrap()` with `map_err` returning `DkgError::CeremonyFailed` |
| `crates/node/dkg/src/protocol.rs` | Lines 76-77: Replace `.unwrap()` with `map_err` returning `commonware_codec::Error::EndOfBuffer`. Lines 1029, 1043: Replace `.unwrap()` with `.ok_or_else` returning `DkgError::CeremonyFailed` |
| `crates/node/dkg/src/state.rs` | Lines 158, 170: Add `warn!` logging for hex decode failures |

## Testing Plan

### Unit Tests

1. **`app.rs` state root failure logging**: Create a test where `compute_root_from_store` returns `Err`, verify that `build_block` returns `None` AND that a `warn!`-level log is emitted (using `tracing-test` or similar).

2. **`reporters/src/lib.rs` persist failure non-ack**: Mock `persist_snapshot` to return `Err`, verify that `ack.acknowledge()` is NOT called. Verify that the marshal redelivers the block.

3. **DKG transport error propagation**: Mock a closed receiver channel, verify that `recv()` returns `Err(DkgError::Network(...))` instead of `None`.

### Integration Tests

4. **Proposer resilience**: In an e2e test, inject a transient `compute_root_from_store` failure. Verify the proposer logs a warning, skips one round, and succeeds on the next round when the error clears.

5. **Finalization persist failure**: In an e2e test, make `persist_snapshot` fail for one block. Verify the block is redelivered and successfully persisted on retry. Verify the chain continues without a gap.

6. **DKG network partition**: In a devnet, partition one node during the DKG ceremony. Verify warn-level logs appear for failed sends. Verify the ceremony times out with an actionable error message.

### Manual Verification

7. **Log audit**: After applying fixes, run a devnet and intentionally trigger each error path. Verify that every error produces at least a `warn!`-level log entry with the error details, the operation that failed, and the block/digest context.

## Verification Checklist

After applying all fixes, verify the following:

### Build and Test
- [ ] `cargo build --release` compiles without errors.
- [ ] `cargo test -p kora-runner` passes all existing tests.
- [ ] `cargo test -p kora-reporters` passes all existing tests.
- [ ] `cargo test -p kora-ledger` passes all existing tests (note: if `query_balance` return type changed, callers in tests must be updated).
- [ ] `cargo test -p kora-dkg` passes all existing tests.
- [ ] `cargo clippy --workspace` produces no new warnings.

### Pattern Elimination Verification
- [ ] Run `grep -n '\.ok()?' crates/node/runner/src/app.rs` -- should return zero results (line 145 replaced with match).
- [ ] Run `grep -n 'ack.acknowledge' crates/node/reporters/src/lib.rs` -- lines 212 and 218 should no longer call `ack.acknowledge()` in error paths. Line 229 (success path) should still call it.
- [ ] Run `grep -n '\.ok()' crates/node/ledger/src/lib.rs` -- line 184 should at minimum have a `warn!` before `.ok()`, or the return type should be changed to `Result`.
- [ ] Run `grep -n 'debug!.*Failed' crates/node/dkg/src/ceremony.rs` -- should return zero results (elevated to `warn!`).

### Integration Verification
- [ ] Run a 4-node Docker devnet: `just docker devnet`. Verify chain starts and produces blocks.
- [ ] The marshal redelivery mechanism works correctly when `ack.acknowledge()` is not called: verify by reviewing the commonware marshal source that non-acked blocks are redelivered on the next cycle.
