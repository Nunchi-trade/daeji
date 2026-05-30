# DKG send_outgoing Silently Drops Failed Phase 1 Broadcasts

**Category**: reliability
**Severity**: high
**Labels**: `reliability`, `dkg`, `bug`

## Summary

The DKG ceremony's `send_outgoing()` method consumes all queued outgoing messages via `take_outgoing()` (which uses `std::mem::take` to permanently drain the queue), then attempts to send each one. If any send fails (e.g., peer not yet connected), the message is permanently lost with no retry. For Phase 1 messages (`DealerPublic` broadcasts and `DealerPrivate` direct messages), these are generated exactly once in `start_dealer()` and never regenerated, so a single failed send causes the entire ceremony to stall irrecoverably. The warning log "will retry on next cycle" is misleading -- no retry occurs.

## Problem

The DKG ceremony runner (`crates/node/dkg/src/ceremony.rs`) uses a queue-based messaging pattern. Protocol operations in `DkgParticipant` push messages onto the `self.outgoing` Vec, and the ceremony runner periodically calls `send_outgoing()` to drain and send them.

The failure mode involves two connected issues:

1. **`take_outgoing()`** at `protocol.rs:880-882` uses `std::mem::take(&mut self.outgoing)` to atomically move all messages out of the participant, replacing the queue with an empty Vec. After this call, the participant no longer has any reference to the messages.

2. **`send_outgoing()`** at `ceremony.rs:411-435` iterates the taken messages and calls `network.send_to()` or `network.broadcast()` for each. On failure, it logs a warning but does nothing to preserve the message. Once the loop iteration ends, the message is dropped.

This is particularly damaging for Phase 1, where `start_dealer()` at `protocol.rs:431-484` generates the dealer messages exactly once:
- A `DealerPublic` broadcast (commitment polynomial) is queued at lines 452-458
- Individual `DealerPrivate` messages (secret shares) are queued at lines 466-473

These messages are the output of `Dealer::start()`, which consumes randomness from `OsRng`. There is no way to regenerate them after the fact without creating a new dealer instance (which would produce different polynomial commitments, invalidating any already-received messages).

## Code Reference

`send_outgoing()` in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` (lines 411-435):

```rust
    /// Send all outgoing messages.
    fn send_outgoing(
        &self,
        network: &DkgNetwork,
        participant: &mut DkgParticipant,
    ) -> Result<(), DkgError> {
        for (target, msg) in participant.take_outgoing() {
            match target {
                Some(pk) => {
                    if let Err(e) = network.send_to(&pk, &msg) {
                        warn!(
                            ?pk,
                            ?e,
                            "Failed to send DKG message to peer (will retry on next cycle)"
                        );
                    }
                }
                None => {
                    if let Err(e) = network.broadcast(&msg) {
                        warn!(?e, "Failed to broadcast DKG message (will retry on next cycle)");
                    }
                }
            }
        }
        Ok(())
    }
```

`take_outgoing()` in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 880-882):

```rust
    /// Take outgoing messages.
    pub fn take_outgoing(&mut self) -> Vec<(Option<ed25519::PublicKey>, ProtocolMessage)> {
        std::mem::take(&mut self.outgoing)
    }
```

Phase 1 message generation in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 451-474):

```rust
        // Queue public message for broadcast
        self.outgoing.push((
            None, // broadcast
            ProtocolMessage::new(
                ceremony_id,
                ProtocolMessageKind::DealerPublic { dealer: my_pk.clone(), msg: pub_msg.clone() },
            ),
        ));

        // Queue private messages for each player, storing our own
        for (player_pk, priv_msg) in priv_msgs {
            if player_pk == my_pk {
                self.dealer_priv_msgs.insert(my_pk.clone(), priv_msg);
            } else {
                self.outgoing.push((
                    Some(player_pk.clone()),
                    ProtocolMessage::new(
                        ceremony_id,
                        ProtocolMessageKind::DealerPrivate { dealer: my_pk.clone(), msg: priv_msg },
                    ),
                ));
            }
        }
```

## Impact

**DKG ceremony failure when any peer is temporarily unreachable during Phase 1.** Concrete scenarios:

1. **Container startup race**: In Docker Compose deployments, nodes start at slightly different times. If node B's TCP listener is not ready when node A broadcasts `DealerPublic`, node B never receives node A's commitment. Without the commitment, node B cannot process node A's `DealerPrivate` share, cannot generate an ack, and the ceremony stalls at Phase 2 timeout (120s).

2. **Network flakiness**: A transient TCP connection failure during the narrow Phase 1 window permanently prevents affected peers from receiving dealer messages. The ceremony stalls and must be completely restarted.

3. **Misleading debugging**: The log message "will retry on next cycle" is factually incorrect -- there is no retry. This wastes operator time investigating retry logic that does not exist.

4. **Scale sensitivity**: With more participants, the probability of at least one send failure during Phase 1 increases, making this more likely to manifest in larger deployments.

## Root Cause

`take_outgoing()` is a destructive operation that permanently removes messages from the participant's state. The `send_outgoing()` method does not re-enqueue messages that fail to send. There is no separate storage of Phase 1 messages for potential resending, and the cryptographic `Dealer` instance that generated them is not re-invokable after `start_dealer()` returns.

## Suggested Fix

**Option 1 (recommended): Re-enqueue failed messages.**

Add a `re_enqueue` method to `DkgParticipant` and use it in `send_outgoing()`:

```rust
// In protocol.rs - add to DkgParticipant
pub fn re_enqueue(&mut self, messages: Vec<(Option<ed25519::PublicKey>, ProtocolMessage)>) {
    self.outgoing.extend(messages);
}

// In ceremony.rs - modify send_outgoing
fn send_outgoing(
    &self,
    network: &DkgNetwork,
    participant: &mut DkgParticipant,
) -> Result<(), DkgError> {
    let mut failed = Vec::new();
    for (target, msg) in participant.take_outgoing() {
        let result = match &target {
            Some(pk) => network.send_to(pk, &msg),
            None => network.broadcast(&msg),
        };
        if let Err(e) = result {
            warn!(?e, "Failed to send DKG message, will retry next cycle");
            failed.push((target, msg));
        }
    }
    if !failed.is_empty() {
        participant.re_enqueue(failed);
    }
    Ok(())
}
```

**Option 2**: At minimum, fix the misleading log message:

```rust
warn!(?e, "Failed to send DKG message -- message is LOST (no retry mechanism)");
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` -- Modify `send_outgoing()` to re-enqueue failed messages (lines 411-435)
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- Add `re_enqueue()` method to `DkgParticipant`

## Related Issues

- Local file `171-dkg-wait-for-peers-static-sleep.md` -- `wait_for_peers` is a static 10s sleep with no connectivity check (compounds the problem)
- Local file `170-dkg-crash-recovery-skips-rebroadcast.md` -- Crash recovery does not re-broadcast Phase 1 messages (related message loss on restart)
