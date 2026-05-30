# 022: DatabaseCommit Swallows QMDB Write Errors During Block Execution

**Category**: bug
**Severity**: high
**Labels**: bug, storage, reliability, correctness

---

## Summary

The REVM `DatabaseCommit` trait's `commit()` method returns `()`, which makes it impossible to propagate QMDB write errors through the standard interface. When a per-transaction commit fails (e.g., due to a disk I/O error), the error is logged and an `AtomicBool` flag is set, but remaining transactions in the block continue executing against potentially stale or inconsistent in-memory state. The flag is only checked after the entire block has finished executing.

---

## Problem

During block execution, each transaction's state changes are committed to the QMDB database via REVM's `DatabaseCommit::commit()` trait method. This method has a return type of `()`, so it cannot return errors. The Kora implementation works around this by:

1. Logging the error at `error!` level (line 251-255 of `adapter.rs`)
2. Setting an `AtomicBool` flag called `commit_failed` (line 256 of `adapter.rs`, defined at line 47 of `qmdb.rs`)
3. Checking that flag after the entire block has executed (lines 469-474 of `revm.rs`)

The problem is that between the failed commit and the post-block check, all remaining transactions in the block continue to execute against the database state. Since the failed commit may have left the in-memory state inconsistent (partial writes), subsequent transaction results (receipts, gas accounting) may be incorrect.

**File**: `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs`

---

## Code Reference

The `DatabaseCommit` implementation that swallows errors (lines 205-258 of `adapter.rs`):

```rust
// crates/storage/handlers/src/adapter.rs:205-258
impl<A, S, C> DatabaseCommit for QmdbHandle<A, S, C>
where
    A: QmdbGettable<Key = Address, Value = [u8; AccountEncoding::SIZE]>
        + QmdbBatchable<Key = Address, Value = [u8; AccountEncoding::SIZE]>,
    S: QmdbGettable<Key = StorageKey, Value = U256> + QmdbBatchable<Key = StorageKey, Value = U256>,
    C: QmdbGettable<Key = B256, Value = Vec<u8>> + QmdbBatchable<Key = B256, Value = Vec<u8>>,
{
    fn commit(&mut self, changes: AddressMap<Account>) {
        // ... builds changeset ...
        // REVM's `DatabaseCommit::commit` returns `()`, so we cannot propagate
        // errors through the return type.  Instead we log at error level and
        // set an atomic flag that callers can check after execution.
        if let Err(err) = block_on(Self::commit(self, changeset)) {
            error!(
                %err,
                "CRITICAL: DatabaseCommit failed — QMDB write error swallowed by infallible \
                 REVM trait. Subsequent transactions in this block may execute against stale state."
            );
            self.mark_commit_failed();
        }
    }
}
```

The `commit_failed` flag definition (lines 41-48 of `qmdb.rs`):

```rust
// crates/storage/handlers/src/qmdb.rs:41-48
pub struct QmdbHandle<A, S, C> {
    inner: Arc<RwLock<QmdbStore<A, S, C>>>,
    root_provider: Option<Arc<RwLock<dyn RootProvider>>>,
    storage_access: Arc<Mutex<()>>,
    /// Flag set when a DatabaseCommit::commit call fails.
    commit_failed: Arc<AtomicBool>,
}
```

The post-block check that occurs too late (lines 469-474 of `revm.rs`):

```rust
// crates/node/executor/src/revm.rs:469-474
// Check the side-channel flag for DatabaseCommit failures.
// REVM's DatabaseCommit::commit() is infallible, so QMDB write errors
// are recorded via an atomic flag on the state handle and checked here.
if state.take_commit_failure() {
    return Err(ExecutionError::StateCommit);
}
```

The per-transaction commit call (line 462 of `revm.rs`) where the check should occur:

```rust
// crates/node/executor/src/revm.rs:461-463
let changes = extract_changes(&evm_state);
evm.ctx.modify_db(|db| db.commit(evm_state));
outcome.changes.merge(changes);
```

---

## Impact

If a QMDB write fails mid-block (due to disk full, I/O error, or corruption):

1. **Incorrect receipts**: The failing transaction's state changes may be partially applied. Subsequent transactions execute against this inconsistent state, producing incorrect receipts with wrong gas accounting, incorrect balance changes, and wrong event logs.
2. **Wasted computation**: All remaining transactions in the block are executed unnecessarily, consuming CPU time, even though the block will ultimately be rejected.
3. **Block rejection**: The block IS correctly rejected after execution when `take_commit_failure()` returns `true`, so this does not cause consensus divergence. However, the computed receipts are invalid.
4. **In-memory state pollution**: The `QmdbHandle`'s in-memory state retains the partial writes from the failed commit and subsequent transaction executions, which may affect the next block's execution until the handle is refreshed.

---

## Root Cause

The REVM `DatabaseCommit` trait defines `commit(&mut self, changes) -> ()`, making error propagation impossible through the standard interface. The `AtomicBool` side-channel is a workaround, but it is only checked after the entire block has executed rather than after each individual transaction commit.

---

## Suggested Fix

**Short-circuit on failure**: Check `take_commit_failure()` immediately after each `db.commit()` call inside the per-transaction loop, aborting the block before executing further transactions.

**Before** (lines 461-463 of `revm.rs`):
```rust
let changes = extract_changes(&evm_state);
evm.ctx.modify_db(|db| db.commit(evm_state));
outcome.changes.merge(changes);
```

**After**:
```rust
let changes = extract_changes(&evm_state);
evm.ctx.modify_db(|db| db.commit(evm_state));
// Short-circuit: if the commit failed, abort the block immediately
// rather than executing more transactions against inconsistent state.
if state.take_commit_failure() {
    return Err(ExecutionError::StateCommit);
}
outcome.changes.merge(changes);
```

Additionally, in `adapter.rs` lines 250-257, capture the specific failing account address in the error log so the context is not lost:

```rust
if let Err(err) = block_on(Self::commit(self, changeset)) {
    let affected_addresses: Vec<_> = changes.keys().take(5).collect();
    error!(
        %err,
        ?affected_addresses,
        "CRITICAL: DatabaseCommit failed"
    );
    self.mark_commit_failed();
}
```

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` -- Add early `take_commit_failure()` check after each `db.commit()` call (around line 462)
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` -- Enhance error logging with affected addresses (around line 250)

---

## Related Issues

- `030-stale-storage-slots-unbounded-disk.md` (storage layer correctness)
