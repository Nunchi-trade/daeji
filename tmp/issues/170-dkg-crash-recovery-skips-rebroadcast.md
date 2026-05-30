# DKG Crash Recovery Skips Re-Broadcasting Dealer Messages

**Category**: reliability
**Severity**: high
**Labels**: `reliability`, `dkg`, `bug`, `recovery`

## Summary

When a DKG participant crashes and restores from persisted state via `try_restore()`, the restoration correctly recovers high-level ceremony state (phase, signed dealer logs) but does not restore or re-broadcast the low-level cryptographic state needed to continue participating. Specifically, the `Dealer` and `Player` instances are not restored, dealer pub/priv messages are not re-queued, and Phase 1 messages are not re-broadcast. If a node crashes during Phase 2 or early Phase 3, the restored participant has a fresh `Player` with no processed dealer messages, causing the ceremony to stall.

## Problem

The DKG crash recovery implementation in `crates/node/dkg/src/protocol.rs` function `try_restore()` (lines 989-1087) creates a fresh `DkgParticipant` via `Self::new()` and then overlays persisted state on top of it. The restoration:

1. Creates a **new** `Player` instance (via `DkgParticipant::new()` at line 1018) -- this player has no memory of previously processed dealer messages
2. Does **not** restore the `Dealer` instance (the `Dealer` type from commonware's DKG library is not serializable)
3. Does **not** restore `dealer_pub_msgs` or `dealer_priv_msgs` maps
4. Does **not** re-broadcast Phase 1 messages (which were generated once and sent once)
5. Only restores signed dealer logs and the current phase

The consequence depends on when the crash occurred:

**Crash during Phase 2 (CollectingMessages):**
- The restored participant has a fresh Player that has never seen any dealer messages
- It cannot re-process dealer messages it already handled (they were received via the network and are not persisted)
- Other participants who sent messages to this node before the crash will not re-send them
- The restored participant cannot generate acks, and the ceremony stalls

**Crash during Phase 3 (DealerFinalized):**
- The `dealer` field is `None` in the new participant, so the dealer cannot be re-finalized
- If the signed dealer log was persisted, it can be restored (this case works)
- If the crash happened before the log was persisted, the dealer log is lost and cannot be regenerated

**Crash during Phase 4 (CollectingLogs):**
- This case works best -- signed logs are correctly restored from persisted state
- The participant can continue collecting logs from other participants via the leader

## Code Reference

`try_restore()` in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 989-1087):

```rust
    pub fn try_restore(config: &DkgConfig, timestamp_nanos: u64) -> Result<Option<Self>, DkgError> {
        if !PersistedDkgState::exists(&config.data_dir) {
            return Ok(None);
        }

        let state = PersistedDkgState::load(&config.data_dir)?;
        let persisted_session = state.session()?;

        let expected_session =
            CeremonySession::new(config.chain_id, &config.participants, timestamp_nanos);

        if persisted_session.ceremony_id != expected_session.ceremony_id {
            // ... session mismatch handling ...
            return Ok(None);
        }

        // ... logging ...

        let mut participant = Self::new(config.clone(), timestamp_nanos)?;  // <-- fresh Player, no Dealer
        participant.current_phase = state.phase;

        // Only restores signed dealer logs, NOT dealer_pub_msgs, dealer_priv_msgs, player state
        if state.dealer_finalized
            && let Some(log_bytes) = state.get_our_signed_log()
        {
            // ... restore our signed log ...
        }

        // ... restore received logs ...

        Ok(Some(participant))
    }
```

The `DkgParticipant::new()` constructor at lines 358-410 creates a fresh Player:

```rust
    pub fn new(config: DkgConfig, timestamp_nanos: u64) -> Result<Self, DkgError> {
        // ...
        let player =
            Player::<MinSig, ed25519::PrivateKey>::new(info.clone(), config.identity_key.clone())
                .map_err(|e| DkgError::Crypto(format!("Failed to create player: {:?}", e)))?;
        // ...
        Ok(Self {
            // ...
            player: Some(player),       // fresh player, no processed messages
            dealer: None,                // no dealer instance
            dealer_pub_msgs: BTreeMap::new(),   // empty
            dealer_priv_msgs: BTreeMap::new(),  // empty
            // ...
        })
    }
```

The ceremony runner's recovery entry point at `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` (lines 85-96):

```rust
        let (mut participant, restored_phase) =
            match DkgParticipant::try_restore(&self.config, timestamp_nanos)? {
                Some(p) => {
                    let phase = p.current_phase();
                    info!(phase = %phase, "Restored DKG participant from persisted state");
                    (p, Some(phase))
                }
                None => {
                    let p = DkgParticipant::new(self.config.clone(), timestamp_nanos)?;
                    (p, None)
                }
            };
```

## Impact

**DKG ceremony failure after crash recovery in Phase 2 or early Phase 3.** The ceremony must be fully restarted, requiring coordination across all participants. Specific consequences:

1. **Phase 2 crash**: The restored participant cannot process dealer messages it already received (they are not persisted). Other participants have already sent their messages and will not re-send. The ceremony stalls at Phase 2 timeout (120s).

2. **Phase 3 crash (before log persistence)**: The dealer log cannot be regenerated because the `Dealer` instance (containing the random polynomial) is lost. The ceremony stalls at Phase 4 timeout (120s).

3. **Operational impact**: In production with many validators across different networks/datacenters, restarting a DKG ceremony requires coordinating a new timestamp and ensuring all nodes participate simultaneously. This is a significant operational burden.

4. **Silent failure**: The restored participant logs "Restored DKG participant from persisted state" suggesting successful recovery, but then fails to make progress because it lacks the necessary internal state.

## Root Cause

The crash recovery was designed to restore high-level ceremony progress (phase, signed logs) but not low-level cryptographic state (`Dealer`, `Player` instances, received messages). The commonware DKG `Dealer` and `Player` types are not serializable, making full state restoration difficult. The design assumes that restoration from Phase 4 (CollectingLogs) is the primary recovery scenario, where signed logs are the only state needed.

## Suggested Fix

**Option 1 (recommended): Detect incomplete state and force full restart.**

If the persisted phase indicates the ceremony was in Phase 2 or early Phase 3 (before dealer finalization), the recovery should not attempt to resume from the incomplete state. Instead, it should clear the state and trigger a full ceremony restart:

```rust
pub fn try_restore(config: &DkgConfig, timestamp_nanos: u64) -> Result<Option<Self>, DkgError> {
    // ... existing session matching logic ...

    // Only attempt restoration from phases where we have sufficient state
    match state.phase {
        DkgPhase::DealerFinalized | DkgPhase::CollectingLogs if state.dealer_finalized => {
            // Safe to restore -- we have our signed log and can continue collecting
        }
        phase => {
            warn!(
                %phase,
                "Cannot safely resume DKG from phase {} -- clearing state for fresh start",
                phase
            );
            PersistedDkgState::clear(&config.data_dir)?;
            return Ok(None);
        }
    }

    // ... existing restoration logic ...
}
```

**Option 2**: Log a clear error explaining why the ceremony cannot continue:

```rust
if matches!(state.phase, DkgPhase::DealerStarted | DkgPhase::CollectingMessages) {
    error!(
        "DKG crash recovery from phase {} requires full ceremony restart. \
         Dealer/Player state cannot be restored.",
        state.phase
    );
    PersistedDkgState::clear(&config.data_dir)?;
    return Err(DkgError::CeremonyFailed("Cannot resume from early phase".into()));
}
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- `try_restore()` method (lines 989-1087): add phase validation before attempting restoration
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` -- Recovery entry point (lines 85-96): handle the case where restoration is not possible

## Related Issues

- Local file `169-dkg-send-outgoing-drops-messages.md` -- `send_outgoing` drops failed messages (compounds this issue: even if we could re-broadcast, the messages are lost after the first send attempt)
- Local file `171-dkg-wait-for-peers-static-sleep.md` -- Static 10s sleep for peer connectivity (if peers are slow to reconnect after a crash, Phase 1 re-broadcasts would fail)
