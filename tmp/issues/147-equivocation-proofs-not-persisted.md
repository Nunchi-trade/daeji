# 147: Equivocation Proofs Are Logged but Never Persisted or Acted Upon

**Category**: security
**Severity**: high
**Component**: consensus

## Summary

When a validator detects equivocation (a peer signing conflicting messages in the same consensus round), the Kora node logs a warning, increments an in-memory counter, and updates a Prometheus metric -- but never persists the cryptographic proof to disk, broadcasts it to other nodes, or uses it to exclude the misbehaving validator. If the detecting node restarts, all evidence of the equivocation is lost. A Byzantine validator can equivocate repeatedly without any lasting consequence.

## Problem

Commonware's simplex BFT consensus engine detects three types of equivocation and reports them to the application via the `Reporter` trait:

1. **ConflictingNotarize**: A validator signed two different notarizations for the same view.
2. **ConflictingFinalize**: A validator signed two different finalizations for the same view.
3. **NullifyFinalize**: A validator signed both a nullification and a finalization for the same view.

Each of these is a cryptographic proof of Byzantine behavior -- the proof contains two conflicting signed messages from the same validator, which can be independently verified by any party.

The `NodeStateReporter` in `crates/node/reporters/src/lib.rs` (lines 1604-1673) handles these events by:

1. Logging a `warn!` message with the signer and view
2. Calling `self.state.inc_equivocations()` -- an in-memory `AtomicU64` counter
3. Incrementing a Prometheus counter via `self.metrics.equivocations`

None of these actions persist the proof. On node restart, the counter resets to zero and the log entry may be rotated away. There is no mechanism to:

- Write the proof to an append-only file or database
- Broadcast the proof to other validators so they can also detect the misbehavior
- Use the proof to exclude the misbehaving validator from future consensus rounds
- Expose the proof via RPC for operator inspection or forensic analysis

**File**: `crates/node/reporters/src/lib.rs` (lines 1604-1673)

## Code Reference

```rust
// crates/node/reporters/src/lib.rs:1604-1673
impl<S> Reporter for NodeStateReporter<S>
where
    S: commonware_cryptography::certificate::Scheme + Clone + Send + 'static,
{
    type Activity = Activity<S, ConsensusDigest>;

    fn report(&mut self, activity: Self::Activity) -> Feedback {
        match &activity {
            // ... normal events ...
            Activity::ConflictingNotarize(proof) => {
                warn!(
                    signer = ?proof.signer(),
                    view = ?proof.view(),
                    "EQUIVOCATION: conflicting notarize detected"
                );
                self.state.inc_equivocations();
                if let Some(ref m) = self.metrics {
                    m.equivocations
                        .get_or_create(&EquivocationTypeLabel {
                            r#type: "conflicting_notarize".into(),
                        })
                        .inc();
                }
                // BUG: Proof is not persisted, broadcast, or acted upon
            }
            Activity::ConflictingFinalize(proof) => {
                warn!(
                    signer = ?proof.signer(),
                    view = ?proof.view(),
                    "EQUIVOCATION: conflicting finalize detected"
                );
                self.state.inc_equivocations();
                // ... same pattern: log + counter only ...
            }
            Activity::NullifyFinalize(proof) => {
                warn!(
                    signer = ?proof.signer(),
                    view = ?proof.view(),
                    "EQUIVOCATION: nullify-finalize conflict detected"
                );
                self.state.inc_equivocations();
                // ... same pattern: log + counter only ...
            }
            // ...
        }
        Feedback::Ok
    }
}
```

## Impact

1. **No accountability**: A Byzantine validator can equivocate in every round without any lasting record. The proof exists transiently in memory and is discarded on restart.
2. **No forensic analysis**: Operators investigating consensus anomalies cannot retrieve equivocation proofs after the fact. The only evidence is a log line that may have been rotated.
3. **No path to slashing**: Future slashing mechanisms require persistent, verifiable proofs of misbehavior. Without persisting proofs now, implementing slashing later requires retroactive changes.
4. **Undermines BFT guarantees**: Commonware's simplex BFT provides Byzantine fault tolerance, but the application layer (Kora) discards the very evidence that enables enforcement. This creates a gap between the theoretical security model and the deployed system.

## Root Cause

The `NodeStateReporter` treats equivocation as a logging event rather than a first-class data artifact. The `Feedback` enum returned from `report()` has no variant for "persist this proof" or "exclude this validator" -- persistence must be implemented by the application layer.

## Suggested Fix

1. **Persist proofs to disk**: Create an append-only file or database table for equivocation proofs, similar to the existing `SelfdestructGcLog` pattern used for tracking selfdestructed addresses.

```rust
// New struct: EquivocationLog
pub struct EquivocationLog {
    file: std::fs::File,
}

impl EquivocationLog {
    pub fn record(&self, proof: &EquivocationProof) {
        // Serialize and append to log file
    }
}
```

2. **Add an RPC endpoint**: Expose `eth_getEquivocationProofs` or a debug namespace method to query equivocation history.

3. **Track per-validator equivocation counts**: Maintain a persistent counter per validator public key, so operators can identify repeat offenders across restarts.

4. **(Future)** Integrate with a slashing or validator exclusion mechanism when key rotation (issue #94) is implemented.

## Files to Modify

- `crates/node/reporters/src/lib.rs` -- persist equivocation proofs in `NodeStateReporter::report()`
- `crates/node/reporters/src/gc_log.rs` -- (reference pattern) the `SelfdestructGcLog` shows how to implement append-only file logging
- `crates/rpc/src/lib.rs` -- (optional) add RPC endpoint for querying equivocation history

## Related Issues

None.

## Labels

`security`, `consensus`, `reliability`
