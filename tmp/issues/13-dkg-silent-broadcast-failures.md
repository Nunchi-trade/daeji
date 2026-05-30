# DKG: Silent broadcast failures can hang ceremony indefinitely

**Severity:** Medium
**Component:** `crates/node/dkg/`
**Labels:** `bug`, `dkg`, `reliability`, `error-handling`

## Summary

During the DKG (Distributed Key Generation) ceremony, network send and broadcast failures are silently suppressed -- either discarded entirely with `let _ =` or logged only at `debug` level. If a network partition or transient connectivity issue occurs during any phase of the ceremony, the DKG will hang until its hardcoded 120-second timeout expires, producing no actionable diagnostics for operators. Since the chain cannot start consensus without a successful DKG, this effectively blocks the entire network launch with no visibility into the root cause.

## Background

### What DKG is in Kora

Kora is a blockchain node built on [commonware](https://github.com/commonwarexyz/monorepo) consensus primitives. Before the Kora chain can begin producing blocks, the validator set must complete a **Distributed Key Generation (DKG) ceremony** to create a shared BLS12-381 threshold keypair. This keypair is used by the Simplex consensus protocol to produce threshold signatures on blocks.

The DKG uses a **Joint-Feldman protocol** (via commonware's `bls12381::dkg` module) with the following 5-phase structure:

1. **Phase 1 -- Start Dealer**: Each participant generates commitments (public polynomial) and encrypted shares for every other participant, then broadcasts them.
2. **Phase 2 -- Collect Messages & Send Acks**: Each participant receives dealer messages from all others, verifies them, and sends back acknowledgements.
3. **Phase 2.5 -- Ready Barrier**: All participants signal "ready" after sending their acks, ensuring no dealer finalizes before all acks are delivered.
4. **Phase 3 -- Finalize Dealer**: Each participant finalizes their dealer using the collected acks and broadcasts their signed dealer log.
5. **Phase 4 -- Collect Dealer Logs**: Participants collect all dealer logs (non-leaders request from the leader). Once enough logs are collected (all `n` participants for the initial ceremony), Phase 5 finalizes and produces the DKG output (group public key + individual secret share).

If the DKG fails, the node cannot start consensus. There is no automatic ceremony-level retry; a failed DKG requires manual operator intervention and process restart.

### Why This Matters

- **Chain launch is blocked**: Without DKG output, the Simplex consensus engine cannot be initialized. The chain literally cannot start.
- **Silent failures waste time**: Operators see only a `DkgError::Timeout` or `DkgError::CeremonyFailed` after 120 seconds, with no indication that messages were failing to send during that entire window.
- **Network partitions are common**: In cloud deployments (especially Kubernetes), transient DNS resolution failures, firewall misconfigurations, and container scheduling delays are routine. The DKG must surface these issues, not hide them.

## Problematic Code

### 1. `let _ =` on send_to (ceremony.rs line 346)

In `crates/node/dkg/src/ceremony.rs`, during Phase 4, when a non-leader node requests dealer logs from the leader, the send result is completely discarded:

```rust
// crates/node/dkg/src/ceremony.rs, line 346
let _ = network.send_to(leader_pk, &request_msg);
```

If this send fails (e.g., leader is unreachable), the node silently continues polling. It will never receive the logs it needs, and will eventually time out after 120 seconds with no indication that its log requests were never delivered.

### 2. Network failures logged at debug level only (ceremony.rs lines 380-400)

The `send_outgoing` method handles ALL outgoing messages for every phase of the protocol. Send failures are logged at `debug` level, which is typically disabled in production:

```rust
// crates/node/dkg/src/ceremony.rs, lines 380-400
fn send_outgoing(
    &self,
    network: &DkgNetwork,
    participant: &mut DkgParticipant,
) -> Result<(), DkgError> {
    for (target, msg) in participant.take_outgoing() {
        match target {
            Some(pk) => {
                if let Err(e) = network.send_to(&pk, &msg) {
                    debug!(?pk, ?e, "Failed to send to peer");     // <-- debug only
                }
            }
            None => {
                if let Err(e) = network.broadcast(&msg) {
                    debug!(?e, "Failed to broadcast");             // <-- debug only
                }
            }
        }
    }
    Ok(())
}
```

This is the primary message dispatch for the entire DKG. Every Phase 1 broadcast, every Phase 2 ack, every Phase 3 dealer log broadcast -- all of them go through this function. If the network is down, every send silently fails, and the function still returns `Ok(())`.

**Important nuance:** The `DkgNetwork::broadcast()` method (network.rs lines 102-112) internally iterates over peers and calls `send_to()` for each, logging individual send failures at `warn!` level. However, `broadcast()` always returns `Ok(())` regardless of how many individual sends failed. This means:
- The `debug!` log at ceremony.rs line 394 (`"Failed to broadcast"`) is effectively dead code -- `broadcast()` never returns `Err`.
- For targeted sends (`Some(pk)` branch), `DkgNetwork::send_to()` (network.rs lines 60-98) already logs connection failures at `warn!` level (line 95) before returning `Err`. The `debug!` log at ceremony.rs line 389 is a duplicate for connection errors, but is the only log for non-connection errors (unknown peer, DNS resolution failure, write failures).
- **The real problem is that `send_outgoing` always returns `Ok(())` and the caller has no visibility into how many messages failed.** The function signature claims to return a `Result` but never actually returns `Err`.

### 3. Transport `.ok()` on channel recv (transport.rs line 250)

In the production transport layer (`crates/node/dkg/src/transport.rs`), the `recv` method discards channel errors:

```rust
// crates/node/dkg/src/transport.rs, line 250
pub async fn recv(&mut self) -> Option<(ed25519::PublicKey, Bytes)> {
    self.receiver.recv().await.ok().map(|(sender, message)| (sender, Bytes::from(message)))
}
```

If the underlying p2p receiver channel is closed (e.g., network layer crashed), this returns `None` as if there are simply no messages, rather than signaling an error. The caller cannot distinguish "no messages yet" from "the transport is permanently broken."

### 4. `.ok()` on set_read_timeout (network.rs line 124)

In the TCP-based DKG network (`crates/node/dkg/src/network.rs`), `set_read_timeout` failures are silently ignored:

```rust
// crates/node/dkg/src/network.rs, line 124
stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
```

If this call fails (which can happen on certain OS configurations), the subsequent `read_exact` calls will block indefinitely, hanging the `poll_incoming` method and stalling the entire ceremony.

### 5. `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` (ceremony.rs line 79)

```rust
// crates/node/dkg/src/ceremony.rs, line 79
let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
```

This will panic if the system clock is set before the Unix epoch (January 1, 1970). While rare, this can occur in containerized environments with misconfigured clocks (e.g., fresh VMs without NTP sync, or time-shifted testing environments). A panic during DKG initialization would crash the entire node process with an unhelpful "called `unwrap()` on an `Err` value" message.

### 6. Byte slice `.try_into().unwrap()` in protocol deserialization (protocol.rs lines 76-77)

```rust
// crates/node/dkg/src/protocol.rs, lines 76-77
let chain_id = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
let round = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
```

These unwraps are technically safe because the slices are exactly 8 and 4 bytes respectively, and the function already checks `bytes.len() < 44` above (line 71). However, they could be converted to `map_err` for defense-in-depth, ensuring future refactors don't introduce a panic path.

### 7. `NonZeroU32::new(max_degree).unwrap()` in crash recovery (protocol.rs lines 1029, 1043)

```rust
// crates/node/dkg/src/protocol.rs, line 1029
&core::num::NonZeroU32::new(max_degree).unwrap(),
// crates/node/dkg/src/protocol.rs, line 1043
&core::num::NonZeroU32::new(max_degree).unwrap(),
```

In `DkgParticipant::try_restore()`, when deserializing persisted dealer logs, `max_degree` is set to `config.t()` (the threshold). If the threshold is somehow 0 (invalid configuration), `NonZeroU32::new(0)` returns `None` and the `unwrap()` panics, crashing the node during startup recovery. While a threshold of 0 should be rejected earlier during config validation, a panic during crash recovery is especially bad -- the node cannot recover from a crash if the recovery code itself panics.

### 8. DKG state `.ok()` on hex decode (state.rs lines 158, 170)

```rust
// crates/node/dkg/src/state.rs, line 158
self.our_signed_log.as_ref().and_then(|s| hex::decode(s).ok())

// crates/node/dkg/src/state.rs, line 170
.filter_map(|(k, v)| hex::decode(v).ok().map(|bytes| (k.clone(), bytes)))
```

When restoring persisted DKG state from `dkg_state.json`, hex decode failures on the signed dealer log (line 158) or received logs (line 170) are silently ignored via `.ok()`. If the state file is corrupted (disk error, partial write), the DKG ceremony will silently start from scratch instead of alerting the operator that crash recovery data is corrupted. This could lead to a session mismatch if the other participants have progressed past the point where the corrupted state was saved.

## Impact

1. **120-second silent hang**: If Phase 1 broadcasts fail, the ceremony waits the full `PHASE2_MAX_TIMEOUT_SECS` (120s) before reporting `DkgError::Timeout`. No intermediate diagnostics.
2. **Manual restart required**: There is no ceremony-level auto-retry. After a `DkgError::Timeout` or `CeremonyFailed`, the operator must restart the node process.
3. **No metrics**: The DKG ceremony does not emit any Prometheus metrics (send failures, phase durations, retries). Grafana dashboards have no visibility into DKG health.
4. **Misleading progress logs**: Phase progress logs (every 5 seconds) show received/ack counts but never mention send failures. An operator watching logs sees "Phase 2 progress: received 0/4 dealer messages, sent 0/4 acks" with no indication that their own sends are failing.

## Proposed Fixes

### 1. Elevate network failure log levels

Change all DKG network failure logs from `debug!` to `warn!`:

```rust
fn send_outgoing(
    &self,
    network: &DkgNetwork,
    participant: &mut DkgParticipant,
) -> Result<(), DkgError> {
    for (target, msg) in participant.take_outgoing() {
        match target {
            Some(pk) => {
                if let Err(e) = network.send_to(&pk, &msg) {
                    warn!(?pk, ?e, "Failed to send DKG message to peer");
                }
            }
            None => {
                if let Err(e) = network.broadcast(&msg) {
                    warn!(?e, "Failed to broadcast DKG message");
                }
            }
        }
    }
    Ok(())
}
```

And for the leader request in Phase 4:

```rust
match network.send_to(leader_pk, &request_msg) {
    Ok(()) => debug!(logs, required, "Requested logs from leader"),
    Err(e) => warn!(?e, "Failed to request logs from leader"),
}
```

### 2. Add retry logic for failed sends with exponential backoff

Track failed sends per-peer and retry them on subsequent loop iterations. The `ExponentialBackoff` struct already exists in ceremony.rs and can be reused.

**Note:** `DkgNetwork::broadcast()` (network.rs lines 102-112) always returns `Ok(())` even when individual sends fail (it logs each failure at `warn!` internally). Therefore, retry logic must be implemented at the `send_to` level for targeted messages, and the broadcast path must either be refactored to return individual failures or the retry must happen by re-queuing broadcast messages for per-peer `send_to` calls.

```rust
// In send_outgoing, return failed targeted messages for retry
fn send_outgoing(
    &self,
    network: &DkgNetwork,
    participant: &mut DkgParticipant,
) -> Result<Vec<(Option<ed25519::PublicKey>, ProtocolMessage)>, DkgError> {
    let mut failed = Vec::new();
    for (target, msg) in participant.take_outgoing() {
        match target {
            Some(ref pk) => {
                if let Err(e) = network.send_to(pk, &msg) {
                    warn!(?pk, ?e, "Failed to send to peer, will retry");
                    failed.push((target, msg));
                }
            }
            None => {
                // broadcast() always returns Ok(()) -- individual send failures
                // are logged at warn! level inside DkgNetwork::broadcast().
                // No retry is possible at this level without refactoring broadcast()
                // to return per-peer results.
                let _ = network.broadcast(&msg);
            }
        }
    }
    Ok(failed)
}
```

**Alternative approach:** Refactor `DkgNetwork::broadcast()` to return a list of peers that failed, allowing `send_outgoing` to re-queue targeted sends for those peers:

```rust
// network.rs: Change broadcast to return failed peers
pub fn broadcast(&self, msg: &ProtocolMessage) -> Vec<ed25519::PublicKey> {
    let my_pk = self.config.my_public_key();
    let mut failed = Vec::new();
    for pk in self.config.participants.iter() {
        if pk != &my_pk {
            if let Err(e) = self.send_to(pk, msg) {
                warn!(?pk, ?e, "Failed to send to peer during broadcast");
                failed.push(pk.clone());
            }
        }
    }
    failed
}
```

### 3. Replace `unwrap()` with proper error handling

```rust
// ceremony.rs line 79
let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_err(|e| DkgError::CeremonyFailed(format!("System clock before Unix epoch: {}", e)))?
    .as_nanos() as u64;
```

```rust
// protocol.rs lines 76-77
let chain_id = u64::from_le_bytes(
    bytes[32..40].try_into().map_err(|_| commonware_codec::Error::EndOfBuffer)?
);
let round = u32::from_le_bytes(
    bytes[40..44].try_into().map_err(|_| commonware_codec::Error::EndOfBuffer)?
);
```

```rust
// protocol.rs lines 1029, 1043 (in try_restore)
let max_degree_nz = core::num::NonZeroU32::new(max_degree)
    .ok_or_else(|| DkgError::CeremonyFailed("threshold must be > 0".into()))?;
// Then use &max_degree_nz instead of &core::num::NonZeroU32::new(max_degree).unwrap()
```

### 4. Add DKG ceremony health monitoring/metrics

Add counters for:
- `dkg_messages_sent_total` (by phase, by result)
- `dkg_messages_received_total` (by type)
- `dkg_send_failures_total` (by peer, by error type)
- `dkg_phase_duration_seconds` (histogram, by phase)
- `dkg_ceremony_result` (success/timeout/failed)

### 5. Add logging for corrupted DKG state file recovery

In `crates/node/dkg/src/state.rs`, replace silent `.ok()` with logged warnings:

```rust
// state.rs line 158 (get_our_signed_log)
pub fn get_our_signed_log(&self) -> Option<Vec<u8>> {
    self.our_signed_log.as_ref().and_then(|s| {
        hex::decode(s).map_err(|e| {
            tracing::warn!(error = ?e, "Failed to decode persisted dealer log hex");
            e
        }).ok()
    })
}
```

```rust
// state.rs line 170 (get_received_logs)
pub fn get_received_logs(&self) -> BTreeMap<String, Vec<u8>> {
    self.received_logs
        .iter()
        .filter_map(|(k, v)| {
            hex::decode(v).map_err(|e| {
                tracing::warn!(pk_hex = %k, error = ?e, "Failed to decode persisted received log hex");
                e
            }).ok().map(|bytes| (k.clone(), bytes))
        })
        .collect()
}
```

### 6. Consider ceremony-level auto-retry

If the DKG fails with `DkgError::Timeout`, the runner could automatically retry the ceremony (with a new timestamp for the ceremony ID) a configurable number of times before failing permanently. This would handle transient network issues at startup without operator intervention.

## Files to Modify

| File | Changes |
|------|---------|
| `crates/node/dkg/src/ceremony.rs` | Lines 79, 346, 380-400: Replace unwrap, add warn logging, track send failures |
| `crates/node/dkg/src/protocol.rs` | Lines 76-77, 1029, 1043: Replace unwraps with proper error handling |
| `crates/node/dkg/src/transport.rs` | Line 250: Change return type to `Result<Option<...>>` |
| `crates/node/dkg/src/network.rs` | Line 124: Add warn + continue on set_read_timeout failure |
| `crates/node/dkg/src/state.rs` | Lines 158, 170: Add warn logging for hex decode failures |

## Testing Plan

1. **Unit test: send_outgoing logs at warn level** -- Mock `DkgNetwork` to return errors, verify warn-level log output.
2. **Unit test: SystemTime error handling** -- Mock or inject a pre-epoch time, verify the ceremony returns `DkgError::CeremonyFailed` instead of panicking.
3. **Integration test: network partition during Phase 2** -- In a multi-node devnet, use network namespaces or iptables to partition one node during Phase 2, verify warn-level logs appear, verify timeout error message includes send failure context.
4. **Integration test: leader unreachable during Phase 4** -- Partition the leader during Phase 4, verify non-leaders log warnings about failed log requests.
5. **Metrics test** -- If metrics are added, verify counters increment on send failures and phase transitions.

## Verification Checklist

After applying all fixes, verify the following:

- [ ] `cargo build --release` compiles without errors.
- [ ] `cargo test -p kora-dkg` passes all existing tests.
- [ ] `cargo clippy -p kora-dkg` produces no new warnings.
- [ ] Run `grep -n 'debug!.*Failed' crates/node/dkg/src/ceremony.rs` -- should return zero results (all elevated to `warn!`).
- [ ] Run `grep -n '\.unwrap()' crates/node/dkg/src/ceremony.rs` -- should return zero results.
- [ ] Run `grep -n '\.unwrap()' crates/node/dkg/src/protocol.rs` -- lines 76-77 should no longer have unwraps. Lines 1029, 1043 should no longer have unwraps.
- [ ] Run `grep -n 'let _ =' crates/node/dkg/src/ceremony.rs` -- should return zero results (line 346 replaced with match).
- [ ] Run `grep -n '\.ok()' crates/node/dkg/src/network.rs` -- line 124 should be replaced with `if let Err` + `continue`.
- [ ] Run a 4-node Docker devnet DKG ceremony: `just docker devnet`. Verify DKG completes successfully.
- [ ] Intentionally block one node's network during DKG (e.g., `docker network disconnect`). Verify `warn!`-level logs appear in other nodes' output mentioning the unreachable peer.

---

## Additional DKG Gaps (Not Covered Elsewhere)

### Leader Dependency in Phase 4

During Phase 4 of the DKG, non-leader nodes request dealer logs exclusively from the leader node (`ceremony.rs` line 346). If the leader is slow, partitioned, or crashes, ALL non-leader nodes are blocked. There is no fallback mechanism to request logs from other participants who may have them.

**Fix**: Allow non-leaders to request dealer logs from any participant, not just the leader. Maintain a list of participants who have completed Phase 3 and try them in order.

### Unauthenticated DKG Transport

The DKG ceremony uses raw TCP connections (`TcpStream`) in `crates/node/dkg/src/transport.rs`. Messages are length-prefixed but NOT authenticated or encrypted. In a network where attackers can inject traffic (e.g., shared cloud VLANs, misconfigured firewalls), a malicious actor could inject fake DKG messages and corrupt the ceremony output.

The Commonware networking layer used for consensus provides authentication, but the DKG runs before consensus is initialized and uses its own transport.

**Fix**: Add TLS or a simple authenticated encryption layer to DKG transport. Alternatively, use Commonware's networking layer for DKG communication (requires refactoring DKG to run as a Commonware module).

### No Overall Ceremony Timeout Configuration

The 120-second DKG timeouts are hardcoded in `ceremony.rs` lines 16-18:
```rust
const PHASE2_MAX_TIMEOUT_SECS: u64 = 120;
const PHASE4_MAX_TIMEOUT_SECS: u64 = 120;
```

Different network environments (high-latency WAN, local Docker) need different timeouts. The timeout should be configurable via `DkgConfig` or environment variable.

### Related Issues

- **No key rotation**: DKG can only run once at chain genesis. No mechanism for key rotation or validator set changes.
- **Docker devnet startup**: DKG race condition at startup is partially mitigated by the 10-second `wait_for_peers` sleep in `ceremony.rs`, but a more robust peer readiness check is needed.
