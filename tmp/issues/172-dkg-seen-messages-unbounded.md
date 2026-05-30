# DKG seen_messages HashSet Grows Unbounded

**Category**: reliability
**Severity**: medium
**Labels**: `reliability`, `dkg`, `performance`

## Summary

The `DkgParticipant` struct maintains a `seen_messages: HashSet<[u8; 32]>` for message deduplication, but this set is never pruned, cleared between phases, or capped in size. During normal operation with a small number of participants, the set remains small (50-100 entries). However, under adversarial conditions where an attacker floods the DKG listener with unique messages, each message's SHA256 hash is inserted unconditionally, and the set grows without bound until the ceremony ends or the process runs out of memory.

## Problem

In `crates/node/dkg/src/protocol.rs`, the `DkgParticipant` struct declares a `seen_messages` field at line 304:

```rust
    /// Set of message hashes we've already processed (for deduplication).
    seen_messages: HashSet<[u8; 32]>,
```

The `handle_message_bytes()` method at lines 490-529 checks this set for deduplication and inserts new hashes:

```rust
    pub fn handle_message_bytes(
        &mut self,
        from: &ed25519::PublicKey,
        bytes: &[u8],
    ) -> Result<(), DkgError> {
        let message_hash = compute_message_hash(bytes);
        if self.seen_messages.contains(&message_hash) {
            debug!(?from, "Rejecting duplicate message");
            return Ok(());
        }
        // ... session verification ...
        self.seen_messages.insert(message_hash);  // <-- no size cap, never pruned
        self.handle_message(from, msg)
    }
```

The set is initialized as empty in `DkgParticipant::new()` at line 396 and is never cleared or pruned anywhere in the codebase. There is no maximum size check before insertion.

The ceremony runner at `ceremony.rs` calls `receive_and_process()` in tight retry loops (Phase 2 at lines 191-228, Phase 4 at lines 334-374), processing all incoming messages on each iteration. While the set provides deduplication (preventing the same message from being processed twice), it accumulates hashes from all messages across all phases.

## Code Reference

Declaration in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (line 304):

```rust
    /// Set of message hashes we've already processed (for deduplication).
    seen_messages: HashSet<[u8; 32]>,
```

Insertion without bounds in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 490-529):

```rust
    pub fn handle_message_bytes(
        &mut self,
        from: &ed25519::PublicKey,
        bytes: &[u8],
    ) -> Result<(), DkgError> {
        let message_hash = compute_message_hash(bytes);
        if self.seen_messages.contains(&message_hash) {
            debug!(?from, "Rejecting duplicate message");
            return Ok(());
        }

        let msg = ProtocolMessage::from_bytes(bytes, self.config.n() as u32)
            .map_err(|e| DkgError::InvalidMessage(format!("Failed to decode: {:?}", e)))?;

        match &msg.session_id {
            Some(session_id) => {
                if *session_id != self.session.ceremony_id {
                    // ... reject mismatched session ...
                    return Err(DkgError::SessionMismatch { ... });
                }
            }
            None => {
                warn!(?from, "Received legacy message without session ID ...");
            }
        }

        self.seen_messages.insert(message_hash);
        self.handle_message(from, msg)
    }
```

Initialization in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (line 396):

```rust
            seen_messages: HashSet::new(),
```

## Impact

**Minor memory pressure under normal operation; potential memory exhaustion under adversarial conditions.**

Normal operation (10-node DKG):
- Phase 1: ~10 DealerPublic + ~90 DealerPrivate + self-messages = ~100 messages
- Phase 2: ~90 PlayerAck messages
- Phase 3: ~10 DealerLog messages
- Phase 4: ~10 RequestLogs + ~10 AllLogs + ready signals
- Total: ~250 hashes * 32 bytes = ~8 KB -- negligible

Adversarial conditions:
- DKG uses a TCP listener (`network.rs:37-38`) that accepts connections from any IP
- An attacker can send unique messages at wire speed
- At 10,000 messages/second over the 120s Phase 2 timeout = 1.2M hashes
- 1.2M * 32 bytes = ~38 MB -- significant but not immediately fatal
- Each hash also has HashSet overhead (~48 bytes per entry with metadata), so total is closer to ~96 MB
- Sustained flooding across all phases (total ~360s of timeouts) could reach ~300 MB

The TCP layer has no authentication check before accepting connections (see `network.rs:120-173` -- the sender's public key is read from the message envelope, not from TLS). Any IP can connect and send messages. The `is_participant()` check happens inside `handle_message()` which is called **after** `seen_messages.insert()`, so invalid messages from non-participants still consume memory in the set.

## Root Cause

The `seen_messages` set was designed purely for deduplication without considering cleanup or adversarial abuse. There is no per-phase clearing (messages from Phase 1 are irrelevant in Phase 4), no maximum size cap, and no authentication gate before hash insertion. The hash insertion happens before participant validation, allowing non-participants to pollute the set.

## Suggested Fix

**Option 1**: Move the hash insertion to after participant validation (already existing `is_participant()` check in `handle_message()`), so only messages from valid participants are recorded:

```rust
pub fn handle_message_bytes(
    &mut self,
    from: &ed25519::PublicKey,
    bytes: &[u8],
) -> Result<(), DkgError> {
    let message_hash = compute_message_hash(bytes);
    if self.seen_messages.contains(&message_hash) {
        return Ok(());
    }
    // Validate participant BEFORE inserting hash
    if !self.is_participant(from) {
        return Err(DkgError::UnknownSender { sender: format!("{:?}", from) });
    }
    let msg = ProtocolMessage::from_bytes(bytes, self.config.n() as u32)?;
    // ... session check ...
    self.seen_messages.insert(message_hash);
    self.handle_message(from, msg)
}
```

**Option 2**: Cap the set size (e.g., 10,000 entries) and reject further messages when full:

```rust
const MAX_SEEN_MESSAGES: usize = 10_000;
if self.seen_messages.len() >= MAX_SEEN_MESSAGES {
    warn!("seen_messages capacity reached, rejecting message");
    return Err(DkgError::TooManyMessages);
}
self.seen_messages.insert(message_hash);
```

**Option 3**: Clear the set between ceremony phases:

```rust
pub fn set_phase(&mut self, phase: DkgPhase) {
    self.current_phase = phase;
    self.seen_messages.clear();  // reset dedup state between phases
}
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- Add bounds checking or phase-based clearing for `seen_messages` (around lines 496, 527)

## Related Issues

- Local file `167-dkg-alllogs-no-count-cap-oom.md` -- `AllLogs` deserialization also has unbounded allocation (same class of unbounded growth issue)
