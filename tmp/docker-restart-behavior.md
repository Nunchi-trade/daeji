# Docker Restart Behavior and State Persistence

## What Is the Kora Devnet?

Kora is a blockchain node that uses BFT (Byzantine Fault Tolerant) consensus with threshold cryptography. The devnet is a local development network composed of:

- **4 validator nodes** (`validator-node0` through `validator-node3`) -- participate in consensus and produce blocks
- **1 secondary node** (`secondary-node0`) -- follows the chain but does not vote

All 5 nodes run as Docker containers orchestrated by Docker Compose, connected via a bridge network (`kora-net`). The cluster requires a 3-of-4 threshold for consensus, meaning at least 3 validators must be online and in agreement to finalize blocks.

The Docker Compose file is located at:
```
docker/compose/devnet.yaml
```

---

## How Containers Start: The devnet-run.sh Script

The primary startup script (`docker/scripts/devnet-run.sh`) orchestrates a multi-phase boot sequence:

### Phase 0: Build

```bash
docker buildx bake --allow=fs.read=.. -f docker-bake.hcl kora-local
```

Builds the `kora:local` Docker image from the project Dockerfile using cargo-chef for dependency caching. The image contains two binaries: `kora` (the node) and `keygen` (key generation utility).

### Phase 1: Configuration and DKG

Two modes are available:

**Trusted Dealer Mode (fast, for local dev):**
Runs `keygen setup` followed by `keygen dkg-deal` in a single init container. This generates validator keys, peer configuration, AND threshold signature shares all at once using a centralized dealer. No inter-node communication required.

**Interactive DKG Mode (production-like):**
1. Runs `keygen setup` to generate validator keys and `peers.json`
2. Starts 4 DKG containers (`dkg-node0` through `dkg-node3`) that perform a real distributed key generation ceremony over P2P
3. Waits up to 5 minutes for all DKG containers to exit successfully
4. Each node produces `share.key` and `output.json` in its data volume

The script checks for existing DKG outputs before running. If `share.key` and `output.json` already exist in all 4 node volumes (with matching `output.json` checksums), it skips DKG entirely and prints "(cached)".

### Phase 2: Validator Launch

Before starting validators, the script:
1. Stops any running validator containers
2. Calls `clear_runtime_state()` to remove `/data/runtime` from all volumes
3. Starts all validator and secondary containers
4. Waits up to 120 seconds for all 4 validators to report "healthy"
5. Waits up to 120 seconds for the secondary peer to report "healthy"

### Phase 3: Ready

Prints a status table showing each node's health and port mappings.

---

## Peer Discovery and Bootstrap

Non-bootstrap nodes wait for the bootstrap peer before starting:

```bash
# In entrypoint.sh (validator mode):
if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
    while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do
        timeout=$((timeout - 1))
        [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
        sleep 1
    done
fi
```

`validator-node0` is the bootstrap node (`IS_BOOTSTRAP=true`). All other nodes set `BOOTSTRAP_PEERS=node0:30303` and wait for node0's P2P port to become reachable before launching the `kora` binary.

Docker Compose enforces this ordering with `depends_on`:
```yaml
validator-node1:
  depends_on:
    validator-node0:
      condition: service_healthy
```

Nodes 1, 2, and 3 will not even be created until node0 passes its health check.

---

## Container Dependencies and Startup Ordering

```
init-config (or init-setup + dkg-nodes)
    |
    v
validator-node0  (bootstrap, starts first)
    |
    +---> validator-node1  (waits for node0 healthy)
    +---> validator-node2  (waits for node0 healthy)
    +---> validator-node3  (waits for node0 healthy)
    +---> secondary-node0  (waits for node0 healthy)
```

The `condition: service_healthy` dependency means Docker waits for node0 to pass its health check (port 30303 open + `.ready` file exists) before starting dependent nodes. The health check has a 30-second `start_period`, so the earliest dependent nodes can start is ~30 seconds after node0 begins.

---

## State Persistence Map

| State | Storage Type | Survives Restart? | Path |
|-------|-------------|-------------------|------|
| DKG shares | Named Docker volume | Yes | `/data/share.key`, `/data/output.json` |
| Validator key | Named Docker volume | Yes | `/data/validator.key` |
| Peers config | Shared Docker volume (read-only) | Yes | `/shared/peers.json` |
| Genesis config | Shared Docker volume (read-only) | Yes | `/shared/genesis.json` |
| Ledger (QMDB) | Named Docker volume | Yes | `/data/qmdb/` |
| Finalized blocks archive | Named Docker volume | Yes | `/data/kora-finalized-blocks/` |
| Finalization certificates | Named Docker volume | Yes | `/data/kora-finalizations-by-height/` |
| Consensus journals | tmpfs | **No** | `/runtime/` |
| Mempool | In-memory (process) | **No** | (Rust BTreeMap in process memory) |
| Snapshot cache | In-memory (process) | **No** | (Rust HashMap in process memory) |
| Pending RPC transactions | In-memory (process) | **No** | (Rust HashMap in process memory) |

---

## The KORA_RUNTIME_DIR Environment Variable

The `KORA_RUNTIME_DIR` environment variable controls where the Commonware runtime stores consensus journals (write-ahead logs for the BFT engine).

From `crates/node/runner/src/runner.rs`:
```rust
const RUNTIME_DIR_ENV: &str = "KORA_RUNTIME_DIR";

/// Resolve the storage directory used by the Commonware runtime.
///
/// By default this lives under `data_dir/runtime` so validator state survives
/// restarts. Local devnets can set `KORA_RUNTIME_DIR` to put consensus journals
/// on tmpfs and avoid Docker-volume fsync latency.
pub fn runtime_storage_directory(data_dir: &Path) -> PathBuf {
    runtime_storage_directory_from(data_dir, std::env::var_os(RUNTIME_DIR_ENV))
}
```

In the devnet Compose file, all validators set:
```yaml
environment:
  - KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
```

And `/runtime` is mounted as tmpfs:
```yaml
tmpfs:
  - /runtime:size=1g,mode=1777
```

This means consensus journals are stored in RAM. They are fast (no disk fsync latency) but ephemeral (lost on any container stop/restart).

---

## What Happens on `docker compose restart`

When you run:
```bash
docker compose -f compose/devnet.yaml restart validator-node0 validator-node1 validator-node2 validator-node3
```

Or equivalently:
```bash
just restart-validators
```

The following occurs:

1. **Containers stop** -- each validator process receives SIGTERM and shuts down
2. **tmpfs is destroyed** -- the `/runtime` tmpfs mount is wiped; all consensus journals are lost
3. **Containers start again** -- same container IDs, same volume mounts
4. **Named volumes persist** -- DKG shares, validator keys, ledger state all remain intact
5. **Entrypoint runs again** -- `entrypoint.sh` checks for required files, waits for bootstrap peer, launches `kora validator`
6. **Consensus engine initializes fresh** -- since the journal is empty, the engine starts at epoch 0, view 0

### The Problem: Consensus State Lost but DKG Keys Persist

After restart, each node has:
- Its DKG shares (can sign threshold signatures) -- PERSISTED
- Its ledger state (knows what was finalized) -- PERSISTED
- No consensus journal (does not know current view/round) -- LOST

The consensus engine starts fresh and must catch up to the other nodes. If all nodes restart simultaneously, they all start at view 0 together and can re-form consensus normally. If only some nodes restart, the restarted nodes must sync with nodes that are already at a higher view.

---

## What Happens on `docker compose down -v && up`

When you run:
```bash
docker compose -f compose/devnet.yaml --profile observability --profile interactive-dkg down -v
```

Or equivalently:
```bash
just reset
```

The following occurs:

1. **All containers stop and are removed**
2. **All named volumes are deleted** -- this destroys:
   - DKG shares (`share.key`, `output.json`)
   - Validator keys (`validator.key`)
   - Peer configuration (`peers.json`, `genesis.json`)
   - Ledger state (QMDB, finalized blocks, finalization certificates)
   - Shared config volume
3. **Network is removed**

After this, the next `just devnet` or `just trusted-devnet` must perform a complete fresh setup:
- Generate new validator keys
- Run a new DKG ceremony (generating new threshold shares)
- Start validators with empty ledger state at genesis

This is a full reset. The chain starts from block 0 with new cryptographic identities.

---

## When You Need a Full Reset vs When Restart Is Safe

### Restart Is Safe When:

- **All nodes restart together** -- they all start at view 0 and can re-form consensus
- **The chain was healthy before restart** -- no stalled/poisoned mempool state
- **The devnet-run.sh script is used** -- it calls `clear_runtime_state()` which removes `/data/runtime` from volumes, ensuring clean consensus initialization

### Full Reset Is Required When:

- **The chain is permanently stalled** -- bad transactions in mempool cannot be cleared without wiping state
- **DKG shares are corrupted or mismatched** -- nodes cannot form threshold signatures
- **Ledger state has diverged** -- nodes disagree on finalized history
- **You want a completely fresh chain** -- new genesis, new keys, block height 0
- **A node crashed during DKG** -- partial DKG state may be inconsistent

### The `devnet-run.sh` Script Handles Most Cases

The script is idempotent. When run against an existing devnet:
1. Checks if peers.json exists (skips keygen if yes)
2. Checks if DKG shares exist with matching checksums (skips DKG if yes)
3. Stops existing validators
4. Clears runtime state from volumes (`rm -rf /data/runtime`)
5. Starts fresh validators

This means `just devnet` or `just trusted-devnet` is effectively a "safe restart" that preserves DKG shares and peer configuration but gives each node a clean consensus slate.

---

## Health Check Configuration and Its Limitations

### Configuration

```yaml
healthcheck:
  test: ["CMD", "/scripts/healthcheck.sh"]
  interval: 10s
  timeout: 5s
  retries: 3
  start_period: 30s
```

### What the Health Check Tests

The health check script (`docker/scripts/healthcheck.sh`) in `ready` mode:
```bash
[[ -f "/data/.ready" ]] && nc -z localhost 30303
```

It checks two things:
1. The `.ready` file exists (written by `entrypoint.sh` before launching the binary)
2. Port 30303 (P2P) is accepting connections

### What the Health Check Does NOT Test

- Whether consensus is active or progressing
- Whether the node is synchronized with peers
- Whether blocks are being finalized
- Whether the node's mempool is healthy
- Whether the node has any connected peers
- Whether the RPC endpoint is responsive

**A node can be "healthy" from Docker's perspective while completely stalled from a consensus perspective.** This is a significant limitation: the `depends_on: condition: service_healthy` ordering guarantee only ensures the process started and the port opened, not that the node is actually participating in consensus.

### Implications for `restart` Behavior

Because `restart: unless-stopped` is the container restart policy, Docker will automatically restart a crashed validator. After restart, the node will quickly pass its health check (`.ready` file + port open), but it may take much longer to actually rejoin consensus -- or it may never rejoin if the resolver has issues syncing state.

---

## Common Failure Modes During Restart

### 1. Resolver Blocks Peers

When a restarted node tries to catch up, it uses the resolver to request blocks from peers. If the resolver receives data it considers invalid (e.g., blocks referencing parent snapshots it does not have), it may block the sending peer. Once peers are blocked, the node cannot receive further sync data, and it gets stuck at a low view.

**Symptoms:**
- Restarted node stays at view 0 or very low view numbers
- Logs show "invalid data received" from resolver
- `just stats` shows one node as "stalled" while others are "online"

### 2. Height Drift

If one node restarts while others continue producing blocks, a height gap develops. The restarted node must replay all missed blocks from its persisted finalized-blocks archive. If the gap is large, this takes significant time.

**Symptoms:**
- `just health` shows "Height drift" warning (max - min finalized height > 5)
- The restarted node's `finalizedCount` is lower than other nodes
- Other nodes may time out waiting for the restarted node to vote

### 3. Bootstrap Dependency on Restart

If `validator-node0` (the bootstrap node) restarts, other non-bootstrap nodes that restart afterward will wait up to 120 seconds for node0's port to become available. If node0 is slow to start, this introduces cascading delays.

### 4. Stale Container DNS After DKG

The DKG containers and validator containers share hostnames (e.g., both `dkg-node0` and `validator-node0` use hostname `node0`). The `devnet-run.sh` script handles this by stopping validators before running DKG and stopping DKG containers before starting validators. Running these manually out of order can cause Docker DNS to route traffic to the wrong container.

---

## Commands for Common Operations

### Start the Devnet (Interactive DKG)
```bash
cd docker
just devnet
```

### Start the Devnet (Trusted Dealer, Faster)
```bash
cd docker
just trusted-devnet
```

### Restart All Validators (Preserves DKG, Clears Runtime)
```bash
cd docker
just restart
```
This runs `just down` then `just devnet` -- stops everything, then does a full idempotent startup (skips DKG if shares exist, clears runtime state).

### Restart Validators In-Place (Quick, No DKG Check)
```bash
cd docker
just restart-validators
```
This sends SIGTERM and restarts the validator containers. Note: does NOT clear `/data/runtime` from volumes (but tmpfs is wiped anyway since containers restart).

### Full Cluster Reset (Wipes Everything)
```bash
cd docker
just reset
```
Removes all containers, networks, and volumes. Next startup runs from scratch with new keys and a new DKG ceremony.

### Stop Everything (Preserves Volumes)
```bash
cd docker
just down
```

### Redo DKG Ceremony (Keep Existing Peer Config)
```bash
cd docker
just redo-dkg
```
Stops validators, removes old DKG shares, runs interactive DKG ceremony, then restarts validators.

### View Logs
```bash
cd docker
just logs             # All validators + secondary
just logs-node validator-node2   # Single node
just logs-dkg         # DKG ceremony logs
```

### Live Monitoring Dashboard
```bash
cd docker
just stats
```
Displays a real-time terminal dashboard showing per-node status, views, finalized counts, blocks/sec, and leader rotation.

### Health Diagnostic (Requires Prometheus)
```bash
cd docker
just health
```
Queries Prometheus and prints a structured report covering heights, throughput, latency, faults, resources, and network stats.

### Check Single Node Status via RPC
```bash
curl -s http://localhost:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}'
```
RPC ports: node0=8545, node1=8546, node2=8547, node3=8548.

---

## Port Mappings

| Service | P2P Port | RPC Port | Metrics Port |
|---------|----------|----------|--------------|
| validator-node0 | 30400 | 8545 | 9000 |
| validator-node1 | 30401 | 8546 | 9001 |
| validator-node2 | 30402 | 8547 | 9002 |
| validator-node3 | 30403 | 8548 | 9003 |
| secondary-node0 | 30500 | (none) | (none) |

Internal P2P port for all nodes is 30303. The above are the host-mapped ports.

---

## Summary of Restart Semantics

| Operation | DKG Shares | Ledger | Consensus Journals | Mempool | Outcome |
|-----------|-----------|--------|-------------------|---------|---------|
| Container auto-restart (crash) | Kept | Kept | Lost (tmpfs) | Lost | Node restarts at view 0, must sync |
| `just restart-validators` | Kept | Kept | Lost (tmpfs) | Lost | All nodes restart at view 0 together |
| `just restart` (down + devnet) | Kept | Kept | Cleared from volume | Lost | Clean restart with DKG skip |
| `just reset` (down -v) | Deleted | Deleted | Deleted | Lost | Full fresh start required |
