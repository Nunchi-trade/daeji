# Graceful Shutdown: Fixed 200ms Sleep and RPC Handle Dropped Without Draining

**Category**: shutdown, reliability
**Severity**: medium

**Labels**: `bug`, `reliability`, `shutdown`, `storage`, `rpc`

## Summary

The validator shutdown path has two problems. First, after receiving SIGTERM/SIGINT, the node sleeps for a fixed 200ms before allowing the runtime to drop all task contexts. If a QMDB commit or archive checkpoint is in-flight (which can take several hundred milliseconds under I/O load), 200ms may not be sufficient, risking a partial write that could corrupt state. Second, the RPC server handle is dropped silently when `run()` returns, immediately cancelling all in-flight HTTP and WebSocket connections without draining. Clients with pending `eth_sendRawTransaction` calls receive connection-reset errors and cannot determine whether their transaction was accepted.

## Problem

### Problem 1: Fixed 200ms grace period

After receiving a shutdown signal, the validator sleeps for exactly 200ms before the runtime drops all task contexts:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs:919-936
tokio::select! {
    _ = tokio::signal::ctrl_c() => {},
    _ = sigterm.recv() => {},
}
info!("Received shutdown signal, initiating graceful shutdown...");

// Allow a brief window for in-flight QMDB commits and log drains
// to complete before the runtime drops all task contexts. The
// watchdog no longer calls abort() on `Error::Closed`, so these
// tasks will terminate cleanly when their contexts are dropped.
tokio::time::sleep(Duration::from_millis(200)).await;

info!("Graceful shutdown complete");
Ok::<(), RunnerError>(())
```

This is a fixed-duration sleep with no awareness of whether in-flight work has actually completed. QMDB persistence happens in a spawned task:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:600-606
if persist_checkpoint {
    let persist_state = state.clone();
    let persist_handle = context
        .child("persist")
        .shared(true)
        .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
    let persist_result = persist_handle
        .await
        .map_err(|err| FinalizationError::PersistTaskFailed(format!("{err}")))?;
}
```

If a persist operation is in progress when the 200ms expires, the task's context is dropped, potentially interrupting a QMDB write.

### Problem 2: RPC handle dropped without draining

The RPC server handle is stored in a variable prefixed with underscore (`_rpc_handle`) and dropped when `run()` returns:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs:1257-1261
// Keep the RPC handle alive so the HTTP and JSON-RPC tasks are not
// cancelled immediately.  The handle is dropped when `run()` returns
// (i.e. after the signal handler completes), which cleanly stops the
// RPC servers during shutdown.
let _rpc_handle = rpc.start();
```

The `RpcServerHandle` struct wraps two `JoinHandle`s but does not store or expose the jsonrpsee `ServerHandle` needed for graceful drain:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs:736-759
pub struct RpcServerHandle {
    http_handle: tokio::task::JoinHandle<()>,
    jsonrpc_handle: tokio::task::JoinHandle<Option<()>>,
}

impl RpcServerHandle {
    pub async fn stopped(self) {
        let _ = tokio::join!(self.http_handle, self.jsonrpc_handle);
    }

    pub fn abort(self) {
        self.http_handle.abort();
        self.jsonrpc_handle.abort();
    }
}
```

The jsonrpsee `ServerHandle` (which has a `stop()` method for graceful shutdown) is consumed inside the spawned task and never stored:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs:727-729
let handle = server.start(module);
handle.stopped().await;  // awaits inside the spawned task
Some(())
```

When `_rpc_handle` is dropped, the inner `JoinHandle`s are dropped, which aborts the tasks, which drops the `ServerHandle`, which immediately closes all connections.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, lines 919-936 (shutdown handler with 200ms sleep)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, lines 1257-1261 (RPC handle stored as `_rpc_handle`)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 727-729 (ServerHandle consumed in spawned task)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 736-759 (RpcServerHandle without ServerHandle)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs`, lines 600-612 (persist task that may be interrupted)

## Impact

- **Data corruption risk**: A partial QMDB write during shutdown could corrupt the state database, requiring a full resync from the last valid checkpoint. This is particularly dangerous because the node has no mechanism to detect a partial write on restart.
- **Unreliable timing**: On busy nodes with heavy I/O, 200ms is insufficient for QMDB persistence. On idle nodes, it adds unnecessary shutdown latency.
- **User-facing data loss**: Clients submitting transactions during shutdown receive connection resets instead of success/failure responses. They cannot determine whether their transaction was accepted into the mempool.
- **Operational ambiguity**: Operators running rolling restarts cannot guarantee that pending RPC calls complete before the node goes down. This makes rolling upgrades unreliable.

## Root Cause

1. The shutdown handler uses a fixed-duration sleep rather than waiting for cooperative completion signals from the finalization pipeline. There is no mechanism for the persist task to signal "I am done" back to the shutdown handler.
2. The `RpcServerHandle` wraps `JoinHandle`s but does not store or expose the jsonrpsee `ServerHandle`. The `ServerHandle` is consumed inside a spawned task (line 728), so there is no way to call `ServerHandle::stop()` from outside to initiate graceful drain.

## Suggested Fix

### Fix 1: Replace fixed sleep with cooperative shutdown signal

Have the `FinalizedReporter` and QMDB commit path expose a signal that resolves when in-flight work completes:

```rust
// Before:
tokio::time::sleep(Duration::from_millis(200)).await;

// After:
tokio::select! {
    _ = in_flight_complete.recv() => { /* clean exit */ }
    _ = tokio::time::sleep(Duration::from_secs(2)) => {
        warn!("Shutdown timeout: forcing exit with in-flight work pending");
    }
}
```

### Fix 2: Expose jsonrpsee `ServerHandle` for graceful drain

```rust
pub struct RpcServerHandle {
    http_handle: tokio::task::JoinHandle<()>,
    jsonrpc_handle: tokio::task::JoinHandle<Option<()>>,
    jsonrpc_server_handle: Option<jsonrpsee::server::ServerHandle>,  // NEW
}

impl RpcServerHandle {
    pub async fn graceful_shutdown(self) {
        if let Some(handle) = self.jsonrpc_server_handle {
            handle.stop().expect("server already stopped");
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::join!(self.http_handle, self.jsonrpc_handle),
        ).await;
    }
}
```

### Restructured shutdown sequence

1. Stop accepting new RPC connections (`rpc_handle.graceful_shutdown()`)
2. Signal consensus to stop proposing (prevent new blocks from starting)
3. Wait for in-flight QMDB commits to complete (cooperative signal with timeout)
4. Drain in-flight RPC responses (wait for pending requests to complete, up to timeout)
5. Force exit if any step exceeds its timeout

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- Replace fixed 200ms sleep (line 932) with cooperative shutdown; restructure shutdown sequence (lines 919-936); use `_rpc_handle` for graceful shutdown (line 1261)
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` -- Modify `RpcServerHandle` to store jsonrpsee `ServerHandle`; add `graceful_shutdown()` method (lines 727-759)
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` -- Add shutdown signal channel for `FinalizedReporter` to check before starting new persist operations (lines 600-612)

## Related Issues

- `098-docker-oom-restart-loop.md` -- OOM restart loop (unclean shutdown compounds data loss during restarts)
- `094-docker-no-qmdb-backup.md` -- No QMDB backup (corrupted state from unclean shutdown would require backup restore)
- `099-e2e-comprehensive-test-coverage.md` -- E2E test coverage (restart tests would exercise the shutdown path)
