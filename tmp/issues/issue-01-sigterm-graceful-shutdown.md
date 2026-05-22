# Kora ignores SIGTERM -- `docker stop` always results in SIGKILL after 30-second timeout

## Status

**Partially fixed by PR #132.** See "What PR #132 Fixed" and "What Remains" sections below.

## Summary

The Kora validator binary does not handle SIGTERM, which is the standard shutdown signal sent by `docker stop`, `systemctl stop`, and orchestration systems like Kubernetes. When Docker sends SIGTERM, the kora process ignores it entirely, Docker waits the full 30-second grace period, then sends SIGKILL. Every container stop is a hard kill with a 30-second delay, resulting in exit code 137, zero shutdown logging, and complete loss of all in-memory state (transaction mempool, snapshot cache, consensus view state). The secondary peer binary has an even more fundamental problem: it blocks on `futures::future::pending::<()>().await`, a future that never resolves, meaning it has no shutdown path at all.

## What PR #132 Fixed

PR #132 (`fix(docker): resource limits, RPC health checks, log rotation, graceful shutdown`, commit `18dd8f9`) made the following changes:

1. **Replaced `futures::future::pending()` with `tokio::signal::ctrl_c()` in `run_standalone()`** -- The validator runner (`crates/node/runner/src/runner.rs` line 480) previously blocked on a future that never resolves. PR #132 changed it to `tokio::signal::ctrl_c().await.ok()`, which at least handles SIGINT. This is an improvement but still does NOT handle SIGTERM (see "What Remains" below).

2. **Added Docker Compose configuration for graceful shutdown** -- Added `stop_grace_period: 30s` and `stop_signal: SIGTERM` to the `x-validator-common` anchor in `docker/compose/devnet.yaml` (lines 35-36).

3. **Added resource limits** -- `deploy.resources.limits` (memory: 4G, cpus: 2) to the validator-common anchor.

4. **Added log rotation** -- `json-file` driver with `max-size: 50m` and `max-file: 5` to the node-common anchor (lines 23-27).

5. **Updated health checks** -- `ready` mode now queries `eth_chainId` via RPC instead of checking port binding.

## What Remains

Three code paths still lack proper SIGTERM handling:

### 1. Validator runner: catches SIGINT only, not SIGTERM

**File**: `crates/node/runner/src/runner.rs`, lines 457-484 (method `ProductionRunner::run_standalone`)

The current code at line 480:
```rust
tokio::signal::ctrl_c().await.ok();
info!("Received shutdown signal, stopping...");
```

`tokio::signal::ctrl_c()` listens exclusively for SIGINT (signal 2), which is sent by pressing Ctrl+C in a terminal. It does NOT listen for SIGTERM (signal 15), which is the signal sent by `docker stop`, `docker compose stop`, `systemctl stop`, Kubernetes pod termination, and `kill <pid>` (default signal).

Since the kora binary runs as PID 1 inside the container (the `entrypoint.sh` uses `exec` to replace the shell at lines 119 and 147), and PID 1 in Linux has special signal semantics where signals without registered handlers are silently dropped rather than triggering the default action, SIGTERM is completely invisible to the process.

### 2. Secondary peer: no shutdown path at all

**File**: `bin/kora/src/cli.rs`, lines 194-246 (method `Cli::run_secondary`)

The relevant code at lines 241-244:
```rust
tracing::info!("secondary peer joined network");
futures::future::pending::<()>().await;
#[allow(unreachable_code)]
Ok::<(), eyre::Error>(())
```

`futures::future::pending::<()>()` creates a future that never completes. There is no signal handling of any kind -- neither SIGINT nor SIGTERM. The `#[allow(unreachable_code)]` annotation on the following line confirms the author knew this code is unreachable. The secondary peer process will always be terminated by SIGKILL. PR #132 did NOT touch this code path.

### 3. Legacy node service: relies on transport handle

**File**: `crates/node/service/src/service.rs`, lines 106-137 (method `LegacyNodeService::run_with_context`)

The relevant code at lines 129-132:
```rust
if let Err(e) = try_join_all(vec![transport.handle]).await {
    tracing::error!(?e, "service task failed");
    return Err(eyre::eyre!("service task failed: {:?}", e));
}
```

The legacy mode (invoked when `kora` is run with no subcommand, dispatched by `Cli::run()` at line 84 of `bin/kora/src/cli.rs`) blocks on a transport handle join that has no signal-aware shutdown. Like the secondary, it has no SIGTERM or SIGINT handler, so it will be SIGKILL'd after the grace period. This code path is used when `kora` is invoked without the `validator`, `dkg`, or `secondary` subcommand. It is not used in the Docker devnet but could be used in other deployment scenarios.

### 4. No `init: true` in Docker Compose

**File**: `docker/compose/devnet.yaml`, lines 32-55 (x-validator-common anchor)

The current compose configuration:
```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  stop_grace_period: 30s
  stop_signal: SIGTERM
  deploy:
    resources:
      limits:
        memory: 4G
        cpus: "2"
  tmpfs:
    - /runtime:size=1g,mode=1777
  healthcheck:
    test: ["CMD", "/scripts/healthcheck.sh"]
    interval: 10s
    timeout: 5s
    retries: 3
    start_period: 30s
```

The compose file correctly configures `stop_signal: SIGTERM` and `stop_grace_period: 30s`, but does not set `init: true`. Without `init: true`, Docker does not inject a tini init process, so the kora binary runs as PID 1. As PID 1, unhandled signals are silently dropped by the kernel rather than triggering the default action (which for SIGTERM would be process termination).

Even if `init: true` were added, it would only be a partial fix: tini would forward SIGTERM to kora, but since kora has no SIGTERM handler registered, the signal would still be ignored. The fix must be made at the Rust binary level. However, `init: true` is still valuable as a defense-in-depth measure and for proper zombie process reaping.

## Impact

### 1. Every `docker stop` wastes 30 seconds and hard-kills the process

The 30-second `stop_grace_period` is entirely wasted because the signal is never handled. Every container stop operation takes 30+ seconds instead of sub-second.

### 2. All in-memory state is lost on every restart

| Component | Storage | Lost on SIGKILL? |
|-----------|---------|------------------|
| Transaction mempool | In-memory (`TransactionPool`) | Yes |
| Snapshot cache | In-memory (`InMemorySnapshotStore`, max 64 entries) | Yes |
| Consensus view state | tmpfs `/runtime` | Yes (tmpfs destroyed) |
| P2P connection state | In-memory | Yes |
| Pending block proposals | In-memory | Yes |
| QMDB account state | Docker volume `/data` | No (survives) |

The critical loss is the consensus state on tmpfs. Even though QMDB data survives on the persistent volume, the consensus runtime state (current view, votes, certifications) is destroyed. Restarted nodes begin at consensus view 1 and cannot catch up to the network.

### 3. Rolling upgrades are impossible

Without graceful shutdown, there is no way to:
- Drain pending RPC requests before stopping
- Complete or cleanly abort the current consensus round
- Flush pending state to persistent storage
- Signal to peers that this validator is intentionally going offline
- Ensure the commit marker is consistent with the last archived block

### 4. Full cluster restart via `docker compose stop` takes 30+ seconds

All 5 containers (4 validators + 1 secondary) ignore SIGTERM simultaneously, all wait 30 seconds, all get SIGKILL. With SIGTERM handling, this would take under 1 second.

## Proposed Solution

The fix has four parts:

1. **Add SIGTERM handling to the validator runner** (`run_standalone` in `runner.rs`) -- upgrade the existing `ctrl_c()` to also catch SIGTERM
2. **Replace the `pending().await` call in the secondary peer** with a signal-aware shutdown (`run_secondary` in `cli.rs`)
3. **Add SIGTERM handling to the legacy node service** (`LegacyNodeService::run` in `service.rs`) -- wrap the transport join with signal awareness
4. **Reduce `stop_grace_period` in Docker Compose** from 30s to 5s and add `init: true`

## Implementation Details

### Step 1: Create a shared shutdown signal helper

Since multiple code paths (validator, secondary, legacy) all need the same signal handling, create a utility function in the CLI utilities crate alongside the existing `Backtracing` and `SigsegvHandler`.

**New file**: `crates/utilities/cli/src/shutdown.rs`

```rust
//! Shutdown signal handler for Kora processes.

/// Wait for either SIGINT (Ctrl+C) or SIGTERM, then return.
///
/// This is the standard shutdown signal handler for Kora processes running
/// in Docker containers or under process supervisors. Docker sends SIGTERM
/// via `docker stop`; Ctrl+C sends SIGINT during interactive use.
///
/// # Panics
///
/// Panics if the SIGTERM handler cannot be registered (should not happen on
/// any supported Unix platform with tokio's `signal` feature enabled).
pub async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate())
        .expect("failed to register SIGTERM handler");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Received SIGINT (Ctrl+C), initiating shutdown...");
        }
        _ = sigterm.recv() => {
            tracing::info!("Received SIGTERM, initiating shutdown...");
        }
    }
}
```

**Modify `crates/utilities/cli/src/lib.rs`** -- add the new module:

Current contents (lines 1-14):
```rust
//! Minimal CLI utilities for the Kora binary.

#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/refcell/kora/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod backtrace;
pub use backtrace::Backtracing;

#[cfg(unix)]
mod sigsegv;
#[cfg(unix)]
pub use sigsegv::SigsegvHandler;
```

Replace with:
```rust
//! Minimal CLI utilities for the Kora binary.

#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/refcell/kora/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod backtrace;
pub use backtrace::Backtracing;

#[cfg(unix)]
mod shutdown;
#[cfg(unix)]
pub use shutdown::wait_for_shutdown_signal;

#[cfg(unix)]
mod sigsegv;
#[cfg(unix)]
pub use sigsegv::SigsegvHandler;
```

**Modify `crates/utilities/cli/Cargo.toml`** -- add tokio and tracing dependencies:

Current contents:
```toml
[package]
name = "kora-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Kora CLI Utilities"

[lints]
workspace = true

[target.'cfg(unix)'.dependencies]
libc = "0.2"
```

Replace with:
```toml
[package]
name = "kora-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Kora CLI Utilities"

[lints]
workspace = true

[dependencies]
tokio = { workspace = true }
tracing = { workspace = true }

[target.'cfg(unix)'.dependencies]
libc = "0.2"
```

The workspace `Cargo.toml` (line 101) already defines `tokio = { version = "1", features = ["full"] }`, which includes the `signal` feature. The `tracing` crate is also already a workspace dependency. No other dependency changes are needed.

### Step 2: Modify `crates/node/runner/src/runner.rs`

In the `run_standalone` method (lines 457-484), replace the SIGINT-only handler with the shared helper.

**Current code** (lines 478-484):
```rust
            let _ledger = self.run(ctx).await?;

            tokio::signal::ctrl_c().await.ok();
            info!("Received shutdown signal, stopping...");
            Ok::<(), RunnerError>(())
        })
    }
```

**Replace with**:
```rust
            let _ledger = self.run(ctx).await?;

            kora_cli::wait_for_shutdown_signal().await;
            info!("Shutdown complete, exiting cleanly");
            Ok::<(), RunnerError>(())
        })
    }
```

Note: `kora_cli` is NOT yet a dependency of `kora-runner`. Add `kora-cli = { workspace = true }` to the `[dependencies]` section of `crates/node/runner/Cargo.toml`.

### Step 3: Modify `bin/kora/src/cli.rs`

In the `run_secondary` method (lines 194-246), replace the `futures::future::pending::<()>().await` with the shared signal handler.

**Current code** (lines 241-244):
```rust
            tracing::info!("secondary peer joined network");
            futures::future::pending::<()>().await;
            #[allow(unreachable_code)]
            Ok::<(), eyre::Error>(())
```

**Replace with**:
```rust
            tracing::info!("secondary peer joined network");

            kora_cli::wait_for_shutdown_signal().await;
            tracing::info!("Secondary peer shutdown complete");
            Ok::<(), eyre::Error>(())
```

Remove the `#[allow(unreachable_code)]` annotation since the `Ok(())` line is now reachable.

### Step 4: Modify `crates/node/service/src/service.rs` (legacy mode)

In the `LegacyNodeService::run_with_context` method (lines 106-137), wrap the transport handle join with signal awareness so that either a signal or transport failure causes exit.

**Current code** (lines 127-135):
```rust
        tracing::info!(chain_id = self.config.chain_id, "kora node initialized");

        if let Err(e) = try_join_all(vec![transport.handle]).await {
            tracing::error!(?e, "service task failed");
            return Err(eyre::eyre!("service task failed: {:?}", e));
        }

        tracing::info!("kora node shutdown");
        Ok(())
```

**Replace with**:
```rust
        tracing::info!(chain_id = self.config.chain_id, "kora node initialized");

        tokio::select! {
            result = try_join_all(vec![transport.handle]) => {
                if let Err(e) = result {
                    tracing::error!(?e, "service task failed");
                    return Err(eyre::eyre!("service task failed: {:?}", e));
                }
            }
            _ = kora_cli::wait_for_shutdown_signal() => {}
        }

        tracing::info!("kora node shutdown");
        Ok(())
```

This requires adding `kora-cli` as a dependency of `kora-service`. It is not currently present in `crates/node/service/Cargo.toml`. Add:
```toml
[dependencies]
kora-cli = { workspace = true }
```

### Step 5: Modify `docker/compose/devnet.yaml`

**Change 1**: Reduce `stop_grace_period` from 30s to 5s in the `x-validator-common` anchor.

**Change 2**: Add `init: true` to the same anchor.

**Current** (lines 32-36):
```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  stop_grace_period: 30s
  stop_signal: SIGTERM
```

**Replace with**:
```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  init: true
  stop_grace_period: 5s
  stop_signal: SIGTERM
```

The `init: true` setting causes Docker to inject tini as PID 1, which:
- Forwards SIGTERM to the kora process (defense-in-depth with the Rust-level handler)
- Reaps zombie child processes (relevant if kora ever spawns subprocesses)
- Provides correct PID 1 signal semantics

## Files to Modify (Summary)

| File | Change |
|------|--------|
| `crates/utilities/cli/src/shutdown.rs` | **New file**: shared `wait_for_shutdown_signal()` helper |
| `crates/utilities/cli/src/lib.rs` | Add `mod shutdown` and `pub use` export |
| `crates/utilities/cli/Cargo.toml` | Add `tokio` and `tracing` dependencies |
| `crates/node/runner/src/runner.rs` | Line 480: replace `tokio::signal::ctrl_c().await.ok()` with `kora_cli::wait_for_shutdown_signal().await` |
| `bin/kora/src/cli.rs` | Lines 241-244: replace `futures::future::pending::<()>().await` with signal handler, remove `#[allow(unreachable_code)]` |
| `crates/node/service/src/service.rs` | Lines 129-132: wrap `try_join_all` with `tokio::select!` and signal handler |
| `crates/node/service/Cargo.toml` | Add `kora-cli = { workspace = true }` to `[dependencies]` |
| `docker/compose/devnet.yaml` | Lines 35-36: change `stop_grace_period: 30s` to `5s`, add `init: true` |

## Testing Plan

### 1. Unit verification: signal handler registration

After building the modified binary, verify the signal mask inside a running container:

```bash
# Start the devnet
docker compose -f devnet.yaml up -d

# Wait for containers to be healthy
docker compose -f devnet.yaml ps

# Check signal mask of kora process (PID 1 inside container, or PID of kora
# if init: true is set -- tini will be PID 1)
docker exec validator-node0 cat /proc/1/status | grep SigCgt

# With init: true, kora will not be PID 1 -- find its PID:
docker exec validator-node0 pgrep kora
docker exec validator-node0 cat /proc/$(pgrep kora)/status | grep SigCgt

# Expected: bit 15 (SIGTERM) should now be set
# The mask should include 0x4000 (bit 15) in addition to existing bits
```

### 2. `docker stop` should exit cleanly in under 5 seconds

```bash
# Verify node is running and producing blocks
curl -s http://localhost:8545 -X POST \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Stop a validator with timing
time docker stop validator-node2

# Expected: exits in < 2 seconds (not 30 seconds)
# Expected exit code: 0 (not 137)
docker inspect validator-node2 --format='{{.State.ExitCode}}'
```

### 3. Shutdown log messages appear

```bash
# Check logs for shutdown messages
docker logs validator-node2 2>&1 | grep -iE 'SIGTERM|shutdown|stopping'

# Expected output should include:
# INFO  Received SIGTERM, initiating shutdown...
# INFO  Shutdown complete, exiting cleanly
```

### 4. `docker compose stop` completes quickly

```bash
time docker compose -f devnet.yaml stop

# Expected: completes in 2-5 seconds (not 30+ seconds)
```

### 5. Secondary peer also handles SIGTERM

```bash
# Stop the secondary peer
time docker stop secondary-node0

# Expected: exits in < 2 seconds
docker inspect secondary-node0 --format='{{.State.ExitCode}}'
# Expected: 0

# Check shutdown logs
docker logs secondary-node0 2>&1 | grep -iE 'SIGTERM|shutdown'
# Expected: shutdown messages present
```

### 6. SIGINT still works (backward compatibility)

```bash
# Run kora in foreground mode (not in Docker)
./target/release/kora validator --data-dir /tmp/test --peers peers.json --chain-id 1337 &
PID=$!

# Send SIGINT (Ctrl+C equivalent)
kill -INT $PID

# Expected: process exits cleanly with shutdown log messages
```

### 7. Full cluster restart timing

```bash
# Time a full stop/start cycle
time docker compose -f devnet.yaml stop
# Expected: < 5 seconds

# Restart
docker compose -f devnet.yaml up -d

# Verify all nodes converge
sleep 10
for port in 8545 8546 8547 8548; do
  curl -s http://localhost:$port -X POST \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
done
```

## References

- PR #132: `fix(docker): resource limits, RPC health checks, log rotation, graceful shutdown` (commit `18dd8f9`) -- partial fix, changed `pending()` to `ctrl_c()` in validator runner only
- Test report: `tmp/graceful-shutdown-test.md` -- full SIGTERM testing methodology and results
- Test report: `tmp/docker-container-lifecycle.md` -- Docker lifecycle deep dive including signal mask analysis and root cause identification
- Test report: `tmp/startup-and-restart-issues.md` -- post-restart recovery failures and state loss characterization
- Test report: `tmp/full-cluster-restart-test.md` -- full cluster restart timing showing 30+ second stop delays
- Source: `crates/node/runner/src/runner.rs` line 480 -- `tokio::signal::ctrl_c().await.ok()` (validator, changed by PR #132 from `pending()`)
- Source: `bin/kora/src/cli.rs` line 242 -- `futures::future::pending::<()>().await` (secondary, NOT changed by PR #132)
- Source: `crates/node/service/src/service.rs` line 129 -- `try_join_all(vec![transport.handle]).await` (legacy mode, no signal handling)
- Source: `crates/utilities/cli/src/sigsegv.rs` line 38 -- SIGSEGV-only signal registration
- Source: `docker/compose/devnet.yaml` lines 32-55 -- `x-validator-common` missing `init: true`
- Tokio docs: [`tokio::signal::unix::signal`](https://docs.rs/tokio/latest/tokio/signal/unix/fn.signal.html)
- Docker docs: [docker stop](https://docs.docker.com/reference/cli/docker/container/stop/) -- sends SIGTERM, waits grace period, then SIGKILL
