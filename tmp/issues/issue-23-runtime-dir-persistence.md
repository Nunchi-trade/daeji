# KORA_RUNTIME_DIR points to tmpfs -- all consensus state lost on container restart

## Summary

The devnet Docker Compose configuration sets `KORA_RUNTIME_DIR=/runtime`, which maps to a tmpfs mount. The Commonware runtime stores consensus journals, view state, and all persistent runtime data in this directory. Because tmpfs is an in-memory filesystem that is destroyed when the container stops, every container restart wipes the consensus journal, forcing the node to restart at view 1 with no chain history. This is the root cause of the catastrophic restart failures observed during devnet testing: restarted nodes cannot catch up because their entire consensus history is gone.

## Priority

**P0 -- Stability & Correctness**

This is a 5-minute Docker Compose change that prevents the most critical devnet failure mode. It is the root cause underlying both Issue 03 (state sync / crash recovery) and Issue 18 (entrypoint bootstrap wait), and fixing it eliminates the need for a state sync protocol in the short term.

## Problem Description

### The tmpfs mount

**File**: `docker/compose/devnet.yaml`, lines 42-43 (inside `x-validator-common` anchor)

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
    - /runtime:size=1g,mode=1777    # <-- LINE 43: THIS IS THE PROBLEM
```

The `tmpfs` directive creates an in-memory filesystem at `/runtime` with a 1GB size limit. This filesystem exists only for the lifetime of the container process. When the container stops (whether gracefully via SIGTERM or via SIGKILL), the tmpfs is destroyed and all data is lost.

The `x-validator-common` anchor is inherited by all four `validator-nodeN` services AND by `secondary-node0` (via `<<: *validator-common`), so all five services are affected.

### The environment variable

**File**: `docker/compose/devnet.yaml`, line 53 (inside `x-validator-common`)

```yaml
  environment:
    - KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
```

This environment variable defaults to `/runtime`, pointing the Commonware runtime storage at the tmpfs mount. Each validator service (`validator-node0` through `validator-node3`, lines 207-284) and `secondary-node0` (line 303) also redeclares `KORA_RUNTIME_DIR` in their own `environment:` blocks, overriding the anchor.

### How the runtime uses this directory

**File**: `crates/node/runner/src/runner.rs`, lines 95-110

```rust
/// Resolve the storage directory used by the Commonware runtime.
///
/// By default this lives under `data_dir/runtime` so validator state survives
/// restarts. Local devnets can set `KORA_RUNTIME_DIR` to put consensus journals
/// on tmpfs and avoid Docker-volume fsync latency.
#[must_use]
pub fn runtime_storage_directory(data_dir: &Path) -> PathBuf {
    runtime_storage_directory_from(data_dir, std::env::var_os(RUNTIME_DIR_ENV))
}

fn runtime_storage_directory_from(data_dir: &Path, override_dir: Option<OsString>) -> PathBuf {
    match override_dir {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => data_dir.join("runtime"),
    }
}
```

The constant `RUNTIME_DIR_ENV` is defined at line 48:

```rust
const RUNTIME_DIR_ENV: &str = "KORA_RUNTIME_DIR";
```

When `KORA_RUNTIME_DIR` is set (as it is in every validator and secondary container), the Commonware runtime stores its journals at that path instead of under the persistent `/data/runtime` directory. The code comment even acknowledges this is for "tmpfs" to "avoid Docker-volume fsync latency" -- a micro-optimization that causes macro-failures.

### What is stored in the runtime directory

The Commonware `cw_tokio::Runner` stores:
- **Consensus journals**: The simplex BFT protocol's persistent log of views, proposals, notarizations, and finalizations
- **Marshal state**: The block delivery log that tracks which blocks have been finalized
- **Runtime metadata**: Configuration and state markers

Without these journals, a restarted node has no memory of what it voted for, which blocks it finalized, or what the current consensus view is. It effectively becomes a brand-new node joining the network for the first time -- but with stale DKG shares and no state sync protocol to catch up.

### The cascade of failures

1. Container stops (docker stop, crash, OOM kill)
2. tmpfs `/runtime` is destroyed -- all consensus journals lost
3. Container restarts via `restart: unless-stopped`
4. Commonware runtime starts fresh at view 1 with empty journals
5. Node attempts to participate in consensus at view 1 while network is at view N
6. Peer nodes see stale messages and reject/ignore them
7. The restarted node can never catch up without a full cluster reset

### Relationship to other issues

- **Issue 03 (State Sync & Crash Recovery)**: Documents the symptom "restarted nodes can NEVER rejoin." This is the root cause: the node's consensus journal is destroyed, not just missing a state sync protocol.
- **Issue 18 (Entrypoint Bootstrap Wait)**: The unconditional bootstrap wait on restart exists partly to work around the fact that the node starts from scratch. With persistent storage, the wait may be avoidable for warm restarts.
- **Issue 01 (SIGTERM Graceful Shutdown)**: Even with SIGTERM handled, tmpfs destruction on container restart means the graceful shutdown's state flush has nowhere persistent to write.

## Observed Impact

During devnet testing on 2026-05-22:
- Single node failure: 99.9% throughput drop (0.08 blocks/s vs 89 blocks/s baseline)
- Restarted node logs: `invalid data received` from resolver, node gives up catching up
- Two-node restart: permanent network death (0 blocks/s), requires full cluster reset
- Every restart is equivalent to a fresh node join with no state history

## Fix

### Recommended fix: Remove KORA_RUNTIME_DIR from environment (simplest)

Remove the `KORA_RUNTIME_DIR` environment variable from the `x-validator-common` anchor AND from every service that redeclares it. Without it, `runtime_storage_directory()` falls back to `data_dir.join("runtime")`, which resolves to `/data/runtime` -- already on the persistent Docker volume `data_nodeN`. This avoids needing any new volumes.

**File**: `docker/compose/devnet.yaml`

In the `x-validator-common` anchor (starting at line 32), remove line 53:

```yaml
  environment:
    - RUST_LOG=${RUST_LOG:-info}
    - CHAIN_ID=${CHAIN_ID:-1337}
    # REMOVE the following line:
    - KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
    - HEALTHCHECK_MODE=ready
```

Then remove `KORA_RUNTIME_DIR` from each of the five service `environment:` blocks that redeclare it:

- `validator-node0` (line 210): `- KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}`
- `validator-node1` (line 232): `- KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}`
- `validator-node2` (line 255): `- KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}`
- `validator-node3` (line 278): `- KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}`
- `secondary-node0` (line 303): `- KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}`

Also remove the `tmpfs:` block from the `x-validator-common` anchor (lines 42-43):

```yaml
# REMOVE these two lines:
  tmpfs:
    - /runtime:size=1g,mode=1777
```

With both changes applied, all five services will store consensus journals under `/data/runtime` on their existing persistent volumes (`data_node0` through `data_node3` and `data_secondary0`).

### Alternative fix: Use named volumes for /runtime

If keeping `/runtime` as a separate mount is desired (e.g., to isolate consensus journal I/O from state I/O), replace the tmpfs with named Docker volumes.

In the top-level `volumes:` section (`docker/compose/devnet.yaml`, lines 7-17), add:

```yaml
volumes:
  data_node0:
  data_node1:
  data_node2:
  data_node3:
  data_secondary0:
  shared_config:
  startup_barrier:
  runtime_node0:     # NEW
  runtime_node1:     # NEW
  runtime_node2:     # NEW
  runtime_node3:     # NEW
  runtime_secondary0: # NEW
  prometheus_data:
  grafana_data:
  loki_data:
```

Then mount the runtime volume in each service's `volumes:` block:

- `validator-node0` (currently lines 203-206): add `- runtime_node0:/runtime`
- `validator-node1` (currently lines 225-228): add `- runtime_node1:/runtime`
- `validator-node2` (currently lines 248-251): add `- runtime_node2:/runtime`
- `validator-node3` (currently lines 271-274): add `- runtime_node3:/runtime`
- `secondary-node0` (currently lines 297-299): add `- runtime_secondary0:/runtime`

Also remove the `tmpfs:` block from `x-validator-common` (lines 42-43):

```yaml
# REMOVE these two lines:
  tmpfs:
    - /runtime:size=1g,mode=1777
```

### Step 3: Verify recovery after restart

After applying either fix:
1. Deploy the devnet: `docker compose -f devnet.yaml up -d`
2. Wait for consensus to stabilize (30+ seconds of block production)
3. Stop a single validator: `docker stop validator-node2`
4. Wait 10 seconds, then restart: `docker start validator-node2`
5. Verify the node catches up by checking logs for advancing view numbers
6. Confirm block production rate recovers to baseline

## Effort Estimate

**30 minutes** total:
- 5 minutes to edit `devnet.yaml`
- 10 minutes to reset and redeploy the devnet
- 15 minutes to validate restart recovery

## Testing Checklist

- [ ] Remove `tmpfs:` block from `x-validator-common` in `devnet.yaml`
- [ ] Either remove `KORA_RUNTIME_DIR` from all six environment blocks (anchor + 5 services) OR add named runtime volumes for all five services (4 validators + 1 secondary)
- [ ] Deploy fresh devnet and confirm consensus starts normally
- [ ] Stop one validator, wait 10s, restart -- confirm it catches up
- [ ] Stop two validators sequentially, restart both -- confirm network recovers
- [ ] Run `docker compose stop && docker compose start` -- confirm full cluster recovery
- [ ] Verify that blocks/s recovers to baseline within 60 seconds of restart
