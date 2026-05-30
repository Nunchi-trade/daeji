# P2P: Network Transport Handle Not Monitored -- Silent Network Death

**Category**: bug
**Severity**: high

## Summary

The consensus monitor watches the engine, marshal, and broadcast task handles for unexpected termination and aborts the process if any of them die. However, the transport handle (the P2P network task) is not monitored. If the transport task panics or exits due to a socket error, OOM, or a bug in commonware's discovery layer, the node continues running but cannot send or receive any P2P messages. The result is a node that appears alive (RPC responds, metrics endpoint works) but is consensus-dead (100% nullification rate, unable to exchange votes or blocks).

## Problem

The `spawn_consensus_monitor` function watches three critical task handles:

```rust
fn spawn_consensus_monitor(
    context: cw_tokio::Context,
    engine_handle: RuntimeHandle<()>,
    marshal_handle: RuntimeHandle<()>,
    broadcast_handle: RuntimeHandle<()>,
) {
    spawn_task_watchdog(&context, "consensus_engine", engine_handle);
    spawn_task_watchdog(&context, "marshal_actor", marshal_handle);
    spawn_task_watchdog(&context, "broadcast_engine", broadcast_handle);
}
```

Each watchdog awaits its handle and, if the handle resolves (meaning the task exited or panicked), logs a fatal error and calls `std::process::abort()` to trigger a supervisor restart.

The transport handle (`transport.handle`) is a `Handle<()>` field on `NetworkTransport` that keeps the P2P network task alive. It is never passed to any watchdog or monitoring system. After the transport's channels are consumed for consensus wiring, the handle is simply dropped silently when the `transport` variable goes out of scope (or is kept alive only as long as the `transport` struct is in scope).

**Call site** -- `crates/node/runner/src/runner.rs:1459`:
```rust
spawn_consensus_monitor(context, engine_handle, marshal_handle, broadcast_handle);
// transport.handle is NOT passed to any watchdog
```

The `NetworkTransport` struct itself documents this dependency:

```rust
/// Network handle to keep the network task alive.
///
/// Drop this and the network shuts down.
pub handle: Handle<()>,
```

## Code Reference

**Consensus monitor function** -- `crates/node/runner/src/runner.rs:777-786`:
```rust
fn spawn_consensus_monitor(
    context: cw_tokio::Context,
    engine_handle: RuntimeHandle<()>,
    marshal_handle: RuntimeHandle<()>,
    broadcast_handle: RuntimeHandle<()>,
) {
    spawn_task_watchdog(&context, "consensus_engine", engine_handle);
    spawn_task_watchdog(&context, "marshal_actor", marshal_handle);
    spawn_task_watchdog(&context, "broadcast_engine", broadcast_handle);
}
```

**Task watchdog** -- `crates/node/runner/src/runner.rs:795-827`:
```rust
fn spawn_task_watchdog(context: &cw_tokio::Context, name: &'static str, handle: RuntimeHandle<()>) {
    context.child(name).shared(true).spawn(move |ctx| async move {
        let reason = match handle.await {
            Ok(()) => {
                error!(task = name, "critical task exited cleanly — this should never happen");
                "exited cleanly (unexpected)"
            }
            Err(commonware_runtime::Error::Exited) => {
                error!(task = name, "critical task panicked");
                "panicked (Error::Exited)"
            }
            Err(commonware_runtime::Error::Closed) => {
                info!(task = name, "task stopped (runtime context closed during shutdown)");
                return;  // Normal shutdown, don't abort
            }
            Err(ref e) => {
                error!(task = name, error = %e, "critical task failed");
                "unexpected error"
            }
        };
        info!(task = name, reason, "consensus infrastructure is dead — aborting process");
        ctx.sleep(Duration::from_millis(100)).await;
        std::process::abort();
    });
}
```

**Call site where transport.handle is NOT included** -- `crates/node/runner/src/runner.rs:1459`:
```rust
spawn_consensus_monitor(context, engine_handle, marshal_handle, broadcast_handle);
```

**Transport handle field** -- `crates/network/transport/src/transport.rs:31-34`:
```rust
/// Network handle to keep the network task alive.
///
/// Drop this and the network shuts down.
pub handle: Handle<()>,
```

**Contrast with LegacyNodeService** -- `crates/node/service/src/service.rs:129`:
In the legacy service, the transport handle IS awaited (it is the only task running):
```rust
if let Err(e) = try_join_all(vec![transport.handle]).await {
    tracing::error!(?e, "service task failed");
```
However, in the production runner, the transport handle is consumed by the struct going out of scope but is never awaited or watched.

## Impact

1. **Silent consensus death**: If the P2P transport task panics (e.g., due to a socket error, a bug in commonware's discovery layer, or an OOM in the network buffer allocator), the node loses all ability to send or receive votes, blocks, and certificates. Consensus produces 100% nullifications.

2. **Node appears alive**: The RPC endpoint continues responding to queries. The metrics endpoint returns data. Health checks pass. The node looks healthy to external monitoring -- only the rising nullification rate indicates something is wrong.

3. **No log signal**: The existing consensus monitor only detects engine/marshal/broadcast failures. A transport failure produces no fatal log message, no process abort, and no supervisor restart. The node remains in a "zombie" state indefinitely.

4. **Debugging difficulty**: An operator seeing high nullification rates would need to manually correlate this with transport-level debug logs to identify the root cause. Without monitoring the handle, there is no clear "transport died" signal.

5. **Compounding effect with partition monitor**: The partition monitor (issue 162) also reads stale peer count data, so even the partition health check would not detect the transport death.

## Root Cause

The `spawn_consensus_monitor` function was designed to watch consensus-level actor tasks (engine, marshal, broadcast) but did not account for the transport layer as a separate failure mode. The transport handle was likely omitted because it was assumed the transport would not fail independently of the consensus actors, or because the transport was started first and the handle was not in scope when the monitor was created.

## Suggested Fix

**Option A -- Add to consensus monitor** (recommended, minimal change):

```rust
fn spawn_consensus_monitor(
    context: cw_tokio::Context,
    engine_handle: RuntimeHandle<()>,
    marshal_handle: RuntimeHandle<()>,
    broadcast_handle: RuntimeHandle<()>,
    transport_handle: RuntimeHandle<()>,  // ADD THIS
) {
    spawn_task_watchdog(&context, "consensus_engine", engine_handle);
    spawn_task_watchdog(&context, "marshal_actor", marshal_handle);
    spawn_task_watchdog(&context, "broadcast_engine", broadcast_handle);
    spawn_task_watchdog(&context, "network_transport", transport_handle);  // ADD THIS
}
```

Update the call site at line 1459 to pass `transport.handle`:
```rust
spawn_consensus_monitor(
    context,
    engine_handle,
    marshal_handle,
    broadcast_handle,
    transport.handle,  // Pass transport handle
);
```

Note: this requires ensuring `transport.handle` is still available at line 1459. The `transport` variable may need to be restructured so the handle is extracted before the channels are consumed.

**Option B -- Dedicated watchdog** (if extracting the handle is complex):

```rust
// Immediately after transport creation (line 912):
let transport_handle = transport.handle;  // Move handle out
spawn_task_watchdog(&context, "network_transport", transport_handle);
```

However, this would drop the handle from the transport struct, potentially shutting down the network. The handle must be `clone()`d if `Handle` supports it, or split from the struct carefully.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- add `transport.handle` to `spawn_consensus_monitor` call (line 1459) and update the function signature (line 777)

## Related Issues

- `162-p2p-partition-monitor-stale-peer-count.md` -- partition monitor also fails to detect transport death because it reads stale data
- `101-shutdown-graceful-shutdown.md` -- graceful shutdown should also handle transport lifecycle

## Labels

`bug`, `p2p`, `reliability`, `shutdown`
