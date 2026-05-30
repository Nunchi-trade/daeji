# LedgerView Single Mutex Serializes Entire Block Pipeline

**Category**: Performance
**Severity**: High
**Labels**: `performance`, `consensus`, `storage`

## Summary

The entire `LedgerState` -- including the mempool, snapshot store, seed tracker, and QMDB handle -- is guarded by a single `futures::lock::Mutex`. Every ledger operation (transaction submission, state queries, snapshot reads/writes, state root computation, QMDB persistence) must acquire this global lock, serializing the entire block pipeline. This single-mutex design is the dominant throughput limiter in the system, preventing parallel snapshot reads during concurrent verification, blocking transaction submission during finalization, and serializing QMDB disk I/O against all other ledger operations.

## Problem

The `LedgerView` struct wraps all mutable ledger state in a single `futures::lock::Mutex<LedgerState>`:

**LedgerView definition** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs`, around line 94):

The struct holds:
```rust
pub struct LedgerView {
    inner: Arc<Mutex<LedgerState>>,  // Single futures::lock::Mutex for everything
    genesis_block: Block,
    snapshot_notify: Arc<::tokio::sync::Notify>,
}
```

`LedgerState` (the inner guarded type) contains four logically independent components:
- `snapshots: InMemorySnapshotStore` -- block snapshots (state overlays, changesets)
- `mempool: LedgerMempool` -- pending transaction pool
- `seeds: InMemorySeedTracker` -- VRF seeds for prevrandao
- `qmdb: QmdbLedger` -- persistent state database handle

Every public method on `LedgerView` acquires this single mutex. Looking at the code:

**submit_tx** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:285-288`):
```rust
pub async fn submit_tx(&self, tx: Tx) -> bool {
    let inner = self.inner.lock().await;  // Blocks on global mutex
    inner.mempool.insert(tx)
}
```

**query_state_root** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:316-319`):
```rust
pub async fn query_state_root(&self, digest: ConsensusDigest) -> Option<StateRoot> {
    let inner = self.inner.lock().await;  // Blocks on global mutex
    inner.snapshots.get(&digest).map(|snapshot| snapshot.state_root)
}
```

**parent_snapshot** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:340-343`):
```rust
pub async fn parent_snapshot(&self, parent: ConsensusDigest) -> Option<LedgerSnapshot> {
    let inner = self.inner.lock().await;  // Blocks on global mutex
    inner.snapshots.get(&parent)
}
```

**persist_snapshot** (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:464-514`) acquires the mutex **twice** -- once to read the changeset chain, then again after disk I/O to mark persistence:
```rust
pub async fn persist_snapshot(&self, digest: ConsensusDigest) -> LedgerResult<bool> {
    let (changes, qmdb, chain) = {
        let inner = self.inner.lock().await;  // First acquisition
        // ... collect changeset chain ...
    };
    let result = qmdb.commit_changes(changes).await;  // Disk I/O (mutex released)
    {
        let inner = self.inner.lock().await;  // Second acquisition
        // ... mark persisted, evict old snapshots ...
    }?;
    Ok(true)
}
```

During a single consensus round, the mutex is acquired many times:
- **Proposer**: `wait_for_snapshot` (polled in a loop), `proposal_components`, `compute_root_from_store`, `insert_snapshot` -- at least 4 acquisitions
- **Each verifier**: `query_state_root`, `parent_snapshot`, `compute_root_from_store`, `insert_snapshot` -- at least 4 acquisitions
- **FinalizedReporter**: `query_state_root`, `parent_snapshot`, `compute_root_from_store`, `insert_snapshot`, `persist_snapshot` (2 acquisitions), `prune_mempool`, `prune_stale_nonces` -- at least 8 acquisitions

At 33 blocks/s with 10 validators, there are roughly 30-40 mutex acquisitions per consensus round, each round lasting ~30ms.

## Code Reference

See code snippets above. The `InMemorySnapshotStore` at `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs` internally uses `parking_lot::RwLock` for fine-grained concurrent access:

```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,
    persisted: Arc<RwLock<BTreeSet<Digest>>>,
    persisting: Arc<RwLock<BTreeSet<Digest>>>,
    persisted_order: Arc<RwLock<VecDeque<Digest>>>,
    max_persisted_retained: usize,
}
```

However, the outer `futures::Mutex` in `LedgerView` wraps all of `LedgerState`, defeating the `InMemorySnapshotStore`'s internal fine-grained locking.

## Impact

The single mutex is the dominant throughput limiter in the system:
- **Snapshot reads block snapshot writes**: Multiple verifiers processing different blocks must wait for each other.
- **Transaction submissions block state queries**: `submit_tx` contends with `query_state_root`, `parent_snapshot`, etc.
- **QMDB persistence blocks everything**: While `persist_snapshot` holds the mutex for its second acquisition (post-disk-I/O), all other ledger operations -- including new block proposals and verifications -- are queued.
- **prune_stale_nonces queries QMDB per sender**: At line 530-555, the method holds the mutex while iterating over senders and performing async nonce queries against QMDB for each one.

Decomposing this mutex is estimated to yield a 10-15% throughput improvement by enabling parallel snapshot reads during concurrent block verification, non-blocking transaction submission during finalization, and overlapped QMDB persistence with block production.

## Root Cause

The initial design used a single mutex for simplicity. The four components inside `LedgerState` (snapshot store, mempool, seed tracker, QMDB handle) are logically independent and rarely need to be accessed atomically together. The pattern of "acquire global lock, do one small operation, release lock" is used throughout, indicating that fine-grained locking would be straightforward.

## Suggested Fix

Split `LedgerState` into independently lockable components:

```rust
pub struct LedgerView {
    snapshots: Arc<RwLock<InMemorySnapshotStore<OverlayState<QmdbState>>>>,
    mempool: Arc<RwLock<LedgerMempool>>,
    seeds: Arc<RwLock<InMemorySeedTracker>>,
    qmdb: QmdbLedger,                         // Already internally locked
    head: Arc<RwLock<ConsensusDigest>>,        // Track current head separately
    snapshot_notify: Arc<Notify>,
}
```

Key benefits:
- `parent_snapshot` and `query_state_root` use `RwLock::read()`, enabling parallel reads
- `submit_tx` only locks `mempool`, not the entire state
- `persist_snapshot` only locks `snapshots` and `qmdb`, not `mempool` or `seeds`
- `seed_for_parent` only locks `seeds`
- The few operations that need multiple components (e.g., `proposal_components` needs mempool + snapshots) can acquire the individual locks in a consistent order to prevent deadlocks

A middle-ground approach is to switch from `futures::lock::Mutex` to `tokio::sync::RwLock`, which would at least allow concurrent reads while still using a single lock. This is less optimal than full decomposition but significantly simpler to implement.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (struct definition around line 94) -- `LedgerView` needs restructured locking
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (lines 280-573) -- all public methods need to acquire individual component locks
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (lines 464-514) -- `persist_snapshot` is the most impactful to fix (double lock acquisition with disk I/O between)

## Related Issues

- `020-qmdb-persistence-blocks-finalization.md` -- QMDB persistence blocking finalization is exacerbated by this single mutex
- `011-snapshot-eviction-race-finalization.md` -- the mutex timing plays a role in the eviction race
- `150-finalize-lock-starves-proposal.md` -- lock starvation between finalization and proposal paths
- `143-qmdb-ledger-commit-double-lock-acquisition.md` -- QMDB ledger commit has its own nested lock issue
