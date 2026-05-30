# 151: InMemorySnapshotStore Acquires Multiple RwLocks in Inconsistent Order -- Latent Deadlock

**Category**: bug
**Severity**: medium
**Component**: consensus

## Summary

The `InMemorySnapshotStore` has four independently locked fields (`snapshots`, `persisted`, `persisting`, `persisted_order`), and multiple methods acquire more than one of these locks simultaneously. While the acquisition order is mostly consistent (snapshots -> persisted -> persisted_order), the `can_persist_chain()` method acquires `persisted` then `persisting`, while `mark_persisted()` acquires `persisted` then `persisted_order`, creating a potential lock ordering inconsistency. Currently, the outer `futures::lock::Mutex` in `LedgerView` serializes all access to the snapshot store, preventing deadlocks. However, if that outer mutex is ever removed or relaxed (as suggested by issue #015), the inconsistent lock ordering becomes a real deadlock risk.

## Problem

The `InMemorySnapshotStore` in `crates/node/consensus/src/components/snapshot.rs` uses four separate `parking_lot::RwLock`-protected fields:

```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,
    persisted: Arc<RwLock<BTreeSet<Digest>>>,
    persisting: Arc<RwLock<BTreeSet<Digest>>>,
    persisted_order: Arc<RwLock<VecDeque<Digest>>>,
    max_persisted_retained: usize,
}
```

Methods that acquire multiple locks:

| Method | Lock 1 | Lock 2 | Lock 3 |
|--------|--------|--------|--------|
| `evict_persisted` (line 145-147) | `snapshots.write()` | `persisted.read()` | `persisted_order.write()` |
| `merged_changes` (line 208-209) | `snapshots.read()` | `persisted.read()` | -- |
| `changes_for_persist` (line 241-242) | `snapshots.read()` | `persisted.read()` | -- |
| `mark_persisted` (line 194-195) | `persisted.write()` | `persisted_order.write()` | -- |
| `unpersisted_count` (line 96-97) | `snapshots.read()` | `persisted.read()` | -- |
| `can_persist_chain` (line 105-106) | `persisted.read()` | `persisting.read()` | -- |

The ordering is mostly consistent: `snapshots` before `persisted` before `persisted_order`. However:

- `can_persist_chain` acquires `persisted.read()` then `persisting.read()`
- `mark_persisting_chain` acquires only `persisting.write()`
- No method acquires both `persisting` and `persisted_order`, but if the code evolves, the lack of a documented ordering protocol makes it easy to introduce deadlocks.

`parking_lot::RwLock` does not support recursive read locks and can deadlock if a write is pending while reads are held by the same thread.

**File**: `crates/node/consensus/src/components/snapshot.rs`

## Code Reference

```rust
// crates/node/consensus/src/components/snapshot.rs:104-108
/// Returns true if every digest in the chain is neither persisted nor in-flight.
pub fn can_persist_chain(&self, chain: &[Digest]) -> bool {
    let persisted = self.persisted.read();    // Lock 1: persisted
    let persisting = self.persisting.read();  // Lock 2: persisting
    chain.iter().all(|digest| !persisted.contains(digest) && !persisting.contains(digest))
}
```

```rust
// crates/node/consensus/src/components/snapshot.rs:138-171
pub fn evict_persisted(&self) -> usize {
    if self.persisted_order.read().len() <= self.max_persisted_retained {
        return 0;
    }

    let mut snapshots = self.snapshots.write();    // Lock 1: snapshots
    let persisted = self.persisted.read();          // Lock 2: persisted
    let mut order = self.persisted_order.write();   // Lock 3: persisted_order

    let mut evicted = 0usize;
    while order.len() > self.max_persisted_retained {
        let Some(oldest) = order.pop_front() else { break; };
        if persisted.contains(&oldest) && snapshots.remove(&oldest).is_some() {
            evicted += 1;
        }
    }
    // ...
}
```

```rust
// crates/node/consensus/src/components/snapshot.rs:193-201
fn mark_persisted(&self, digests: &[Digest]) {
    let mut persisted = self.persisted.write();          // Lock 1: persisted
    let mut order = self.persisted_order.write();        // Lock 2: persisted_order
    for digest in digests {
        if persisted.insert(*digest) {
            order.push_back(*digest);
        }
    }
}
```

## Impact

**Current risk**: None. The outer `futures::lock::Mutex` in `LedgerView` (the `inner` field) serializes all calls to snapshot store methods, so the RwLocks are never contended across threads.

**Future risk**: If the outer mutex is removed or relaxed (as proposed in issue #015 to reduce mutex contention on the proposal path), concurrent access to the snapshot store would expose the inconsistent lock ordering. A deadlock scenario:

1. Thread A calls `evict_persisted()`: acquires `snapshots.write()`, then `persisted.read()`, then waits for `persisted_order.write()`
2. Thread B calls `mark_persisted()`: acquires `persisted.write()` (blocked by A's read), then would need `persisted_order.write()` (also blocked)
3. Thread C holds `persisted_order.read()` from some other method

With `parking_lot::RwLock`, a pending write lock (from B on `persisted`) will block new read locks, so A's `persisted.read()` could prevent B from ever getting its write lock, creating a livelock.

## Root Cause

The snapshot store was built incrementally with separate locks for each field, without establishing and documenting a strict lock ordering protocol. The separate locks were likely intended to reduce contention, but the multi-lock acquisition patterns create ordering hazards.

## Suggested Fix

**Option A**: Consolidate into a single lock:

```rust
struct SnapshotStoreInner<S> {
    snapshots: BTreeMap<Digest, Snapshot<S>>,
    persisted: BTreeSet<Digest>,
    persisting: BTreeSet<Digest>,
    persisted_order: VecDeque<Digest>,
}

pub struct InMemorySnapshotStore<S> {
    inner: Arc<RwLock<SnapshotStoreInner<S>>>,
    max_persisted_retained: usize,
}
```

This eliminates all multi-lock acquisition and is simpler to reason about. Since the outer ledger mutex already serializes access, the single lock adds no additional contention.

**Option B**: Document and enforce strict lock ordering:

```
Lock order: snapshots -> persisted -> persisting -> persisted_order
All methods MUST acquire locks in this order. Never acquire a lock earlier in the order while holding a lock later in the order.
```

Then audit all methods to ensure compliance.

Option A is preferred for simplicity and safety.

## Files to Modify

- `crates/node/consensus/src/components/snapshot.rs` -- consolidate four locks into one, or document and enforce ordering

## Related Issues

- [015-ledger-single-mutex-bottleneck.md](./015-ledger-single-mutex-bottleneck.md) -- the outer mutex that currently prevents this deadlock; removing it would expose this issue
- [150-finalize-lock-starves-proposal.md](./150-finalize-lock-starves-proposal.md) -- related lock contention in the finalization path

## Labels

`bug`, `reliability`, `consensus`
