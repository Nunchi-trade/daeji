# InMemorySeedTracker Grows Without Bound -- Memory Leak

**Category**: Bug
**Severity**: High
**Labels**: `bug`, `reliability`, `consensus`, `metrics`

## Summary

The `InMemorySeedTracker` stores VRF seeds (used for prevrandao) in a `BTreeMap<Digest, B256>` that is never pruned. Seeds are inserted for every notarized and finalized block but are never removed, causing the map to grow monotonically. At 33 blocks/second, this leaks approximately 182 MB per day. Only the most recent block's seed is ever queried, making all older entries useless.

## Problem

The `InMemorySeedTracker` at `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/seed.rs:12-46` is a simple append-only wrapper around a `BTreeMap`:

```rust
#[derive(Debug, Clone)]
pub struct InMemorySeedTracker {
    inner: Arc<RwLock<BTreeMap<Digest, B256>>>,
}
```

The `SeedTracker` trait implementation at lines 38-45 provides `get` and `insert` methods, but no `remove`, `prune`, or `evict` method:

```rust
impl SeedTracker for InMemorySeedTracker {
    fn get(&self, digest: &Digest) -> Option<B256> {
        self.inner.read().get(digest).copied()
    }

    fn insert(&self, digest: Digest, seed: B256) {
        self.inner.write().insert(digest, seed);
        // No eviction, no size limit, no pruning
    }
}
```

Seeds are inserted via `LedgerView::set_seed()` at `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:334-337`:

```rust
pub async fn set_seed(&self, digest: ConsensusDigest, seed_hash: B256) {
    let inner = self.inner.lock().await;
    inner.seeds.insert(digest, seed_hash);
}
```

This is called from the `SeedReporter` on every notarization and finalization event.

The only consumer of seed data is `seed_for_parent()` at `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs:328-331`, which looks up the seed for the parent of the block being proposed:

```rust
pub async fn seed_for_parent(&self, parent: ConsensusDigest) -> Option<B256> {
    let inner = self.inner.lock().await;
    inner.seeds.get(&parent)
}
```

This is called from `get_prevrandao()` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:253-255`:

```rust
async fn get_prevrandao(&self, parent_digest: ConsensusDigest) -> B256 {
    self.ledger.seed_for_parent(parent_digest).await.unwrap_or(B256::ZERO)
}
```

Only the parent block's seed is ever needed -- seeds for blocks more than a few blocks old are never accessed again.

## Code Reference

**Full InMemorySeedTracker implementation** (`/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/seed.rs:1-72`):

```rust
pub struct InMemorySeedTracker {
    inner: Arc<RwLock<BTreeMap<Digest, B256>>>,
}

impl InMemorySeedTracker {
    pub fn new(genesis_digest: Digest) -> Self {
        let mut seeds = BTreeMap::new();
        seeds.insert(genesis_digest, B256::ZERO);
        Self { inner: Arc::new(RwLock::new(seeds)) }
    }

    pub fn empty() -> Self {
        Self { inner: Arc::new(RwLock::new(BTreeMap::new())) }
    }
}

impl SeedTracker for InMemorySeedTracker {
    fn get(&self, digest: &Digest) -> Option<B256> {
        self.inner.read().get(digest).copied()
    }

    fn insert(&self, digest: Digest, seed: B256) {
        self.inner.write().insert(digest, seed);
    }
}
```

## Impact

Each entry is approximately 64 bytes (32-byte `Digest` key + 32-byte `B256` value) plus BTreeMap node overhead (~40 bytes per node). At 33 blocks/second:

- ~33 entries/second = ~3.4 KB/s (including BTreeMap overhead)
- ~12.2 MB/hour
- ~293 MB/day (including overhead)

After 24 hours of continuous operation, the seed tracker alone consumes roughly 200-300 MB. Combined with other unbounded caches (the `block_fees` HashMap leaking ~132 MB/day per issue #010), this is a significant contributor to the 4 GB memory ceiling observed on the live devnet.

The `InMemorySeedTracker` is wrapped in `LedgerState` which is behind the global `futures::Mutex`. The growing BTreeMap also increases lock hold time for every `set_seed` and `seed_for_parent` call as BTreeMap operations are O(log n) and the constant factor grows with the tree depth.

## Root Cause

The seed tracker was designed as a simple append-only data structure with no eviction logic. Since only the most recent parent's seed is needed for block proposal, seeds for blocks more than a few blocks old serve no purpose. No pruning mechanism was implemented.

## Suggested Fix

Replace the unbounded `BTreeMap` with a bounded data structure. Two options:

**Option 1: LRU cache** (cleanest):

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

pub struct InMemorySeedTracker {
    inner: Arc<RwLock<LruCache<Digest, B256>>>,
}

impl InMemorySeedTracker {
    pub fn new(genesis_digest: Digest, capacity: usize) -> Self {
        let mut seeds = LruCache::new(NonZeroUsize::new(capacity).unwrap());
        seeds.put(genesis_digest, B256::ZERO);
        Self { inner: Arc::new(RwLock::new(seeds)) }
    }
}
```

A capacity of 256 (matching the snapshot store's eviction limit) would cap memory at ~25 KB.

**Option 2: Prune from finalization pipeline** (if LRU dependency is unwanted):

Add a `prune_below(&self, keep_count: usize)` method that trims the BTreeMap to the most recent `keep_count` entries, called from the finalization pipeline after each block:

```rust
impl InMemorySeedTracker {
    pub fn prune(&self, max_entries: usize) {
        let mut inner = self.inner.write();
        while inner.len() > max_entries {
            inner.pop_first(); // Remove oldest entry
        }
    }
}
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/seed.rs` (lines 12-46) -- `InMemorySeedTracker` needs bounded storage
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (line 337) -- `set_seed` could trigger pruning after insert
- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/lib.rs` or traits file -- `SeedTracker` trait may need a `prune` method

## Related Issues

- `010-block-fees-hashmap-unbounded.md` -- identical pattern: unbounded HashMap leaking memory for `block_fees` cache
- `009-graduated-blocker-never-cleared.md` -- another unbounded memory growth issue
- `015-ledger-single-mutex-bottleneck.md` -- growing BTreeMap increases lock hold time under the global mutex
