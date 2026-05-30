# Entrypoint Bootstrap Wait Blocks on Every Startup, Not Just First Startup

## Summary

The container entrypoint script (`docker/scripts/entrypoint.sh`) unconditionally blocks non-bootstrap nodes with a `nc -z` connectivity check against the bootstrap peer on every container start, including restarts of nodes that already have persisted state. If the bootstrap node happens to be down when any other node restarts, that node blocks for up to 120 seconds and then exits with an error. Combined with the `restart: unless-stopped` Docker policy, this creates a restart loop where the non-bootstrap node repeatedly starts, waits 120 seconds, exits, and restarts -- unable to join the network until the bootstrap peer comes back.

The bootstrap wait only serves a purpose on first startup (initial peer discovery). On subsequent startups the node already has its DKG keys and peer configuration. It does not need the bootstrap peer to be reachable before launching the `kora` binary -- the Commonware P2P layer handles reconnection internally.

## Problem

### Current behavior

In `docker/scripts/entrypoint.sh`, lines 106-117, the `validator` mode unconditionally runs a bootstrap peer reachability check:

```bash
if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
    BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
    BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)

    log "Waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
    timeout=120
    while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
        timeout=$((timeout - 1))
        [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
        sleep 1
    done
fi
```

The identical pattern is duplicated in the `secondary` mode (lines 134-145).

The check has no awareness of whether the node has been started before. It runs identically on:
- First startup with no data (where bootstrap discovery is genuinely needed)
- Restart after a crash (where the node has all persisted state it needs)
- Restart after a Docker upgrade (same situation)
- Restart triggered by an OOM kill (same situation)

### DKG mode is not affected

The DKG mode (lines 56-86) also has the bootstrap wait (lines 67-79), but it is effectively immune to this issue. Lines 62-65 check whether the DKG has already completed and exit early before reaching the bootstrap wait:

```bash
if [[ -f "${DATA_DIR}/share.key" && -f "${DATA_DIR}/output.json" ]]; then
    log "DKG already completed (share.key exists)"
    exit 0
fi
```

Since `share.key` and `output.json` are on the persistent `/data` volume, this early exit fires on every restart, so the DKG mode never hits the bootstrap wait on non-first-run starts. The DKG mode is also not launched with `restart: unless-stopped` -- it runs as a one-shot service.

### The 120-second timeout and restart loop

The compose file (`docker/compose/devnet.yaml`, line 34) sets `restart: unless-stopped` for all validators. When the bootstrap wait times out at 120 seconds and `error` calls `exit 1`, Docker immediately restarts the container. The restarted container enters the same 120-second wait. This produces a cycle:

```
T+0s:    Container starts
T+0.1s:  Entrypoint begins nc -z loop against bootstrap peer
T+120s:  Timeout reached, "error" exits with code 1
T+120.5s: Docker restarts container (restart: unless-stopped)
T+120.6s: New entrypoint begins nc -z loop against bootstrap peer
T+240.6s: Timeout again, exit 1
...repeats indefinitely until bootstrap peer comes back
```

Each cycle wastes 120 seconds. During this time the node is completely unavailable -- it never reaches the `exec /usr/local/bin/kora validator` command that actually starts the consensus engine.

### Why this matters: cascade during rolling restarts

In the devnet configuration, all three non-bootstrap nodes (node1, node2, node3) have `BOOTSTRAP_PEERS=node0:30303`. Consider this scenario:

1. Node0 (bootstrap) needs a restart for an upgrade
2. Operator stops node0
3. Node2 OOM-crashes independently (observed in testing at 4GB memory limits)
4. Node2's container restarts automatically via `restart: unless-stopped`
5. Node2's entrypoint blocks on `nc -z node0:30303` -- node0 is down for the upgrade
6. Node2 loops for 120 seconds, exits, restarts, loops again
7. Network is now running with only node1 and node3 (2 out of 4) -- below the 3/4 BFT quorum
8. Consensus halts entirely
9. Even after node0 comes back, node2 must complete its current 120-second wait cycle before it can start

This scenario was observed during testing. An unrelated OOM crash on one node, combined with a planned restart of the bootstrap node, caused complete consensus failure because the OOM-crashed node could not restart while the bootstrap was down.

### Where the bootstrap wait does matter

The bootstrap wait is genuinely useful on **first startup**. When a node has never connected to the network before, it needs the bootstrap peer to be available for initial peer discovery via the Commonware P2P layer. Without the bootstrap peer, the new node has no way to find the other validators.

After the first successful startup, the node has on the persistent `/data` volume:
- `/data/validator.key` -- the node's identity key
- `/data/share.key` -- the DKG share key
- `/data/output.json` -- the DKG output with the validator set and public keys
- `/data/genesis.json` -- copied from the shared volume
- `/data/.ready` -- marker file created by the entrypoint
- `/data/last_committed_digest` -- commit marker written by the Kora binary after each finalized block (see `crates/node/runner/src/commit_marker.rs`)

The Commonware P2P runtime handles peer reconnection internally. When the `kora` binary starts, it reads `peers.json` (mounted at `/shared/peers.json`) which contains the addresses of all validators. It does not depend on the bootstrap peer being up first -- it will connect to whichever peers are available and discover the rest through the P2P protocol.

### Additional issue: .ready marker created before bootstrap wait

On line 97 of `entrypoint.sh`, the validator mode creates `${DATA_DIR}/.ready` **before** the `wait_for_barrier` call (line 104) and **before** the bootstrap wait (lines 106-117):

```bash
touch "${DATA_DIR}/.ready"         # line 97 -- created immediately

wait_for_barrier "$VALIDATOR_COUNT" # line 104 -- blocks up to 120s
                                    # lines 106-117 -- bootstrap wait blocks up to 120s

exec /usr/local/bin/kora validator  # line 119 -- actual process starts
```

While the Docker healthcheck (`docker/scripts/healthcheck.sh`) does not check `.ready` -- it checks RPC responsiveness via `eth_chainId` -- the `.ready` marker's premature creation could mislead any tooling or scripts that use it as a readiness signal. The `.ready` file should be created after all blocking startup checks pass, not before.

## Affected Code

### Primary file: `docker/scripts/entrypoint.sh`

The bootstrap wait logic appears in two modes that are affected:

**Validator mode** (lines 106-117):
```bash
if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
    BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
    BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)

    log "Waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
    timeout=120
    while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
        timeout=$((timeout - 1))
        [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
        sleep 1
    done
fi
```

**Secondary mode** (lines 134-145): identical logic.

**DKG mode** (lines 67-79): has the same bootstrap wait code, but is not affected because of the early exit on lines 62-65 (see above).

### Compose configuration: `docker/compose/devnet.yaml`

The restart policy (line 34) and bootstrap peer configuration (lines 236, 259, 282) interact with this bug:

```yaml
x-validator-common: &validator-common
  restart: unless-stopped    # Line 34 -- auto-restarts after exit 1
```

Non-bootstrap nodes all point to node0:
```yaml
  environment:
    - BOOTSTRAP_PEERS=node0:30303   # Lines 236, 259, 282
```

### Storage layout: tmpfs vs persistent volumes

Understanding the storage layout is critical for choosing the correct restart-detection strategy.

**Persistent (`/data` -- named Docker volumes like `data_node0`):**
- `validator.key` -- node identity key
- `share.key` -- DKG share key
- `output.json` -- DKG output
- `genesis.json` -- copied from shared volume
- `.ready` -- marker written by entrypoint
- `last_committed_digest` -- commit marker written by Kora binary (`crates/node/runner/src/commit_marker.rs`)

**Ephemeral (`/runtime` -- tmpfs, wiped on every container restart):**
- Commonware runtime storage (consensus journals)
- Archive databases (`kora-finalizations-by-height`, `kora-finalized-blocks`)
- QMDB state database (`kora-qmdb` with 3 partitions)

The `KORA_RUNTIME_DIR=/runtime` environment variable (set in `devnet.yaml` line 53, read in `crates/node/runner/src/runner.rs` line 48/101-102) redirects the Commonware runtime storage directory from its default location (`/data/runtime`) to the tmpfs mount at `/runtime`. This means all archive and QMDB data is destroyed on every container restart.

**Shared read-only (`/shared` -- `shared_config` volume):**
- `peers.json` -- peer addresses for all validators

**Shared read-write (`/barrier` -- `startup_barrier` volume):**
- `nodeN.ready` -- barrier marker files

## Proposed Fix

### Why the original detection strategy is broken

The original version of this issue proposed checking `${DATA_DIR}/archive` as the restart-detection signal. **This does not work.** Archive and QMDB data are stored under `/runtime` (tmpfs), not `/data`. The tmpfs mount is wiped on every container restart, so there is never any archive data to detect after a restart.

More precisely: in `crates/node/runner/src/runner.rs` lines 95-110, the `runtime_storage_directory()` function resolves the Commonware storage root. When `KORA_RUNTIME_DIR` is set (as it is in the devnet compose file), it overrides the default `data_dir/runtime` path. In the devnet, `KORA_RUNTIME_DIR=/runtime` points to the tmpfs mount. The archive databases (`kora-finalizations-by-height`, `kora-finalized-blocks`) and QMDB (`kora-qmdb`) are all created under this runtime directory by the Commonware runtime, so they all live on tmpfs and are lost on restart.

### Correct detection: check for DKG keys on persistent storage

The simplest and most reliable restart-detection signal is the presence of DKG key files on the persistent `/data` volume. The entrypoint already validates these files exist (lines 92-94 for validator mode):

```bash
[[ -f "${DATA_DIR}/validator.key" ]] || error "validator.key not found"
[[ -f "${DATA_DIR}/share.key" ]] || error "share.key not found (run DKG first)"
[[ -f "${DATA_DIR}/output.json" ]] || error "output.json not found (run DKG first)"
```

If these files exist, the node has completed DKG and participated in at least one startup. It does not need the bootstrap peer to be available. This is the same signal the DKG mode already uses for its early exit (lines 62-65).

An alternative signal is the `last_committed_digest` commit marker file, which is written to `/data` after each finalized block. If this file exists, the node has definitely run before and finalized at least one block. However, this file may not exist on a node that started but crashed before finalizing any blocks, so `share.key` is a more reliable indicator of "has completed initial setup."

### Conditional bootstrap wait

Replace the bootstrap wait in the validator mode (lines 106-117) with:

```bash
if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
    BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
    BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)

    # Only wait for bootstrap on first startup (no existing DKG keys).
    # On restarts, the node already has its keys and peer configuration.
    # The Commonware P2P layer handles reconnection to peers internally.
    #
    # Note: share.key and output.json are on the persistent /data volume,
    # NOT on tmpfs (/runtime), so they survive container restarts.
    # DO NOT use archive or QMDB paths as detection signals -- those are
    # on tmpfs and are wiped on every restart.
    if [[ ! -f "${DATA_DIR}/share.key" ]]; then
        log "First startup: waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
        timeout=120
        while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
            timeout=$((timeout - 1))
            [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
            sleep 1
        done
        log "Bootstrap peer reachable"
    else
        log "Restart detected (share.key exists), skipping bootstrap peer wait"
    fi
fi
```

Apply the same change to the secondary mode (lines 134-145). For secondary mode, check `validator.key` instead (secondaries do not have `share.key`):

```bash
if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
    BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
    BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)

    if [[ ! -f "${DATA_DIR}/validator.key" ]]; then
        log "First startup: waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
        timeout=120
        while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
            timeout=$((timeout - 1))
            [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
            sleep 1
        done
        log "Bootstrap peer reachable"
    else
        log "Restart detected (validator.key exists), skipping bootstrap peer wait"
    fi
fi
```

The DKG mode does not need changes -- it already handles restarts correctly with its early exit on lines 62-65.

### Move .ready marker after blocking checks

Move the `touch "${DATA_DIR}/.ready"` call (line 97) to after the bootstrap wait completes:

```bash
# Current (broken) order:
#   touch "${DATA_DIR}/.ready"         # line 97
#   wait_for_barrier "$VALIDATOR_COUNT" # line 104
#   bootstrap wait                      # lines 106-117
#   exec kora validator                 # line 119

# Fixed order:
#   wait_for_barrier "$VALIDATOR_COUNT"
#   bootstrap wait (conditional)
#   touch "${DATA_DIR}/.ready"
#   exec kora validator
```

### Barrier directory cleanup on restart

The `wait_for_barrier` function (lines 22-48) uses a shared volume (`/barrier`) with `.ready` marker files. The `devnet-run.sh` script clears these markers before starting validators (line 179), but if validators restart without going through `devnet-run.sh` (e.g., Docker auto-restart after a crash), the stale barrier markers cause the barrier check to pass immediately even though not all validators are actually ready. The barrier should also be conditional on first startup -- skip it when `share.key` already exists.

## Alternative Detection Approaches

If checking for `share.key` is not preferred, two other approaches are available:

### Option A: Dedicated `.bootstrap_done` marker on persistent volume

Write a `.bootstrap_done` marker file to `/data` after the first successful bootstrap wait. On subsequent starts, check for this file and skip the wait if it exists. This is more explicit than checking DKG keys but adds another file to manage.

```bash
BOOTSTRAP_MARKER="${DATA_DIR}/.bootstrap_done"
if [[ ! -f "$BOOTSTRAP_MARKER" ]]; then
    # ... run bootstrap wait ...
    touch "$BOOTSTRAP_MARKER"
else
    log "Bootstrap already completed, skipping wait"
fi
```

### Option B: Environment variable flag

Add a `SKIP_BOOTSTRAP_WAIT=true` environment variable that operators can set when restarting containers. This requires manual operator intervention and is more error-prone, but gives explicit control.

### Recommendation

Option B (check for `share.key`) is the recommended approach. It uses existing infrastructure, requires no new files, and has the correct semantics: "this node has completed its initial setup and does not need the bootstrap peer for discovery."

## Impact

**Severity**: Medium. The bug does not cause data loss or consensus corruption, but it causes unnecessary downtime during routine operational scenarios (planned restarts, OOM recovery, rolling upgrades).

**Affected configurations**: Any deployment where non-bootstrap nodes have `BOOTSTRAP_PEERS` set and may restart while the bootstrap node is unavailable. This includes the standard 4-node devnet and any multi-server deployment using the same entrypoint script.

**Operational impact**:
- A planned bootstrap node restart creates a 120-second vulnerability window where any other node that crashes cannot recover
- Rolling restarts must follow a strict ordering (bootstrap node last) to avoid triggering the wait
- OOM crashes on non-bootstrap nodes during bootstrap maintenance cause cascading consensus failure
- Each failed restart cycle wastes 120 seconds of wall time before the node even attempts to start

## How to Reproduce

```bash
# SSH to devnet host
ssh root@65.21.232.29
cd /opt/kora/docker/compose

# Start a clean devnet and wait for convergence
docker compose -f devnet.yaml down --remove-orphans
docker volume prune -f
docker compose -f devnet.yaml up -d
sleep 120

# Verify all nodes are healthy
docker compose -f devnet.yaml ps

# Stop the bootstrap node
docker compose -f devnet.yaml stop validator-node0

# Now restart a non-bootstrap node (simulating OOM or manual restart)
docker compose -f devnet.yaml restart validator-node2

# Watch node2's logs -- it will block on the bootstrap wait
docker compose -f devnet.yaml logs -f validator-node2

# Expected output:
#   [entrypoint] Running validator mode...
#   [entrypoint] Barrier: marked node2 ready (waiting for 4 validators)
#   [entrypoint] Barrier: WARNING timeout after 120s (1/4 ready), proceeding anyway
#   [entrypoint] Waiting for bootstrap peer node0:30303...
#   ... (120 seconds of silence) ...
#   [entrypoint] ERROR: Timeout waiting for bootstrap peer
#   (container exits, Docker restarts it, cycle repeats)

# Start bootstrap node back -- node2 will recover on next cycle
docker compose -f devnet.yaml start validator-node0
```

## Estimated Effort

45-60 minutes. The fix is a conditional check in a shell script with no Rust code changes required, but the original 30-minute estimate assumed checking `${DATA_DIR}/archive` which does not work due to tmpfs. The correct fix requires:
1. Understanding the tmpfs vs persistent volume split (which files survive restart)
2. Choosing the right detection signal (`share.key` on `/data`)
3. Applying the conditional to both validator and secondary modes
4. Moving the `.ready` marker after blocking checks
5. Optionally making the barrier conditional on first startup
6. Testing that the detection works correctly for both first-run and restart scenarios
