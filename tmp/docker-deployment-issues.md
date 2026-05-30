# Kora Docker Deployment: Configuration Analysis and Issues

## What is Kora?

Kora is a blockchain network built in Rust that implements a BFT (Byzantine Fault Tolerant) consensus protocol with threshold cryptography. Validators participate in a Distributed Key Generation (DKG) ceremony to establish shared signing keys, then run consensus to produce and finalize blocks. The network uses a "Commonware" runtime for storage and networking, exposes a JSON-RPC interface for transaction submission, and emits Prometheus metrics for observability.

## Deployment Architecture

Kora is deployed on a Hetzner dedicated server using Docker Compose, orchestrated via Ansible playbooks. The deployment consists of:

**Consensus Nodes:**
- 4 validator nodes (`validator-node0` through `validator-node3`) — participate in BFT consensus
- 1 secondary peer (`secondary-node0`) — follows the chain without voting

**Observability Stack (optional profile):**
- Prometheus — metrics scraping at 10s intervals
- Grafana — dashboards (overview, performance, stall diagnostics, transaction flow, logs)
- Loki — log aggregation with 72h retention
- Promtail — log collection via Docker socket

**Network Configuration:**
- Single Docker bridge network (`kora-net`)
- P2P ports: 30400-30403 (validators), 30500 (secondary)
- RPC ports: 8545-8548 (one per validator)
- Metrics ports: 9000-9003 (mapped from internal 9002)
- Observability: Prometheus 9090, Grafana 3000, Loki 3100

**Threshold Parameters:**
- 4 validators, threshold of 3 (3-of-4 must agree to finalize)
- Chain ID: 1337 (configurable via environment variable)

---

## Current Docker Configuration Analysis

### Build System (`docker-bake.hcl` + Multi-Stage Dockerfile)

The build uses Docker Buildx Bake with `cargo-chef` for Rust dependency caching:

```
Stage 1 (chef):     Install cargo-chef on rust:nightly-bookworm
Stage 2 (planner):  COPY entire source, run cargo chef prepare (recipe.json)
Stage 3 (builder):  Cook dependencies from recipe (cached layer), then build kora + keygen
Stage 4 (runtime):  debian:bookworm-slim with only runtime deps (ca-certificates, libssl3, netcat, curl, jq)
```

Build targets defined in `docker-bake.hcl`:
- `kora` — production multi-platform (linux/amd64 + linux/arm64), pushed to `ghcr.io/refcell/kora`
- `kora-local` — single platform (linux/amd64), tagged `kora:local`, used by devnet
- `kora-dev` — debug symbols, tagged `kora:dev`

The Ansible deploy pipeline: `rsync source -> docker buildx bake kora-local -> docker compose up`

### Container Resource Limits

**No CPU or memory limits are configured.** The compose file defines `x-validator-common` with restart policy and health checks, but no `deploy.resources.limits` or `mem_limit`/`cpus` fields. All containers share the host's resources without constraints.

### Health Check Configuration

Health checks ARE configured in the `x-validator-common` anchor:

```yaml
healthcheck:
  test: ["CMD", "/scripts/healthcheck.sh"]
  interval: 10s
  timeout: 5s
  retries: 3
  start_period: 30s
```

The `healthcheck.sh` script supports three modes via `HEALTHCHECK_MODE` environment variable:
- `p2p` — checks if port 30303 is listening (`nc -z localhost 30303`)
- `ready` — checks `.ready` file exists AND port 30303 is listening
- `dkg` — checks if `share.key` and `output.json` exist

All validators use `HEALTHCHECK_MODE=ready`. The entrypoint script creates `/data/.ready` immediately before starting the binary, so the health check passes once the P2P port is bound.

### Volume Mounts

**Persistent Docker volumes (survive restarts):**
- `data_node0` through `data_node3` — mounted at `/data`, contains validator keys, DKG shares, genesis, and `.ready` marker
- `data_secondary0` — same for the secondary peer
- `shared_config` — mounted at `/shared` read-only, contains `peers.json` and `genesis.json`
- `prometheus_data`, `grafana_data`, `loki_data` — observability state

**tmpfs (ephemeral, lost on container restart):**
- `/runtime:size=1g,mode=1777` — used by Commonware for runtime journals (WAL, mempool state, etc.)
- Configured via `KORA_RUNTIME_DIR=/runtime` environment variable

**Notable:** The `devnet-run.sh` script explicitly clears `/data/runtime` from persistent volumes before each start (`clear_runtime_state`), ensuring validators always start fresh.

### Network Configuration

- Single bridge network `kora-net` — all containers communicate by hostname
- Internal ports: P2P 30303, RPC 8545, Metrics 9002
- Port mapping exposes different host ports per validator (30400-30403 for P2P, 8545-8548 for RPC, 9000-9003 for metrics)
- Observability containers also attach to `kora-net` so Prometheus can scrape by hostname

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `RUST_LOG` | `info` | Tracing subscriber log level |
| `CHAIN_ID` | `1337` | Network identifier |
| `KORA_RUNTIME_DIR` | `/runtime` | Commonware storage directory (points to tmpfs) |
| `VALIDATOR_INDEX` | `0` | Which validator this node is |
| `IS_BOOTSTRAP` | `false` | Whether this node is the bootstrap peer |
| `BOOTSTRAP_PEERS` | `""` | `host:port` of bootstrap node (e.g., `node0:30303`) |
| `HEALTHCHECK_MODE` | `ready` | Health check strategy |
| `COMPOSE_PROFILES` | (unset) | Set to `none` to skip observability |

---

## Identified Issues

### 1. No Container Resource Limits

**What:** No `mem_limit`, `cpus`, `memswap_limit`, or `deploy.resources` constraints in the compose file.

**Why it matters:** A single validator experiencing a memory leak (alert threshold is 2GB RSS) can consume all host RAM, triggering the Linux OOM killer which may kill a different validator's process non-deterministically. With 4 validators + 1 secondary + observability stack on one host, memory pressure is compounded.

**Severity:** HIGH — can cause cascading failure of the entire cluster.

**Evidence:** The `alerts.yml` includes `HighMemoryUsage` alert at 2GB and `MemoryLeakSuspected` alert on sustained growth, confirming this is a known risk.

**Proposed fix:**
```yaml
x-validator-common: &validator-common
  deploy:
    resources:
      limits:
        memory: 4g
        cpus: '2.0'
      reservations:
        memory: 512m
        cpus: '0.5'
```

Set limits based on observed production RSS (likely 1-2GB per node). Reserve memory to prevent Linux overcommit killing validators.

---

### 2. tmpfs for Runtime Journals — State Lost on Restart

**What:** Commonware runtime state (consensus journals, WAL) is stored on a tmpfs mount at `/runtime` (1GB, mode 1777). Additionally, `devnet-run.sh` explicitly purges `/data/runtime` before starting.

**Why it matters:** Every restart requires validators to re-sync from peers. If all nodes restart simultaneously (host reboot, Docker daemon restart), there is no local state to recover from — consensus must rebuild entirely from genesis or peer catch-up. The `StorageWriteStall` alert exists precisely because this architecture means persistence problems go undetected until restart.

**Severity:** MEDIUM — acceptable for devnet iteration speed, but incompatible with production uptime requirements.

**Proposed fix:** For production, store runtime journals on the persistent `/data` volume:
```yaml
environment:
  - KORA_RUNTIME_DIR=/data/runtime
```
Remove the `tmpfs` mount and the `clear_runtime_state` call from the startup script. Implement proper WAL replay on startup instead.

---

### 3. Health Check Only Validates Port Binding, Not Consensus Readiness

**What:** The health check (`HEALTHCHECK_MODE=ready`) verifies:
1. `/data/.ready` file exists (created by entrypoint BEFORE binary starts)
2. P2P port 30303 is listening

**Why it matters:** A node is declared "healthy" as soon as its P2P socket binds, which happens well before it joins consensus, completes peer discovery, or finalizes its first block. Other nodes (`validator-node1` through `validator-node3`) depend on `validator-node0: condition: service_healthy`, so they start connecting before node0 is actually participating in consensus.

**Severity:** MEDIUM — causes false-positive health status and premature traffic routing. The compose `depends_on` ordering partially mitigates this because node0 starts first and the 30s `start_period` gives it time, but it is not a guarantee.

**Proposed fix:** Implement an RPC-based readiness probe:
```bash
# healthcheck.sh - ready mode
curl -sf -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' \
  http://localhost:8545 | jq -e '.result.currentView > 0'
```

This ensures the node is actually participating in consensus before being declared healthy.

---

### 4. DKG Race Condition at Startup

**What:** During interactive DKG, all 4 DKG nodes must discover each other and complete the ceremony within a 300-second timeout. Non-bootstrap nodes wait for `node0:30303` to be reachable via `nc -z` before starting, but "reachable" only means TCP accept — not that the DKG coordinator is ready to receive participants.

**Why it matters:** If node0's DKG service takes time to initialize after binding the port, other nodes may connect and fail. The 120-second `nc -z` timeout in `entrypoint.sh` helps, but there is no retry logic once the DKG binary starts — if the ceremony times out internally, the entire process fails.

**Severity:** MEDIUM — mitigated by the trusted-dealer fast path for development. Interactive DKG is only used for production-like testing.

**Proposed fix:** Add a DKG-specific readiness endpoint that non-bootstrap nodes can poll, or implement retry-with-backoff in the DKG participant logic. The entrypoint could also add a small sleep after `nc -z` succeeds to allow the coordinator to fully initialize.

---

### 5. No Graceful Shutdown (SIGTERM Handling)

**What:** The compose file uses `restart: unless-stopped` but does not configure `stop_grace_period` (defaults to 10 seconds). The entrypoint uses `exec` to replace the shell with the Kora binary, so SIGTERM goes directly to the Rust process. Whether the Rust binary handles SIGTERM gracefully (flushing state, closing P2P connections) is an application-level concern not addressed in the Docker configuration.

**Why it matters:** Abrupt termination during block production can leave corrupted partial writes on the persistent volume. Peers may not learn about the disconnection promptly, leading to timeouts waiting for votes from the stopped node.

**Severity:** LOW-MEDIUM — the 10-second default grace period is reasonable if the binary handles signals, but the Docker config provides no `stop_grace_period` override and no `STOPSIGNAL` directive.

**Proposed fix:**
```yaml
x-validator-common: &validator-common
  stop_grace_period: 30s
  stop_signal: SIGTERM
```
Ensure the Rust binary has a SIGTERM handler that: (1) stops accepting new proposals, (2) flushes any pending writes, (3) broadcasts a disconnect message to peers.

---

### 6. No Log Rotation Configuration

**What:** No Docker log driver configuration in the compose file. The default is `json-file` with no rotation limits. The Ansible-deployed `daemon.json` only configures DNS servers — no `log-driver`, `log-opts`, or `max-size`/`max-file` settings.

**Why it matters:** Validators produce verbose logs (especially at `RUST_LOG=info` or `debug`). Without rotation, `/var/lib/docker/containers/*/` will grow unbounded and eventually fill the disk, causing Docker to fail to start new containers.

**Severity:** HIGH for long-running deployments — disk exhaustion is a common cause of unexpected downtime.

**Proposed fix:** Add to the Docker daemon config (`/etc/docker/daemon.json`):
```json
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "50m",
    "max-file": "5"
  }
}
```
Or configure per-service in compose:
```yaml
x-validator-common: &validator-common
  logging:
    driver: json-file
    options:
      max-size: "50m"
      max-file: "5"
```

---

### 7. Bootstrap Peer Hardcoded to node0 — Single Point of Failure

**What:** All non-bootstrap validators have `BOOTSTRAP_PEERS=node0:30303`. The compose ordering uses `depends_on: validator-node0: condition: service_healthy`. If node0 is unhealthy or crashes after the initial startup, the remaining nodes cannot re-bootstrap their P2P connections.

**Why it matters:** If node0 restarts and gets a new container IP (bridge network DHCP), existing connections from other nodes may break. Nodes 1-3 have no fallback bootstrap peer and their entrypoint script will loop for 120 seconds waiting for node0 before erroring out. In a 3-of-4 threshold system, losing node0 at the wrong moment can break quorum.

**Severity:** MEDIUM — partially mitigated by the `unless-stopped` restart policy and the fact that once P2P mesh is formed, nodes maintain direct connections. But on full cluster restart, node0 is still a serial dependency.

**Proposed fix:** Support multiple bootstrap peers:
```yaml
BOOTSTRAP_PEERS: "node0:30303,node1:30303"
```
Modify the entrypoint to try each peer and proceed once any is reachable. For production, consider running a dedicated seed node that is not a validator.

---

### 8. No True Readiness Probe — Traffic Routed Before Consensus Active

**What:** The RPC port (8545) is exposed immediately when the container starts. There is no mechanism to delay RPC traffic until the node has joined consensus and is at the chain tip.

**Why it matters:** A load balancer or client connecting to a validator's RPC port immediately after startup will get stale state or errors. Transaction submissions may fail silently. The `devnet-stats.sh` script handles this gracefully (showing "offline" for unreachable nodes), but external clients have no such protection.

**Severity:** LOW for current single-server devnet (no external load balancer), but HIGH if the deployment grows to include RPC traffic from external users.

**Proposed fix:** Use an RPC-level health endpoint. Only return HTTP 200 from a `/health` endpoint when `currentView > 0` and `finalizedCount > 0`. Configure any reverse proxy or load balancer to use this as the readiness check.

---

### 9. Build Cache Partially Optimized, But Full Rebuild on Source Changes

**What:** The Dockerfile uses `cargo-chef` which caches dependency compilation in a dedicated layer. However, `COPY . .` in the builder stage means ANY source file change (including comments, tests, documentation) invalidates the final build layer and triggers a full `cargo build --release`.

**Why it matters:** Rust release builds are slow (10-30+ minutes for a project this size). On the Hetzner server, this blocks the entire deploy pipeline. The Ansible `build` role sets an async timeout (`build_timeout`) indicating awareness of this issue.

**Severity:** MEDIUM — the `cargo-chef` approach correctly caches dependency compilation (the slowest part). Only application code changes trigger rebuilds, which is the expected behavior. However, the `.dockerignore` excludes `target/` and `.git/` but includes test files and other non-essential code.

**Proposed fix:** The current architecture is already reasonably optimized. Further improvements:
1. Use sccache or a mounted cargo registry volume for cross-build caching
2. Split the monorepo into workspace members and only rebuild affected crates
3. Consider pre-built CI images that the Hetzner server pulls rather than building locally

---

### 10. Security Hardening Gaps

**What:** The Dockerfile creates a non-root user (`kora`, UID 1000) and switches to it for runtime. However:
- The `init-setup` and `init-config` services run as `user: root` (necessary for `chown`)
- No Linux capabilities are dropped (`cap_drop: ALL`)
- No read-only root filesystem (`read_only: true`)
- No seccomp profile configured
- Docker socket is mounted into promtail (read-only, but still privileged)
- Grafana has hardcoded `admin/admin` credentials with anonymous access enabled

**Why it matters:** If a validator binary is compromised (e.g., via a malicious transaction exploiting an execution bug), the container has broader access than necessary. The Docker socket mount in promtail could allow container escape if promtail itself is compromised.

**Severity:** LOW for a devnet (no real assets at risk), MEDIUM for any network holding value.

**Proposed fix:**
```yaml
x-validator-common: &validator-common
  cap_drop:
    - ALL
  cap_add:
    - NET_BIND_SERVICE
  read_only: true
  tmpfs:
    - /tmp:size=100m
    - /runtime:size=1g,mode=1777
  security_opt:
    - no-new-privileges:true
```

For Grafana, use environment-sourced secrets and disable anonymous access in production.

---

## Comparison to Production-Ready Deployment

| Aspect | Current Devnet | Production Standard |
|--------|---------------|-------------------|
| Resource limits | None | Per-container CPU/memory limits and reservations |
| Health checks | Port binding only | Application-level readiness (consensus active) |
| State persistence | tmpfs (ephemeral) | Durable volumes with WAL replay |
| Log management | Unbounded json-file | Rotation + centralized shipping (Loki partially addresses this) |
| Secrets management | Keys in Docker volumes | HashiCorp Vault or sealed secrets |
| Network isolation | Single bridge | Separate networks for P2P, RPC, and metrics |
| TLS | None | mTLS for P2P, TLS termination for RPC |
| Redundancy | All on one host | Multi-host with geographic distribution |
| Upgrades | Full stop + restart | Rolling upgrades with version compatibility |
| Backup | None | Automated volume snapshots |
| Monitoring | Optional profile | Always-on with PagerDuty/Slack alerting |
| Security | Non-root user only | Dropped caps, read-only FS, seccomp, no-new-privileges |
| Disaster recovery | Manual DKG redo | Automated failover with standby validators |

---

## Ansible Deployment Pipeline

The Kora devnet is deployed to Hetzner via Ansible playbooks in `/ansible/`:

**Provisioning (one-time):** `ansible-playbook playbooks/provision.yml`
1. `base` role — installs system packages (Arch Linux/pacman), sets UTC timezone, enables NTP
2. `firewall` role — deploys nftables rules via Jinja2 template
3. `docker` role — installs Docker + buildx, configures DNS, creates buildx builder with host networking

**Deployment (repeatable):** `ansible-playbook playbooks/deploy.yml`
1. `sync` role — rsync source code to remote (excludes `.git`, `target/`, `ansible/`)
2. `build` role — runs `docker buildx bake kora-local` on the remote server (async with timeout)
3. `devnet` role — stops validators, checks DKG state, runs DKG if needed, starts validators, waits for health

**Other playbooks:**
- `reset.yml` — tears down everything and removes volumes
- `observe.yml` — deploys the observability stack

The pipeline is designed for single-command deploys (`ansible-playbook playbooks/deploy.yml`) with idempotent DKG handling — existing shares are preserved across deploys, but runtime state is always cleared for fresh consensus startup.

---

## Summary of Priority Fixes

| # | Issue | Severity | Effort | Impact |
|---|-------|----------|--------|--------|
| 1 | No resource limits | HIGH | Low | Prevents OOM cascading failures |
| 6 | No log rotation | HIGH | Low | Prevents disk exhaustion |
| 3 | Shallow health check | MEDIUM | Medium | Accurate readiness detection |
| 2 | tmpfs state loss | MEDIUM | High | Enables restart resilience |
| 7 | Single bootstrap peer | MEDIUM | Medium | Removes startup SPOF |
| 5 | No graceful shutdown | MEDIUM | Medium | Clean state on stop |
| 4 | DKG race condition | MEDIUM | Medium | Reliable ceremony |
| 10 | Security hardening | LOW-MEDIUM | Low | Defense in depth |
| 8 | No readiness probe | LOW | Low | Correct traffic routing |
| 9 | Build cache gaps | MEDIUM | Low | Faster deploys |
