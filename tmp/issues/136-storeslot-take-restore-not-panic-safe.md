# StoreSlot Take/Restore Pattern Is Not Panic-Safe

## Category
bug -- storage / reliability

## Severity
high

## Summary
The `StoreSlot<T>` type implements a take/restore ownership pattern used by all three QMDB partition stores (`AccountStore`, `StorageStore`, `CodeStore`) during batch writes. If any operation between `take()` and `restore()` panics (e.g., OOM during Merkle computation, assertion failure in commonware-storage), the store is left in a permanent `None` state. All subsequent operations will fail with `NotInitialized`, and the node cannot recover without a full process restart.

## Problem
`StoreSlot<T>` is a simple `Option<T>` wrapper that temporarily moves ownership out of the slot during batch writes. The `write_batch()` implementations in all three store types follow the same pattern:

1. `take()` -- moves the inner database out, leaving `None`
2. Perform merkleize, apply_batch, commit (multiple fallible operations)
3. `restore()` -- puts the inner database back

If any step between take and restore panics or propagates a panic (via `.unwrap()`, allocation failure, or any other panic source in the called code), the `restore()` call is never reached, leaving the `StoreSlot` permanently empty.

**File:** `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/types.rs`, lines 106-128 (StoreSlot definition):
```rust
pub(crate) struct StoreSlot<T>(Option<T>);

impl<T> StoreSlot<T> {
    pub(crate) const fn new(inner: T) -> Self {
        Self(Some(inner))
    }

    pub(crate) fn get(&self) -> Result<&T, BackendError> {
        self.0.as_ref().ok_or(BackendError::NotInitialized)
    }

    pub(crate) fn take(&mut self) -> Result<T, BackendError> {
        self.0.take().ok_or(BackendError::NotInitialized)
    }

    pub(crate) fn restore(&mut self, inner: T) {
        self.0 = Some(inner);
    }

    pub(crate) fn into_inner(self) -> Result<T, BackendError> {
        self.0.ok_or(BackendError::NotInitialized)
    }
}
```

**File:** `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/accounts.rs`, lines 88-107 (AccountStore::write_batch):
```rust
impl QmdbBatchable for AccountStore {
    async fn write_batch<I>(&mut self, ops: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = (Self::Key, Option<Self::Value>)> + Send,
        I::IntoIter: Send,
    {
        let inner = self.inner.take()?;           // Step 1: take
        let mut batch = inner.new_batch();
        for (address, value) in ops {
            batch = batch.write(account_key(address), value.map(AccountValue));
        }
        let merkleized = batch
            .merkleize(&inner, None)               // Can panic on OOM
            .await
            .map_err(|e| BackendError::Storage(e.to_string()))?;
        let mut inner = inner;
        inner.apply_batch(merkleized).await        // I/O operation, can panic
            .map_err(|e| BackendError::Storage(e.to_string()))?;
        inner.commit().await                       // I/O operation, can panic
            .map_err(|e| BackendError::Storage(e.to_string()))?;
        self.inner.restore(inner);                 // Step 3: restore (skipped on panic)
        Ok(())
    }
}
```

The identical pattern appears in:
- **StorageStore:** `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/storage.rs`, lines 87-107
- **CodeStore:** `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/code.rs`, lines 79-99

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/storage/backend/src/accounts.rs`, lines 93-105:
```rust
let inner = self.inner.take()?;
let mut batch = inner.new_batch();
for (address, value) in ops {
    batch = batch.write(account_key(address), value.map(AccountValue));
}
let merkleized = batch
    .merkleize(&inner, None)
    .await
    .map_err(|e| BackendError::Storage(e.to_string()))?;
let mut inner = inner;
inner.apply_batch(merkleized).await.map_err(|e| BackendError::Storage(e.to_string()))?;
inner.commit().await.map_err(|e| BackendError::Storage(e.to_string()))?;
self.inner.restore(inner);
```

## Impact
- **Permanent store failure:** A single panic during any batch write permanently bricks the affected store (accounts, storage, or code) for the remainder of the process lifetime. Every subsequent `get()`, `take()`, or `root()` call returns `BackendError::NotInitialized`.
- **Cascade to node failure:** Since all three stores are required for block execution and finalization, a bricked store means the node can no longer execute blocks, persist state, or produce proposals. The node becomes a non-participant in consensus.
- **No recovery without restart:** The only recovery path is a full process restart, which re-opens the stores from disk. In a Docker deployment, this requires the container to crash and be restarted by the orchestrator, but the health endpoint (issue 128) will not detect the failure.
- **Realistic panic sources:** `merkleize()`, `apply_batch()`, and `commit()` are I/O-bound operations that call into commonware-storage. Any `unwrap()`, assertion failure, or allocation failure in that code path would trigger a panic. While Rust async code has limited panic propagation, `catch_unwind` boundaries are not universally used.

## Root Cause
The take/restore pattern temporarily moves ownership out of the `StoreSlot` wrapper, creating a `None` gap. There is no RAII guard or `Drop`-based safety net to ensure the value is restored on unwind. This is a well-known anti-pattern in Rust when the gap between take and restore contains fallible or panicking code.

## Suggested Fix
**Option 1 (preferred): Use a RAII guard pattern** similar to `MutexGuard`:

```rust
struct StoreGuard<'a, T> {
    slot: &'a mut StoreSlot<T>,
    inner: Option<T>,
}

impl<'a, T> StoreGuard<'a, T> {
    fn new(slot: &'a mut StoreSlot<T>) -> Result<Self, BackendError> {
        let inner = slot.take()?;
        Ok(Self { slot, inner: Some(inner) })
    }

    fn as_ref(&self) -> &T {
        self.inner.as_ref().unwrap()
    }

    fn as_mut(&mut self) -> &mut T {
        self.inner.as_mut().unwrap()
    }
}

impl<T> Drop for StoreGuard<'_, T> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            self.slot.restore(inner);
        }
    }
}
```

Usage in `write_batch()`:
```rust
let mut guard = StoreGuard::new(&mut self.inner)?;
let batch = guard.as_ref().new_batch();
// ... merkleize, apply_batch, commit using guard.as_mut() ...
// guard is dropped at scope exit, restoring the inner value even on panic
```

**Option 2: Use `&mut` access instead of take/restore.** If the commonware-storage API allows `&mut` borrows for batch operations (rather than owned values), the take/restore pattern can be eliminated entirely.

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/types.rs` -- add `StoreGuard` RAII type
- `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/accounts.rs` -- rewrite `write_batch()` to use `StoreGuard`
- `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/storage.rs` -- rewrite `write_batch()` to use `StoreGuard`
- `/Users/will/dev/nunchi/daeji/crates/storage/backend/src/code.rs` -- rewrite `write_batch()` to use `StoreGuard`

## Related Issues
- `022-database-commit-swallows-errors.md` -- error handling during commit operations; a swallowed error could mask the conditions leading to a panic in the take/restore gap
- `020-qmdb-persistence-blocks-finalization.md` -- QMDB persistence issues that interact with the batch write path

## Labels
bug, reliability, storage, correctness
