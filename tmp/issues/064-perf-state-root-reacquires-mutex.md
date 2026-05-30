# State Root Computation Re-Acquires LedgerView Mutex Unnecessarily

**Category**: performance -- ledger
**Severity**: medium

**Labels**: `performance`, `storage`, `consensus`

---

## Summary

The `compute_root_from_store()` method acquires the `LedgerView` inner mutex to look up the parent snapshot's state root, even though both callers (`build_block` and `verify_block`) already have the parent snapshot in hand from an earlier step. This creates a redundant mutex acquisition on the central bottleneck mutex that serializes most ledger operations, adding unnecessary contention at 34 blocks/s.

---

## Problem

Kora uses a `LedgerService` backed by a `futures::lock::Mutex` to coordinate access to the snapshot store, mempool, and QMDB state. The `compute_root_from_store()` method acquires this mutex to look up the parent snapshot's state root, then immediately releases it to perform the (CPU-bound) root computation.

However, both call sites -- `build_block` and `verify_block` in `app.rs` -- have already fetched the parent snapshot (and thus have access to its `state_root`) before calling `compute_root_from_store`. The method re-acquires the same mutex to look up the same data the caller already holds.

---

## Code Reference

**The redundant mutex acquisition** -- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:448-458`:
```rust
    pub async fn compute_root_from_store(
        &self,
        parent: ConsensusDigest,
        changes: &QmdbChangeSet,
    ) -> LedgerResult<StateRoot> {
        let parent_root = {
            let inner = self.inner.lock().await;  // <-- Acquires mutex
            inner.snapshots.get(&parent).ok_or(ConsensusError::SnapshotNotFound(parent))?.state_root
        };
        Ok(StateRoot(QmdbStateRoot::transition(parent_root.0, changes)))
    }
```

**Caller already has parent snapshot in build_block** -- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:271-303` and `:387-402`:
```rust
        // Line 271-303: parent_snapshot is fetched here
        let parent_snapshot = {
            let wait_start = Instant::now();
            match self.ledger.wait_for_snapshot(parent_digest, SNAPSHOT_WAIT_TIMEOUT).await {
                Some(s) => { /* ... */ s }
                None => { /* ... */ return None; }
            }
        };

        // ... execution happens ...

        // Line 387-402: compute_root_from_store re-acquires the mutex to get parent_root
        let state_root =
            match self.ledger.compute_root_from_store(parent_digest, &outcome.changes).await {
                Ok(root) => root,
                Err(err) => { /* ... */ return None; }
            };
```

**Same pattern in verify_block** -- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:536` and `:620-624`:
```rust
        // Line 536: parent_snapshot fetched
        let parent_snapshot = match self.ledger.parent_snapshot(parent_digest).await {
            Some(snap) => snap,
            None => { /* ... */ }
        };

        // Line 620-624: compute_root_from_store re-acquires mutex
        let state_root = match self
            .ledger
            .compute_root_from_store(parent_digest, &execution.outcome.changes)
            .await
```

---

## Impact

Every block execution (both propose and verify) causes one redundant mutex acquisition on the central `LedgerView` mutex. At 34 blocks/s with 10 validators, this produces approximately 340 extra mutex lock/unlock cycles per second on the most contended lock in the system.

While each individual acquisition is fast (microseconds), the contention is cumulative. Every redundant lock cycle adds wait time for other concurrent operations that also need this mutex:
- Snapshot insertion (from other validators' verify_block calls)
- Mempool access (from proposal building)
- Persistence pipeline (from finalization)
- State root queries (from RPC)

This mutex was identified in a separate audit finding (`015-ledger-single-mutex-bottleneck.md`) as the central bottleneck for throughput.

---

## Root Cause

The `compute_root_from_store` method was designed as a self-contained operation that fetches everything it needs internally. It does not accept the parent's state root as a parameter, so it must re-read it from the snapshot store via the mutex. The callers were not refactored to pass the already-available state root.

---

## Suggested Fix

Add a new method that accepts the parent root directly, avoiding the mutex:

**Before**:
```rust
pub async fn compute_root_from_store(
    &self,
    parent: ConsensusDigest,
    changes: &QmdbChangeSet,
) -> LedgerResult<StateRoot> {
    let parent_root = {
        let inner = self.inner.lock().await;
        inner.snapshots.get(&parent).ok_or(ConsensusError::SnapshotNotFound(parent))?.state_root
    };
    Ok(StateRoot(QmdbStateRoot::transition(parent_root.0, changes)))
}
```

**After** (add a new method, keep the old one for backward compatibility):
```rust
/// Compute the state root given a known parent root and changeset.
///
/// This avoids re-acquiring the inner mutex when the caller already
/// has the parent snapshot.
pub fn compute_root_with_parent(
    parent_root: StateRoot,
    changes: &QmdbChangeSet,
) -> StateRoot {
    StateRoot(QmdbStateRoot::transition(parent_root.0, changes))
}
```

Then update callers in `app.rs`:

**Before** (build_block):
```rust
let state_root = self.ledger.compute_root_from_store(parent_digest, &outcome.changes).await?;
```

**After** (build_block):
```rust
let state_root = LedgerService::compute_root_with_parent(
    parent_snapshot.state_root,  // Already available from wait_for_snapshot
    &outcome.changes,
);
```

The same change applies to `verify_block`. This eliminates one mutex acquisition per block per validator with approximately 10 lines of code change.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` -- add `compute_root_with_parent` method
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- update `build_block` and `verify_block` to pass parent state root directly

---

## Related Issues

- `015-ledger-single-mutex-bottleneck.md` -- the central mutex this issue adds contention to
- `065-perf-snapshot-chain-walk-tx-exclusion.md` -- another performance issue involving redundant work in the proposal path
- `068-perf-snapshot-cloning-deep-copies-changeset.md` -- snapshot access patterns that compound contention
