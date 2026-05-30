# devnet-run.sh Unconditionally Clears Runtime State Before Starting Validators

**Category:** docker, reliability
**Severity:** medium

## Summary

The `devnet-run.sh` script unconditionally stops all validators and calls `clear_runtime_state` before starting them, erasing QMDB state and the commonware archive from runtime volumes. This happens every time the script runs -- even if the operator intends to restart an existing devnet with preserved state. All finalized block history is lost, and the commit marker on the persistent `/data` volume may become inconsistent with the now-empty runtime volume.

## Problem

Kora is an EVM execution client that stores its blockchain state across two volume types in the Docker devnet:
- **Persistent `/data` volumes** (`data_node0`, etc.): contain validator keys, DKG output, genesis config, and the commit marker (`last_committed_digest`)
- **Runtime `/runtime` volumes** (`runtime_node0`, etc.): contain QMDB state database, commonware archive (finalized blocks), and resolver storage

The `devnet-run.sh` script (in `docker/scripts/devnet-run.sh`) is the primary way to start the devnet. At lines 316-319, it unconditionally clears all runtime state:

```bash
docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 >/dev/null 2>&1 || true
clear_runtime_state
clear_startup_barrier
```

The `clear_runtime_state` function (lines 162-173) removes everything from each runtime volume:

```bash
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

This deletes QMDB data, the finalized block archive, and all commonware storage -- effectively resetting the chain to pre-genesis state while leaving the persistent data intact (including DKG keys, genesis config, and the commit marker).

## Code Reference

File: `docker/scripts/devnet-run.sh`, lines 314-319 (the unconditional clear):

```bash
# Phase 2: Validators and secondary peers
print_phase "2/3" "Starting validators and secondary peers"

docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 >/dev/null 2>&1 || true
clear_runtime_state
clear_startup_barrier
```

File: `docker/scripts/devnet-run.sh`, lines 162-173 (the `clear_runtime_state` function):

```bash
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

Note: The `restart-validators` target in `docker/Justfile` (line 38-39) does NOT clear runtime state:

```
restart-validators:
    docker compose -f compose/devnet.yaml restart validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0
```

This is a viable workaround, but the naming distinction between `just devnet` (destructive) and `just restart-validators` (non-destructive) is not obvious.

## Impact

1. **Data loss**: Running `just devnet` or `./scripts/devnet-run.sh` after a cluster has been running for hours or days destroys all block history and chain state. The cluster starts from genesis, discarding potentially significant chain state. There is no confirmation prompt or warning.

2. **Commit marker mismatch**: The persistent data volume retains the commit marker file (`last_committed_digest`) which points to the last finalized block. After runtime state is wiped, the archive and QMDB are empty. On the next startup, `restore_checkpoint_and_replay_tail()` (in `crates/node/runner/src/runner.rs`, line 360) reads the commit marker, tries to find the corresponding block in the (now empty) archive, and fails with: "commit marker does not match any archived block; cannot safely determine QMDB state height". This causes a startup abort that requires manual intervention (deleting the commit marker file from the data volume).

3. **Operational surprise**: An operator running `just devnet` might reasonably expect it to start the existing devnet, not wipe it. The destructive behavior is the default, with no opt-out flag. The non-destructive `just restart-validators` exists but only restarts -- it does not handle cases where containers do not exist yet.

## Root Cause

The script was designed for a development workflow where starting fresh every time is the expected behavior. As the devnet became longer-running and restart testing became important (e.g., for testing node recovery after crash), the always-clean behavior was not revisited.

## Suggested Fix

Make the destructive behavior opt-in via an environment variable:

**Before** (`docker/scripts/devnet-run.sh`, lines 316-319):
```bash
docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 >/dev/null 2>&1 || true
clear_runtime_state
clear_startup_barrier
```

**After**:
```bash
docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 >/dev/null 2>&1 || true

CLEAN=${CLEAN:-false}
if [[ "$CLEAN" == "true" ]]; then
    print_success "Clearing runtime state (CLEAN=true specified)"
    clear_runtime_state
    clear_startup_barrier
else
    # Only clear the startup barrier (safe to clear even with preserved state)
    clear_startup_barrier
    print_success "Preserving existing runtime state (use CLEAN=true to start fresh)"
fi
```

Operators who want the old behavior can run `CLEAN=true just devnet`. The `just reset` target (which already runs `docker compose down -v`) remains the nuclear option for completely fresh starts.

## Files to Modify

- `docker/scripts/devnet-run.sh` -- make `clear_runtime_state` conditional on `CLEAN=true` at lines 316-319

## Related Issues

- `054-recovery-crash-during-replay-inconsistent-marker.md` -- the commit marker vs. runtime state mismatch created by this script is the same failure mode described in that issue
- `055-docker-no-config-toml-in-entrypoint.md` -- configuration inflexibility in the Docker setup

## Labels

bug, docker, reliability
