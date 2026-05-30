# DKG Leader Is Always participants[0] -- No Leader Election or Failover

**Category**: reliability
**Severity**: medium
**Labels**: `reliability`, `dkg`, `enhancement`

## Summary

The DKG ceremony leader (the coordinator node that collects and redistributes dealer logs during Phase 4) is always hardcoded to `participants[0]` -- the first public key in the participant list. If this node is slow, crashes, becomes network-partitioned, or behaves maliciously, the ceremony stalls at Phase 4 timeout (120s) with no failover mechanism. There is no leader election, no leader rotation, and no fallback to an alternative coordinator. This makes the first participant a single point of failure for every DKG ceremony.

## Problem

In `crates/node/dkg/src/protocol.rs`, the `DkgParticipant::leader()` method at line 910-912 returns a hardcoded reference to the first participant:

```rust
    /// Get the leader (participant at index 0).
    fn leader(&self) -> &ed25519::PublicKey {
        &self.config.participants[0]
    }
```

The ceremony runner at `crates/node/dkg/src/ceremony.rs` uses `is_leader()` at lines 392-394 based on the same logic:

```rust
    /// Check if this node is the leader (coordinator).
    const fn is_leader(&self) -> bool {
        self.config.validator_index == 0
    }
```

During Phase 4 (collecting dealer logs), the leader plays a critical coordination role:
- The leader collects signed dealer logs from all participants
- Non-leaders request logs from the leader every 5 seconds (ceremony.rs lines 357-371)
- The leader responds to `RequestLogs` messages by sending `AllLogs` with all collected logs (protocol.rs lines 625-634)

If the leader fails:
- Non-leaders will repeatedly request logs but receive no response
- Phase 4 will timeout after `PHASE4_MAX_TIMEOUT_SECS` (120 seconds)
- The ceremony fails and must be manually restarted

## Code Reference

Leader selection in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 909-912):

```rust
    /// Get the leader (participant at index 0).
    fn leader(&self) -> &ed25519::PublicKey {
        &self.config.participants[0]
    }
```

Leader check in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` (lines 392-394):

```rust
    /// Check if this node is the leader (coordinator).
    const fn is_leader(&self) -> bool {
        self.config.validator_index == 0
    }
```

Phase 4 log request loop in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` (lines 357-371):

```rust
            // Request logs from leader if we don't have enough and haven't requested recently
            if !self.is_leader()
                && logs < required
                && last_request_time.elapsed() >= Duration::from_secs(5)
                && let Some(leader_pk) = self.config.participants.first()
            {
                info!(logs, required, "Requesting dealer logs from leader");
                let request_msg = ProtocolMessage::new(
                    participant.ceremony_id(),
                    ProtocolMessageKind::RequestLogs,
                );
                if let Err(e) = network.send_to(leader_pk, &request_msg) {
                    warn!(?e, "Failed to send log request to leader");
                }
                last_request_time = Instant::now();
            }
```

Authorization check for AllLogs in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 637-639):

```rust
            ProtocolMessageKind::AllLogs { logs } => {
                if from != self.leader() {
                    return Err(DkgError::UnauthorizedSender);
                }
```

## Impact

**Single point of failure for all DKG ceremonies.** Specific scenarios:

1. **Leader crash during Phase 3-4**: If participant 0 crashes after Phase 2 but before all logs are distributed, no one can collect and redistribute the logs. The ceremony stalls for 120 seconds then fails. All participants must coordinate a restart.

2. **Slow leader**: If participant 0 is resource-constrained (slow CPU, high memory pressure, network congestion), Phase 4 log distribution is bottlenecked through a single slow node.

3. **Malicious leader**: A compromised participant 0 can selectively withhold or delay log distribution, preventing the ceremony from completing. Since only the leader can send `AllLogs` messages (other nodes reject them at `protocol.rs:638`), there is no workaround.

4. **Network partition**: If participant 0 becomes network-partitioned from some participants, those participants cannot receive logs and the ceremony fails for them.

5. **Predictability**: Since the leader is always the same node, an attacker knows exactly which node to target to disrupt DKG ceremonies.

## Root Cause

The leader selection was implemented with the simplest possible approach: always use the first participant in the sorted participant list. No leader election protocol, rotation mechanism, or failover logic was built. This is common in early-stage protocol implementations but becomes a reliability concern in production.

## Suggested Fix

**Option 1 (minimal): Deterministic leader rotation based on ceremony parameters.**

Derive the leader from the ceremony timestamp, so different ceremonies use different leaders:

```rust
fn leader(&self) -> &ed25519::PublicKey {
    let leader_idx = (self.timestamp_nanos as usize) % self.config.participants.len();
    &self.config.participants[leader_idx]
}
```

And correspondingly in `ceremony.rs`:

```rust
fn is_leader(&self) -> bool {
    let leader_idx = (self.timestamp_nanos as usize) % self.config.participants.len();
    self.config.validator_index == leader_idx
}
```

Note: This requires `timestamp_nanos` to be available in both `DkgParticipant` (already stored at line 338) and `DkgCeremony` (would need to be stored after computation at ceremony.rs:78-82).

**Option 2 (better): Leader failover.**

If the leader is unresponsive for N seconds, try the next participant in the list:

```rust
fn leader_for_attempt(&self, attempt: usize) -> &ed25519::PublicKey {
    let idx = attempt % self.config.participants.len();
    &self.config.participants[idx]
}
```

**Option 3 (best): Eliminate the hub-and-spoke pattern.**

Have all participants broadcast their signed dealer logs to all other participants, removing the coordinator bottleneck entirely. Each participant collects logs directly from all others, with the same `required_dealer_logs()` completion criterion.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- `leader()` method at line 910 and related `AllLogs` authorization check at line 638
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` -- `is_leader()` at line 392 and Phase 4 request logic at lines 357-371

## Related Issues

- Local file `167-dkg-alllogs-no-count-cap-oom.md` -- Malicious leader can also exploit `AllLogs` deserialization for OOM (amplified by the leader being a fixed, known target)
- Local file `170-dkg-crash-recovery-skips-rebroadcast.md` -- Leader crash recovery does not restore coordination state
