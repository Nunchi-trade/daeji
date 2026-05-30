# DKG wait_for_peers Is a Static 10s Sleep, Not Connectivity-Based

**Category**: reliability
**Severity**: medium
**Labels**: `reliability`, `dkg`, `bug`

## Summary

The `wait_for_peers()` method in the DKG ceremony runner sleeps for a fixed 10 seconds regardless of actual peer reachability, then proceeds to Phase 1 message broadcasts. If peers take longer than 10 seconds to start (common in container orchestration environments), Phase 1 broadcasts will be sent to unreachable peers. Since Phase 1 messages are generated once and not retried on failure (see local file 169), late-starting peers will permanently miss the `DealerPublic` broadcast and the ceremony will stall at Phase 2 timeout (120s). The unused `_network` parameter in the method signature suggests connectivity probing was planned but never implemented.

## Problem

The DKG ceremony runner at `crates/node/dkg/src/ceremony.rs` implements `wait_for_peers()` (lines 397-408) as a simple `tokio::time::sleep(Duration::from_secs(10))`. The method takes a `_network: &DkgNetwork` parameter (note the underscore prefix indicating it is unused), suggesting the original intent was to probe peer connectivity before proceeding.

After the 10-second sleep, the ceremony immediately starts Phase 1 by calling `start_dealer()`, which generates dealer messages and sends them via `send_outgoing()`. If any peer is not yet listening on its TCP port, the `network.send_to()` or `network.broadcast()` call will fail, and the messages are lost (see local file 169).

In the TCP-based `DkgNetwork` implementation (`network.rs:83`), `send_to()` uses `TcpStream::connect_timeout()` with a 5-second timeout. If a peer's listener is not yet bound, the connection attempt fails, the message is logged as a warning, and the message is dropped.

## Code Reference

`wait_for_peers()` in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` (lines 396-408):

```rust
    /// Wait for peers to be reachable.
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

The call site at line 73 of the same file:

```rust
        // Wait for peers to be ready
        self.wait_for_peers(&network).await?;
```

## Impact

**DKG ceremony failure if any peer takes more than 10 seconds to start.** Concrete scenarios:

1. **Container orchestration**: Docker Compose, Kubernetes, and similar systems start containers in parallel with no guaranteed ordering. A container that needs to pull an image, initialize storage, or wait for resource limits can easily take more than 10 seconds to start. This is especially common in resource-constrained environments (the project's devnet uses 1.2 CPU/node and 4GB RAM).

2. **Network initialization**: Even after a container starts, binding the DKG TCP listener, loading identity keys, and initializing the participant state adds latency. On a cold start with key generation, this can exceed 10 seconds.

3. **Crash recovery**: If a DKG participant crashes and restarts, it must re-initialize its network layer. Combined with the 10-second sleep, the restarted node may miss Phase 1 messages from participants that started earlier.

4. **Cascade with message loss**: This issue directly compounds with the `send_outgoing` message loss bug (local file 169). The 10-second sleep is the primary mitigation for the message loss bug, and its inadequacy makes the message loss bug more likely to trigger.

The ceremony will timeout at Phase 2 (120s) with error messages about missing dealer messages, requiring a full restart with all participants.

## Root Cause

The method was implemented as a simple sleep placeholder. The code comment ("Give all DKG containers time to join the p2p overlay before the phase-1 broadcasts") acknowledges the purpose, and the unused `_network` parameter indicates that connectivity probing was intended but never implemented.

## Suggested Fix

Replace the static sleep with actual connectivity probing. The `DkgNetwork` already has `send_to()` which attempts TCP connections, so a lightweight probe loop can check reachability:

**Before** (`crates/node/dkg/src/ceremony.rs:396-408`):
```rust
async fn wait_for_peers(&self, _network: &DkgNetwork) -> Result<(), DkgError> {
    info!("Waiting for peers to be ready...");
    let start = Instant::now();
    tokio::time::sleep(Duration::from_secs(10)).await;
    info!(elapsed = ?start.elapsed(), "Peer initialization complete");
    Ok(())
}
```

**After**:
```rust
async fn wait_for_peers(&self, network: &DkgNetwork) -> Result<(), DkgError> {
    let timeout = Duration::from_secs(60);
    let probe_interval = Duration::from_secs(2);
    let start = Instant::now();
    let my_pk = self.config.my_public_key();

    loop {
        let mut reachable = 0usize;
        for pk in &self.config.participants {
            if *pk == my_pk {
                reachable += 1;
                continue;
            }
            // Attempt a lightweight TCP connection check
            if network.probe_peer(pk).is_ok() {
                reachable += 1;
            }
        }
        let required = self.config.n();
        info!(reachable, required, elapsed = ?start.elapsed(), "Peer connectivity check");

        if reachable >= required {
            info!(elapsed = ?start.elapsed(), "All peers reachable");
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(DkgError::Timeout);
        }
        tokio::time::sleep(probe_interval).await;
    }
}
```

This requires adding a `probe_peer()` method to `DkgNetwork` that attempts a TCP connection without sending data, or reusing the existing `send_to()` infrastructure.

A simpler alternative is to increase the static sleep and add a configurable timeout:

```rust
let wait_secs = std::env::var("KORA_DKG_PEER_WAIT_SECS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(10u64);
tokio::time::sleep(Duration::from_secs(wait_secs)).await;
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/ceremony.rs` -- Replace static sleep in `wait_for_peers()` with connectivity probing (lines 396-408)
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/network.rs` -- Optionally add a `probe_peer()` method for lightweight connectivity checks

## Related Issues

- Local file `169-dkg-send-outgoing-drops-messages.md` -- `send_outgoing` permanently drops failed messages (this is the downstream consequence when `wait_for_peers` does not wait long enough)
- Local file `170-dkg-crash-recovery-skips-rebroadcast.md` -- Crash recovery does not re-broadcast Phase 1 messages (related startup timing issue)
