# persisted_order VecDeque Capacity Never Shrinks

**Category**: bug -- consensus
**Severity**: medium

## Summary

The `InMemorySnapshotStore` uses a `persisted_order: Arc<RwLock<VecDeque<Digest>>>` to track the insertion order of persisted snapshots for oldest-first eviction. While the VecDeque's logical length is bounded in steady state at approximately `max_persisted_retained + 1` entries (because `evict_persisted()` pops from the front), Rust's `VecDeque` never automatically shrinks its allocated capacity. Over time, if there are burst insertions followed by evictions, the underlying allocation ratchets upward and never reclaims memory.

## Problem

The `persisted_order` VecDeque is used to maintain insertion order so that `evict_persisted()` can remove the oldest persisted snapshots first. The eviction loop pops from the front:

```rust
while order.len() > self.max_persisted_retained {
    let Some(oldest) = order.pop_front() else { break; };
    // ...
}
```

While the logical length stays bounded near `max_persisted_retained` (default: 256), the VecDeque's underlying allocation only grows -- `pop_front()` does not release memory. In Rust, `VecDeque::pop_front()` moves the internal head pointer but does not reallocate. The capacity only grows when `push_back()` needs more space.

In steady-state operation at 34 blocks/s, the VecDeque length oscillates between `max_persisted_retained` and `max_persisted_retained + batch_size` (where `batch_size` is the number of blocks persisted in each batch). The capacity never exceeds the peak length, so in practice the memory waste is small -- at most a few extra kilobytes.

This issue is significantly less severe than the companion `persisted` BTreeSet leak (issue #205), which is truly unbounded. This finding is included for completeness since both issues are in the same component.

## Code Reference

File: `crates/node/consensus/src/components/snapshot.rs`

**Field declaration (line 38):**
```rust
pub struct InMemorySnapshotStore<S> {
    snapshots: Arc<RwLock<BTreeMap<Digest, Snapshot<S>>>>,
    persisted: Arc<RwLock<BTreeSet<Digest>>>,
    persisting: Arc<RwLock<BTreeSet<Digest>>>,
    persisted_order: Arc<RwLock<VecDeque<Digest>>>,  // <-- capacity never shrinks
    max_persisted_retained: usize,
}
```

**evict_persisted() pops from front (lines 138-171):**
```rust
pub fn evict_persisted(&self) -> usize {
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
        if persisted.contains(&oldest) && snapshots.remove(&oldest).is_some() {
            evicted += 1;
        }
    }
    // order.len() is now <= max_persisted_retained
    // but order.capacity() may be much larger
    // ...
}
```

**mark_persisted() pushes to back (lines 193-201):**
```rust
fn mark_persisted(&self, digests: &[Digest]) {
    let mut persisted = self.persisted.write();
    let mut order = self.persisted_order.write();
    for digest in digests {
        if persisted.insert(*digest) {
            order.push_back(*digest);
        }
    }
}
```

## Impact

In practice, the memory impact is minimal. The VecDeque length is bounded at approximately `max_persisted_retained + batch_size` (roughly 256 + ~10 = ~266 entries). Each entry is a 32-byte digest, so the total is about 8.5 KB of used data. Even if the capacity is 2x or 4x the length (due to VecDeque's growth strategy), the wasted memory is only ~8-24 KB.

The primary concern is theoretical: if there were burst patterns that caused large spikes in the VecDeque length, the capacity would ratchet up and never shrink. But in Kora's steady-state operation, this is unlikely to be significant.

## Root Cause

Rust's `VecDeque` does not automatically shrink its capacity when elements are removed via `pop_front()`. This is a well-known property of Rust's standard collections. The eviction loop maintains a bounded logical length but the allocated capacity may exceed the actual usage.

## Suggested Fix

Periodically call `VecDeque::shrink_to_fit()` after eviction:

```rust
if evicted > 0 {
    order.shrink_to_fit();
    // ... existing logging ...
}
```

Alternatively, since the steady-state length is bounded and the memory impact is minimal, this may not be worth fixing independently. The more impactful fix is addressing the persisted BTreeSet leak in issue #205.

## Files to Modify

- `crates/node/consensus/src/components/snapshot.rs` -- Add `order.shrink_to_fit()` after the eviction loop in `evict_persisted()` (around line 159)

## Related Issues

- `205-snapshot-persisted-set-unbounded.md` -- The more severe companion leak in the same component (persisted BTreeSet grows without bound)
- `010-block-fees-hashmap-unbounded.md` -- Same pattern of unbounded growth in the block fees cache
- `016-seed-tracker-unbounded.md` -- Same pattern of unbounded growth in the seed tracker

## Labels

bug, reliability, consensus, performance
