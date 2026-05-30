# 152: acknowledge_checkpoint Uses std::sync::Mutex in Async Context -- Cascading Panic on Poisoning

**Category**: bug
**Severity**: medium
**Component**: consensus

## Summary

The `pending_acks` vector in `FinalizedReporter` is protected by `Arc<std::sync::Mutex<Vec<Exact>>>` (note: `std::sync::Mutex`, not `tokio::sync::Mutex`). This mutex is locked inside async functions using `.lock().expect("pending_acks mutex poisoned")`. If the finalization task panics while holding this mutex, the mutex becomes poisoned, and all subsequent finalization attempts will panic with the "pending_acks mutex poisoned" message. Because the `FinalizedReporter::report()` method uses a fire-and-forget spawn pattern, the initial panic is not caught, and the cascading failures cause the node to crash-loop.

## Problem

In `crates/node/reporters/src/lib.rs`, the `pending_acks` field is declared as:

```rust
// crates/node/reporters/src/lib.rs:1316
pending_acks: Arc<Mutex<Vec<Exact>>>,
```

Where `Mutex` is `std::sync::Mutex` (imported at line 13: `use std::sync::{Arc, Mutex}`).

The `acknowledge_checkpoint()` function (lines 348-373) acquires this mutex with `.expect()`:

```rust
let mut guard = pending_acks.lock().expect("pending_acks mutex poisoned");
```

This is called from within `handle_finalized_update()`, which is spawned as a fire-and-forget background task in `FinalizedReporter::report()`:

```rust
self.context.child("report_task").spawn(move |_| async move {
    let _guard = finalize_lock.lock().await;
    handle_finalized_update(...).await;
});
Feedback::Ok
```

If `handle_finalized_update()` panics while `pending_acks` is locked (e.g., during a `std::mem::take()` or `guard.push(ack)` operation), two things happen:

1. The `std::sync::Mutex` becomes poisoned (its `PoisonError` flag is set).
2. The spawned task is dropped, and the panic is caught by the runtime but not propagated to the caller.

On the next finalization attempt, `pending_acks.lock().expect(...)` will panic immediately with "pending_acks mutex poisoned", causing another spawned task to panic. This creates a crash-loop where every finalization attempt fails.

Additionally, using `std::sync::Mutex` in an async context blocks the tokio runtime thread for the duration of the lock hold. While the critical section here is very short (`std::mem::take()` and `push()`), it is still a violation of the async best practice of not holding sync locks across `.await` points.

**File**: `crates/node/reporters/src/lib.rs` (lines 348-373, 1316, 1442-1461)

## Code Reference

```rust
// crates/node/reporters/src/lib.rs:13
use std::{
    fmt,
    marker::PhantomData,
    sync::{Arc, Mutex},  // <-- std::sync::Mutex, NOT tokio::sync::Mutex
    time::Duration,
};
```

```rust
// crates/node/reporters/src/lib.rs:1316
/// Marshal acknowledgements held until the next checkpoint boundary.
pending_acks: Arc<Mutex<Vec<Exact>>>,
```

```rust
// crates/node/reporters/src/lib.rs:348-373
async fn acknowledge_checkpoint(
    pending_acks: Arc<Mutex<Vec<Exact>>>,
    height: u64,
    checkpoint_interval: u64,
    ack: Exact,
) {
    let is_checkpoint = checkpoint_interval <= 1 || height.is_multiple_of(checkpoint_interval);
    if is_checkpoint {
        let pending = {
            let mut guard = pending_acks.lock().expect("pending_acks mutex poisoned");
            //                                  ^^^^^^^^
            //                   Will panic on all subsequent calls if mutex is poisoned
            std::mem::take(&mut *guard)
        };
        for pending_ack in pending {
            pending_ack.acknowledge();
        }
        ack.acknowledge();
    } else {
        let mut guard = pending_acks.lock().expect("pending_acks mutex poisoned");
        guard.push(ack);
    }
}
```

```rust
// crates/node/reporters/src/lib.rs:1442-1459
// Fire-and-forget spawn -- panics are not propagated:
self.context.child("report_task").spawn(move |_| async move {
    let _guard = finalize_lock.lock().await;
    handle_finalized_update(...).await;
    // If this panics while pending_acks is locked,
    // the mutex is poisoned and all future calls will panic
});
Feedback::Ok  // Returned before the task even starts
```

## Impact

1. **Cascading crash-loop**: If any finalization task panics while holding `pending_acks`, all subsequent finalization tasks will immediately panic with "pending_acks mutex poisoned". This makes the node unable to process finalized blocks.
2. **Silent initial failure**: The fire-and-forget spawn pattern means the initial panic goes unnoticed (no log, no error return). The first symptom is the cascading panic on the next finalization.
3. **Hard to diagnose**: The crash-loop log will show "pending_acks mutex poisoned" repeatedly, but the root cause (the original panic) may have already been lost from log buffers by the time an operator investigates.

## Root Cause

The code uses `std::sync::Mutex` (which supports poisoning) in a context where tasks can panic without being caught. The combination of a poisoning mutex with a fire-and-forget spawn pattern creates a cascading failure mode. `parking_lot::Mutex` does not support poisoning and would not have this issue.

## Suggested Fix

Replace `std::sync::Mutex` with `parking_lot::Mutex`, which does not poison:

```rust
// Before:
use std::sync::{Arc, Mutex};
pending_acks: Arc<Mutex<Vec<Exact>>>,
// ...
let mut guard = pending_acks.lock().expect("pending_acks mutex poisoned");

// After:
use parking_lot::Mutex;
pending_acks: Arc<Mutex<Vec<Exact>>>,
// ...
let mut guard = pending_acks.lock();  // No .expect() needed, cannot poison
```

`parking_lot::Mutex` is already a dependency in the project (used throughout the codebase, e.g., in `BlockIndex`). The critical section is very short (no `.await` points between lock and unlock), so `parking_lot::Mutex` is appropriate.

Alternatively, use `tokio::sync::Mutex` if future changes might add `.await` points within the critical section.

## Files to Modify

- `crates/node/reporters/src/lib.rs` -- replace `std::sync::Mutex` with `parking_lot::Mutex` for `pending_acks`

## Related Issues

- [082-storage-block-on-async-deadlock.md](./082-storage-block-on-async-deadlock.md) -- related async/sync mismatch at a different location
- [154-finalized-reporter-premature-ok.md](./154-finalized-reporter-premature-ok.md) -- the fire-and-forget spawn pattern that enables this bug

## Labels

`bug`, `reliability`, `consensus`
