# Docker Container Hardening and Infrastructure Fixes

## Status Summary

PR #132 (`18dd8f9`) addressed several items from the original version of this issue. This updated version cross-references what was fixed and focuses on the remaining work.

### Already Fixed by PR #132

| Item | What PR #132 Did |
|------|------------------|
| Resource limits | Added `deploy.resources.limits` (memory: 4G, cpus: 2) to `x-validator-common` |
| RPC health checks | Updated `ready` mode in `healthcheck.sh` to query `eth_chainId` via RPC |
| Log rotation | Added `json-file` driver with `max-size: 50m`, `max-file: 5` to `x-node-common` |
| Graceful shutdown signal | Added `stop_grace_period: 30s` and `stop_signal: SIGTERM` to `x-validator-common` |
| Secondary signal handling | Replaced `futures::future::pending()` in `run_standalone()` with `tokio::signal::ctrl_c()` |

### Still Remaining (This Issue)

| # | Item | Severity |
|---|------|----------|
| 1 | No `init: true` -- zombie reaping and PID 1 signal issues | High |
| 2 | `override.yml` never applied (4G vs intended 8G) | Medium |
| 3 | Restart policy creates tight crash loops | Medium |
| 4 | No PID limits | Low |
| 5 | Security hardening (read-only rootfs, cap_drop, no-new-privileges) | Medium |
| 6 | Barrier files persist across Ansible restarts | Medium |
| 7 | tmpfs mode too permissive (1777) | Low |
| 8 | `read_only: true` requires `/tmp` tmpfs mount | Medium |
| 9 | Barrier cleanup should also happen in entrypoint.sh | Medium |
| 10 | Loadgen binary in production image | Low |

---

## Remaining Item 1: No `init: true` (Zombie Processes and PID 1 Signal Issues)

**File:** `docker/compose/devnet.yaml` (line 32, `x-validator-common` anchor)

### Current behavior

The entrypoint script uses `exec` to replace the shell with the kora binary, making kora PID 1 inside the container:

```bash
# docker/scripts/entrypoint.sh, line 119
exec /usr/local/bin/kora validator \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    "$@"
```

PID 1 in Linux has special semantics:
- It is responsible for reaping orphaned child processes. If it does not call `waitpid()`, zombie processes accumulate.
- Signals without explicitly registered handlers are silently ignored rather than triggering the default action. This means SIGTERM (which Docker sends on `docker stop`) is dropped unless the application explicitly registers a handler.

The kora binary registers a handler for SIGINT (`tokio::signal::ctrl_c()`) but not SIGTERM:

```rust
// crates/node/runner/src/runner.rs, line 480
tokio::signal::ctrl_c().await.ok();
info!("Received shutdown signal, stopping...");
```

### Consequence

Every `docker stop` sends SIGTERM, kora ignores it (PID 1 semantics), Docker waits 30 seconds, then sends SIGKILL. Every graceful shutdown is actually a hard kill with a 30-second delay.

### Fix

Add `init: true` to the `x-validator-common` anchor in `docker/compose/devnet.yaml`:

```yaml
x-validator-common: &validator-common
  <<: *node-common
  init: true                        # ADD THIS
  restart: unless-stopped
  stop_grace_period: 30s
  stop_signal: SIGTERM
```

This causes Docker to run `tini` as PID 1, which properly reaps zombie child processes and forwards signals to the kora process.

**Note:** A complementary SIGTERM handler in Rust code (`runner.rs`) is desirable for graceful resource cleanup but is out of scope for this issue.

---

## Remaining Item 2: `override.yml` Exists but Is Never Applied

**File:** `docker/compose/override.yml` (no longer exists in the repo -- file was apparently already deleted)

### Current state

The `docker/compose/override.yml` file referenced in the original issue no longer exists in the repository. However, the memory limit in `x-validator-common` is still set to `4G` (line 40 of `docker/compose/devnet.yaml`), not the intended `8G`.

```yaml
# docker/compose/devnet.yaml, lines 37-41
  deploy:
    resources:
      limits:
        memory: 4G
        cpus: "2"
```

### Fix

Update the memory limit directly in `docker/compose/devnet.yaml`:

```yaml
  deploy:
    resources:
      limits:
        memory: 8G          # Change from 4G to 8G
        cpus: "2"
```

---

## Remaining Item 3: Restart Policy Creates Crash Loops Without Backoff

**File:** `docker/compose/devnet.yaml` (line 34, in `x-validator-common`)

### Current behavior

```yaml
restart: unless-stopped
```

The `unless-stopped` policy has unlimited retries. If a container enters a crash loop, Docker restarts it immediately and indefinitely. Each restart attempt burns 120 seconds of CPU in the entrypoint's bootstrap wait loop before failing again.

### Fix

Switch to `on-failure` with a retry limit:

```yaml
  restart: "on-failure:10"       # Change from: unless-stopped
```

This limits crash-loop restarts to 10 attempts. After 10 failures, the container stays stopped and can be investigated.

---

## Remaining Item 4: No PID Limits

**File:** `docker/compose/devnet.yaml` (`x-validator-common` anchor)

### Current behavior

No `pids_limit` is set. A thread leak or fork bomb has no ceiling and could exhaust the host's PID space.

### Fix

Add `pids_limit` to the `x-validator-common` anchor:

```yaml
  pids_limit: 4096
```

A Rust async runtime with tokio should never need thousands of OS threads. 4096 provides generous headroom.

---

## Remaining Item 5: Security Hardening

**File:** `docker/compose/devnet.yaml` (`x-validator-common` anchor)

### Current state

| Control | Status |
|---------|--------|
| Non-root user | Yes (`kora`, UID 1000) |
| Read-only rootfs | No |
| Capabilities dropped | None |
| `no-new-privileges` | Not set |
| Ulimits | None configured |

### Fix

Add the following to `x-validator-common`:

```yaml
  read_only: true
  security_opt:
    - no-new-privileges:true
  cap_drop:
    - ALL
  ulimits:
    nofile:
      soft: 65536
      hard: 65536
    core: 0                  # Disable core dumps (they may contain key material)
```

**Important:** `read_only: true` makes the root filesystem read-only. All mutable data already goes to `/data` (named volume), `/runtime` (tmpfs), and `/barrier` (named volume). However, some tools (curl, jq used in healthcheck, and potentially the Rust runtime) need to write to `/tmp`. See Item 8 below for the required `/tmp` tmpfs mount.

---

## Remaining Item 6: Startup Barrier Files Persist Across Ansible Restarts

**File:** `ansible/roles/devnet/tasks/main.yml`

### Current behavior

The `devnet-run.sh` script correctly calls `clear_startup_barrier()` (line 319) before starting validators. The Ansible `devnet` role does not -- it clears `/data/runtime` (lines 74-84) but never the barrier volume. On subsequent Ansible deploys, stale `.ready` files cause the barrier to be satisfied instantly, allowing validators to start before peers are actually ready.

### Fix

Add a barrier cleanup task to `ansible/roles/devnet/tasks/main.yml`, after the "Clear runtime state from data volumes" task (after line 85):

```yaml
- name: Clear startup barrier
  ansible.builtin.shell: |
    volume="{{ compose_project_name }}_startup_barrier"
    docker volume inspect "$volume" >/dev/null 2>&1 || exit 0
    docker run --rm -v "${volume}:/barrier" alpine sh -c 'rm -f /barrier/*.ready'
  changed_when: true
```

---

## Remaining Item 7: tmpfs Mode Too Permissive

**File:** `docker/compose/devnet.yaml` (line 43, in `x-validator-common`)

### Current behavior

```yaml
tmpfs:
  - /runtime:size=1g,mode=1777
```

Mode `1777` (world-writable with sticky bit) is unnecessarily permissive. The container runs as a single user (`kora`, UID 1000).

### Fix

```yaml
tmpfs:
  - /runtime:size=1g,mode=0700
```

Mode `0700` restricts access to the owning user only.

---

## Remaining Item 8: `read_only: true` Requires `/tmp` tmpfs Mount

**File:** `docker/compose/devnet.yaml` (`x-validator-common` anchor)

### Problem

When `read_only: true` is set (Item 5), the root filesystem becomes read-only. Several things may need to write to `/tmp`:
- The healthcheck script uses `curl`, which may write temporary files.
- The Rust runtime and libraries may create temporary files.
- The `jq` binary used in healthcheck may need temp space.

Without a writable `/tmp`, the healthcheck will fail and the container will be marked unhealthy.

### Fix

Add a `/tmp` tmpfs mount to the `x-validator-common` anchor alongside the existing `/runtime` tmpfs:

```yaml
tmpfs:
  - /runtime:size=1g,mode=0700
  - /tmp:size=64m,mode=1777
```

The `/tmp` tmpfs uses mode `1777` (the standard `/tmp` permissions) since it is ephemeral and its contents are not security-sensitive. 64 MB is more than sufficient for healthcheck temp files.

---

## Remaining Item 9: Barrier File Cleanup in entrypoint.sh

**File:** `docker/scripts/entrypoint.sh`

### Problem

Currently, barrier file cleanup depends entirely on external orchestration (the `clear_startup_barrier()` function in `devnet-run.sh` or the proposed Ansible task in Item 6). If a container restarts on its own (due to the restart policy), it will see stale barrier files from the previous run and skip the barrier wait entirely.

### Fix

Add barrier cleanup at the beginning of the `validator` case in `docker/scripts/entrypoint.sh`, before the `wait_for_barrier` call (before line 104):

```bash
    validator)
        log "Running validator mode..."

        [[ -f "${SHARED_DIR}/genesis.json" ]] || error "genesis.json not found"
        [[ -f "${DATA_DIR}/validator.key" ]] || error "validator.key not found"
        [[ -f "${DATA_DIR}/share.key" ]] || error "share.key not found (run DKG first)"
        [[ -f "${DATA_DIR}/output.json" ]] || error "output.json not found (run DKG first)"

        cp "${SHARED_DIR}/genesis.json" "${DATA_DIR}/" 2>/dev/null || true
        touch "${DATA_DIR}/.ready"

        # Clean up our own barrier file from any previous run so that
        # the barrier waits for all validators to actually restart.
        rm -f "${BARRIER_DIR}/node${VALIDATOR_INDEX}.ready" 2>/dev/null || true

        # Wait for all validators to be ready before starting consensus.
        wait_for_barrier "$VALIDATOR_COUNT"
```

This way, each validator cleans up only its own barrier marker on startup, so the barrier mechanism works correctly even on individual container restarts. The barrier will re-synchronize because all restarting validators must re-touch their marker files.

---

## Remaining Item 10: Loadgen Binary in Production Image

**File:** `docker/Dockerfile` (lines 41, 66)

### Problem

The Dockerfile builds and copies the `loadgen` binary into the production image:

```dockerfile
# docker/Dockerfile, line 41 (builder stage)
RUN cargo build --release -p kora -p keygen -p loadgen

# docker/Dockerfile, line 66 (runtime stage)
COPY --from=builder /app/target/release/loadgen /usr/local/bin/
```

The `loadgen` binary is a load testing tool that has no business being in the production validator image. Including it:
- Increases the image size unnecessarily.
- Expands the attack surface -- if an attacker gains shell access to the container, `loadgen` is a ready-made tool to spam the network.

### Fix

**Option A (recommended): Remove loadgen from the production image.**

In `docker/Dockerfile`, change:

```dockerfile
# Line 41: Remove loadgen from the build
RUN cargo build --release -p kora -p keygen

# Line 66: Remove the COPY for loadgen
# DELETE: COPY --from=builder /app/target/release/loadgen /usr/local/bin/
```

If `loadgen` is needed in Docker, build it as a separate image or use a multi-stage build target:

```dockerfile
# Add at the end of the Dockerfile
FROM runtime AS loadgen
COPY --from=builder /app/target/release/loadgen /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/loadgen"]
```

Then build it separately: `docker build --target loadgen -t kora-loadgen:local .`

**Option B: Separate Dockerfile for loadgen.**

Create `docker/Dockerfile.loadgen` that builds only the loadgen binary.

---

## Proposed Combined Diff for `docker/compose/devnet.yaml`

Here is the complete set of changes to the `x-validator-common` anchor:

```yaml
x-validator-common: &validator-common
  <<: *node-common
  init: true                         # NEW: proper PID 1 / zombie reaping
  restart: "on-failure:10"           # CHANGED from: unless-stopped
  stop_grace_period: 30s
  stop_signal: SIGTERM
  read_only: true                    # NEW: immutable root filesystem
  security_opt:                      # NEW: prevent privilege escalation
    - no-new-privileges:true
  cap_drop:                          # NEW: drop all Linux capabilities
    - ALL
  ulimits:                           # NEW: file descriptor and core dump limits
    nofile:
      soft: 65536
      hard: 65536
    core: 0
  pids_limit: 4096                   # NEW: prevent runaway thread/process creation
  deploy:
    resources:
      limits:
        memory: 8G                   # CHANGED from: 4G
        cpus: "2"
  tmpfs:
    - /runtime:size=1g,mode=0700     # CHANGED mode from: 1777
    - /tmp:size=64m,mode=1777        # NEW: required for read_only + healthcheck
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

---

## Files to Modify

| File | Change |
|------|--------|
| `docker/compose/devnet.yaml` | Update `x-validator-common` anchor (see combined diff above) |
| `docker/Dockerfile` | Remove `loadgen` from build and COPY steps (Item 10) |
| `docker/scripts/entrypoint.sh` | Add own-barrier-file cleanup before `wait_for_barrier` (Item 9) |
| `ansible/roles/devnet/tasks/main.yml` | Add barrier cleanup task after "Clear runtime state" (Item 6) |

---

## Verification

After applying these changes:

1. **`init: true`**: Run `docker compose up -d`, then `docker exec <container> cat /proc/1/status | grep Name`. It should show `tini` (not `kora`). Verify `docker stop <container>` completes in under 5 seconds.

2. **Resource limits**: Run `docker inspect <container> | jq '.[0].HostConfig.Memory'`. Should return `8589934592` (8 GB).

3. **PID limits**: Run `docker inspect <container> | jq '.[0].HostConfig.PidsLimit'`. Should return `4096`.

4. **Read-only root**: Run `docker exec <container> touch /test`. Should fail with "Read-only file system".

5. **`/tmp` writable**: Run `docker exec <container> touch /tmp/test`. Should succeed (tmpfs mount).

6. **Capabilities dropped**: Run `docker inspect <container> | jq '.[0].HostConfig.CapDrop'`. Should return `["ALL"]`.

7. **Restart policy**: Run `docker inspect <container> | jq '.[0].HostConfig.RestartPolicy'`. Should show `{"Name": "on-failure", "MaximumRetryCount": 10}`.

8. **Barrier cleanup (Ansible)**: Deploy via Ansible, verify no stale `.ready` files exist in the barrier volume before validators start: `docker run --rm -v kora-devnet_startup_barrier:/barrier alpine ls -la /barrier/`.

9. **Barrier cleanup (entrypoint)**: Restart a single validator container, verify it cleans its own barrier marker and re-creates it during `wait_for_barrier`.

10. **tmpfs mode**: Run `docker exec <container> stat -c '%a' /runtime`. Should return `700`.

11. **Loadgen absent**: Run `docker exec <container> which loadgen`. Should return "not found" / exit 1.

12. **Healthcheck works with read_only**: Wait for `start_period` (30s), then check `docker inspect <container> | jq '.[0].State.Health.Status'`. Should show `healthy`.

---

## Out of Scope (Tracked Separately)

- **SIGTERM handler in Rust code** (`runner.rs` line 480) -- requires application code changes. The `init: true` change provides partial mitigation.
- **Init container idempotency** -- `init-config` regenerates DKG keys on every run. Separate design issue.
- **Network security** (firewall, port binding to 127.0.0.1) -- separate infrastructure concern.
- **Observability stack resource limits** (Prometheus, Loki, Grafana have no memory/CPU limits) -- separate issue.
- **Docker socket in Promtail** -- security concern but requires switching log collection strategy.
