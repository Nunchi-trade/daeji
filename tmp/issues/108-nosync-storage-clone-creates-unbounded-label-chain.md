# NoSyncStorage::clone Creates Unbounded Context Label Chain

**Category**: Infrastructure / Resource Leak
**Severity**: Low
**Labels**: `bug`, `performance`, `consensus`

## Summary

The `Clone` implementation for `NoSyncStorage` calls `self.inner.child("nosync_storage")` instead of cloning the inner context directly. Every clone appends a new `nosync_storage` segment to the context's label chain, producing progressively longer tracing span names like `scratch.nosync_storage.nosync_storage.nosync_storage...`. This wastes memory and makes log output increasingly unreadable.

## Problem

`NoSyncStorage` wraps a commonware context (implementing `Supervisor`) and is cloned by the marshal and other consensus subsystems. In commonware, `context.child("label")` creates a new child context whose tracing/metrics label is the parent's label with the new label appended. The `Clone` implementation for `NoSyncStorage` calls `child()` on every clone, so each successive clone extends the label chain by one segment.

The `NoSyncStorage` type is used as the storage context for the marshal (`Inline`), which may clone it internally when spawning sub-tasks. Each clone lengthens the label string permanently because contexts carry their full ancestry path.

## Code Reference

`crates/node/runner/src/no_sync_storage.rs` lines 43-54:

```rust
impl<C> Clone for NoSyncStorage<C>
where
    C: Supervisor,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.child("nosync_storage"),
            partitions: self.partitions.clone(),
            checkpoint_interval: self.checkpoint_interval,
        }
    }
}
```

The initial creation uses `child("scratch")` in `crates/node/runner/src/runner.rs` line 1368:

```rust
let scratch_context = NoSyncStorage::new(context.child("scratch"), checkpoint_interval);
```

After N clones, the label becomes: `scratch.nosync_storage.nosync_storage...nosync_storage` (N repetitions).

## Impact

- **Log noise**: Tracing output contains progressively longer, repetitive span names, making it difficult to read and filter logs. In a long-running system with many clones, labels can grow to hundreds of characters.
- **Memory waste**: Each clone allocates a new `String` for the label that includes all ancestor labels. These strings are never freed as long as the context is alive. The waste is proportional to `O(N^2)` where N is the number of clones in a chain.
- **Not a correctness issue**: The behavior does not affect consensus correctness, block production, or state integrity. It is purely an efficiency and observability concern.

## Root Cause

The `Clone` trait implementation calls `self.inner.child("nosync_storage")` instead of `self.inner.clone()`. The `child()` method is intended for creating named sub-contexts (e.g., "engine", "resolver"), not for duplicating a context at the same level.

## Suggested Fix

Use `self.inner.clone()` instead of `self.inner.child("nosync_storage")` in the `Clone` implementation.

Before:
```rust
fn clone(&self) -> Self {
    Self {
        inner: self.inner.child("nosync_storage"),
        partitions: self.partitions.clone(),
        checkpoint_interval: self.checkpoint_interval,
    }
}
```

After:
```rust
fn clone(&self) -> Self {
    Self {
        inner: self.inner.clone(),
        partitions: self.partitions.clone(),
        checkpoint_interval: self.checkpoint_interval,
    }
}
```

Note: This requires that the inner context type `C: Supervisor` also implements `Clone`. The commonware `tokio::Context` type implements `Clone` (it clones the `Arc`-wrapped internals), so this should work. If `Clone` is not available on the `Supervisor` bound, add a `Clone` bound to the `where` clause.

## Files to Modify

- `crates/node/runner/src/no_sync_storage.rs` -- `Clone` implementation (lines 43-54)

## Related Issues

None directly related.
