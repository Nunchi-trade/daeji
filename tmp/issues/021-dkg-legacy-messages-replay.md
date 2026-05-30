# 021: DKG Protocol Accepts Legacy Messages Without Session Binding -- Replay Attack Vector

**Category**: security
**Severity**: high
**Labels**: security, dkg, bug

---

## Summary

The interactive DKG (Distributed Key Generation) protocol accepts incoming messages that lack a session ID, bypassing anti-replay protection. This allows an attacker who captured messages from a prior DKG ceremony to replay them in a new ceremony by stripping the session ID field, potentially biasing the generated group key or causing protocol confusion.

---

## Problem

Kora's DKG protocol uses a `CeremonySession` struct (with a unique `ceremony_id` derived from chain ID, sorted participant keys, and a timestamp) to bind each protocol message to a specific ceremony instance. When a message arrives, the handler checks its `session_id` field:

- If the session ID is present but mismatched, the message is rejected (lines 504-517 of `protocol.rs`).
- If the session ID is present and matches, processing continues.
- If the session ID is **absent** (`None`), the message is accepted with only a warning log -- no rejection occurs.

This third branch is the vulnerability. The message format explicitly supports a "legacy" variant (`ProtocolMessage::legacy()` at line 159) that omits the session ID. After the `None` branch logs a warning, execution falls through to `self.handle_message()` at line 528, which processes the message against the current ceremony state.

**File**: `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs`

---

## Code Reference

The vulnerable code is at lines 504-528 of `protocol.rs`:

```rust
// crates/node/dkg/src/protocol.rs:504-528
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
    None => {
        warn!(
            ?from,
            "Received legacy message without session ID - accepting for backward compatibility"
        );
    }
}

self.seen_messages.insert(message_hash);
self.handle_message(from, msg)
```

The legacy message constructor at line 159:

```rust
// crates/node/dkg/src/protocol.rs:158-161
/// Create a legacy message without session binding (for backward compatibility).
pub const fn legacy(kind: ProtocolMessageKind) -> Self {
    Self { session_id: None, kind }
}
```

The `CeremonySession::new()` at lines 40-58 derives a deterministic `ceremony_id`:

```rust
// crates/node/dkg/src/protocol.rs:40-58
pub fn new(chain_id: u64, participants: &[ed25519::PublicKey], timestamp_nanos: u64) -> Self {
    let mut hasher = Sha256::default();
    hasher.update(b"kora-dkg-ceremony-v1");
    hasher.update(&chain_id.to_le_bytes());
    hasher.update(&(participants.len() as u64).to_le_bytes());
    // ... sorts and hashes participant keys, includes timestamp ...
    Self { ceremony_id, chain_id, round: 0 }
}
```

---

## Impact

An attacker who has observed a previous DKG ceremony's messages (which are transmitted over plaintext TCP in the current implementation) can replay those messages in a new ceremony by constructing them in the v1 (legacy) format, which omits the session ID. Concrete consequences:

1. **Share injection**: Replayed `DealerPublic` and `DealerPrivate` messages from a previous ceremony could inject stale shares into the new ceremony's `dealer_pub_msgs` and `dealer_priv_msgs` maps, causing the player to process incorrect share data.
2. **Protocol confusion**: Mixing old and new messages can cause ack mismatches or dealer log inconsistencies, potentially preventing successful finalization.
3. **Group key bias**: If enough replayed messages are accepted by enough participants, the resulting group secret key could be influenced, weakening the threshold signature scheme that underpins consensus finality.

This is a realistic attack because Kora has no production deployments with legacy message formats -- the "backward compatibility" path was never needed.

---

## Root Cause

The `handle_message_bytes()` method was designed with a backward-compatibility path for legacy messages that lack session IDs. Since this is a new system with no existing deployments, this path is unnecessary and creates a bypass around the session-binding anti-replay protection.

---

## Suggested Fix

Reject all messages that lack a session ID. Change the `None` branch to return an error instead of falling through:

**Before:**
```rust
None => {
    warn!(
        ?from,
        "Received legacy message without session ID - accepting for backward compatibility"
    );
}
```

**After:**
```rust
None => {
    warn!(?from, "Rejecting message without session ID");
    return Err(DkgError::SessionMismatch {
        expected: hex::encode(self.session.ceremony_id),
        received: String::from("<none>"),
    });
}
```

Additionally, remove the `legacy()` constructor to prevent new code from accidentally creating session-less messages:

**Remove:**
```rust
/// Create a legacy message without session binding (for backward compatibility).
pub const fn legacy(kind: ProtocolMessageKind) -> Self {
    Self { session_id: None, kind }
}
```

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- Lines 519-524: change `None` branch to reject. Lines 158-161: remove `legacy()` constructor.

---

## Related Issues

- `023-getlogs-no-result-limit-dos.md` (another input validation gap)
- `024-trusted-dealer-no-cleanup.md` (DKG security concern)
