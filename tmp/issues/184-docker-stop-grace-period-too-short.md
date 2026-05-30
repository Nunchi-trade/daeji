# Docker `stop_grace_period` of 5s Too Short for QMDB Flush

**Category:** Bug -- Docker/Deployment
**Severity:** Medium

## Summary

The Docker Compose devnet configuration sets `stop_grace_period: 5s` for all validator containers. After this period, Docker sends SIGKILL regardless of whether the Kora node has finished flushing its QMDB state database to disk. Since QMDB persistence is an asynchronous operation that can take several seconds for large state writes, a 5-second grace period creates a significant risk of data corruption on every `docker compose stop` or `docker compose down`, leading to longer restart times due to state replay from the last consistent checkpoint.

## Problem

Kora validators use QMDB (a high-performance state database with 3 partitions: accounts, storage, code) to persist blockchain state. QMDB commits are triggered at a configurable checkpoint interval (default: every 256 blocks, as set by `KORA_CHECKPOINT_INTERVAL` in the compose file). At the network's current throughput of ~33 blocks/s, this means a commit occurs approximately every 7.7 seconds.

When Docker stops a container, it:
1. Sends SIGTERM to the main process.
2. Waits `stop_grace_period` (5 seconds in this configuration).
3. Sends SIGKILL, which cannot be caught or handled.

The validator's shutdown handler needs to:
- Complete any in-progress QMDB commit (which can take multiple seconds for large state flushes).
- Close open file handles and network connections.
- Flush any buffered log output.

With only 5 seconds, if a QMDB commit is in progress when SIGTERM arrives, the process is likely to be killed mid-write by SIGKILL. This leaves the QMDB in an inconsistent state, requiring replay from the last consistent checkpoint on restart.

The configuration itself acknowledges that restarts can be slow: the health check `start_period` is set to 120 seconds (line 66), implying the team expects nodes to take up to 2 minutes to recover after a restart. Reducing the chance of unclean shutdowns would reduce this recovery time.

**File:** `docker/compose/devnet.yaml`, line 41

## Code Reference

```yaml
# docker/compose/devnet.yaml:37-66
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  init: true
  stop_grace_period: 5s                    # <-- Too short for QMDB flush
  stop_signal: SIGTERM
  read_only: true
  security_opt:
    - no-new-privileges:true
  cap_drop:
    - ALL
  ulimits:
    nofile:
      soft: 65536
      hard: 65536
    core: 0
  deploy:
    resources:
      limits:
        memory: 4G
        cpus: "2"
        pids: 4096
  tmpfs:
    - /tmp:size=64m,mode=1777
  healthcheck:
    test: ["CMD", "/scripts/healthcheck.sh"]
    interval: 30s
    timeout: 10s
    retries: 6
    start_period: 120s                     # <-- 120s start period acknowledges slow recovery
```

The checkpoint interval that controls QMDB commit frequency:

```yaml
# docker/compose/devnet.yaml:71
    - KORA_CHECKPOINT_INTERVAL=${KORA_CHECKPOINT_INTERVAL:-256}
```

## Impact

1. **QMDB corruption risk on every container stop.** Every `docker compose stop`, `docker compose down`, `docker compose restart`, or Docker-initiated restart (e.g., due to OOM) risks interrupting a QMDB commit. At 33 blocks/s with commits every 256 blocks (~7.7s), there is roughly a `5/7.7 = 65%` probability that a commit is in progress during any random 5-second window (though the actual commit duration is shorter than the interval, so the real probability depends on the commit duration).
2. **Extended restart times.** After an unclean shutdown, the node must replay all blocks since the last successful checkpoint. With 33 blocks/s and commits every 256 blocks, this means replaying up to 256 blocks of EVM state transitions, which involves re-executing all transactions and recomputing the state root.
3. **Cascading impact on the network.** During replay, the restarting node cannot participate in consensus. If multiple nodes restart simultaneously (e.g., during a rolling deploy), this can temporarily reduce the validator set below quorum threshold.
4. **Silent data loss.** A partially-written QMDB commit may appear valid on restart but contain truncated or corrupted state, potentially leading to state divergence that is only detected much later (see issue 008, catch-up silent state divergence).

## Root Cause

The `stop_grace_period` was set to the Docker default of 5 seconds (actually Docker's default is 10s, so this was explicitly shortened) without accounting for the time QMDB needs to complete in-progress writes and flush buffers to disk. The application's SIGTERM handler and QMDB shutdown behavior were not factored into the container lifecycle configuration.

## Suggested Fix

1. **Increase `stop_grace_period`** to 30 seconds to allow QMDB flush to complete:

   ```yaml
   # BEFORE (docker/compose/devnet.yaml:41)
   stop_grace_period: 5s

   # AFTER
   stop_grace_period: 30s
   ```

2. **Ensure the application handles SIGTERM correctly** by initiating a clean shutdown sequence:
   - Block new consensus participation.
   - Wait for any in-progress QMDB commit to complete.
   - Flush all pending writes.
   - Close file handles and network connections.
   - Exit with code 0.

3. **Consider a pre-stop hook** that signals the node to begin draining before Docker sends SIGTERM:

   ```yaml
   # In the entrypoint script, trap SIGTERM and initiate clean shutdown
   trap 'echo "Received SIGTERM, shutting down..."; kill -TERM $PID; wait $PID' SIGTERM
   ```

## Files to Modify

- `docker/compose/devnet.yaml` -- increase `stop_grace_period` from `5s` to `30s` in the `x-validator-common` anchor

## Related Issues

- [020 - QMDB persistence blocks finalization pipeline](/Users/will/dev/nunchi/daeji/tmp/kora/issues/020-qmdb-persistence-blocks-finalization.md) -- QMDB persistence is already known to be slow; a short grace period makes this worse
- [054 - Crash during replay leaves inconsistent commit marker](/Users/will/dev/nunchi/daeji/tmp/kora/issues/054-recovery-crash-during-replay-inconsistent-marker.md) -- unclean shutdowns can leave the commit marker inconsistent, causing crash-loops on restart
- [101 - No graceful shutdown for QMDB commits and RPC connections](/Users/will/dev/nunchi/daeji/tmp/kora/issues/101-shutdown-graceful-shutdown.md) -- the application currently lacks proper SIGTERM handling; fixing the grace period alone is necessary but not sufficient

## Labels

`bug`, `docker`, `storage`, `reliability`, `shutdown`
