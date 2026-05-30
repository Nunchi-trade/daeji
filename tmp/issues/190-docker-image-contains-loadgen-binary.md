# Docker Image Contains `loadgen` Binary -- Unnecessary Attack Surface in Production

**Category:** Security -- Docker/Deployment
**Severity:** Medium

## Summary

The production Docker image includes the `loadgen` load-testing binary, which is designed to generate high volumes of transactions for stress testing. An attacker who gains access to a running container (e.g., via a container escape vulnerability, compromised management interface, or `docker exec`) can use this binary to flood the network with transactions from within the trusted network perimeter, bypassing any external rate limiting or firewall rules. The binary should only be included in development/testing images, not in production builds.

## Problem

Kora's Dockerfile (`docker/Dockerfile`) uses a multi-stage build with cargo-chef for dependency caching. In the builder stage (line 41), it compiles three packages: `kora` (the validator node), `keygen` (the DKG key generation tool), and `loadgen` (the load testing tool). In the runtime stage (line 65), all three binaries are copied into the final image:

```dockerfile
# Builder stage
RUN cargo build --release -p kora -p keygen -p loadgen

# Runtime stage
COPY --from=builder /app/target/release/kora /usr/local/bin/
COPY --from=builder /app/target/release/keygen /usr/local/bin/
COPY --from=builder /app/target/release/loadgen /usr/local/bin/
```

The `loadgen` binary (`bin/loadgen/src/main.rs`) is a transaction generator that:
- Generates funded accounts with known private keys.
- Sends transactions at a configurable rate with cross-account parallelism.
- Is designed specifically to stress-test the network at maximum throughput.

This binary is available at `/usr/local/bin/loadgen` in every running container, including production validator containers. The container runs as the non-root `kora` user (line 72), but the binary is world-executable.

**File:** `docker/Dockerfile`, lines 41 and 65

## Code Reference

```dockerfile
# docker/Dockerfile:37-41
# Copy source and build application
COPY . .

# Build all binaries
RUN cargo build --release -p kora -p keygen -p loadgen

# docker/Dockerfile:62-65
# Copy binaries from builder
COPY --from=builder /app/target/release/kora /usr/local/bin/
COPY --from=builder /app/target/release/keygen /usr/local/bin/
COPY --from=builder /app/target/release/loadgen /usr/local/bin/
```

The docker-bake.hcl defines separate build targets but all use the same Dockerfile:

```hcl
# docker/docker-bake.hcl:59-67
target "kora-local" {
  context    = ".."
  dockerfile = "docker/Dockerfile"
  platforms  = ["linux/amd64"]
  tags       = ["kora:local"]
  args = {
    BUILD_PROFILE = "release"
  }
}
```

## Impact

1. **Network flooding from inside the trust boundary.** An attacker with container access can run `/usr/local/bin/loadgen` to generate high-volume transaction traffic directly from within the Docker network. This traffic originates from inside the `kora-net` bridge, bypassing any external firewall rules or rate limiting that protect the RPC endpoints.
2. **Mempool poisoning.** The `loadgen` binary generates transactions with funded pre-genesis accounts. If the genesis allocation matches the production devnet (which it does, since the same image is used), the attacker has access to funded accounts and can craft transactions that poison the mempool (see issue 041 for the unbounded mempool size, and the MempoolPoisoning alert rule in `docker/config/alerts.yml` for the known risk).
3. **Increased image size.** While a minor concern compared to the security impact, including `loadgen` adds unnecessary binary size to the production image and increases build time.
4. **Principle of least privilege violation.** Production containers should contain only the binaries necessary for their function. A validator needs `kora`; an init container needs `keygen`. Neither needs `loadgen`.

## Root Cause

The Dockerfile was written as a single-purpose build that produces one image containing all project binaries. No distinction was made between production binaries (`kora`, `keygen`) and testing binaries (`loadgen`). The `docker-bake.hcl` defines separate targets (`kora`, `kora-local`, `kora-dev`) but they all use the same Dockerfile without parameterization.

## Suggested Fix

**Option A: Use a build argument to control which packages are built and copied:**

```dockerfile
# docker/Dockerfile
ARG PACKAGES="kora keygen"

# Builder stage
RUN cargo build --release $(echo $PACKAGES | sed 's/[^ ]* */-p &/g')

# Runtime stage
COPY --from=builder /app/target/release/kora /usr/local/bin/
COPY --from=builder /app/target/release/keygen /usr/local/bin/
# Only copy loadgen if it was built
RUN if [ -f /app/target/release/loadgen ]; then \
      cp /app/target/release/loadgen /usr/local/bin/; \
    fi
```

Then in `docker-bake.hcl`:

```hcl
target "kora-local" {
  # Production image: no loadgen
  args = { PACKAGES = "kora keygen" }
}

target "kora-loadgen" {
  # Testing image: includes loadgen
  args = { PACKAGES = "kora keygen loadgen" }
}
```

**Option B: Simply remove `loadgen` from the Dockerfile** (simpler, if loadgen is rarely needed in Docker):

```dockerfile
# BEFORE (docker/Dockerfile:41)
RUN cargo build --release -p kora -p keygen -p loadgen

# AFTER
RUN cargo build --release -p kora -p keygen
```

```dockerfile
# BEFORE (docker/Dockerfile:65)
COPY --from=builder /app/target/release/loadgen /usr/local/bin/

# AFTER (remove this line entirely)
```

When loadgen is needed for testing, build it separately with `docker run --rm -v ... cargo build --release -p loadgen`.

## Files to Modify

- `docker/Dockerfile` -- remove `-p loadgen` from the build command (line 41) and remove the `COPY` for loadgen (line 65)
- `docker/docker-bake.hcl` -- optionally add a separate `kora-loadgen` target for testing images

## Related Issues

- [041 - In-memory mempool has no size limits](/Users/will/dev/nunchi/daeji/tmp/kora/issues/041-txpool-in-memory-mempool-no-limits.md) -- loadgen can exploit the unbounded mempool to cause memory exhaustion
- [025 - Transaction gossip has no rate limit](/Users/will/dev/nunchi/daeji/tmp/kora/issues/025-tx-gossip-no-rate-limit.md) -- loadgen-generated transactions are gossiped without rate limiting to all peers
- [177 - Loadgen uses guessable private keys](/Users/will/dev/nunchi/daeji/tmp/kora/issues/177-loadgen-guessable-private-keys.md) -- the funded accounts used by loadgen have deterministic keys

## Labels

`security`, `docker`, `enhancement`
