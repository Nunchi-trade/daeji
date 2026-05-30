# Docker: Missing resource limits, consensus-aware health checks, and graceful shutdown

**Severity:** Medium-High
**Component:** `docker/compose/devnet.yaml`, `docker/scripts/healthcheck.sh`, `docker/scripts/entrypoint.sh`, `crates/node/runner/src/runner.rs`
**Labels:** infrastructure, docker, production-readiness

## Summary

The Docker Compose devnet deployment has multiple gaps that prevent production use: no container resource limits, health checks that only validate port binding (not consensus health), no graceful shutdown handling, no log rotation, a DKG race condition at startup, and tmpfs for runtime journals causing state loss on container restart.

## Background

Kora is a BLS12-381 threshold-signature blockchain built on the Commonware consensus framework (simplex). The devnet runs a 4-validator + 1 secondary-peer cluster plus an observability stack (Prometheus, Grafana, Loki, Promtail) via Docker Compose, deployed on a Hetzner dedicated server.

The Compose file is at `docker/compose/devnet.yaml`. It defines:
- An `x-node-common` YAML anchor (lines 18-24) with the base image, network, `RUST_LOG`, and `CHAIN_ID`
- An `x-validator-common` YAML anchor (lines 26-41) that extends `x-node-common` with restart policy, tmpfs, health check, and `KORA_RUNTIME_DIR`
- 4 validator containers (`validator-node0` through `validator-node3`, lines 186-272)
- 1 secondary (follower) container (`secondary-node0`, lines 274-292)
- Init containers for key generation and DKG ceremony (lines 44-184)
- Observability stack (Prometheus, Loki, Promtail, Grafana) behind the `observability` Compose profile (lines 294-353)

The startup flow is orchestrated by `docker/scripts/devnet-run.sh`, which builds the image, runs key generation + DKG, starts validators, and waits for health checks to pass.

---

## Issue 1: No container resource limits (HIGH)

### Problem

The `x-validator-common` anchor and all service definitions have no `mem_limit`, `cpus`, `memswap_limit`, or `ulimits` settings. A single validator that leaks memory (the `HighMemoryUsage` alert fires at 2 GB RSS -- see `docker/config/alerts.yml:95-102`) can OOM the entire host and take down the other three validators, breaking quorum.

Current anchor (`docker/compose/devnet.yaml:26-41`; note that lines 18-25 define the separate `x-node-common` anchor):

```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  tmpfs:
    - /runtime:size=1g,mode=1777
  healthcheck:
    test: ["CMD", "/scripts/healthcheck.sh"]
    interval: 10s
    timeout: 5s
    retries: 3
    start_period: 30s
  environment:
    - RUST_LOG=${RUST_LOG:-info}
    - CHAIN_ID=${CHAIN_ID:-1337}
    - KORA_RUNTIME_DIR=${KORA_RUNTIME_DIR:-/runtime}
    - HEALTHCHECK_MODE=ready
```

No `deploy.resources.limits` block exists anywhere in the file. Note that the observability containers (Prometheus, Loki, Grafana, Promtail at lines 294-353) also lack resource limits.

**Important YAML merge note:** Each validator service (e.g., `validator-node0` at line 186) uses `<<: *validator-common` but also defines its own `environment:` block. YAML merge semantics mean the service-level `environment` completely replaces the anchor's `environment` -- it does not append. This is why each service repeats `RUST_LOG`, `CHAIN_ID`, and `KORA_RUNTIME_DIR`. Any environment variable added to `x-validator-common` must also be added to each individual service's `environment` block.

### Fix

Add resource constraints to the shared validator anchor. Note: `deploy.resources.limits` requires Docker Compose v2 (`docker compose` CLI, not the legacy `docker-compose`). The Justfile already uses `docker compose` (see `docker/Justfile`).

```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  deploy:
    resources:
      limits:
        memory: 4G
        cpus: '2.0'
      reservations:
        memory: 1G
        cpus: '0.5'
  ulimits:
    nofile:
      soft: 65536
      hard: 65536
  tmpfs:
    - /runtime:size=1g,mode=1777
  # ... rest unchanged (healthcheck, environment)
```

The observability containers should also get limits (Prometheus and Loki can consume significant memory):

```yaml
  prometheus:
    # ... existing config ...
    deploy:
      resources:
        limits:
          memory: 2G
          cpus: '1.0'

  loki:
    # ... existing config ...
    deploy:
      resources:
        limits:
          memory: 1G
          cpus: '0.5'

  grafana:
    # ... existing config ...
    deploy:
      resources:
        limits:
          memory: 512M
          cpus: '0.5'
```

---

## Issue 2: tmpfs for runtime journals (MEDIUM)

### Problem

Consensus journals (Commonware simplex WAL, finalized block archives, finalization certificates) are stored in a tmpfs mount at `/runtime` with a 1 GB size cap. This data is destroyed on every container restart.

From `docker/compose/devnet.yaml:29-30`:

```yaml
  tmpfs:
    - /runtime:size=1g,mode=1777
```

The `KORA_RUNTIME_DIR` environment variable (line 40) points to this tmpfs path. Inside the Rust code at `crates/node/runner/src/runner.rs:74-76`, this directory is resolved and passed to the Commonware runtime:

```rust
pub fn runtime_storage_directory(data_dir: &Path) -> PathBuf {
    runtime_storage_directory_from(data_dir, std::env::var_os(RUNTIME_DIR_ENV))
}
```

When `KORA_RUNTIME_DIR=/runtime` is set, the runtime stores its journals there instead of the default `data_dir/runtime` (which lives on a named Docker volume). The startup script `docker/scripts/devnet-run.sh` even explicitly clears runtime state before each launch (lines 309-311):

```bash
docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 >/dev/null 2>&1 || true
clear_runtime_state
```

This means:
1. Any container restart loses all consensus journal state
2. The resolver catch-up mechanism must replay from scratch, which has known issues (see the `ResolverPeersBlocked` alert at `docker/config/alerts.yml:181-188`)
3. The 1 GB tmpfs cap can be exhausted if archive data grows, causing write failures

### Fix

Replace tmpfs with a named volume for `KORA_RUNTIME_DIR`:

```yaml
volumes:
  runtime_node0:
  runtime_node1:
  runtime_node2:
  runtime_node3:
  runtime_secondary0:
```

Then in each validator service, mount a dedicated volume:

```yaml
  validator-node0:
    <<: *validator-common
    volumes:
      - shared_config:/shared:ro
      - data_node0:/data
      - runtime_node0:/runtime
    # Remove tmpfs from x-validator-common
```

And remove the `tmpfs` block from `x-validator-common`. The `clear_runtime_state` function in `devnet-run.sh` (lines 162-173) can remain for fresh-start scenarios but should not run by default.

---

## Issue 3: Health check only validates port binding (MEDIUM)

### Problem

The health check script at `docker/scripts/healthcheck.sh` has three modes:

```bash
case "$MODE" in
    dkg)
        [[ -f "/data/share.key" && -f "/data/output.json" ]]
        ;;
    p2p)
        nc -z localhost 30303
        ;;
    ready)
        [[ -f "/data/.ready" ]] && nc -z localhost 30303
        ;;
esac
```

All validators use `HEALTHCHECK_MODE=ready` (set in `docker/compose/devnet.yaml:41`). The `ready` mode checks that:
1. A `.ready` sentinel file exists (created by `entrypoint.sh` at line 64: `touch "${DATA_DIR}/.ready"`)
2. TCP port 30303 is accepting connections (`nc -z localhost 30303`)

Neither of these verifies that the consensus engine is actually functioning. A node where the voter actor has panicked (a known failure mode -- the `VoterCrash` alert exists for this at `docker/config/alerts.yml:25-35`) will still pass the health check because the P2P listener remains bound. The `depends_on: condition: service_healthy` chain in the Compose file (e.g., lines 208-210) then allows dependent services to start against a node that is not participating in consensus.

The RPC endpoint `kora_nodeStatus` (defined in `crates/node/rpc/src/kora.rs:15`) returns a `NodeStatus` struct (from `crates/node/rpc/src/state.rs:99-118`) that includes `currentView`, `finalizedCount`, `peerCount`, and `isLeader` fields. All fields use `camelCase` serialization (line 98: `#[serde(rename_all = "camelCase")]`). The `devnet-stats.sh` script already queries this endpoint for its live dashboard. Inside the container, the RPC server listens on port 8545.

### Fix

Add a `consensus` health check mode that verifies the view is advancing:

```bash
#!/bin/bash
set -e

MODE="${HEALTHCHECK_MODE:-p2p}"

case "$MODE" in
    dkg)
        [[ -f "/data/share.key" && -f "/data/output.json" ]]
        ;;
    p2p)
        nc -z localhost 30303
        ;;
    ready)
        [[ -f "/data/.ready" ]] && nc -z localhost 30303
        ;;
    consensus)
        # Check that the RPC server responds and consensus view is advancing
        RESULT=$(curl -sf --max-time 2 -X POST -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' \
            http://localhost:8545 2>/dev/null)
        VIEW=$(echo "$RESULT" | jq -r '.result.currentView // 0')
        [[ "$VIEW" -gt 0 ]]
        ;;
    *)
        exit 1
        ;;
esac
```

Then update the validator anchor to use this mode once validators have had time to start consensus:

```yaml
  healthcheck:
    test: ["CMD", "/scripts/healthcheck.sh"]
    interval: 10s
    timeout: 5s
    retries: 5
    start_period: 60s
  environment:
    # ...
    - HEALTHCHECK_MODE=consensus
```

For a more robust check, compare the current view against a previously saved value to ensure it is still advancing (not just nonzero). This could be done by storing the view in a temp file and comparing on the next health check invocation.

---

## Issue 4: DKG race condition at startup (MEDIUM)

### Problem

The interactive DKG ceremony (`docker/compose/devnet.yaml:100-184`) starts DKG nodes in sequence using `depends_on: condition: service_started`. The `service_started` condition only waits for the container process to start -- it does not wait for P2P connectivity to be established.

From `docker/compose/devnet.yaml:124-128`:

```yaml
  dkg-node1:
    depends_on:
      init-setup:
        condition: service_completed_successfully
      dkg-node0:
        condition: service_started
```

The entrypoint script (`docker/scripts/entrypoint.sh:34-46`) does wait for the bootstrap peer's TCP port to be reachable:

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

However, TCP port reachability does not mean the node has completed its P2P handshake and is ready for DKG messages. There is a window between port bind and authenticated peer discovery completing where DKG messages can be lost. The DKG ceremony (`crates/node/dkg/src/ceremony.rs`) broadcasts dealer messages and ready signals -- if a peer is not fully connected when these are sent, the ceremony can stall or fail.

For the trusted-dealer path, this is less of an issue because the DKG is computed offline. But for the interactive DKG (the production-recommended path), this race can cause ceremony failures, especially on slow networks.

### Fix

Add a readiness probe to the DKG containers that verifies P2P connectivity is established, or add a startup delay. A simpler approach is to use the DKG health check mode:

```yaml
  dkg-node0:
    healthcheck:
      test: ["CMD", "/scripts/healthcheck.sh"]
      interval: 5s
      timeout: 3s
      retries: 5
      start_period: 15s
    environment:
      - HEALTHCHECK_MODE=p2p
```

Then change dependent DKG nodes to wait for `service_healthy` instead of `service_started`:

```yaml
  dkg-node1:
    depends_on:
      init-setup:
        condition: service_completed_successfully
      dkg-node0:
        condition: service_healthy
```

A more thorough fix would add a DKG-specific readiness endpoint to the Kora binary that confirms the authenticated P2P discovery has completed peer handshakes for all expected participants.

---

## Issue 5: No graceful shutdown handling (MEDIUM)

### Problem

The Kora binary has no SIGTERM handler. The entrypoint script (`docker/scripts/entrypoint.sh`) uses `exec` to replace the shell with the Kora process (e.g., line 79: `exec /usr/local/bin/kora validator ...`), so Docker's `STOPSIGNAL` (SIGTERM by default) goes directly to the Kora process.

Searching the codebase for signal handling reveals:
- `crates/utilities/cli/src/sigsegv.rs` handles SIGSEGV for crash diagnostics
- `crates/node/service/src/service.rs:134` has a log line `tracing::info!("kora node shutdown")` but no actual shutdown orchestration
- No SIGTERM/SIGINT handler exists anywhere in the `bin/kora/` or `crates/node/` code

Without a SIGTERM handler:
1. QMDB state may not be flushed to disk (pending writes lost)
2. Commonware runtime journals may be left in an inconsistent state
3. Active P2P connections are dropped without a graceful disconnect
4. `docker stop` falls back to SIGKILL after the 10-second default grace period

The `run_standalone` method (`crates/node/runner/src/runner.rs:295-321`) blocks on `futures::future::pending::<()>().await` at line 318 -- it will never return unless the task is killed:

```rust
// crates/node/runner/src/runner.rs:295-321
pub fn run_standalone(self, config: kora_config::NodeConfig) -> Result<(), RunnerError> {
    use commonware_runtime::Runner;
    use kora_transport::NetworkConfigExt;

    let runtime_dir = runtime_storage_directory(&config.data_dir);
    info!(runtime_dir = %runtime_dir.display(), "Starting Commonware runtime");
    let executor =
        cw_tokio::Runner::new(cw_tokio::Config::default().with_storage_directory(runtime_dir));
    executor.start(|context| async move {
        let validator_key = config
            .validator_key()
            .map_err(|e| anyhow::anyhow!("failed to load validator key: {}", e))?;

        let transport = config
            .network
            .build_local_transport(validator_key, context.clone())
            .map_err(|e| anyhow::anyhow!("failed to build transport: {}", e))?;

        let ctx =
            kora_service::NodeRunContext::new(context, std::sync::Arc::new(config), transport);

        let _ledger = self.run(ctx).await?;

        futures::future::pending::<()>().await;  // line 318: blocks forever
        Ok::<(), RunnerError>(())
    })
}
```

### Fix

Add a SIGTERM handler that initiates graceful shutdown:

```rust
pub fn run_standalone(self, config: kora_config::NodeConfig) -> Result<(), RunnerError> {
    use commonware_runtime::Runner;
    let runtime_dir = runtime_storage_directory(&config.data_dir);
    let executor = cw_tokio::Runner::new(
        cw_tokio::Config::default().with_storage_directory(runtime_dir)
    );
    executor.start(|context| async move {
        // ... setup ...
        let _ledger = self.run(ctx).await?;

        // Wait for shutdown signal
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Received shutdown signal, flushing state...");

        // Flush QMDB and close connections
        // Drop transport handle to close P2P connections
        // Allow Commonware runtime to flush journals

        Ok::<(), RunnerError>(())
    })
}
```

Also extend Docker's stop grace period in the Compose file:

```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  stop_grace_period: 30s
  # ...
```

---

## Issue 6: No log rotation (HIGH)

### Problem

No logging driver configuration exists in the Compose file. Docker defaults to the `json-file` logging driver with no size limit. On a long-running devnet, validator logs (especially at `info` or `debug` level) will fill the disk.

The default `RUST_LOG` level is `info` (line 23 in `x-node-common` and line 38 in `x-validator-common`: `RUST_LOG=${RUST_LOG:-info}`), and validators produce log output for every finalized block, view transition, and P2P event. Over days of operation, this can consume tens of gigabytes.

### Fix

Add a global logging configuration to each service via the shared anchor:

```yaml
x-validator-common: &validator-common
  <<: *node-common
  restart: unless-stopped
  logging:
    driver: json-file
    options:
      max-size: "100m"
      max-file: "5"
      compress: "true"
  # ...
```

Alternatively, add logging config to the `x-node-common` anchor so all containers (including init and DKG) benefit. The current `x-node-common` anchor at lines 18-24 is:

```yaml
x-node-common: &node-common
  image: kora:local
  networks:
    - kora-net
  environment:
    - RUST_LOG=${RUST_LOG:-info}
    - CHAIN_ID=${CHAIN_ID:-1337}
```

Add the logging block:

```yaml
x-node-common: &node-common
  image: kora:local
  networks:
    - kora-net
  logging:
    driver: json-file
    options:
      max-size: "50m"
      max-file: "3"
  environment:
    - RUST_LOG=${RUST_LOG:-info}
    - CHAIN_ID=${CHAIN_ID:-1337}
```

The Promtail container already scrapes Docker logs via the Docker socket (`docker/compose/devnet.yaml:330: /var/run/docker.sock:/var/run/docker.sock:ro`), so local log files are only needed as a buffer.

---

## Issue 7: Bootstrap peer hardcoded to node0 (MEDIUM)

### Problem

All non-bootstrap validators and the secondary peer are configured to bootstrap exclusively from `node0`:

From `docker/compose/devnet.yaml:221`:
```yaml
  - BOOTSTRAP_PEERS=node0:30303
```

This pattern repeats for `validator-node1` (line 221), `validator-node2` (line 245), `validator-node3` (line 268), and `secondary-node0` (line 289).

If `validator-node0` crashes or restarts before the other nodes have established peer connections through discovery, the remaining nodes cannot discover each other. The P2P layer (Commonware authenticated discovery) uses bootstrappers only for initial peer introduction -- once connected, peers exchange peer lists. But if the initial bootstrap fails, no connections are made.

The `depends_on: condition: service_healthy` chain (e.g., line 208-210) ensures node0 is healthy before others start, but a later crash of node0 during a full cluster restart leaves the other nodes unable to re-establish connectivity.

### Fix

Configure mutual bootstrapping so each node lists multiple peers:

```yaml
  validator-node1:
    environment:
      - BOOTSTRAP_PEERS=node0:30303,node2:30303,node3:30303

  validator-node2:
    environment:
      - BOOTSTRAP_PEERS=node0:30303,node1:30303,node3:30303

  validator-node3:
    environment:
      - BOOTSTRAP_PEERS=node0:30303,node1:30303,node2:30303
```

This requires updating the entrypoint script to parse multiple comma-separated bootstrap peers and wait for at least one to be reachable. The current entrypoint (`docker/scripts/entrypoint.sh`) handles bootstrap peers identically in three places -- DKG mode (lines 34-46), validator mode (lines 66-77), and secondary mode (lines 94-105) -- and each only handles a single `HOST:PORT` pair:

```bash
BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)
```

The entrypoint should be updated to:
1. Split `BOOTSTRAP_PEERS` on commas
2. Iterate over each `HOST:PORT` pair
3. Proceed once at least one is reachable (rather than requiring a specific one)
4. Pass all peers to the `--bootstrap-peers` flag

Note: The `kora` binary's bootstrap peer format uses `PUBLIC_KEY_HEX@HOST:PORT` (see `crates/network/transport/src/config.rs:144-171`). The entrypoint currently does not pass the `BOOTSTRAP_PEERS` env var directly to the binary -- the binary reads bootstrap peers from `peers.json`. The entrypoint's `nc -z` wait loop is only for startup ordering, not for configuring the binary.

---

## Testing Plan

1. **Resource limits** -- Deploy with limits, then run the load generator (`bin/loadgen`) at maximum throughput. Verify that no single container exceeds its memory/CPU limit. Check that `docker stats` shows enforcement.

2. **tmpfs vs. named volume** -- Start devnet, finalize 1000+ blocks, restart a single validator with `docker compose restart validator-node2`. Verify that the restarted node catches up without errors (no `ResolverPeersBlocked` alert). Compare catch-up time between tmpfs (clean start) and named volume (journal replay).

3. **Consensus health check** -- Kill the voter actor by sending SIGUSR1 to the inner process (or inject a panic). Verify that the health check transitions to `unhealthy` within 30 seconds. Verify that `docker compose ps` reflects the unhealthy state.

4. **DKG race** -- Run the interactive DKG ceremony 10 times with `just devnet`. Track success rate. Compare with the fix (using `service_healthy` for DKG dependencies). Expect 100% success rate with the fix.

5. **Graceful shutdown** -- Run `docker compose stop validator-node1` and check logs for the "flushing state" message. Verify that the container exits with code 0 (not 137/SIGKILL). Restart the node and verify it recovers from its journal without needing full catch-up.

6. **Log rotation** -- Run the devnet for 24 hours under load. Verify that per-container log files do not exceed 100 MB each. Check that old log files are rotated and compressed.

7. **Bootstrap resilience** -- Start devnet, stop `validator-node0`, then restart all other validators simultaneously. Verify that they can discover each other and resume consensus without node0.

---

## Verification Steps

After implementing all fixes, run the following checks to confirm correctness:

```bash
# 1. Validate the Compose file parses without errors
cd docker && just validate

# 2. Verify resource limits are applied
docker compose -f compose/devnet.yaml config | grep -A5 "resources"

# 3. Start the devnet and check resource enforcement
cd docker && just trusted-devnet
docker stats --no-stream  # verify MEM LIMIT column shows 4GiB for validators

# 4. Test the consensus health check
# After devnet is running:
docker exec kora-devnet-validator-node0-1 /scripts/healthcheck.sh
echo $?  # should exit 0

# Query the RPC endpoint directly to confirm view is advancing:
curl -sf -X POST -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' \
    http://localhost:8545 | jq '.result.currentView'

# 5. Verify log rotation is configured
docker inspect kora-devnet-validator-node0-1 --format '{{.HostConfig.LogConfig}}'
# should show max-size and max-file settings

# 6. Verify named volumes replaced tmpfs (if Issue 2 fix applied)
docker inspect kora-devnet-validator-node0-1 --format '{{json .Mounts}}' | jq '.[] | select(.Destination=="/runtime")'
# should show Type: "volume" not "tmpfs"

# 7. Test graceful shutdown (if Issue 5 fix applied)
docker compose -f compose/devnet.yaml stop validator-node1
docker inspect kora-devnet-validator-node1-1 --format '{{.State.ExitCode}}'
# should be 0, not 137 (SIGKILL)

# 8. Test bootstrap resilience (if Issue 7 fix applied)
docker compose -f compose/devnet.yaml stop validator-node0
docker compose -f compose/devnet.yaml restart validator-node1 validator-node2 validator-node3
# nodes should form consensus without node0
just stats  # verify 3 nodes are online and finalizing
```

---

## Related Files

| File | Description |
|------|-------------|
| `docker/compose/devnet.yaml` | Main Compose file: `x-node-common` (L18-24), `x-validator-common` (L26-41), validators (L186-272), secondary (L274-292), observability (L294-353) |
| `docker/scripts/healthcheck.sh` | Health check script (3 modes: dkg, p2p, ready -- 19 lines) |
| `docker/scripts/entrypoint.sh` | Container entrypoint (handles setup, dkg, validator, secondary modes -- 117 lines) |
| `docker/scripts/devnet-run.sh` | Devnet orchestration: build, DKG, start validators, health wait (409 lines). Key functions: `clear_runtime_state` (L162-173), `check_dkg_outputs` (L120-151) |
| `docker/scripts/devnet-stats.sh` | Live TUI dashboard -- queries `kora_nodeStatus` RPC per node |
| `docker/scripts/devnet-health.sh` | Health diagnostic script -- queries Prometheus for aggregate metrics |
| `docker/config/alerts.yml` | 22 Prometheus alert rules: `HighMemoryUsage` (L95), `ResolverPeersBlocked` (L181), `VoterCrash` (L25) |
| `docker/config/prometheus.yml` | Scrape config: 10s interval, targets at `validator-nodeN:9002` and `secondary-node0:9002` |
| `docker/config/recording-rules.yml` | 22 pre-computed recording rules for dashboards |
| `docker/Justfile` | Developer commands: `devnet`, `trusted-devnet`, `restart-validators`, `reset`, `redo-dkg` |
| `docker/Dockerfile` | Multi-stage build with cargo-chef. No STOPSIGNAL override (defaults to SIGTERM). Creates non-root user `kora` (UID 1000) |
| `crates/node/runner/src/runner.rs` | Production runner: `run_standalone` (L295-321), `runtime_storage_directory` (L74-76), `KORA_RUNTIME_DIR` env var (L57), consensus timeouts (L48-53), `futures::future::pending` (L318) |
| `crates/node/service/src/service.rs` | Node service: shutdown log at L134 (`tracing::info!("kora node shutdown")`) but no actual shutdown orchestration |
| `crates/utilities/cli/src/sigsegv.rs` | SIGSEGV handler for crash diagnostics (the only signal handler in the codebase) |
| `crates/node/rpc/src/state.rs` | `NodeStatus` struct (L99-118): `currentView`, `finalizedCount`, `peerCount`, `isLeader` |
| `crates/node/rpc/src/kora.rs` | `kora_nodeStatus` RPC method (L15) |

---

## Issue 8: No Security Hardening (MEDIUM)

### Problem

The devnet deployment has no security hardening measures appropriate for a production or publicly-accessible environment:

1. **No network isolation between validator P2P and RPC**: All ports (P2P 30303, RPC 8545, metrics 9002) are exposed from the same container. In production, RPC should be on a separate network from P2P consensus traffic.

2. **No TLS on RPC endpoints**: All JSON-RPC endpoints serve plain HTTP. Transactions contain signed data (so confidentiality is less critical), but authentication tokens or admin endpoints would be exposed.

3. **No admin RPC separation**: There is no distinction between public RPC methods (eth_*) and admin methods (kora_nodeStatus, future kora_flushMempool). All methods are accessible on the same port.

4. **Grafana anonymous access**: `docker/compose/devnet.yaml` (lines 348-349) sets `GF_AUTH_ANONYMOUS_ENABLED=true` and `GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer` via environment variables. This exposes internal metrics to anyone with network access.

5. **Prometheus has no auth**: Port 9090 is exposed with no authentication. Anyone can query or modify Prometheus.

6. **No secrets management**: DKG shares (`share.key`) and validator keys (`validator.key`) are stored as plain files in Docker volumes with no encryption at rest.

### Fix

For a production deployment:
- Use Docker network segmentation (separate networks for P2P, RPC, and metrics)
- Add TLS termination via reverse proxy (nginx/caddy) for RPC endpoints
- Separate admin and public RPC ports
- Enable Grafana authentication and disable anonymous access
- Add basic auth or firewall rules for Prometheus
- Consider encrypted volumes or secrets management for key material

### Related Issues

- **Issue #22 (Rate limiting not wired)**: RPC endpoints accept unlimited requests with no rate limiting, compounding the security concern.
- **Issue #23 (No key rotation)**: Static keys that cannot be rotated increase the impact of key compromise.
