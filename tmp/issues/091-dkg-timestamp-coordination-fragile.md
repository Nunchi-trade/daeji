# DKG Ceremony Timestamp Coordination Is Fragile Near 5-Minute Boundaries

**Category**: dkg, consensus
**Severity**: medium

**Labels**: `bug`, `reliability`, `dkg`, `good first issue`

## Summary

The interactive DKG (Distributed Key Generation) ceremony derives a session identifier by hashing a timestamp that is rounded down to the nearest 5-minute boundary. Each participant computes this timestamp independently from its own system clock. If participants start near a 5-minute boundary, even small clock skew or startup timing differences cause them to compute different timestamps, which produces different ceremony IDs. Messages with mismatched ceremony IDs are rejected, causing the entire ceremony to time out and fail.

## Problem

Kora's DKG ceremony is the process by which validators jointly generate a threshold signing key. All validators must agree on a single `CeremonySession` -- identified by a 32-byte `ceremony_id` -- so that messages from one ceremony are not accepted in another. The `ceremony_id` is derived from a hash of the chain ID, sorted participant public keys, and a timestamp. The timestamp is the source of fragility.

### Timestamp computation (ceremony.rs:78-82)

Each participant independently rounds the current wall-clock time down to the nearest 5-minute interval:

```rust
// crates/node/dkg/src/ceremony.rs:78-82
let timestamp_nanos = {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
    let interval = 5 * 60 * 1_000_000_000u64; // 5 minutes in nanos
    (now / interval) * interval
};
```

### Timestamp hashed into ceremony ID (protocol.rs:40-57)

The timestamp flows into the ceremony ID via `CeremonySession::new()`:

```rust
// crates/node/dkg/src/protocol.rs:40-57
pub fn new(chain_id: u64, participants: &[ed25519::PublicKey], timestamp_nanos: u64) -> Self {
    let mut hasher = Sha256::default();
    hasher.update(b"kora-dkg-ceremony-v1");
    hasher.update(&chain_id.to_le_bytes());
    hasher.update(&(participants.len() as u64).to_le_bytes());
    let mut sorted_participants = participants.to_vec();
    sorted_participants.sort_by(|a, b| a.as_ref().cmp(b.as_ref()));
    for pk in &sorted_participants {
        hasher.update(pk.as_ref());
    }
    hasher.update(&timestamp_nanos.to_le_bytes()); // <-- timestamp goes into hash
    // ...
}
```

### Session ID validation (protocol.rs:504-517)

Messages with mismatched session IDs are rejected:

```rust
// crates/node/dkg/src/protocol.rs:504-517
match &msg.session_id {
    Some(session_id) => {
        if *session_id != self.session.ceremony_id {
            warn!(
                ?from,
                expected = hex::encode(self.session.ceremony_id),
                received = hex::encode(session_id),
                "Rejecting message with mismatched session ID"
            );
            return Err(DkgError::SessionMismatch {
                expected: hex::encode(self.session.ceremony_id),
                received: hex::encode(session_id),
            });
        }
    }
    // ...
}
```

### Hardcoded peer wait (ceremony.rs:397-408)

Before the ceremony begins, participants wait a hardcoded 10 seconds for peers, with no actual readiness detection:

```rust
// crates/node/dkg/src/ceremony.rs:397-408
async fn wait_for_peers(&self, _network: &DkgNetwork) -> Result<(), DkgError> {
    info!("Waiting for peers to be ready...");
    let start = Instant::now();
    // Give all DKG containers time to join the p2p overlay before the phase-1
    // broadcasts, which are not replayed if sent before peers are reachable.
    tokio::time::sleep(Duration::from_secs(10)).await;
    info!(elapsed = ?start.elapsed(), "Peer initialization complete");
    Ok(())
}
```

The code acknowledges this limitation in a comment at line 76-77:

```rust
// In production, this should be coordinated via the leader or a shared clock.
// For now, we round down to 5-minute intervals for coordination tolerance.
```

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs`, lines 75-82 (timestamp rounding)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs`, lines 40-57 (ceremony ID hash)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs`, lines 504-517 (session mismatch rejection)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs`, lines 397-408 (hardcoded wait)

## Impact

- **Non-deterministic ceremony failures**: If Node A starts at `12:04:59.999` (rounds to `12:00:00`) and Node B starts at `12:05:00.001` (rounds to `12:05:00`), they compute different ceremony IDs. Node A rejects Node B's messages and vice versa, causing the ceremony to time out after 120 seconds.
- **Manual operator intervention**: After a timeout, the entire ceremony must be restarted manually. There is no automated retry.
- **Denial-of-service vector**: An attacker who can influence the startup timing of even one validator can force repeated ceremony failures by ensuring participants straddle a 5-minute boundary.
- **Impractical coordination**: Operators must coordinate startup times to avoid boundaries, which becomes increasingly difficult with more validators and across different time zones.

## Root Cause

The timestamp is computed independently by each participant using `SystemTime::now()` with no coordination mechanism. Different participants can observe different 5-minute intervals if they start near a boundary. The 10-second hardcoded `wait_for_peers` sleep does not help because it only delays the start -- it does not synchronize the timestamp across participants.

## Suggested Fix

**Option A (recommended): Leader-coordinated timestamp**

Have the leader broadcast the timestamp as part of a "ceremony start" message. Non-leaders wait for this message before computing their `CeremonySession`.

1. Add a new `ProtocolMessageKind::CeremonyStart { timestamp_nanos: u64 }` variant
2. Leader computes timestamp and broadcasts it
3. Non-leaders receive the timestamp and use it for their `CeremonySession::new()` call
4. Timeout if the leader's message is not received within a configurable window

**Option B: Wider rounding with grace period**

1. Round to a wider interval (e.g., 15 or 30 minutes)
2. Add a grace period: if the current time is within 30 seconds of a boundary, wait until the next interval begins
3. Add a retry loop: if `SessionMismatch` errors are received, re-derive the timestamp and restart

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` -- Timestamp rounding logic (lines 75-82), `wait_for_peers` (lines 397-408)
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- `CeremonySession::new` hash inputs (lines 40-57), add `CeremonyStart` variant to `ProtocolMessageKind` (around line 96), session validation (lines 504-517)

## Related Issues

- `103-dkg-resharing-tracking.md` -- DKG architecture overhaul (resharing would also need coordinated session IDs)
