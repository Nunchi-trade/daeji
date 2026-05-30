# SeedReporter::report Spawns Unbounded Detached Tokio Tasks

**Category**: Consensus / Resource Leak
**Severity**: Medium
**Labels**: `bug`, `performance`, `reliability`, `consensus`

## Summary

Every consensus activity event (notarization, finalization, equivocation proof, etc.) causes `SeedReporter::report()` to spawn a new, detached Tokio task. At Kora's production throughput of 30+ blocks/second with multiple activity types per block, this creates 100+ detached tasks per second with no backpressure. If the tasks contend on the ledger mutex, they pile up unboundedly, increasing memory pressure and mutex contention.

## Problem

The `SeedReporter::report()` method in `crates/node/reporters/src/lib.rs` (lines 226-232) spawns a new `tokio::spawn` for every incoming activity event. The `report()` method is called synchronously by the commonware simplex engine and must return `Feedback::Ok` immediately, so spawning is necessary. However, there is no mechanism to limit the number of concurrently running or queued tasks.

Each spawned task calls `seed_report_inner()`, which for `Notarization` and `Finalization` events calls `state.set_seed(...).await`. The `set_seed` method acquires a mutex inside `LedgerService`. When many tasks are spawned concurrently, they all contend on this mutex, causing pile-up.

## Code Reference

`crates/node/reporters/src/lib.rs` lines 220-233:

```rust
impl<V> Reporter for SeedReporter<V>
where
    V: Variant,
{
    type Activity = Activity<Scheme<PublicKey, V>, ConsensusDigest>;

    fn report(&mut self, activity: Self::Activity) -> Feedback {
        let state = self.state.clone();
        ::tokio::spawn(async move {
            seed_report_inner(state, activity).await;
        });
        Feedback::Ok
    }
}
```

`crates/node/reporters/src/lib.rs` lines 143-191 (`seed_report_inner`):

```rust
async fn seed_report_inner<V: Variant>(
    state: LedgerService,
    activity: Activity<Scheme<PublicKey, V>, ConsensusDigest>,
) {
    match activity {
        Activity::Notarization(notarization) => {
            state
                .set_seed(
                    notarization.proposal.payload,
                    SeedReporter::<V>::hash_seed(notarization.seed()),
                )
                .await;
        }
        Activity::Finalization(finalization) => {
            state
                .set_seed(
                    finalization.proposal.payload,
                    SeedReporter::<V>::hash_seed(finalization.seed()),
                )
                .await;
        }
        // Equivocation events only log warnings
        Activity::ConflictingNotarize(ref proof) => { /* warn! */ }
        Activity::ConflictingFinalize(ref proof) => { /* warn! */ }
        Activity::NullifyFinalize(ref proof) => { /* warn! */ }
        // Normal per-vote events are no-ops
        Activity::Notarize(_) | Activity::Certification(_) | Activity::Nullify(_)
        | Activity::Nullification(_) | Activity::Finalize(_) => {}
    }
}
```

## Impact

- **Memory growth under load**: At 30 blocks/s with multiple activity events per block (notarize, notarization, finalize, finalization, certification, etc.), 100+ tasks are spawned per second. If `set_seed` takes longer than the inter-event interval (due to mutex contention or other lock holders), tasks accumulate without bound. Each queued task holds a cloned `LedgerService` (which contains `Arc<Mutex<...>>`).
- **Mutex convoy**: Many tasks waiting on the same ledger mutex create a convoy effect, increasing tail latency for seed updates. This can delay `get_prevrandao` for block proposals.
- **No error visibility**: The detached `tokio::spawn` handles are dropped immediately (`let _ = ...` is implicit). If `seed_report_inner` panics, the panic is silently caught by the Tokio runtime with no visibility to the operator.

## Root Cause

The `Reporter` trait's `report()` method is synchronous (returns `Feedback` immediately), which necessitates spawning. However, the current implementation uses unbounded `tokio::spawn` with no concurrency limit, no channel-based serialization, and no error propagation.

## Suggested Fix

Replace the unbounded `tokio::spawn` pattern with a bounded MPSC channel feeding a single worker task. This serializes seed updates (which is semantically correct since they update the same state), provides natural backpressure, and eliminates the unbounded task spawning.

Before:
```rust
fn report(&mut self, activity: Self::Activity) -> Feedback {
    let state = self.state.clone();
    ::tokio::spawn(async move {
        seed_report_inner(state, activity).await;
    });
    Feedback::Ok
}
```

After:
```rust
fn report(&mut self, activity: Self::Activity) -> Feedback {
    if self.tx.try_send(activity).is_err() {
        warn!("seed reporter channel full, dropping activity event");
    }
    Feedback::Ok
}
```

With a background worker spawned once during construction:
```rust
// In SeedReporter::new():
let (tx, mut rx) = tokio::sync::mpsc::channel(256);
tokio::spawn(async move {
    while let Some(activity) = rx.recv().await {
        seed_report_inner(state.clone(), activity).await;
    }
});
```

## Files to Modify

- `crates/node/reporters/src/lib.rs` -- `SeedReporter` struct (add channel sender field) and `report()` method (lines 226-232)

## Related Issues

- `016-seed-tracker-unbounded.md` -- both involve seed-related resource growth
- `150-finalize-lock-starves-proposal.md` -- related mutex contention in the finalization path
