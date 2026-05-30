# 003: RootProvider Opens Fresh QMDB Stores on Every State Root Query

**Category:** bug / storage / performance
**Severity:** critical
**Labels:** bug, performance, storage, correctness

---

## Summary

The `CommonwareRootProvider` opens all three QMDB partition stores from disk on every invocation of `state_root()` and `compute_root()`. At 33 blocks/second with multiple root computations per block (build path + verify path), this creates hundreds of unnecessary filesystem operations per second. Additionally, the `compute_root()` path opens "dirty" stores, applies the full changeset, reads the root, and then discards everything -- introducing a potential TOCTOU (time-of-check-to-time-of-use) race with concurrent writes.

## Problem

The `CommonwareRootProvider` struct at `crates/storage/backend/src/backend.rs:38-41` stores only a `context` and `config` -- it does not cache any opened store handles. Every call to `state_root()` or `compute_root()` re-opens the stores from disk via `open_stores()` or `open_dirty_stores()`.

In the `RootProvider` trait implementation at lines 164-195:

1. **`state_root()` (line 165)**: Calls `open_stores()` which initializes all three commonware-storage partitions (accounts, storage, code) from disk. After reading the root hashes, the stores are dropped.

2. **`compute_root()` (line 173)**: Calls `open_dirty_stores()` (line 178) which opens fresh mutable stores, applies the entire changeset via `qmdb.commit_changes()`, reads the resulting root hashes, and then discards everything. If another thread is concurrently writing to the same on-disk stores, the root computation may reflect partially-committed data from an unrelated block.

The `open_stores()` helper at lines 236-269 initializes three separate commonware-storage instances, each requiring filesystem I/O to read partition metadata, journal indices, and merkle tree state.

## Code Reference

**`state_root()` opens fresh stores every call -- `crates/storage/backend/src/backend.rs:163-171`:**

```rust
#[async_trait]
impl RootProvider for CommonwareRootProvider {
    async fn state_root(&self) -> Result<B256, HandleError> {
        let stores = open_stores(&self.context, &self.config)  // opens fresh each time!
            .await
            .map_err(|e| HandleError::RootComputation(e.to_string()))?;
        state_root_from_stores(&stores.accounts, &stores.storage, &stores.code)
            .map_err(|e| HandleError::RootComputation(e.to_string()))
    }
```

**`compute_root()` opens dirty stores, applies changes, discards -- `crates/storage/backend/src/backend.rs:173-190`:**

```rust
    async fn compute_root(&mut self, changes: &ChangeSet) -> Result<B256, HandleError> {
        if changes.is_empty() {
            return self.state_root().await;
        }

        let stores = open_dirty_stores(&self.context, &self.config)  // opens fresh dirty stores
            .await
            .map_err(|e| HandleError::RootComputation(e.to_string()))?;
        let mut qmdb = QmdbStore::new(stores.accounts, stores.storage, stores.code);
        qmdb.commit_changes(changes.clone())  // applies changes to throwaway stores
            .await
            .map_err(|e| HandleError::RootComputation(e.to_string()))?;
        let stores = qmdb.take_stores().map_err(|e| HandleError::RootComputation(e.to_string()))?;
        let accounts = stores.accounts.root();
        let storage = stores.storage.root();
        let code = stores.code.root();
        Ok(state_root_from_roots(accounts, storage, code))
    }
```

**Stateless struct with no cached handles -- `crates/storage/backend/src/backend.rs:37-41`:**

```rust
/// Root provider that computes state roots from commonware-storage partitions.
pub struct CommonwareRootProvider {
    context: Context,
    config: QmdbBackendConfig,
}
```

**Store opening helper performs 3 filesystem operations -- `crates/storage/backend/src/backend.rs:236-269`:**

```rust
async fn open_stores(
    context: &Context,
    config: &QmdbBackendConfig,
) -> Result<Stores, BackendError> {
    let page_cache = CacheRef::from_pooler(context, config.page_size, config.page_cache_size);

    let accounts = AccountStore::init(
        context.child("accounts"),
        store_config(&config.partition_prefix, "accounts", page_cache.clone(), ()),
    )
    .await
    .map_err(|e| BackendError::Storage(e.to_string()))?;

    let storage = StorageStore::init(/* ... */).await?;
    let code = CodeStore::init(/* ... */).await?;

    Ok(Stores { accounts, storage, code })
}
```

## Impact

1. **Performance**: Each `open_stores()` call involves 3 filesystem init operations (one per partition). At 33 blocks/s with multiple calls per block (build + verify paths), this adds hundreds of unnecessary filesystem operations per second. This is a significant contributor to I/O latency on the critical consensus path.

2. **Data integrity (TOCTOU)**: The `compute_root()` path opens dirty stores and applies changes. If another thread is concurrently writing to the same on-disk stores via the main `QmdbStore::apply_batches()` path, the root computation may incorporate partial writes from an unrelated block, producing an incorrect state root. This could cause a node to reject a valid block or accept an invalid one.

3. **Resource churn**: Each opened store allocates file handles, page cache entries, and potentially memory-mapped regions. Opening and immediately discarding stores on every call creates allocation churn and GC pressure.

4. **Amplified memory pressure**: On the devnet where 8/10 nodes are at their 4 GB memory limit, the transient allocations from repeated store openings exacerbate memory exhaustion.

## Root Cause

The `CommonwareRootProvider` was designed as a stateless utility that opens fresh stores on each call, likely to avoid cache invalidation complexity. However, this approach sacrifices both performance and correctness. The `CommonwareBackend` struct (lines 29-35) already holds persistent store handles, but the `RootProvider` was not designed to share them.

## Suggested Fix

Cache the opened stores inside `CommonwareRootProvider` behind an `Arc<RwLock<>>`:

```rust
pub struct CommonwareRootProvider {
    context: Context,
    config: QmdbBackendConfig,
    cached_stores: Arc<RwLock<Option<Stores>>>,
}

#[async_trait]
impl RootProvider for CommonwareRootProvider {
    async fn state_root(&self) -> Result<B256, HandleError> {
        let stores = self.get_or_open_stores().await?;
        let guard = stores.read();
        let s = guard.as_ref().unwrap();
        state_root_from_stores(&s.accounts, &s.storage, &s.code)
            .map_err(|e| HandleError::RootComputation(e.to_string()))
    }
}
```

Invalidate the cache only when QMDB commits new changes (after `apply_batches()`). This reduces the open-per-call pattern to open-once-reuse-many, eliminating hundreds of filesystem operations per second.

Alternatively, share the `CommonwareBackend`'s already-opened store handles with the `RootProvider` via `Arc`, avoiding the need for a separate cache entirely.

## Files to Modify

- `crates/storage/backend/src/backend.rs` -- Lines 37-41: Add cached store handles to `CommonwareRootProvider`
- `crates/storage/backend/src/backend.rs` -- Lines 163-195: Use cached stores in `state_root()` and `compute_root()`
- `crates/storage/backend/src/backend.rs` -- Lines 117-120: `root_provider()` should share store references

## Related Issues

- [001 -- Non-Atomic Cross-Partition QMDB Writes](./001-qmdb-non-atomic-cross-partition-writes.md) -- Related storage integrity concern; both involve the partition architecture
