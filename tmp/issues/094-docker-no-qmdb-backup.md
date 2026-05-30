# No Backup Mechanism for QMDB Runtime Volumes

**Category**: docker, storage, reliability
**Severity**: medium

**Labels**: `enhancement`, `reliability`, `storage`, `docker`

## Summary

The runtime volumes containing the QMDB state database and finalized block archive are the most valuable data in the deployment but have no backup, snapshot, or disaster recovery mechanism. Docker named volumes grow unboundedly without size limits or disk usage alerting. Worse, the `devnet-run.sh` script actively clears all runtime state on every restart via `clear_runtime_state`, treating QMDB as disposable. A volume loss from host disk failure, accidental `docker volume rm`, or the `just reset` command (which runs `docker compose down -v`) means full resync from genesis.

## Problem

### Volume architecture

The Docker Compose file declares two volume types per node -- a persistent `/data` volume for DKG keys and config, and a `/runtime` volume for QMDB state and archive journals:

```yaml
# /Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml:7-22
volumes:
  data_node0:       # DKG keys, genesis, commit marker (persistent)
  runtime_node0:    # QMDB state + archive journals (volatile in devnet)
  # ... (repeated for each node)
```

The entrypoint script separates these directories:

```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/entrypoint.sh:9, 13
DATA_DIR=${DATA_DIR:-/data}
RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
```

### Runtime state cleared on every restart

The `devnet-run.sh` script calls `clear_runtime_state` before starting validators:

```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh:316-319
docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 >/dev/null 2>&1 || true
clear_runtime_state
clear_startup_barrier
```

Where `clear_runtime_state` wipes all runtime data:

```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh:162-173
clear_runtime_state() {
    for volume in \
        kora-devnet_runtime_node0 \
        kora-devnet_runtime_node1 \
        kora-devnet_runtime_node2 \
        kora-devnet_runtime_node3 \
        kora-devnet_runtime_secondary0; do
        docker volume inspect "$volume" >/dev/null 2>&1 || continue
        docker run --rm -v "${volume}:/runtime" alpine \
            sh -c 'rm -rf /runtime/* /runtime/.[!.]* /runtime/..?*' >/dev/null 2>&1 || true
    done
}
```

### QMDB checkpoint and recovery logic

The runner has configurable checkpoint intervals:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs:168-174
fn checkpoint_interval() -> u64 {
    std::env::var(CHECKPOINT_INTERVAL_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CHECKPOINT_INTERVAL)
}
```

Recovery logic in `restore_checkpoint_and_replay_tail` (runner.rs:360-461) replays blocks from the archive to rebuild state. This replay is sequential and becomes increasingly expensive as the chain grows.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml`, lines 7-22 (volume declarations)
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh`, lines 162-173 (`clear_runtime_state`), lines 316-319 (clears before starting)
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/entrypoint.sh`, lines 9, 13 (`DATA_DIR` vs `RUNTIME_DIR`)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, lines 168-174 (checkpoint interval), lines 288-358 (`recover_finalized_state`), lines 360-461 (`restore_checkpoint_and_replay_tail`)

## Impact

- **Data loss risk**: At approximately 470K blocks and growing, full resync from genesis is increasingly expensive. The archive replay in `restore_checkpoint_and_replay_tail` must replay all blocks sequentially, which can take hours.
- **No disaster recovery**: If the host disk fails, all consensus state is permanently lost. There is no offsite backup, no periodic snapshots, and no recovery runbook.
- **Silent disk exhaustion**: Docker named volumes have no built-in size quotas or alerting. The host disk can fill up silently, causing all 10 nodes to fail simultaneously.
- **OOM compounds the problem**: Nodes that OOM-restart lose their `/runtime` state because Docker restarts trigger a clean start, and subsequent devnet-run.sh invocations call `clear_runtime_state`. This forces a fresh catch-up from genesis each time, making recovery progressively harder.

## Root Cause

1. No backup or snapshot mechanism was implemented alongside the QMDB storage layer.
2. The `devnet-run.sh` script aggressively clears runtime state as a simplicity measure for development, treating QMDB as fully disposable.
3. Docker named volumes have no built-in size quotas or alerting, and no monitoring is in place.

## Suggested Fix

### 1. Periodic QMDB checkpoint snapshots

Add a periodic job (cron or sidecar container) that copies the latest QMDB checkpoint to an external location (S3, rsync, or a separate volume), tagged with the block height for point-in-time recovery. The QMDB checkpoint interval is already configurable via the `KORA_CHECKPOINT_INTERVAL` environment variable (default 256 blocks).

### 2. Add disk usage alerting

Add a `DiskUsageHigh` alert rule to `/Users/will/dev/nunchi/daeji/docker/config/alerts.yml` and include `node-exporter` in the observability stack to expose host-level disk metrics.

### 3. Stop clearing runtime state unconditionally

Modify `devnet-run.sh` to only clear runtime state with an explicit `--clean` flag, preserving QMDB state across restarts by default:

```bash
# Before:
clear_runtime_state

# After:
if [[ "${CLEAN_STATE:-false}" == "true" ]]; then
    clear_runtime_state
fi
```

### 4. Document recovery procedures

Create a runbook covering: disk failure recovery, accidental `docker volume rm`, data corruption scenarios, and cross-server migration.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh` -- Make `clear_runtime_state` conditional (lines 162-173, 316-319)
- `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml` -- Add volume size limits or sidecar backup container (lines 7-22)
- `/Users/will/dev/nunchi/daeji/docker/config/alerts.yml` -- Add disk usage alert (currently no disk alert exists)

## Related Issues

- `098-docker-oom-restart-loop.md` -- OOM restart loop (nodes in crash-loops lose state repeatedly)
- `093-docker-10node-compose-not-in-vcs.md` -- Unversioned compose (backup config should be tracked)
- `097-docker-observability-not-enabled.md` -- Observability not enabled (disk alerting requires monitoring stack)
