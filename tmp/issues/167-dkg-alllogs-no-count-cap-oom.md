# DKG AllLogs Deserialization Has No Count Cap -- Remote OOM Vector

**Category**: security
**Severity**: high
**Labels**: `security`, `dkg`, `bug`, `reliability`

## Summary

When deserializing an `AllLogs` DKG protocol message, the count field is read as a `u32` and used directly as the argument to `Vec::with_capacity()` without any validation against the actual participant count or remaining buffer size. A malicious DKG leader can send a crafted message with `count = u32::MAX` (4,294,967,295), causing the recipient to attempt a ~137 GB+ memory allocation. This allocation attempt crashes the process immediately via OOM before any loop iteration reads actual data.

## Problem

The DKG protocol module (`crates/node/dkg/src/protocol.rs`) deserializes incoming ceremony messages in `ProtocolMessage::from_bytes()`. For message tag `5` (`AllLogs`), the code reads a `u32` count value from the wire and passes it directly to `Vec::with_capacity(count)`. This pre-allocation happens **before** the loop that reads individual log entries, so the OOM crash occurs immediately on receiving a crafted message.

The `AllLogs` message is sent by the DKG leader to distribute all collected dealer logs to participants during Phase 4 of the ceremony. Participants request logs from the leader via `RequestLogs` messages, and the leader responds with `AllLogs`.

The DKG network layer has message size limits, but they do not protect against this vulnerability:
- The TCP-based `DkgNetwork` in `network.rs` limits total message size to 1 MB (line 151)
- The authenticated `DkgTransport` in `transport.rs` limits messages to 256 KB (line 27, `DEFAULT_MAX_MESSAGE_SIZE`)

These limits prevent receiving oversized payloads, but the `with_capacity()` call allocates based on the `count` integer embedded in the message, not the actual message size. A valid 256 KB message can contain a `count` field set to `u32::MAX` followed by just a few bytes of actual data.

On a 64-bit system: `u32::MAX * sizeof::<(PublicKey, SignedDealerLog)>` = approximately 256 GB allocation attempt.

## Code Reference

The vulnerable deserialization in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 269-281):

```rust
            5 => {
                let count = u32::read(&mut reader)? as usize;
                let mut logs = Vec::with_capacity(count);  // UNBOUNDED allocation
                for _ in 0..count {
                    let pk = ed25519::PublicKey::read(&mut reader)?;
                    let log = SignedDealerLog::<MinSig, ed25519::PrivateKey>::read_cfg(
                        &mut reader,
                        &max_degree_nz,
                    )?;
                    logs.push((pk, log));
                }
                ProtocolMessageKind::AllLogs { logs }
            }
```

The TCP message size limit in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/network.rs` (lines 149-154):

```rust
                    let len = u32::from_le_bytes(len_bytes) as usize;

                    if len > 1024 * 1024 {
                        warn!(%addr, len, "Message too large");
                        continue;
                    }
```

The authenticated transport message size limit in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/transport.rs` (line 27):

```rust
/// Default maximum message size for DKG (256 KB).
pub const DEFAULT_MAX_MESSAGE_SIZE: u32 = 256 * 1024;
```

## Impact

**Remote denial of service.** A malicious or compromised DKG leader can crash any DKG participant by:

1. Waiting for Phase 4, when participants send `RequestLogs` messages to the leader
2. Responding with a crafted `AllLogs` message where the count field is set to `u32::MAX` but the payload only contains a few bytes
3. The recipient calls `Vec::with_capacity(4294967295)`, the OS OOM killer terminates the process
4. The DKG ceremony fails and cannot complete, blocking network bootstrap

Since the DKG leader is always `participants[0]` (see local file 174), a single compromised first-listed participant can block all DKG ceremonies.

Even without a malicious actor, a serialization bug or network corruption that garbles the count field would trigger the same crash.

## Root Cause

The deserialization code trusts the wire-format count value without validation. The `max_degree` parameter (computed from the participant count and passed to `from_bytes()`) provides a natural upper bound on how many logs can legitimately exist in an `AllLogs` message, but this bound is not applied to the `count` field before the `Vec::with_capacity()` call.

## Suggested Fix

Cap the count to a reasonable maximum before allocation. The number of dealer logs can never legitimately exceed the participant count:

**Before** (`crates/node/dkg/src/protocol.rs:269-271`):
```rust
5 => {
    let count = u32::read(&mut reader)? as usize;
    let mut logs = Vec::with_capacity(count);
```

**After**:
```rust
5 => {
    let count = u32::read(&mut reader)? as usize;
    // Cap to participant count (max_degree >= n for initial DKG) to prevent OOM
    let safe_count = count.min(max_degree as usize + 1);
    let mut logs = Vec::with_capacity(safe_count);
    for _ in 0..safe_count {
```

Alternatively, avoid `with_capacity()` entirely for untrusted input and let the Vec grow organically:

```rust
5 => {
    let count = u32::read(&mut reader)? as usize;
    let safe_count = count.min(max_degree as usize + 1);
    let mut logs = Vec::new();
    for _ in 0..safe_count {
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- Cap the count in `AllLogs` deserialization (around line 270)

## Related Issues

- Local file `174-dkg-leader-always-first-participant.md` -- DKG leader is always `participants[0]` with no failover (amplifies this issue since a compromised leader can exploit it)
- Local file `172-dkg-seen-messages-unbounded.md` -- `seen_messages` HashSet also grows unbounded (related class of unbounded growth)
