# Snapshot Store TOCTOU Race Condition in Eviction Path

## Summary

The `InMemorySnapshotStore` has a TOCTOU (time-of-check-to-time-of-use) race condition: `evict_persisted()` runs outside the `LedgerState` mutex in `persist_snapshot()`, allowing a concurrent `persist_snapshot()` or `changes_for_persist()` call to observe partially-evicted state. The race can cause `SnapshotNotFound` errors during ancestor chain-walking when a snapshot is evicted between the chain-walk read and the snapshot data lookup.

**Severity: High** (downgraded from Critical -- see analysis below).

## Background: What PR #125 Fixed

PR #125 (`0ea4b18`, "bound InMemorySnapshotStore to prevent OOM") added oldest-first eviction of persisted snapshots to prevent unbounded memory growth. Before #125, persisted snapshots were never freed, causing OOM on long-running nodes.

PR #125 introduced:
- `DEFAULT_MAX_PERSISTED_RETAINED = 64` (line 24 of `snapshot.rs`)
- `evict_persisted()` method (lines 127-160 of `snapshot.rs`)
- `persisted_order` queue for FIFO eviction tracking
- The call to `evict_persisted()` at line 448 of `ledger/src/lib.rs`, placed **outside** the `LedgerState` mutex

The eviction logic itself is correct. The race condition arises from **where** it is called in the ledger.

## The Race Condition (What Remains After PR #125)

### Where eviction is called

In `crates/node/ledger/src/lib.rs`, lines 401-449, `persist_snapshot()` calls `evict_persisted()` after releasing the `LedgerState` mutex:

```rust
// crates/node/ledger/src/lib.rs, lines 401-449
pub async fn persist_snapshot(&self, digest: ConsensusDigest) -> LedgerResult<bool> {
    let (changes, qmdb, chain) = {
        let inner = self.inner.lock().await;                    // Lock acquired
        let (chain, changes) = inner.snapshots.changes_for_persist(digest)?;
        if chain.is_empty() { return Ok(false); }
        if !inner.snapshots.can_persist_chain(&chain) { return Ok(false); }
        inner.snapshots.mark_persisting_chain(&chain);
        (changes, inner.qmdb.clone(), chain)
    };                                                          // Lock RELEASED

    let result = qmdb.commit_changes(changes).await;           // QMDB commit (slow)
    let snapshots_handle = {
        let inner = self.inner.lock().await;                    // Lock re-acquired
        inner.snapshots.clear_persisting_chain(&chain);
        match result {
            Ok(_) => {
                for digest in &chain {
                    let snapshot = inner.snapshots.get(digest)
                        .ok_or(ConsensusError::SnapshotNotFound(*digest))?;
                    let compact_state =
                        OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
                    inner.snapshots.insert(*digest, Snapshot::new(
                        snapshot.parent, compact_state, snapshot.state_root,
                        QmdbChangeSet::default(), snapshot.tx_ids,
                    ));
                }
                inner.snapshots.mark_persisted(&chain);
                Ok(inner.snapshots.clone())                     // Clone the Arc-wrapped store
            }
            Err(err) => Err(LedgerError::from(err)),
        }
    }?;                                                         // Lock RELEASED again

    // Line 448: eviction happens OUTSIDE the LedgerState mutex
    snapshots_handle.evict_persisted();
    Ok(true)
}
```

The comment on line 446 explains the rationale: "Done outside the `inner` mutex since `InMemorySnapshotStore` uses its own fine-grained `RwLock`s internally." However, the snapshot store's internal `parking_lot::RwLock`s and the outer `futures::lock::Mutex` (on `LedgerState`) are two independent lock hierarchies. The inner locks protect the store's data structures from corruption, but they do **not** coordinate with other ledger operations that depend on snapshot presence.

### The TOCTOU window

The race occurs when two tasks concurrently call `persist_snapshot()`:

```
Task A (persist block N):                   Task B (persist block N+K):
  mark_persisted(chain_A) [inside mutex]
  clone snapshots_handle
  release LedgerState mutex
                                              acquire LedgerState mutex
                                              changes_for_persist(N+K)
                                                -> walks ancestors back to
                                                   persisted boundary
                                                -> reads snapshot data for
                                                   unpersisted ancestors
                                              release LedgerState mutex
  evict_persisted()
    -> removes oldest persisted snapshots
    -> snapshot for ancestor X removed
                                              (Task B already has its
                                               ChangeSet -- no problem HERE)
```

More critically, the race between `evict_persisted()` and a concurrent `changes_for_persist()` call that happens **inside** the snapshot store's own locks:

```
Task A:                                     Task B:
  evict_persisted()
    Phase 1: read persisted + order locks
      -> determines [X, Y] should be evicted
    Release persisted + order locks
                                              changes_for_persist(Z)
                                                -> acquires snapshots read lock
                                                -> walks from Z back through
                                                   ancestors, finds X is not
                                                   yet persisted? No -- X IS
                                                   persisted, so chain-walk
                                                   terminates. But if Task B
                                                   needs X's snapshot data for
                                                   merged_changes()...
    Phase 2: acquire snapshots write lock
      -> removes X, Y from snapshots map
                                                -> X's data now gone from map
```

The current `evict_persisted()` implementation (lines 127-160 of `snapshot.rs`) holds `snapshots`, `persisted`, and `persisted_order` locks simultaneously in its single-phase approach:

```rust
// crates/node/consensus/src/components/snapshot.rs, lines 127-160
pub fn evict_persisted(&self) -> usize {
    // Fast path: check with a read lock to avoid write-lock contention
    // when no eviction is needed (the common case).
    if self.persisted_order.read().len() <= self.max_persisted_retained {
        return 0;
    }

    let mut snapshots = self.snapshots.write();
    let persisted = self.persisted.read();
    let mut order = self.persisted_order.write();

    let mut evicted = 0usize;
    while order.len() > self.max_persisted_retained {
        let Some(oldest) = order.pop_front() else {
            break;
        };
        // Only remove snapshot data if it is actually persisted.
        // (Guards against stale entries in the order queue.)
        if persisted.contains(&oldest) && snapshots.remove(&oldest).is_some() {
            evicted += 1;
        }
    }

    if evicted > 0 {
        debug!(
            evicted,
            retained = snapshots.len(),
            persisted = persisted.len(),
            "evicted persisted snapshots"
        );
    }

    evicted
}
```

Note: `evict_persisted()` acquires `snapshots`(W) then `persisted`(R), while `merged_changes()` and `changes_for_persist()` acquire `snapshots`(R) then `persisted`(R). Under `parking_lot`'s write-preferring policy, this could also cause a potential deadlock (tracked separately on the `fix/snapshot-store-bounded-eviction` branch at commit `e1f2576`).

## Why Severity is High, Not Critical

The original issue rated this as Critical, claiming it causes "permanent state loss" and "missing parent snapshot" cascades. After closer analysis, the severity should be **High** for these reasons:

1. **Parent snapshot cloning provides natural protection.** The `parent_snapshot()` method (line 320-323 of `ledger/src/lib.rs`) returns a `.cloned()` copy of the snapshot. The `Snapshot` struct derives `Clone` (line 19 of `traits.rs`). Once a caller obtains a parent snapshot clone, subsequent eviction of the original from the store does NOT invalidate the clone. This means:
   - `build_block()` (line 96 of `app.rs`) clones the parent snapshot before using it -- safe from eviction.
   - `verify_block()` (line 204 of `app.rs`) clones the parent snapshot before using it -- safe from eviction.
   - `finalize_block()` (line 193 of `reporters/src/lib.rs`) clones the parent snapshot before using it -- safe from eviction.

2. **The race window is narrow in practice.** The vulnerability requires `evict_persisted()` to remove a snapshot that a concurrent `changes_for_persist()` is actively walking through inside the snapshot store's own locks. Because `evict_persisted()` holds the `snapshots` write lock while removing, and `changes_for_persist()` holds the `snapshots` read lock while walking, these cannot actually interleave at the data-access level. The race can only manifest if Task B calls `changes_for_persist()` AFTER Task A's eviction completes, expecting an ancestor snapshot to still be present. At the current retention of 64 snapshots, this requires the finalization pipeline to be at least 64 blocks behind -- unlikely during normal operation but possible during recovery or burst finalization.

3. **The "missing parent snapshot" errors in devnet logs are more likely caused by snapshot absence** (block never received/verified, not eviction). The finalize_block path re-executes blocks that lack a cached snapshot, so eviction of already-persisted snapshots does not prevent finalization -- `changes_for_persist` only walks to the persisted boundary and does not need the snapshot data of already-persisted blocks.

The race IS real and should be fixed, but it is not the primary cause of the observed devnet failures. The more likely root causes are resolver peer-blocking (PR #131) and consensus nullification (PR #136).

## When This Can Cause a Problem in Practice

The TOCTOU manifests specifically when:

1. **Concurrent persist calls**: Two `persist_snapshot()` calls for different blocks run simultaneously. Task A finishes and calls `evict_persisted()` while Task B is between acquiring the LedgerState mutex (to call `changes_for_persist()`) and completing its chain walk. If Task A evicts an ancestor that Task B's chain walk will need, Task B gets `SnapshotNotFound`.

2. **Burst finalization after recovery**: If a node falls behind and finalization catches up in a burst, many `persist_snapshot()` calls may overlap. Each call evicts old snapshots, and the combined eviction pressure can remove snapshots that other in-flight persist calls still need for their ancestor chain walks.

3. **The `persisting` set is not consulted during eviction**: `evict_persisted()` only checks the `persisted` set (line 145). A snapshot whose children are currently being persisted (marked in `persisting`) could be evicted if it is already in the `persisted` set. This could cause the in-flight persist task to fail during overlay compaction when it tries to read the evicted snapshot's data (lines 422-437 of `ledger/src/lib.rs`).

## Proposed Fix

### Fix 1: Move eviction inside the LedgerState mutex (recommended)

The simplest and most correct fix is to call `evict_persisted()` while the LedgerState mutex is still held. This eliminates the TOCTOU entirely because no other ledger operation can interleave:

```rust
// crates/node/ledger/src/lib.rs - persist_snapshot()
pub async fn persist_snapshot(&self, digest: ConsensusDigest) -> LedgerResult<bool> {
    let (changes, qmdb, chain) = {
        let inner = self.inner.lock().await;
        let (chain, changes) = inner.snapshots.changes_for_persist(digest)?;
        if chain.is_empty() { return Ok(false); }
        if !inner.snapshots.can_persist_chain(&chain) { return Ok(false); }
        inner.snapshots.mark_persisting_chain(&chain);
        (changes, inner.qmdb.clone(), chain)
    };

    let result = qmdb.commit_changes(changes).await;

    {
        let inner = self.inner.lock().await;    // Single lock scope for mark + evict
        inner.snapshots.clear_persisting_chain(&chain);
        match result {
            Ok(_) => {
                for digest in &chain {
                    let snapshot = inner.snapshots.get(digest)
                        .ok_or(ConsensusError::SnapshotNotFound(*digest))?;
                    let compact_state =
                        OverlayState::new(inner.qmdb.state(), QmdbChangeSet::default());
                    inner.snapshots.insert(*digest, Snapshot::new(
                        snapshot.parent, compact_state, snapshot.state_root,
                        QmdbChangeSet::default(), snapshot.tx_ids,
                    ));
                }
                inner.snapshots.mark_persisted(&chain);
                inner.snapshots.evict_persisted();  // INSIDE the mutex
                Ok(())
            }
            Err(err) => Err(LedgerError::from(err)),
        }
    }?;
    Ok(true)
}
```

This is safe because `evict_persisted()` uses `parking_lot::RwLock`s internally (synchronous, non-async), so holding the async `futures::lock::Mutex` while calling it will not cause issues. The only downside is slightly longer mutex hold times, but eviction is fast (O(n) in the number of evicted entries, typically small).

### Fix 2: Guard eviction against in-flight persists

Add a check in `evict_persisted()` to skip snapshots that are ancestors of any entry in the `persisting` set:

```rust
pub fn evict_persisted(&self) -> usize {
    if self.persisted_order.read().len() <= self.max_persisted_retained {
        return 0;
    }

    let mut snapshots = self.snapshots.write();
    let persisted = self.persisted.read();
    let persisting = self.persisting.read();    // Also check persisting set
    let mut order = self.persisted_order.write();

    let mut evicted = 0usize;
    while order.len() > self.max_persisted_retained {
        let Some(oldest) = order.pop_front() else { break; };
        // Skip if any in-flight persist depends on this snapshot
        if !persisting.is_empty() {
            // Re-push to back of queue and skip
            order.push_back(oldest);
            continue;
        }
        if persisted.contains(&oldest) && snapshots.remove(&oldest).is_some() {
            evicted += 1;
        }
    }
    // ... logging ...
    evicted
}
```

### Fix 3: Increase default retention (defense-in-depth)

```rust
// crates/node/consensus/src/components/snapshot.rs
const DEFAULT_MAX_PERSISTED_RETAINED: usize = 256;  // ~2.5s at 100 bps
```

Also expose via `ConsensusConfig` so operators can tune for their block production rate.

## Files Involved

| File | Lines | Role |
|------|-------|------|
| `crates/node/consensus/src/components/snapshot.rs` | 24, 127-160 | `DEFAULT_MAX_PERSISTED_RETAINED`, `evict_persisted()` |
| `crates/node/consensus/src/traits.rs` | 19-31 | `Snapshot` struct (derives `Clone`) |
| `crates/node/ledger/src/lib.rs` | 320-323 | `parent_snapshot()` returns `.cloned()` |
| `crates/node/ledger/src/lib.rs` | 401-449 | `persist_snapshot()` -- eviction called outside mutex (line 448) |
| `crates/node/reporters/src/lib.rs` | 169-279 | `finalize_block()` -- clones parent snapshot at line 193, fails on missing at line 255 |
| `crates/node/runner/src/app.rs` | 91-107, 194-213 | `build_block()` / `verify_block()` -- clone parent snapshot before use |

## Related PRs

- **PR #125** (`0ea4b18`): Introduced bounded eviction. Fixed the OOM problem but introduced the TOCTOU race by placing `evict_persisted()` outside the LedgerState mutex.
- **PR #131** (`e0f941f`): Fixed resolver peer-blocking, which was the more likely root cause of the "missing parent snapshot" errors observed in devnet logs.
- **Branch `fix/snapshot-store-bounded-eviction`** (commit `e1f2576`): Contains an unmerged deadlock fix that restructures `evict_persisted()` into a two-phase approach to avoid holding `snapshots` and `persisted` locks simultaneously. This fix addresses the internal lock-ordering issue but does NOT address the outer TOCTOU race documented here.

## Priority

**High** -- The TOCTOU race is a correctness bug that can cause `SnapshotNotFound` errors under concurrent persist calls, particularly during burst finalization after node recovery. However, the parent snapshot cloning pattern means the race window is narrower than originally assessed, and the observed devnet failures are more likely attributable to resolver peer-blocking (fixed in PR #131) and consensus nullification (fixed in PR #136).

Fix 1 (move eviction inside mutex) should be applied as it is a minimal, low-risk change. Fix 3 (increase retention) provides additional margin.
