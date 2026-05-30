# 10-Node Docker Compose File Not in Version Control -- Configuration Drift

**Category**: docker, config
**Severity**: medium

**Labels**: `bug`, `reliability`, `docker`, `config`

## Summary

The repository only contains a 4-validator Docker Compose file (`docker/compose/devnet.yaml`), but the live 10-node devnet on the remote server runs from a separate compose file (`/opt/kora/docker/compose/devnet-10node.yaml`) that is not tracked in version control. This creates configuration drift between the repository and the running deployment, eliminates code review for production settings, and means the actual deployment configuration exists only on a single server with no disaster recovery.

## Problem

### Only 4-validator compose in the repository

The repository contains only `docker/compose/devnet.yaml`, which defines 4 validators (node0-node3) plus 1 secondary node. The live 10-node devnet at `65.21.232.29` runs from a compose file that was created directly on the server and never committed.

### Operational scripts hardcoded for 4 nodes

Multiple scripts assume a 4-node topology:

**devnet-run.sh** (line 86) -- Hardcoded display:
```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh:86
echo -e "  ${DIM}Chain ID:${NC} ${CHAIN_ID:-1337}  ${DIM}|${NC}  ${DIM}Validators:${NC} 4  ${DIM}|${NC}  ${DIM}Threshold:${NC} 3"
```

**devnet-run.sh** (lines 154, 162-173) -- Hardcoded node loops:
```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh:154
for i in 0 1 2 3; do
    local volume="kora-devnet_data_node${i}"
    # ...
```

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

**devnet-stats.sh** (line 17) -- Hardcoded RPC ports:
```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-stats.sh:17
RPC_PORTS=(8545 8546 8547 8548)
```

**devnet-health.sh** (line 27) -- Hardcoded validator count:
```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-health.sh:27
echo "  Validators up: $(val "$up") / 4"
```

**Justfile** (line 39) -- Hardcoded node list:
```
# /Users/will/dev/nunchi/daeji/docker/Justfile:38-39
restart-validators:
    docker compose -f compose/devnet.yaml restart validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0
```

**devnet.yaml** (lines 85, 122) -- Hardcoded `--validators=4`:
```yaml
# /Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml:85
/usr/local/bin/keygen setup --validators=4 --secondary-peers=1 ...
```

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml` -- Only tracked compose (4 validators + 1 secondary)
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh`, line 86, 154, 162-173, 316-318
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-stats.sh`, line 17
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-health.sh`, line 27
**File**: `/Users/will/dev/nunchi/daeji/docker/Justfile`, line 39

## Impact

- **Single point of failure**: The actual production configuration exists only on one server. A disk failure or accidental deletion loses the deployment configuration entirely, and the 10-node compose must be recreated from memory.
- **No code review**: The 10-node compose has different CPU limits, memory limits, quorum parameters, and port mappings that have never been reviewed.
- **Inconsistent testing**: Developers test against the 4-node compose locally, but the live environment runs a 10-node topology with different resource limits, quorum thresholds (7/10 vs 3/4), and peer connectivity patterns.
- **Operational scripts fail at scale**: Scripts like `devnet-stats.sh` only query 4 RPC ports and `devnet-run.sh` only clears runtime state for 4 nodes, making them incorrect for the 10-node deployment.
- **Security audit gap**: The unversioned config may have different port exposure, volume mounts, or security settings than the reviewed 4-node compose.

## Root Cause

The devnet started as a 4-node setup and the 10-node compose was created directly on the server to test at higher validator counts. No workflow exists to generate compose files for arbitrary validator counts, and no process requires configuration changes to go through version control.

## Suggested Fix

### Option A: Parameterized compose generation (preferred)

Create a generation script (`docker/scripts/generate-compose.sh`) that takes `N_VALIDATORS` as a parameter and produces a valid compose file:

```bash
#!/bin/bash
N=${1:-4}
QUORUM=$(( (2 * N) / 3 + 1 ))
# Generate services for node0..node(N-1)
# Set PEER_NODES, BOOTSTRAP_PEERS, VALIDATOR_COUNT accordingly
# Generate volume declarations, port mappings, etc.
```

### Option B: Add the 10-node compose directly

Commit `docker/compose/devnet-10node.yaml` to the repository alongside the existing 4-node compose.

### Additional cleanup

1. Make `devnet-stats.sh` read the node count dynamically from the compose file or accept a parameter
2. Make `devnet-run.sh` node loops parameterizable (derive from compose file or env var)
3. Make `Justfile` `restart-validators` target accept a compose file argument
4. Update `devnet-health.sh` Prometheus queries to handle N validators instead of hardcoded 4

## Files to Modify

- `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml` -- Only tracked compose (4 validators + 1 secondary), lines 85, 116, 122 (`--validators=4` hardcoded)
- `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh` -- Hardcoded `Validators: 4` (line 86), `for i in 0 1 2 3` loops (line 154), `clear_runtime_state` volumes (lines 162-173), validator stop/start (lines 316-318)
- `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-stats.sh` -- `RPC_PORTS=(8545 8546 8547 8548)` hardcoded (line 17)
- `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-health.sh` -- `Validators up: ... / 4` hardcoded (line 27)
- `/Users/will/dev/nunchi/daeji/docker/Justfile` -- `restart-validators` lists 4 nodes explicitly (line 39)

## Related Issues

- `094-docker-no-qmdb-backup.md` -- No backup mechanism (backup config should be tracked in VCS)
- `097-docker-observability-not-enabled.md` -- Observability not enabled (Prometheus config only scrapes 4 nodes)
- `098-docker-oom-restart-loop.md` -- OOM restart loop (10-node compose has different resource limits)
