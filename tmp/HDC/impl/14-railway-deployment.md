# Railway Deployment: 3-Validator Daeji Testnet (50ms Blocks)

> **Status: NOT STARTED -- blocked on env-var-to-config bridging**
>
> The block_time_ms migration (doc 01) is code-complete in the config/runner
> layer, but the `kora` binary does NOT read `BLOCK_TIME_MS`, `GAS_LIMIT`, or
> `HDC_ENABLED` from environment variables. Setting these in Railway will be
> silently ignored. Validators will run with defaults: 2-second blocks, HDC
> disabled.
>
> **Prerequisites checklist:**
>
> - [x] Block time migration code-complete (`block_time_ms` field in `ExecutionConfig`)
> - [ ] **BLOCKER:** Env-var-to-config bridge (binary must read `BLOCK_TIME_MS` env var) -- see audit finding F01
> - [ ] **BLOCKER:** `CONFIG_FILE` env var support in entrypoint OR TOML config generation in init-config
> - [ ] HDC precompile working and tested end-to-end (doc 13 blockers must be resolved)
> - [ ] Docker image builds successfully (`cd docker && just build`)
> - [ ] Keygen binary produces valid keys (`keygen setup --validators=3 --threshold=2`)
> - [ ] `healthcheckPath` removed from `railway.toml` (uses Dockerfile HEALTHCHECK instead) -- see audit finding F02
> - [ ] Fast-devnet compose generates/mounts TOML config file -- see audit finding F03
>
> **Resource recommendations (Railway):**
>
> | Resource | Per Validator | Total (3 validators) | Notes |
> |----------|-------------|---------------------|-------|
> | Memory | 2 GB minimum | 6 GB | Rust binary + QMDB + HDC index |
> | CPU | 2 vCPU | 6 vCPU | Consensus + REVM execution |
> | Storage | 10 GB | 30 GB | QMDB state + HDC knowledge store + logs |
> | Network | Internal only | -- | P2P on Railway private network |

## Goal

Deploy a 3-validator daeji/kora testnet to Railway with 50ms block times. Each
validator runs as a separate Railway service. An init service generates keys and
threshold shares before validators start. P2P traffic uses Railway's private
internal network; one validator exposes RPC publicly.

**Prerequisite:** The block-time migration from `01-block-time-migration.md` must
be complete. The codebase must accept `block_time_ms` (milliseconds) instead of
`block_time` (seconds).

---

## 1. Railway Project Structure

```
railway-project/
  init-config/      One-shot service: keygen setup + trusted dealer DKG
  validator-0/      Bootstrap validator (node0)
  validator-1/      Validator (node1)
  validator-2/      Validator (node2)
  prometheus/       (optional) Metrics aggregation
```

All services share a single Railway volume mounted at `/shared`. Each validator
also has its own persistent volume mounted at `/data`.

### BFT Threshold

3 validators with a threshold of 2 (2-of-3). This satisfies BFT: the system
tolerates 1 faulty validator (`n = 3, f = 1, threshold = n - f = 2`).

---

## 2. Dockerfile (Railway)

The existing `docker/Dockerfile` works on Railway with no modifications. It
already:

- Uses multi-stage build with `cargo-chef` caching.
- Installs runtime dependencies (`ca-certificates`, `libssl3`, `netcat-openbsd`,
  `curl`, `jq`).
- Creates a non-root `kora` user (UID 1000).
- Copies `kora` and `keygen` binaries.
- Copies entrypoint/healthcheck scripts from `docker/scripts/`.
- Exposes ports 30303, 8545, 8546, 9002.
- Declares volumes `/data` and `/shared`.
- Uses `/scripts/entrypoint.sh` as entrypoint.

Railway builds from the repo root. Point the build context to the repo root and
specify the Dockerfile path as `docker/Dockerfile`.

If you need a health check in the Dockerfile itself (Railway uses it for
readiness), add this to the end of the Dockerfile before the `ENTRYPOINT` line:

```dockerfile
HEALTHCHECK --interval=10s --timeout=5s --retries=3 --start-period=30s \
    CMD /scripts/healthcheck.sh
```

The existing Dockerfile does not have an inline `HEALTHCHECK` -- it relies on
Docker Compose's `healthcheck` block. Railway supports Dockerfile-level health
checks, so adding one is recommended.

---

## 3. railway.toml Per Service

Railway uses `railway.toml` at the repo root (or per-service directory) for
build/deploy configuration. Since all services share the same Dockerfile but
differ in environment variables, you have two options:

**Option A: Single repo, multiple Railway services (recommended).** Create one
Railway project. Add 4 services (init-config, validator-0, validator-1,
validator-2), each pointing to the same repo. Differentiate via environment
variables.

**Option B: Per-service `railway.toml` in subdirectories.** Not needed here --
the Dockerfile is the same for all services.

### railway.toml (shared across all services)

Place this at the repo root:

```toml
[build]
# Build from repo root, using the Docker build path
builder = "DOCKERFILE"
dockerfilePath = "docker/Dockerfile"
# watchPatterns filters what triggers rebuilds
watchPatterns = [
    "bin/**",
    "crates/**",
    "docker/**",
    "Cargo.toml",
    "Cargo.lock",
]

[deploy]
# 1 replica per service (each validator is its own service)
numReplicas = 1
# Graceful shutdown
sleepApplication = false
# Restart on crash
restartPolicyType = "ON_FAILURE"
restartPolicyMaxRetries = 10
# Health check
healthcheckPath = "/"
healthcheckTimeout = 5000
```

> **Note:** Railway's `healthcheckPath` is for HTTP health checks. For TCP-based
> health checks (P2P port), rely on the Dockerfile `HEALTHCHECK` instruction
> instead. The `healthcheckPath` setting can be removed if you are not exposing
> HTTP on the health check port.

---

## 4. Environment Variables

Set these per-service in the Railway dashboard or via `railway variables set`.

### Common (all validators)

```bash
CHAIN_ID=13370
BLOCK_TIME_MS=50
RUST_LOG=info
DATA_DIR=/data
SHARED_DIR=/shared
GAS_LIMIT=250000000
```

### Per validator

| Variable | validator-0 | validator-1 | validator-2 |
|---|---|---|---|
| `VALIDATOR_INDEX` | `0` | `1` | `2` |
| `IS_BOOTSTRAP` | `true` | `false` | `false` |
| `BOOTSTRAP_PEERS` | (empty) | `validator-0.railway.internal:30303` | `validator-0.railway.internal:30303` |

### init-config service

```bash
CHAIN_ID=13370
RUST_LOG=info
SHARED_DIR=/shared
```

The init-config service does not use `VALIDATOR_INDEX`, `IS_BOOTSTRAP`, or
`BOOTSTRAP_PEERS`. It runs `keygen` commands directly, not the `entrypoint.sh`.

---

## 5. Networking

### Internal (P2P)

Railway assigns each service an internal DNS name:
`<service-name>.railway.internal`. All P2P traffic stays on Railway's private
network.

| Service | Internal DNS | P2P Port |
|---|---|---|
| validator-0 | `validator-0.railway.internal` | 30303 |
| validator-1 | `validator-1.railway.internal` | 30303 |
| validator-2 | `validator-2.railway.internal` | 30303 |

The `BOOTSTRAP_PEERS` env var for non-bootstrap validators must use the internal
DNS name: `validator-0.railway.internal:30303`.

### Important: peers.json bootstrapper hostnames

The `keygen setup` command writes `peers.json` with bootstrapper addresses in the
format `node{i}:30303` (e.g., `node0:30303`, `node1:30303`, `node2:30303`). On
Railway, these hostnames do not resolve.

**You must patch peers.json after `keygen setup` to replace hostnames** with
Railway internal DNS names. The init-config startup command (see section 6)
includes a `sed` command to do this:

```bash
# Replace node0:30303 -> validator-0.railway.internal:30303, etc.
sed -i \
    -e 's/node0:30303/validator-0.railway.internal:30303/g' \
    -e 's/node1:30303/validator-1.railway.internal:30303/g' \
    -e 's/node2:30303/validator-2.railway.internal:30303/g' \
    /shared/peers.json
```

### Public (RPC)

Expose a public endpoint on **one** validator for RPC access. In the Railway
dashboard, enable "Public Networking" on `validator-0` and map port 8545. Railway
assigns a public URL like `validator-0-production-XXXX.up.railway.app`.

Do **not** expose port 30303 publicly. P2P must stay internal.

### Port Configuration

Each service in Railway listens on these ports. Railway's internal networking
routes to them by service name.

| Port | Protocol | Purpose |
|---|---|---|
| 30303 | TCP | P2P (internal only) |
| 8545 | HTTP | JSON-RPC (public on validator-0 only) |
| 9002 | HTTP | Prometheus metrics (internal only) |

Set the `PORT` environment variable in Railway to `8545` for the validator that
exposes RPC. Railway uses `PORT` to know which port to route public traffic to.

---

## 6. Init/Bootstrap Sequence

### Step 1: Run init-config (one-shot)

The init-config service generates identity keys, `peers.json`, `genesis.json`,
and threshold shares. It runs once and exits.

**Service configuration in Railway:**

- **Start command override:**

```bash
/bin/bash -c '
set -euo pipefail

echo "[init] Running keygen setup (3 validators, threshold 2)..."
/usr/local/bin/keygen setup \
    --validators=3 \
    --threshold=2 \
    --chain-id=${CHAIN_ID:-13370} \
    --output-dir=/shared

echo "[init] Patching peers.json hostnames for Railway..."
sed -i \
    -e "s/node0:30303/validator-0.railway.internal:30303/g" \
    -e "s/node1:30303/validator-1.railway.internal:30303/g" \
    -e "s/node2:30303/validator-2.railway.internal:30303/g" \
    /shared/peers.json

echo "[init] Running trusted dealer DKG..."
/usr/local/bin/keygen dkg-deal \
    --validators=3 \
    --threshold=2 \
    --output-dir=/shared

echo "[init] Setting permissions..."
chown -R 1000:1000 /shared/node0 /shared/node1 /shared/node2

echo "[init] Init complete. Contents of /shared:"
ls -la /shared/
ls -la /shared/node0/
cat /shared/peers.json

echo "[init] Done. Stop this service and start validators."
'
```

- **User:** Must run as root (to `chown`). Set the Railway Dockerfile user
  override or prepend the command with a user context.

**Volume mounts:**

| Volume | Mount Path | Notes |
|---|---|---|
| `shared-config` | `/shared` | Shared across all services |
| `data-node0` | `/shared/node0` | Validator 0's identity + shares |
| `data-node1` | `/shared/node1` | Validator 1's identity + shares |
| `data-node2` | `/shared/node2` | Validator 2's identity + shares |

> **Railway volume note:** Railway volumes persist across deploys. A single
> volume can be mounted into multiple services (read-only or read-write). Create
> one volume called `shared-config` and mount it at `/shared` for all services.
> Create per-validator volumes (`data-node0`, `data-node1`, `data-node2`) and
> mount each at `/data` on the corresponding validator and at
> `/shared/node{i}` on init-config.

### Step 2: Verify init-config output

After init-config runs, check the logs for:

```
[init] Init complete. Contents of /shared:
```

Confirm these files exist:

```
/shared/peers.json          (peer config with Railway hostnames)
/shared/genesis.json        (genesis allocations)
/shared/node0/validator.key (identity key)
/shared/node0/share.key     (BLS threshold share)
/shared/node0/output.json   (DKG output with group key)
/shared/node0/setup.json    (validator index metadata)
/shared/node1/...           (same files)
/shared/node2/...           (same files)
```

### Step 3: Stop init-config

After successful completion, stop (or remove) the init-config service. It should
not be running when validators start. In Railway, set the service to "Stopped" or
delete it.

### Step 4: Start validator-0 (bootstrap)

Start `validator-0` first. It is the bootstrap node. Its entrypoint:

```bash
/scripts/entrypoint.sh validator
```

This is the default -- the Dockerfile already has `ENTRYPOINT ["/scripts/entrypoint.sh"]`
and `CMD ["validator"]`.

The entrypoint script:
1. Checks for `genesis.json`, `validator.key`, `share.key`, `output.json`.
2. Copies `genesis.json` into the data directory.
3. Touches `/data/.ready` (for health check).
4. Since `IS_BOOTSTRAP=true`, skips waiting for a bootstrap peer.
5. Runs `kora validator --data-dir /data --peers /shared/peers.json --chain-id 13370`.

Wait until `validator-0` is healthy (health check passes: `/data/.ready` exists
AND port 30303 is listening).

### Step 5: Start validator-1 and validator-2

Start both remaining validators. They will:
1. Wait for the bootstrap peer (`validator-0.railway.internal:30303`) to be
   reachable (up to 120 seconds).
2. Start the validator with the same flags.

All three validators discover each other through `peers.json` and begin
consensus.

### Startup Order Summary

```
init-config  ──[runs once, exits]──>  (stop/remove)
                                           |
                                           v
validator-0  ──[starts, healthy]───>  validator-1  ──[connects]──>  consensus
                                      validator-2  ──[connects]──/
```

---

## 7. 50ms Block Time Configuration

### Environment variable

Set `BLOCK_TIME_MS=50` on all validators. After the migration from
`01-block-time-migration.md`, the `kora` binary reads this from
`config.execution.block_time_ms`.

### Resulting consensus timeouts

| Timeout | Formula | Value |
|---|---|---|
| `leader_timeout` | 1x `block_time_ms` | 50ms |
| `certification_timeout` | 2x `block_time_ms` | 100ms |
| `timeout_retry` | 1x `block_time_ms` | 50ms |
| `fetch_timeout` | 2x `block_time_ms` | 100ms |

### Important caveats

50ms is aggressive. Railway's internal network latency between services is
typically 1-5ms within the same region, but can spike under load. If consensus
stalls:

1. **Check logs for timeout messages.** If you see frequent `leader_timeout`
   events, increase `BLOCK_TIME_MS` to 100 or 200.

2. **Latency testing.** From inside a validator container, ping the other
   validators:
   ```bash
   # From validator-1
   nc -z validator-0.railway.internal 30303
   ```
   If latency exceeds 20ms, 50ms blocks will be unreliable.

3. **Region colocation.** All services must be in the same Railway region. If
   services land in different regions, cross-region latency will exceed the block
   time. Railway does not currently let you pin services to specific
   infrastructure within a region, but all services in a project typically
   colocate.

4. **Tuning fallback values.** If 50ms proves too aggressive:
   ```
   BLOCK_TIME_MS=100  -> leader=100ms, cert=200ms (reliable on Railway)
   BLOCK_TIME_MS=200  -> leader=200ms, cert=400ms (conservative)
   ```

### How BLOCK_TIME_MS reaches the binary

The flow after the `01-block-time-migration.md` changes:

1. `BLOCK_TIME_MS=50` environment variable is set on the Railway service.
2. The `kora` binary loads `NodeConfig` from its config file (TOML/JSON). The
   `ExecutionConfig` struct has field `block_time_ms` with serde default `2000`.
3. If the binary is wired to read `BLOCK_TIME_MS` from the environment (see
   note in `01-block-time-migration.md` section 11), it overrides
   `config.execution.block_time_ms`.
4. `ProductionRunner::new(...)` receives `block_time_ms` and sets the simplex
   consensus config timeouts from it.

**If the env-var-to-config wiring is not yet implemented,** create a small TOML
config file and mount it into the container:

```toml
# /shared/kora.toml
[execution]
block_time_ms = 50
gas_limit = 250000000
```

Then add `--config /shared/kora.toml` to the validator start command via the
entrypoint, or override the `CMD`:

```bash
/scripts/entrypoint.sh validator --config /shared/kora.toml
```

The entrypoint script passes extra arguments (`"$@"`) through to the `kora
validator` command, but only **after** its own positional arguments. Currently
the entrypoint calls:

```bash
exec /usr/local/bin/kora validator \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    "$@"
```

So extra args appended via `CMD` work. However, `--config` is a global CLI flag
(parsed before subcommands), so it needs to go before `validator`:

```bash
kora --config /shared/kora.toml validator --data-dir /data --peers /shared/peers.json --chain-id 13370
```

The entrypoint script does not support this ordering. **You will need to either:**

(a) Modify the entrypoint to support a `CONFIG_FILE` env var:
```bash
CONFIG_ARG=""
if [[ -n "${CONFIG_FILE:-}" ]]; then
    CONFIG_ARG="--config $CONFIG_FILE"
fi
exec /usr/local/bin/kora $CONFIG_ARG validator \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    "$@"
```

(b) Or bypass the entrypoint entirely and set the Railway start command to:
```bash
/usr/local/bin/kora --config /shared/kora.toml validator --data-dir /data --peers /shared/peers.json --chain-id 13370
```

Option (b) is simpler for a Railway deployment. You lose the health-check
readiness touch (`/data/.ready`) and the bootstrap-peer wait logic from the
entrypoint -- but Railway's own health checks and service dependencies can
replace those.

---

## 8. Deploy Script

This script deploys the full testnet to Railway using the `railway` CLI. Run it
from the repo root.

```bash
#!/usr/bin/env bash
# deploy-railway.sh -- Deploy 3-validator daeji testnet to Railway
set -euo pipefail

PROJECT_NAME="${RAILWAY_PROJECT:-daeji-testnet}"
CHAIN_ID="${CHAIN_ID:-13370}"
BLOCK_TIME_MS="${BLOCK_TIME_MS:-50}"

echo "=== Daeji Railway Deployment ==="
echo "  Project:      $PROJECT_NAME"
echo "  Chain ID:     $CHAIN_ID"
echo "  Block time:   ${BLOCK_TIME_MS}ms"
echo ""

# Ensure railway CLI is installed
if ! command -v railway &>/dev/null; then
    echo "ERROR: railway CLI not found. Install: npm install -g @railway/cli"
    exit 1
fi

# Check login
if ! railway whoami &>/dev/null; then
    echo "ERROR: Not logged in. Run: railway login"
    exit 1
fi

echo "[1/5] Creating Railway project..."
railway init --name "$PROJECT_NAME" 2>/dev/null || echo "  Project may already exist"

echo "[2/5] Creating volumes..."
# Railway CLI volume creation -- adjust syntax per CLI version.
# These may need to be created via the dashboard if the CLI does not support it.
echo "  NOTE: Create these volumes in the Railway dashboard if CLI does not support:"
echo "    - shared-config (mount at /shared on all services)"
echo "    - data-node0    (mount at /data on validator-0, at /shared/node0 on init-config)"
echo "    - data-node1    (mount at /data on validator-1, at /shared/node1 on init-config)"
echo "    - data-node2    (mount at /data on validator-2, at /shared/node2 on init-config)"

echo "[3/5] Creating services..."
# Create each service. Each points to the same repo but has different env vars.
for svc in init-config validator-0 validator-1 validator-2; do
    echo "  Creating service: $svc"
    railway service create --name "$svc" 2>/dev/null || echo "    Service may already exist"
done

echo "[4/5] Setting environment variables..."

# Common vars for all services
for svc in init-config validator-0 validator-1 validator-2; do
    railway variables set \
        --service "$svc" \
        CHAIN_ID="$CHAIN_ID" \
        RUST_LOG=info \
        SHARED_DIR=/shared \
        DATA_DIR=/data
done

# Validator-specific vars
railway variables set --service validator-0 \
    VALIDATOR_INDEX=0 \
    IS_BOOTSTRAP=true \
    BOOTSTRAP_PEERS="" \
    BLOCK_TIME_MS="$BLOCK_TIME_MS" \
    GAS_LIMIT=250000000 \
    HEALTHCHECK_MODE=ready

railway variables set --service validator-1 \
    VALIDATOR_INDEX=1 \
    IS_BOOTSTRAP=false \
    BOOTSTRAP_PEERS="validator-0.railway.internal:30303" \
    BLOCK_TIME_MS="$BLOCK_TIME_MS" \
    GAS_LIMIT=250000000 \
    HEALTHCHECK_MODE=ready

railway variables set --service validator-2 \
    VALIDATOR_INDEX=2 \
    IS_BOOTSTRAP=false \
    BOOTSTRAP_PEERS="validator-0.railway.internal:30303" \
    BLOCK_TIME_MS="$BLOCK_TIME_MS" \
    GAS_LIMIT=250000000 \
    HEALTHCHECK_MODE=ready

echo "[5/5] Deploying..."
echo ""
echo "  Deployment order:"
echo "    1. Build all services (Railway builds on push)"
echo "    2. Run init-config first (manually trigger or deploy)"
echo "    3. Wait for init-config to complete (check logs)"
echo "    4. Stop init-config"
echo "    5. Start validator-0, wait for healthy"
echo "    6. Start validator-1 and validator-2"
echo ""
echo "  To deploy:"
echo "    railway up --service init-config"
echo "    # Wait for init logs to show '[init] Done'"
echo "    railway service stop --name init-config"
echo "    railway up --service validator-0"
echo "    # Wait for health check"
echo "    railway up --service validator-1"
echo "    railway up --service validator-2"
echo ""
echo "=== Setup complete. Follow the steps above to deploy. ==="
```

---

## 9. Verifying the Deployment

### 9.1 Check service health

```bash
# Railway dashboard: each validator should show "Running" with green health
# Or via CLI:
railway logs --service validator-0 --tail 50
railway logs --service validator-1 --tail 50
railway logs --service validator-2 --tail 50
```

### 9.2 Check logs for consensus

Look for these log lines in each validator:

```
[entrypoint] Running validator mode...
Starting validator
Loaded DKG output
Loaded threshold signing scheme
```

And then consensus activity:

```
leader_timeout=50ms
Finalized block
```

If you see repeated `leader_timeout` without `Finalized block`, consensus is
stalling -- increase `BLOCK_TIME_MS`.

### 9.3 RPC smoke test

If you exposed RPC on validator-0:

```bash
# Get the Railway public URL from the dashboard
RAILWAY_URL="https://validator-0-production-XXXX.up.railway.app"

# Check node status (custom RPC method)
curl -s -X POST "$RAILWAY_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq .

# Expected response includes: validatorIndex, uptimeSecs, currentView, finalizedCount
```

### 9.4 Block production rate

With 50ms blocks, you should see ~20 blocks per second. After 60 seconds of
uptime, `finalizedCount` should be roughly 1,200. If it is significantly lower,
the block time is too aggressive for the current network conditions.

### 9.5 Check peer connectivity

Each validator's logs should show it connecting to the other two validators.
If validator-1 or validator-2 cannot reach validator-0, you will see:

```
Waiting for bootstrap peer validator-0.railway.internal:30303...
Timeout waiting for bootstrap peer
```

This means Railway's internal DNS is not resolving. Verify the service name in
Railway matches exactly what `BOOTSTRAP_PEERS` expects.

---

## 10. Monitoring (Optional)

### Prometheus

Add a Prometheus service to the Railway project:

- **Docker image:** `prom/prometheus:latest`
- **Volume:** Mount a `prometheus-data` volume at `/prometheus`
- **Config file:** Create a `prometheus.yml` and mount it at
  `/etc/prometheus/prometheus.yml`

```yaml
# prometheus.yml for Railway
global:
  scrape_interval: 5s
  evaluation_interval: 5s

scrape_configs:
  - job_name: 'kora-validators'
    static_configs:
      - targets:
          - 'validator-0.railway.internal:9002'
          - 'validator-1.railway.internal:9002'
          - 'validator-2.railway.internal:9002'
    relabel_configs:
      - source_labels: [__address__]
        regex: 'validator-(\d+)\.railway\.internal:.*'
        target_label: validator_index
        replacement: '$1'
```

### Grafana

- **Docker image:** `grafana/grafana:latest`
- **Volume:** Mount `grafana-data` at `/var/lib/grafana`
- **Public endpoint:** Expose port 3000
- **Environment:**
  ```
  GF_SECURITY_ADMIN_USER=admin
  GF_SECURITY_ADMIN_PASSWORD=<strong-password>
  GF_AUTH_ANONYMOUS_ENABLED=true
  GF_AUTH_ANONYMOUS_ORG_ROLE=Viewer
  ```
- **Datasource:** Point to `http://prometheus.railway.internal:9090`

The Grafana dashboard JSON at `docker/grafana/dashboards/kora-overview.json` can
be imported directly. Update the Prometheus datasource name to match.

### Railway Logs

Even without Prometheus/Grafana, Railway's built-in log viewer works. Use:

```bash
railway logs --service validator-0 --follow
```

---

## 11. Troubleshooting

### init-config fails with "Failed to read setup.json"

The `keygen dkg-deal` command reads `setup.json` from each `node{i}` directory.
If `keygen setup` did not run first (or the volume was not mounted correctly),
this file will be missing.

**Fix:** Ensure the init-config start command runs `keygen setup` *before*
`keygen dkg-deal`. Check volume mounts -- `data-node0` must be mounted at
`/shared/node0`, not `/shared/node-0` or `/data/node0`.

### Validators crash with "genesis.json not found"

The entrypoint checks for `/shared/genesis.json`. If init-config did not run, or
the `shared-config` volume is not mounted on the validator, this file is missing.

**Fix:** Verify the `shared-config` volume is mounted at `/shared` on all
validators (read-only is fine: `/shared:ro`).

### Validators crash with "share.key not found (run DKG first)"

The validator's data directory (`/data`) must contain `share.key` and
`output.json` from the DKG. These are generated by `keygen dkg-deal` and written
to `/shared/node{i}/`.

**Fix:** The init-config service must mount `data-node{i}` at `/shared/node{i}`.
The validator must mount the same `data-node{i}` volume at `/data`. This is how
the DKG output from init-config becomes available to the validator.

### "Timeout waiting for bootstrap peer"

The entrypoint waits up to 120 seconds for `validator-0.railway.internal:30303`
to be reachable.

Common causes:
1. **validator-0 is not running.** Start it first.
2. **Wrong hostname.** The Railway service must be named exactly `validator-0`
   for `validator-0.railway.internal` to resolve.
3. **Services in different Railway projects.** Internal DNS only works within the
   same project.
4. **Railway internal networking disabled.** Check the Railway dashboard --
   internal networking must be enabled for the project.

### Consensus stalls (no blocks finalized)

If logs show repeated `leader_timeout` events but no `Finalized block`:

1. **Block time too low.** Increase `BLOCK_TIME_MS` to 100 or 200.
2. **Only 1 of 3 validators running.** Need at least 2 for the 2-of-3 threshold.
3. **DKG output mismatch.** All validators must use shares from the *same* DKG
   run. If you re-ran init-config for one validator but not the others, their
   shares are incompatible. **Fix:** Delete all data volumes, re-run init-config,
   restart all validators.
4. **Clock skew.** Unlikely on Railway but possible. The consensus timeouts are
   relative (not wall-clock), so this is rare.

### Railway build fails

- **Rust nightly not available.** The Dockerfile uses `rustlang/rust:nightly-bookworm`.
  Railway's Docker builder pulls this from Docker Hub. If Docker Hub rate limits
  apply, the build may fail.
- **Build timeout.** Rust builds are slow. The first build (before cargo-chef
  caching kicks in) can take 10-20 minutes. Railway's default build timeout may
  be too short. Increase it in project settings if available.
- **Out of memory during build.** Rust linking is memory-intensive. If Railway's
  builder OOMs, try reducing parallel codegen units in `.cargo/config.toml`:
  ```toml
  [profile.release]
  codegen-units = 1
  ```

### Port conflict on Railway

Railway assigns ports dynamically. If you see "address already in use" errors,
it is likely because:
1. The previous deploy did not shut down cleanly.
2. Two replicas of the same service are running.

**Fix:** Set `numReplicas = 1` in `railway.toml`. Redeploy.

### Volumes lost after redeploy

Railway volumes persist across deploys by default. However, if you delete and
recreate a service, the volume attachment may be lost.

**Fix:** Reattach volumes in the Railway dashboard after service recreation.
Never delete volumes unless you want to lose state (keys, shares).

---

## 12. Anti-Patterns

These are mistakes that will break the deployment. Do not do them.

1. **Do not use public networking for P2P.** All `BOOTSTRAP_PEERS` values must
   use `*.railway.internal` hostnames. Public URLs add TLS termination and
   HTTP routing that break raw TCP P2P connections.

2. **Do not hardcode IP addresses.** Railway IP addresses change between
   deploys. Always use the `<service-name>.railway.internal` DNS names.

3. **Do not skip health checks.** Without health checks, Railway cannot
   determine if a service is ready. Dependent services (validator-1, validator-2)
   may start before validator-0 is ready, causing connection failures.

4. **Do not use interactive DKG for Railway.** The interactive DKG requires all
   nodes to be online simultaneously and coordinate in real time. Use
   `keygen dkg-deal` (trusted dealer) instead. It runs in a single process, is
   deterministic, and does not require network connectivity between DKG
   participants.

5. **Do not run init-config and validators simultaneously.** Init-config writes
   to the shared volume. If validators start before init-config finishes, they
   will find partial or missing files and crash.

6. **Do not run multiple replicas of the same validator.** Each validator has a
   unique identity key and BLS share. Running two instances of validator-0 will
   cause duplicate message signing and consensus violations.

7. **Do not set `BLOCK_TIME_MS=0`.** A zero-length timeout causes the consensus
   engine to spin. Minimum practical value is ~10ms for local testing, ~50ms for
   Railway.

8. **Do not share data volumes between validators.** Each validator must have
   its own `/data` volume. Sharing a data volume between validator-0 and
   validator-1 means they would use the same identity key and BLS share, which
   breaks consensus.

9. **Do not forget to patch `peers.json` hostnames.** The `keygen setup` command
   writes `node0:30303` style hostnames. These do not resolve on Railway. The
   init-config command must `sed` them to `validator-0.railway.internal:30303`.

---

## 13. Checklist

### Pre-deploy

- [ ] `01-block-time-migration.md` changes are merged (codebase uses `block_time_ms`)
- [ ] Railway CLI installed (`npm install -g @railway/cli`)
- [ ] Logged into Railway (`railway login`)
- [ ] Railway project created
- [ ] Docker builds successfully locally (`cd docker && just build`)

### Volume setup

- [ ] Volume `shared-config` created, mounted at `/shared` on all services
- [ ] Volume `data-node0` created, mounted at `/data` on validator-0 and `/shared/node0` on init-config
- [ ] Volume `data-node1` created, mounted at `/data` on validator-1 and `/shared/node1` on init-config
- [ ] Volume `data-node2` created, mounted at `/data` on validator-2 and `/shared/node2` on init-config

### Environment variables

- [ ] `CHAIN_ID=13370` on all services
- [ ] `BLOCK_TIME_MS=50` on all validators
- [ ] `RUST_LOG=info` on all services
- [ ] `VALIDATOR_INDEX` set correctly per validator (0, 1, 2)
- [ ] `IS_BOOTSTRAP=true` on validator-0 only
- [ ] `BOOTSTRAP_PEERS=validator-0.railway.internal:30303` on validator-1 and validator-2
- [ ] `HEALTHCHECK_MODE=ready` on all validators

### Init sequence

- [ ] init-config service deployed and started
- [ ] init-config logs show `[init] Done`
- [ ] `peers.json` contains `validator-X.railway.internal:30303` (not `nodeX:30303`)
- [ ] `share.key` and `output.json` exist in each node directory
- [ ] init-config service stopped

### Validator startup

- [ ] validator-0 started and healthy (P2P port 30303 listening)
- [ ] validator-1 started and connected to bootstrap
- [ ] validator-2 started and connected to bootstrap
- [ ] All 3 validators show `Finalized block` in logs

### Verification

- [ ] `kora_nodeStatus` RPC returns data for all validators
- [ ] `finalizedCount` is increasing over time
- [ ] Block production rate is ~20 blocks/sec (for 50ms blocks)
- [ ] No `leader_timeout` storms in logs

### Networking

- [ ] P2P uses internal DNS only (`*.railway.internal`)
- [ ] RPC exposed publicly on exactly one validator
- [ ] No P2P ports exposed publicly

---

## 14. Reference: File Locations

| File | Path | Purpose |
|---|---|---|
| Dockerfile | `docker/Dockerfile` | Multi-stage build |
| Entrypoint | `docker/scripts/entrypoint.sh` | Mode dispatch (validator, dkg, secondary) |
| Health check | `docker/scripts/healthcheck.sh` | P2P port + readiness check |
| Compose (reference) | `docker/compose/devnet.yaml` | Local devnet compose file (not used on Railway) |
| Prometheus config | `docker/config/prometheus.yml` | Scrape config template |
| Grafana dashboards | `docker/grafana/dashboards/` | Pre-built dashboard JSON |
| keygen binary | Built from `bin/keygen/` | Key generation and setup |
| kora binary | Built from `bin/kora/` | Validator node |
| Node config crate | `crates/node/config/` | `NodeConfig`, `ExecutionConfig`, `NetworkConfig` |
| Runner crate | `crates/node/runner/` | `ProductionRunner` with consensus timeouts |
| Block time migration | `tmp/HDC/impl/01-block-time-migration.md` | Prerequisite for 50ms blocks |

---

## 15. Disaster Recovery

### 15.1 Key Backup Procedure

Validator keys and BLS threshold shares are the most critical data. Loss of
keys requires a full DKG re-ceremony.

```bash
# After init-config completes, back up the shared volume immediately
railway volume export shared-config --output daeji-keys-backup.tar.gz

# Back up individual validator data volumes
for i in 0 1 2; do
    railway volume export data-node${i} --output daeji-node${i}-backup.tar.gz
done

# Store backups in at least two locations:
# 1. Encrypted cloud storage (S3, GCS)
# 2. Local encrypted disk
```

**Critical files to back up per validator:**

| File | Path | Purpose |
|------|------|---------|
| `validator.key` | `/data/validator.key` | Ed25519 identity key |
| `share.key` | `/data/share.key` | BLS threshold share |
| `output.json` | `/data/output.json` | DKG output with group public key |
| `setup.json` | `/data/setup.json` | Validator index metadata |
| `peers.json` | `/shared/peers.json` | Peer configuration |
| `genesis.json` | `/shared/genesis.json` | Genesis state |

### 15.2 Volume Snapshot Schedule

Railway volumes should be snapshotted regularly:

- **Daily:** Snapshot all data volumes (`data-node0`, `data-node1`, `data-node2`)
- **On every re-deploy:** Snapshot before destroying services
- **Before DKG re-ceremony:** Snapshot all volumes as rollback point

### 15.3 Validator Restart Procedure

If a single validator crashes or needs restart:

```bash
# 1. Check logs for crash reason
railway logs --service validator-1 --tail 100

# 2. If data volume is intact, simply restart
railway service restart --name validator-1

# 3. If data volume is corrupted, restore from backup
railway volume import data-node1 --input daeji-node1-backup.tar.gz
railway service restart --name validator-1

# 4. Verify re-joining consensus
railway logs --service validator-1 --follow
# Look for: "Finalized block" entries within 30 seconds
```

If ALL validators crash simultaneously:

1. Check that `shared-config` volume is intact
2. Restart `validator-0` first (bootstrap node)
3. Wait for `validator-0` to be healthy (30303 listening)
4. Restart `validator-1` and `validator-2`
5. Monitor for consensus resumption

### 15.4 Full Recovery from Backup

```bash
# 1. Create new Railway project
railway init --name daeji-testnet-recovery

# 2. Create volumes
# (via dashboard -- create shared-config, data-node0, data-node1, data-node2)

# 3. Import backups
railway volume import shared-config --input daeji-keys-backup.tar.gz
for i in 0 1 2; do
    railway volume import data-node${i} --input daeji-node${i}-backup.tar.gz
done

# 4. Create services and set env vars (same as initial deploy)
# 5. Start validators in order
```

---

## 16. Validator Onboarding (Adding a 4th Validator Post-Launch)

Adding a 4th validator to a running 3-validator network requires a new DKG
ceremony because BLS threshold shares are not additive.

### Step 1: Plan the new threshold

With 4 validators, the BFT threshold should be 3 (3-of-4). This tolerates 1
faulty validator.

### Step 2: Generate new keys (offline)

```bash
# On a secure machine, NOT on Railway
keygen setup --validators=4 --threshold=3 --chain-id=13370 --output-dir=/tmp/dkg4

# Patch hostnames for Railway
sed -i \
    -e 's/node0:30303/validator-0.railway.internal:30303/g' \
    -e 's/node1:30303/validator-1.railway.internal:30303/g' \
    -e 's/node2:30303/validator-2.railway.internal:30303/g' \
    -e 's/node3:30303/validator-3.railway.internal:30303/g' \
    /tmp/dkg4/peers.json

keygen dkg-deal --validators=4 --threshold=3 --output-dir=/tmp/dkg4
```

### Step 3: Coordinate shutdown window

All 4 validators must use shares from the SAME DKG ceremony. You cannot mix
3-of-3 shares with 3-of-4 shares.

1. **Stop all 3 existing validators** (briefly -- consensus will halt)
2. Replace keys on all existing validators with new 4-validator DKG output
3. Create `validator-3` service on Railway with new keys
4. Start all 4 validators

### Step 4: Create the new Railway service

```bash
railway service create --name validator-3

railway variables set --service validator-3 \
    CHAIN_ID=13370 \
    VALIDATOR_INDEX=3 \
    IS_BOOTSTRAP=false \
    BOOTSTRAP_PEERS="validator-0.railway.internal:30303" \
    BLOCK_TIME_MS=50 \
    GAS_LIMIT=250000000 \
    RUST_LOG=info \
    DATA_DIR=/data \
    SHARED_DIR=/shared \
    HEALTHCHECK_MODE=ready
```

### Step 5: Verify

```bash
# All 4 validators should show "Finalized block" within 60 seconds
for svc in validator-0 validator-1 validator-2 validator-3; do
    railway logs --service $svc --tail 20 | grep "Finalized block"
done
```

---

## 17. Monitoring and Alerting Rules

### Critical alerts (page immediately)

| Condition | Check | Threshold |
|-----------|-------|-----------|
| Block height stale | `finalizedCount` not increasing | Stale for > 10 seconds |
| Finalized block lag | Gap between proposed and finalized | > 5 blocks |
| Validator down | Health check failing | Any validator offline > 30s |
| DKG share mismatch | Consensus errors in logs | Any occurrence |

### Warning alerts (review within 1 hour)

| Condition | Check | Threshold |
|-----------|-------|-----------|
| Memory usage high | Container memory | > 80% of 2GB allocation |
| Disk usage high | Volume usage | > 70% of 10GB allocation |
| Leader timeout storms | `leader_timeout` log frequency | > 10 per second |
| Peer disconnection | P2P connection count | < 2 peers for > 60s |

### Prometheus alerting rules

```yaml
# prometheus/alerts.yml
groups:
  - name: daeji-critical
    rules:
      - alert: BlockHeightStale
        expr: increase(kora_finalized_blocks_total[10s]) == 0
        for: 10s
        labels:
          severity: critical
        annotations:
          summary: "Block height stale on {{ $labels.validator_index }}"

      - alert: FinalizedBlockLag
        expr: kora_proposed_height - kora_finalized_height > 5
        for: 30s
        labels:
          severity: critical
        annotations:
          summary: "Finalized block lag > 5 on {{ $labels.validator_index }}"

  - name: daeji-warnings
    rules:
      - alert: HighMemoryUsage
        expr: process_resident_memory_bytes / 2147483648 > 0.8
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Memory > 80% on {{ $labels.validator_index }}"

      - alert: LeaderTimeoutStorm
        expr: rate(kora_leader_timeouts_total[1m]) > 10
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "Excessive leader timeouts on {{ $labels.validator_index }}"
```

### Health check verification commands

```bash
# Quick health check from local machine (requires public RPC)
RAILWAY_URL="https://validator-0-production-XXXX.up.railway.app"

# Check block production is ongoing
curl -s -X POST "$RAILWAY_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' \
    | jq '.result.finalizedCount'

# Wait 10 seconds, check again -- count should have increased by ~200
sleep 10
curl -s -X POST "$RAILWAY_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' \
    | jq '.result.finalizedCount'

# Check peer count (should be 2 for a 3-validator network)
curl -s -X POST "$RAILWAY_URL" \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}' \
    | jq '.result'
```

---

## Audit Findings

Audit date: 2026-05-08. Compared spec sections 1-14 against actual implementation
files on branch `hdc`.

### F01: `BLOCK_TIME_MS`, `GAS_LIMIT`, `HDC_ENABLED` env vars are never read by the binary

**Severity: Critical**

The spec (section 7) says `BLOCK_TIME_MS=50` is set as an environment variable on
each Railway service and the binary picks it up. In reality, the `kora` binary has
**zero** env-var-to-config bridging. There is no `std::env::var("BLOCK_TIME_MS")`
call anywhere in:

- `bin/kora/src/cli.rs` (lines 76-87, the `load_config` method only merges CLI
  `--chain-id` and `--data-dir` overrides, nothing else)
- `crates/node/config/src/node.rs` (pure file-based deserialization, no env overlay)
- `crates/node/config/src/execution.rs` (serde defaults only)

The same applies to `GAS_LIMIT` and `HDC_ENABLED`. These env vars are set in
`deploy/railway/validator-*.env` and `docker/compose/fast-devnet.yaml` but are
**silently ignored** by the binary. The validators will run with the compiled-in
defaults: `block_time_ms=2000`, `gas_limit=250_000_000`, `hdc.enabled=false`.

The entrypoint (`docker/scripts/entrypoint.sh`, line 79-82) does support a
`CONFIG_FILE` env var to pass `--config <file>` to the binary. But neither the
fast-devnet compose file nor the Railway env files set `CONFIG_FILE`, and no TOML
config file is generated or mounted for Railway.

**Result:** On Railway, validators will produce 2-second blocks (not 50ms), and HDC
precompiles will be disabled, silently contradicting the env vars set in the
dashboard.

### F02: `healthcheckPath = "/"` in `railway.toml` is likely wrong

**Severity: Medium**

`railway.toml` line 17 sets `healthcheckPath = "/"`. This tells Railway to send an
HTTP GET to `/` on the service's port. The kora RPC server listens on 8545 and
speaks JSON-RPC, not REST. A GET to `/` will likely return an error or empty
response, causing Railway to consider the service unhealthy.

The spec itself (section 3) notes:

> Railway's `healthcheckPath` is for HTTP health checks. For TCP-based health
> checks (P2P port), rely on the Dockerfile `HEALTHCHECK` instruction instead.
> The `healthcheckPath` setting can be removed if you are not exposing HTTP on
> the health check port.

The Dockerfile does include a proper `HEALTHCHECK` instruction (line 85-86) that
uses `/scripts/healthcheck.sh`, which checks both `/data/.ready` and
`nc -z localhost 30303`. The `healthcheckPath` in `railway.toml` should be removed
to avoid conflicting with the Dockerfile health check.

### F03: `fast-devnet.yaml` does not generate or mount a config TOML file

**Severity: High**

The compose file `docker/compose/fast-devnet.yaml` sets `BLOCK_TIME_MS=50`,
`GAS_LIMIT=250000000`, and `HDC_ENABLED=true` as environment variables (lines
33-36, 78-81, etc.), but:

1. No `CONFIG_FILE` env var is set on any validator service.
2. No TOML/JSON config file is generated by `init-config` or mounted into
   validator containers.
3. The entrypoint script will see `CONFIG_FILE` is unset, set `CONFIG_ARG=""`,
   and invoke `kora validator` without `--config`, causing it to use
   `NodeConfig::default()` which has `block_time_ms=2000` and `hdc.enabled=false`.

The `init-config` service (lines 40-64) runs `keygen setup` and `keygen dkg-deal`
but does not produce a `kora.toml` config file.

### F04: `deploy/railway/init-config.sh` patches hostnames but does not produce a config file

**Severity: High**

The Railway init script (`deploy/railway/init-config.sh`) correctly patches
`peers.json` hostnames (lines 17-22) and runs the DKG, but does not generate a
`kora.toml` with `block_time_ms = 50`, `gas_limit = 250000000`, or
`[hdc] enabled = true`. Without this file, the Railway validators use defaults.

### F05: `CHAIN_ID` default mismatch between entrypoint and spec

**Severity: Low**

The entrypoint script (`docker/scripts/entrypoint.sh`, line 7) defaults
`CHAIN_ID` to `1337`:

```bash
CHAIN_ID=${CHAIN_ID:-1337}
```

The spec and all env files use `13370`. The Railway env files do set
`CHAIN_ID=13370` explicitly, so this is not a production bug -- but the default
mismatch is a latent hazard if the env var is ever omitted.

### F06: Deploy script from spec section 8 does not exist

**Severity: Low**

The spec describes a `deploy-railway.sh` script (section 8) but it was not
implemented as a standalone file. The `docker/Justfile` has a `railway-init`
recipe (lines 115-131) that partially replaces it -- it builds the image and runs
`init-config.sh` in a Docker container. But there is no automated `railway
variables set` or `railway service create` workflow. Deployment is manual via the
Railway dashboard.

### F07: `fast-devnet.yaml` `init-config` does not patch `peers.json` hostnames

**Severity: Low (local devnet only)**

The `init-config` service in `docker/compose/fast-devnet.yaml` (lines 40-59) runs
`keygen setup` and `keygen dkg-deal` but does not `sed` the `peers.json`
hostnames. This is acceptable for local compose (where `hostname: node0` on line
68 makes `node0:30303` resolve), but the Railway `init-config.sh` does patch
them. This asymmetry means the compose file cannot be used directly on Railway.

---

## Second-Pass Remediation Detail

This remediation keeps the existing image and entrypoint model: `deploy/railway`
owns Railway service variables, `deploy/railway/init-config.sh` owns generated
shared artifacts, and validators continue to start through
`/scripts/entrypoint.sh validator`. The critical change is to make
`/shared/kora.toml` the authoritative generated runtime config and set
`CONFIG_FILE=/shared/kora.toml` on every validator.

### Config generation path

Generate the validator config in the init-config service at:

```
/shared/kora.toml
```

The concrete generation point is after `keygen setup` and the Railway hostname
rewrite of `/shared/peers.json`, before the init service prints completion. The
file must be written to a temporary path and atomically moved into place so a
terminated init run cannot leave a partial config:

```bash
CONFIG_TMP="${SHARED_DIR}/kora.toml.tmp"
CONFIG_FILE="${SHARED_DIR}/kora.toml"

cat > "${CONFIG_TMP}" <<EOF
chain_id = ${CHAIN_ID}
data_dir = "/data"

[execution]
block_time_ms = ${BLOCK_TIME_MS}
gas_limit = ${GAS_LIMIT}

[network]
listen_addr = "0.0.0.0:30303"

[hdc]
enabled = ${HDC_TOML_ENABLED}
knowledge_store_path = "/data/hdc/knowledge"
max_entries = ${HDC_MAX_ENTRIES}
EOF

mv "${CONFIG_TMP}" "${CONFIG_FILE}"
```

Validators then load this file through the already implemented entrypoint path:

```bash
CONFIG_FILE=/shared/kora.toml
/scripts/entrypoint.sh validator
```

The entrypoint places `--config "$CONFIG_FILE"` before the `validator`
subcommand, so this path reaches `NodeConfig::load(...)` correctly. Keep
`--data-dir "$DATA_DIR"` and `--chain-id "$CHAIN_ID"` in the entrypoint because
those CLI globals intentionally override the generated config for the mounted
volume and chain ID.

### Env var handling

Do not rely on `BLOCK_TIME_MS`, `GAS_LIMIT`, or `HDC_ENABLED` being read by the
binary. They are not read by the current CLI/config layer. Treat them as inputs
to init-config only, render them into `/shared/kora.toml`, and treat the TOML as
the source of truth at validator runtime.

Required init-config variables:

```bash
CHAIN_ID=13370
BLOCK_TIME_MS=50
GAS_LIMIT=250000000
HDC_ENABLED=true
HDC_MAX_ENTRIES=100000
NUM_VALIDATORS=3
THRESHOLD=2
SHARED_DIR=/shared
```

Required validator variables:

```bash
CHAIN_ID=13370
DATA_DIR=/data
SHARED_DIR=/shared
CONFIG_FILE=/shared/kora.toml
HEALTHCHECK_MODE=ready
VALIDATOR_INDEX=<0|1|2>
IS_BOOTSTRAP=<true only for validator-0>
BOOTSTRAP_PEERS=<empty for validator-0, validator-0.railway.internal:30303 otherwise>
```

Normalize booleans and validate numeric values before writing TOML:

```bash
case "${HDC_ENABLED:-true}" in
  true|1|yes|on) HDC_TOML_ENABLED=true ;;
  false|0|no|off) HDC_TOML_ENABLED=false ;;
  *) echo "ERROR: invalid HDC_ENABLED=${HDC_ENABLED}" >&2; exit 1 ;;
esac

for numeric in CHAIN_ID BLOCK_TIME_MS GAS_LIMIT HDC_MAX_ENTRIES NUM_VALIDATORS THRESHOLD; do
  value="${!numeric:-}"
  case "$value" in
    ''|*[!0-9]*) echo "ERROR: ${numeric} must be numeric, got '${value}'" >&2; exit 1 ;;
  esac
done
```

`RPC_ADDR`, `P2P_ADDR`, `NODE_ID`, `BLOCK_TIME_MS`, `GAS_LIMIT`, and
`HDC_ENABLED` may remain in Railway for operator visibility, but only
`CONFIG_FILE`, `CHAIN_ID`, `DATA_DIR`, `SHARED_DIR`, `IS_BOOTSTRAP`,
`BOOTSTRAP_PEERS`, and `HEALTHCHECK_MODE` affect validator startup unless the
generated config is updated. `NUM_VALIDATORS` and `THRESHOLD` affect only
init-config.

### Health check strategy

Remove or avoid `healthcheckPath = "/"` for the validator services. Railway's
HTTP health check is not the right readiness gate for this process because the
validator's JSON-RPC port is not a REST health endpoint.

Use the Dockerfile `HEALTHCHECK` and set this on validators:

```bash
HEALTHCHECK_MODE=ready
```

`/scripts/healthcheck.sh` then requires both:

1. `/data/.ready` exists, meaning the entrypoint found `genesis.json`,
   `validator.key`, `share.key`, and `output.json` and copied genesis into
   `/data`.
2. `localhost:30303` accepts TCP connections, meaning the P2P listener is up.

The init-config service is a one-shot job, not a long-running HTTP service.
Treat exit code 0 plus file validation as its health signal. If Railway applies
the image-level healthcheck to init-config before the job exits, disable service
health checks for init-config in Railway or run it manually and validate the
artifacts before starting validators.

### Startup, volume, and DNS sequencing

Use exactly one shared config volume and one data volume per validator:

| Service | Mounts |
|---|---|
| init-config | `shared-config:/shared`, `data-node0:/shared/node0`, `data-node1:/shared/node1`, `data-node2:/shared/node2` |
| validator-0 | `shared-config:/shared:ro`, `data-node0:/data` |
| validator-1 | `shared-config:/shared:ro`, `data-node1:/data` |
| validator-2 | `shared-config:/shared:ro`, `data-node2:/data` |

The deploy order is strict:

1. Create all four volumes and attach them as above.
2. Run init-config once. It generates `genesis.json`, `peers.json`,
   `kora.toml`, `validator.key`, `share.key`, `output.json`, and `setup.json`.
3. Verify `/shared/peers.json` contains Railway DNS names:
   `validator-0.railway.internal:30303`,
   `validator-1.railway.internal:30303`, and
   `validator-2.railway.internal:30303`. No `node0`, `node1`, or `node2`
   hostname may remain in the Railway artifact.
4. Stop init-config and leave it stopped.
5. Start `validator-0` first with `IS_BOOTSTRAP=true` and no
   `BOOTSTRAP_PEERS`.
6. Wait for `validator-0` to pass the Docker healthcheck.
7. Start `validator-1` and `validator-2` with
   `BOOTSTRAP_PEERS=validator-0.railway.internal:30303`.

Railway internal DNS only works inside the same Railway project. If a service is
renamed, update both `BOOTSTRAP_PEERS` and the `peers.json` hostname rewrite.
Do not use public Railway URLs for P2P.

### Signal handling

Keep validators on the Dockerfile entrypoint path or use an explicit `exec` in
any Railway start-command override. The existing entrypoint ends validator mode
with:

```bash
exec /usr/local/bin/kora $CONFIG_ARG validator ...
```

That makes `kora` replace the shell process, so Railway `SIGTERM` is delivered
to the validator process instead of being trapped by an intermediate shell. Do
not use a validator command shaped like this:

```bash
/bin/bash -c "/usr/local/bin/kora --config /shared/kora.toml validator ..."
```

If a custom command is required, make the shell hand off the process:

```bash
/bin/bash -lc 'exec /usr/local/bin/kora --config /shared/kora.toml validator --data-dir /data --peers /shared/peers.json --chain-id 13370'
```

For init-config, write generated files via `*.tmp` plus `mv`, and only log or
write an init-complete marker after all files validate. A terminated init run
must not look complete to an operator or dependent service.

### Non-root init

The runtime image already creates user `kora` with UID 1000 and switches to it.
The target Railway init-config service should also run as UID 1000 and should
not need `user: root` or `chown` in normal operation. Use restrictive creation
defaults:

```bash
umask 077
install -d -m 0700 "${SHARED_DIR}"
```

For the per-validator mounted directories, init-config should write directly to
`/shared/node0`, `/shared/node1`, and `/shared/node2` as UID 1000. Validators
then mount those same volumes at `/data` and continue as UID 1000. If Railway
creates a volume that UID 1000 cannot write, fix the volume ownership once as an
operational setup step before running init-config; do not make the normal
init-config path depend on root.

### Deployment validation

Validate the generated artifacts before any validator starts:

```bash
test -s /shared/genesis.json
test -s /shared/peers.json
test -s /shared/kora.toml
grep -q 'block_time_ms = 50' /shared/kora.toml
grep -q 'enabled = true' /shared/kora.toml
! grep -Eq 'node[0-9]+:30303' /shared/peers.json

for i in 0 1 2; do
  test -s "/shared/node${i}/validator.key"
  test -s "/shared/node${i}/share.key"
  test -s "/shared/node${i}/output.json"
  test -s "/shared/node${i}/setup.json"
done
```

Validate each Railway service before exposing RPC publicly:

```bash
railway logs --service validator-0 --tail 100
railway logs --service validator-1 --tail 100
railway logs --service validator-2 --tail 100
```

Required log evidence:

- `Loaded DKG output`
- `Loaded threshold signing scheme`
- `HDC subsystem enabled`
- `HDC RPC API registered`
- `Validator started successfully`

After `validator-0` public networking is enabled on port 8545, validate runtime
behavior through JSON-RPC:

```bash
curl -s -X POST "$RAILWAY_URL" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq .

curl -s -X POST "$RAILWAY_URL" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"hdc_encode","params":["railway smoke test"],"id":2}' | jq .
```

Sample `finalizedCount` twice, 10 seconds apart. At `BLOCK_TIME_MS=50`, a
healthy network should finalize roughly 200 blocks over that interval after the
validators have settled. A much lower rate or repeated timeout logs means the
config was not loaded or 50ms is too aggressive for the Railway placement.

### F08: No signal handling or graceful shutdown in entrypoint

**Severity: Low**

The entrypoint uses `exec` (line 84) which replaces the shell with the `kora`
process, correctly passing signals. However, the `railway.toml` sets
`sleepApplication = false` (line 14), meaning Railway will not attempt to sleep
the app before termination. The entrypoint does not install a `trap` for SIGTERM.
Since `exec` is used, SIGTERM goes directly to `kora`, which is correct -- but the
spec does not document the expected signal handling behavior.

### F09: Security -- `init-config` runs as root

**Severity: Medium**

The `fast-devnet.yaml` init-config runs with `user: root` (line 42) and the
Railway spec recommends the same (section 6, "Must run as root to chown"). This
is necessary because the keygen writes files that must be owned by UID 1000
(the `kora` user). However:

1. If Railway runs init-config as root and the volume permissions are set, then
   an attacker who compromises init-config gets root access to the shared volume.
2. The `chown -R 1000:1000 /shared/node*` command is the only reason root is
   needed. An alternative is to run keygen as UID 1000 from the start (it only
   writes to `/shared` which is already owned by `kora`).

### F10: `railway.toml` `restartPolicyMaxRetries = 10` may cause restart storms

**Severity: Low**

If a validator crashes due to a config error (e.g., missing DKG output), Railway
will restart it 10 times. Each restart will fail identically. The restart policy
is correct for transient failures but will cause noisy logs for persistent
configuration errors.

---

## Implementation Status

| Spec Section | Status | Notes |
|---|---|---|
| 1. Project Structure | Partial | No init-config Railway service definition; compose file has it for local use only |
| 2. Dockerfile | **Done** | Multi-stage build, cargo-chef, non-root user, HEALTHCHECK instruction all present |
| 3. railway.toml | **Done (with bug)** | `healthcheckPath = "/"` is wrong for JSON-RPC; should be removed |
| 4. Environment Variables | **Broken** | Env vars `BLOCK_TIME_MS`, `GAS_LIMIT`, `HDC_ENABLED` are set but never consumed by the binary |
| 5. Networking | Partial | Hostnames patched in `init-config.sh` but not in compose `init-config`; Railway DNS names correct in env files |
| 6. Init/Bootstrap Sequence | **Done** | `init-config.sh` exists, does keygen + DKG + hostname patching + chown |
| 7. 50ms Block Time | **Broken** | No config file is generated; `CONFIG_FILE` env var is never set; binary uses 2000ms default |
| 8. Deploy Script | Not implemented | Spec describes `deploy-railway.sh`; only `docker/Justfile:railway-init` exists (partial) |
| 9. Verification | N/A | Runbook-style section; no automation |
| 10. Monitoring | Not implemented | No Prometheus or Grafana services configured for Railway |
| 11. Troubleshooting | N/A | Documentation only |
| 12. Anti-patterns | N/A | Documentation only |
| 13. Checklist | N/A | Documentation only |
| 14. Reference | Accurate | File paths match actual repo structure |

---

## Anti-Patterns & Duct Tape

### AP-01: Phantom environment variables (Critical)

`BLOCK_TIME_MS`, `GAS_LIMIT`, and `HDC_ENABLED` are set as environment variables
in multiple places but are **never read** by any code path. This is the most
dangerous anti-pattern in the deployment: operators will see these values in the
Railway dashboard and believe they are in effect, but the binary ignores them.

**Files affected:**
- `docker/compose/fast-devnet.yaml` lines 33, 34, 36, 78-81, 99-101, 129-131
- `deploy/railway/validator-0.env` line 4
- `deploy/railway/validator-1.env` line 4
- `deploy/railway/validator-2.env` line 4

**Root cause:** The spec (section 7) acknowledges that env-var-to-config wiring
may not be implemented and describes a fallback (generate a TOML config file),
but neither approach was completed.

### AP-02: Entrypoint has `CONFIG_FILE` support but nothing uses it

`docker/scripts/entrypoint.sh` lines 79-82 implement the `CONFIG_FILE` env var
support exactly as the spec recommends in section 7, option (a). But no compose
file, env file, or init-config script sets `CONFIG_FILE` or generates the TOML
file that it would point to.

**Files affected:**
- `docker/scripts/entrypoint.sh` lines 79-82 (implemented but unused)
- `docker/compose/fast-devnet.yaml` (missing `CONFIG_FILE` env var)
- `deploy/railway/validator-*.env` (missing `CONFIG_FILE` env var)
- `deploy/railway/init-config.sh` (does not generate a TOML config file)

### AP-03: Two-layer configuration that silently fails

The system has two configuration layers that do not connect:
1. Environment variables (`BLOCK_TIME_MS=50`) set in compose/Railway
2. Config file deserialization (`NodeConfig::load()`) in the Rust binary

Neither layer knows about the other. The CLI (`cli.rs` lines 76-87) only bridges
`--chain-id` and `--data-dir` from CLI args to config, not from env vars. The
config crate (`crates/node/config/src/node.rs`) does not use `envy`, `figment`,
or any env-var overlay library.

### AP-04: HTTP health check on a JSON-RPC endpoint

`railway.toml` line 17 sets `healthcheckPath = "/"` which sends HTTP GET to the
RPC port. JSON-RPC servers expect POST requests with a JSON body. A GET to `/`
typically returns 405 Method Not Allowed or an empty response. This will cause
Railway to mark the service as unhealthy even when it is running correctly.

### AP-05: Root-mode init-config to chown files

Running init-config as root solely to `chown` the output files is a smell.
The keygen binary could be run as the `kora` user (UID 1000) directly, since the
output directories (`/shared/node*`) are mounted volumes that can be given the
right permissions at volume creation time.

### AP-06: Hardcoded 3-validator, threshold-2 assumptions

The init-config.sh and fast-devnet.yaml hardcode `--validators=3 --threshold=2`.
The Railway env files also hardcode `NUM_VALIDATORS=3` and `THRESHOLD=2`. There
is no parameterization path for changing the validator count without editing
multiple files. This is acceptable for an MVP but will not scale.

---

## Recommended Changes Checklist

### P0 -- Must fix before Railway deployment

- [ ] **Generate a TOML config file in init-config.** Modify
  `deploy/railway/init-config.sh` to write a `kora.toml` to `/shared/` containing
  `[execution] block_time_ms`, `gas_limit`, and `[hdc] enabled`. Template the
  values from env vars `BLOCK_TIME_MS`, `GAS_LIMIT`, `HDC_ENABLED`.
  **File:** `deploy/railway/init-config.sh`

- [ ] **Set `CONFIG_FILE=/shared/kora.toml` on all validator Railway env files.**
  The entrypoint already supports this env var (`docker/scripts/entrypoint.sh`
  lines 79-82). Add `CONFIG_FILE=/shared/kora.toml` to:
  - `deploy/railway/validator-0.env`
  - `deploy/railway/validator-1.env`
  - `deploy/railway/validator-2.env`

- [ ] **Do the same for `docker/compose/fast-devnet.yaml`.** Add `CONFIG_FILE`
  env var to each validator service and have init-config generate the TOML file.
  **File:** `docker/compose/fast-devnet.yaml` lines 74-85, 97-110, 117-135

- [ ] **Remove `healthcheckPath = "/"` from `railway.toml`.** The Dockerfile
  HEALTHCHECK instruction handles readiness checking. The HTTP health check path
  will cause false negatives.
  **File:** `railway.toml` line 17

### P1 -- Should fix before production use

- [ ] **Add env-var override support to `NodeConfig::load()`.** Use `figment`,
  `config`, or manual `std::env::var` calls to let env vars override config file
  values. At minimum, support `BLOCK_TIME_MS`, `GAS_LIMIT`, `CHAIN_ID`,
  `HDC_ENABLED`. This removes the dependency on generating a config file.
  **File:** `crates/node/config/src/node.rs` lines 66-77

- [ ] **Add `--block-time-ms` and `--gas-limit` CLI args.** Mirror the existing
  `--chain-id` and `--data-dir` pattern in `cli.rs` `load_config()`.
  **File:** `bin/kora/src/cli.rs` lines 76-87

- [ ] **Fix `CHAIN_ID` default in entrypoint.** Change line 7 of
  `docker/scripts/entrypoint.sh` from `CHAIN_ID=${CHAIN_ID:-1337}` to
  `CHAIN_ID=${CHAIN_ID:-13370}` to match the spec and all env files.
  **File:** `docker/scripts/entrypoint.sh` line 7

- [ ] **Run init-config as UID 1000 instead of root.** Remove `user: root` from
  the init-config service. Have the volumes created with correct ownership, or
  use an init container that only sets permissions.
  **File:** `docker/compose/fast-devnet.yaml` line 42,
  `deploy/railway/init-config.sh` lines 30-33

### P2 -- Nice to have

- [ ] **Implement `deploy-railway.sh` from spec section 8.** Automate service
  creation, volume creation, and env var setting via the Railway CLI.

- [ ] **Add `healthcheckTimeout` to `railway.toml` deploy section.** Increase
  the timeout or add `startPeriod` for initial build warmup.

- [ ] **Add Prometheus/Grafana Railway service definitions.** The spec describes
  these in section 10 but they are not implemented.

- [ ] **Parameterize validator count.** Replace hardcoded `3`/`2` with env vars
  in `init-config.sh` (already uses `NUM_VALIDATORS` and `THRESHOLD`) and
  extend to compose/Railway service definitions.

- [ ] **Add `railway.toml` `watchPatterns` for `deploy/` directory.** Currently
  changes to `deploy/railway/*.env` do not trigger rebuilds. Add `"deploy/**"`
  to the watch patterns.
  **File:** `railway.toml` lines 4-10
