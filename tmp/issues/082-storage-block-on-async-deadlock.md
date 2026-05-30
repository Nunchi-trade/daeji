# QmdbHandle DatabaseRef Uses futures::executor::block_on() Which Can Deadlock Under Tokio

**Category**: Storage / Reliability
**Severity**: Medium

## Summary

The `DatabaseRef` implementation for `QmdbHandle` in the storage adapter uses `futures::executor::block_on()` to bridge async QMDB operations into REVM's synchronous `DatabaseRef` trait. If this code path is accidentally called from within a Tokio async context, it will either panic ("Cannot start a runtime from within a runtime") or deadlock, because `futures::executor::block_on()` creates a nested single-threaded executor that conflicts with the Tokio runtime. A separate `QmdbRefDb` wrapper exists for Tokio contexts, but the raw `DatabaseRef for QmdbHandle` implementation is public and could be called from the wrong context.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs`, the `DatabaseRef` implementation for `QmdbHandle` (lines 83-132) uses a helper `block_on()` function (lines 79-81) that calls `futures::executor::block_on()`. This function creates a single-threaded executor to drive a future to completion. When called from within a Tokio runtime (e.g., from an async task, or from code holding a Tokio `Handle`), it creates a nested executor which is incompatible with Tokio's threading model.

The problem is compounded by the fact that the `QmdbHandle` internally uses `tokio::sync::RwLock` (via `self.read()` returning an `RwLockReadGuard`). This lock type requires a Tokio runtime context to function correctly -- its waiters are woken via the Tokio reactor. Under `futures::executor::block_on()`, the Tokio reactor may not be running, so lock waiters may never be woken, causing a permanent deadlock.

Note: The executor's `StateDbAdapter` (in `crates/node/executor/src/adapter.rs`) has a separate, correctly implemented `block_on()` helper that tries `tokio::task::block_in_place` + `Handle::block_on` first and only falls back to `futures::executor::block_on()` when no Tokio runtime is present. The storage adapter's `QmdbHandle` implementation does not have this safeguard.

## Code Reference

The problematic `block_on` helper:

```rust
// crates/storage/handlers/src/adapter.rs:76-81
/// Wrapper for blocking async operations in sync contexts.
///
/// This is used to bridge async QMDB operations into REVM's sync DatabaseRef trait.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    futures::executor::block_on(f)
}
```

The `DatabaseRef` implementation that uses it:

```rust
// crates/storage/handlers/src/adapter.rs:83-132
impl<A, S, C> DatabaseRef for QmdbHandle<A, S, C>
where
    A: QmdbGettable<Key = Address, Value = [u8; AccountEncoding::SIZE]>,
    S: QmdbGettable<Key = StorageKey, Value = U256>,
    C: QmdbGettable<Key = B256, Value = Vec<u8>>,
{
    type Error = HandleError;

    fn basic_ref(&self, address: Address) -> Result<Option<revm::state::AccountInfo>, Self::Error> {
        let store = block_on(self.read());    // acquires tokio::sync::RwLock
        match block_on(store.get_account(&address))? {
            // ...
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let store = block_on(self.read());    // acquires tokio::sync::RwLock
        let generation = match block_on(store.get_account(&address))? {
            Some((_, _, _, generation)) => generation,
            None => return Ok(U256::ZERO),
        };
        let key = StorageKey::new(address, generation, index);
        Ok(block_on(store.get_storage(&key))?.unwrap_or(U256::ZERO))
    }

    // ... code_by_hash_ref and block_hash_ref also use block_on
}
```

The safe alternative wrapper that already exists:

```rust
// crates/storage/handlers/src/adapter.rs:24-51
pub struct QmdbRefDb<A, S, C> {
    inner: Arc<WrapDatabaseAsync<QmdbHandle<A, S, C>>>,
}

impl<A, S, C> QmdbRefDb<A, S, C> {
    /// Wraps a QMDB handle with the current Tokio runtime.
    ///
    /// Returns `None` if no multi-threaded runtime is available.
    pub fn new(handle: QmdbHandle<A, S, C>) -> Option<Self> {
        WrapDatabaseAsync::new(handle).map(|wrapped| Self { inner: Arc::new(wrapped) })
    }
}
```

The `DatabaseCommit` implementation at line 250 also calls this `block_on`:

```rust
// crates/storage/handlers/src/adapter.rs:250
        if let Err(err) = block_on(Self::commit(self, changeset)) {
```

## Impact

- **Potential panic or deadlock**: If any caller invokes `DatabaseRef for QmdbHandle` methods from within a Tokio runtime, the node will either panic with "Cannot start a runtime from within a runtime" or deadlock. The `tokio::sync::RwLock` acquired under `futures::executor::block_on()` may never wake its waiters, causing a permanent hang.
- **Latent bug**: This may not manifest until a code path change causes the sync `DatabaseRef` on `QmdbHandle` to be called from an async context. The production code currently uses `QmdbRefDb` (the safe wrapper), so the bug is not currently triggered. But the `DatabaseRef for QmdbHandle` implementation is public and could be used by new code (e.g., tests, debugging tools, RPC handlers) without awareness of this constraint.
- **Silent correctness issue in DatabaseCommit**: The `DatabaseCommit::commit()` at line 250 also uses the same unsafe `block_on`, and it already has a comment noting it "swallows" errors. If this deadlocks, the node silently stops processing state updates.

## Root Cause

The `DatabaseRef` trait is synchronous, but QMDB operations are async. The bridging uses `futures::executor::block_on()` which creates a standalone single-threaded executor. This executor is incompatible with Tokio's runtime because: (1) it blocks the current thread while the Tokio scheduler may need that thread, and (2) `tokio::sync::RwLock` internally relies on the Tokio reactor to wake waiters, which is not running inside `futures::executor::block_on()`.

## Suggested Fix

**Option A (recommended)** -- Remove the `DatabaseRef for QmdbHandle` implementation entirely and force all callers through `QmdbRefDb`, which correctly uses `WrapDatabaseAsync` to integrate with the Tokio runtime. If the implementation must remain for non-Tokio test contexts, gate it behind `#[cfg(test)]`.

**Option B** -- Replace the `block_on` helper in `adapter.rs` with the same Tokio-aware pattern used in the executor's adapter:

```rust
// BEFORE (unsafe):
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    futures::executor::block_on(f)
}

// AFTER (Tokio-aware):
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current()
        && handle.runtime_flavor() == RuntimeFlavor::MultiThread
    {
        return tokio::task::block_in_place(|| handle.block_on(f));
    }
    futures::executor::block_on(f)
}
```

**Option C** -- Add a runtime guard that panics early with a clear error message when called from a Tokio context, preventing the silent deadlock:

```rust
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "QmdbHandle::DatabaseRef must not be used from async context; use QmdbRefDb instead"
    );
    futures::executor::block_on(f)
}
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (lines 79-81) -- replace or remove `block_on()` helper
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (lines 83-132) -- `DatabaseRef for QmdbHandle` implementation
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (line 250) -- `DatabaseCommit::commit()` also uses `block_on`

## Related Issues

- `086-executor-sequential-async-reads.md` -- the executor's adapter has a correctly implemented `block_on()` helper that should serve as the reference pattern

## Labels

bug, reliability, storage
