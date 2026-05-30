# 143: QmdbLedger and StateDbWrite Have Duplicate Commit Paths with Different Lock Patterns

**Category**: bug
**Severity**: medium
**Component**: storage

## Summary

There are two independent code paths that commit state changes to the same underlying QMDB store: `QmdbLedger::commit_changes()` in `ledger.rs` and `StateDbWrite::commit()` in `state.rs`. Both acquire the same pair of locks (a `Mutex` for storage access and an `RwLock` for store write access), but they perform different post-commit actions (one computes the root from raw partition roots, the other delegates to a root provider). If both paths are ever invoked for the same block due to a logic error, the double-commit would corrupt state by applying the same changeset twice with different commit sequence numbers.

## Problem

**Path 1 -- `QmdbLedger::commit_changes()`** in `crates/storage/qmdb-ledger/src/ledger.rs` (line 112):

```rust
pub async fn commit_changes(&self, changes: QmdbChangeSet) -> Result<StateRoot, Error> {
    let _storage_access = self.handle.storage_access().await;  // Mutex
    let mut store = self.handle.write().await;                 // RwLock write
    store.commit_changes(changes).await.map_err(/* ... */)?;
    let stores = store.stores().map_err(/* ... */)?;
    let root = QmdbStateRoot::compute(
        B256::from_slice(stores.accounts.root()?.as_ref()),
        B256::from_slice(stores.storage.root()?.as_ref()),
        B256::from_slice(stores.code.root()?.as_ref()),
    );
    Ok(StateRoot(root))
}
```

**Path 2 -- `StateDbWrite::commit()`** in `crates/storage/handlers/src/state.rs` (line 98):

```rust
async fn commit(&self, changes: ChangeSet) -> Result<B256, StateDbError> {
    let _storage_access = self.storage_access().await;  // Mutex
    let mut store = self.write().await;                 // RwLock write
    store.commit_changes(changes).await.map_err(/* ... */)?;
    drop(store);
    if let Some(provider) = self.root_provider() {
        let mut provider = provider.write().await;
        provider.commit_and_get_root().await.map_err(/* ... */)
    } else {
        Ok(B256::ZERO)
    }
}
```

Both paths call `store.commit_changes()` on the same underlying `QmdbStore`, which increments the commit sequence counter and writes sentinel markers to all three partitions. If the same changeset is committed through both paths, the commit sequence advances twice (from N to N+2), and the partition sentinel markers will be written with two different sequence numbers. This is the exact inconsistency that the commit sequence mechanism was designed to detect.

**Files**: `crates/storage/qmdb-ledger/src/ledger.rs` (lines 112-127), `crates/storage/handlers/src/state.rs` (lines 98-114)

## Code Reference

```rust
// crates/storage/qmdb-ledger/src/ledger.rs:112-127
pub async fn commit_changes(&self, changes: QmdbChangeSet) -> Result<StateRoot, Error> {
    let _storage_access = self.handle.storage_access().await;
    let mut store = self.handle.write().await;
    store
        .commit_changes(changes)
        .await
        .map_err(|e| kora_traits::StateDbError::Storage(e.to_string()))?;
    let stores =
        store.stores().map_err(|e| kora_traits::StateDbError::Storage(e.to_string()))?;
    let root = QmdbStateRoot::compute(
        B256::from_slice(stores.accounts.root()?.as_ref()),
        B256::from_slice(stores.storage.root()?.as_ref()),
        B256::from_slice(stores.code.root()?.as_ref()),
    );
    Ok(StateRoot(root))
}
```

```rust
// crates/storage/handlers/src/state.rs:98-114
async fn commit(&self, changes: ChangeSet) -> Result<B256, StateDbError> {
    let _storage_access = self.storage_access().await;
    let mut store = self.write().await;
    store.commit_changes(changes).await.map_err(|e| StateDbError::Storage(e.to_string()))?;
    drop(store);

    if let Some(provider) = self.root_provider() {
        let mut provider = provider.write().await;
        provider
            .commit_and_get_root()
            .await
            .map_err(|e| StateDbError::RootComputation(e.to_string()))
    } else {
        Ok(B256::ZERO)
    }
}
```

## Impact

1. **Maintenance hazard**: Two commit paths for the same store creates confusion about which to use. A developer could accidentally call both, causing a double-commit that corrupts the commit sequence and potentially the state.
2. **Different post-commit behavior**: Path 1 reads roots directly from partition stores; Path 2 delegates to a root provider. These may return different root values for the same committed state, leading to divergent state root computations.
3. **Unnecessary code duplication**: The lock acquisition pattern is duplicated, which means lock ordering changes in one path may not be reflected in the other.

Currently, the production finalization path in `crates/node/ledger/src/lib.rs` uses `QmdbLedger::commit_changes()` (Path 1), so Path 2 is not invoked for finalized blocks in practice. However, the existence of both paths is a latent correctness risk.

## Root Cause

The two commit paths were developed independently as part of different abstractions (`QmdbLedger` for the ledger layer and `StateDbWrite` for the trait-based state access layer). Neither delegates to the other.

## Suggested Fix

Consolidate to a single commit path:

1. Remove `QmdbLedger::commit_changes()` and make the ledger call `StateDbWrite::commit()` instead.
2. Or, make `StateDbWrite::commit()` delegate to `QmdbLedger::commit_changes()`.
3. Ensure there is exactly one code path that acquires the locks and calls `store.commit_changes()`.

## Files to Modify

- `crates/storage/qmdb-ledger/src/ledger.rs` -- remove or delegate `commit_changes()`
- `crates/storage/handlers/src/state.rs` -- ensure this is the single commit path, or delegate to ledger
- `crates/node/ledger/src/lib.rs` -- update callers to use the surviving path

## Related Issues

None.

## Labels

`bug`, `reliability`, `storage`
