# CI Pipeline Missing Docker Image Build and Compose Smoke Test

**Category**: ci, docker
**Severity**: medium

**Labels**: `enhancement`, `ci`, `docker`, `reliability`

## Summary

The CI pipeline (`.github/workflows/ci.yml`) runs 7 Rust-level jobs (build, test, e2e, doctest, fmt, clippy, deny) but does not build the Docker image, test the Docker Compose deployment, or validate any Docker-specific code paths. This means the entire container deployment pipeline -- Dockerfile, entrypoint.sh (237 lines), healthcheck.sh (162 lines), devnet-run.sh (417 lines), compose configuration, and Docker DNS networking -- is completely untested in CI. Breakage is only discovered during manual builds on the remote server, which take approximately 45 minutes under QEMU.

## Problem

### No Docker image build in CI

The complete CI configuration is 88 lines long with 7 jobs:

```yaml
# /Users/will/dev/nunchi/daeji/.github/workflows/ci.yml:16-88
jobs:
  build:     # cargo build --workspace
  test:      # cargo nextest run --workspace
  e2e:       # cargo nextest run -p kora-e2e
  doctest:   # cargo test --doc
  fmt:       # cargo fmt --check
  clippy:    # cargo clippy
  deny:      # cargo deny
```

None of these jobs:
- Build the Docker image (`docker buildx bake`)
- Validate the Dockerfile syntax or multi-stage build
- Test the `cargo-chef` dependency caching strategy
- Push images to the `ghcr.io/refcell/kora` registry declared in `docker-bake.hcl` (line 6)

### No automated devnet smoke test

The E2E tests run against an in-process test harness (see `crates/e2e/src/harness.rs`), not the Docker Compose deployment. Docker-specific code paths are never tested automatically:

- **entrypoint.sh** (237 lines): Config generation, startup barrier mechanism, bootstrap peer waiting, restart detection, environment variable handling
- **healthcheck.sh** (162 lines): Multi-mode health checks (dkg/p2p/ready), stall detection, consensus participation verification
- **devnet-run.sh** (417 lines): Full deployment orchestration, DKG ceremony coordination, volume management
- Container networking, Docker DNS resolution, and the `/data` vs `/runtime` volume separation

### No shared Docker build cache

The `docker-bake.hcl` does not configure remote caching:

```hcl
# /Users/will/dev/nunchi/daeji/docker/docker-bake.hcl:45-53
target "kora" {
  inherits   = ["docker-metadata-action"]
  context    = ".."
  dockerfile = "docker/Dockerfile"
  platforms  = split(",", PLATFORMS)
  args = {
    BUILD_PROFILE = BUILD_PROFILE
  }
  # No cache-from or cache-to configured
}
```

The `cargo-chef` caching strategy works for local layer re-use, but CI and the remote devnet build from scratch independently.

### Registry never receives images

The registry is configured but never used:

```hcl
# /Users/will/dev/nunchi/daeji/docker/docker-bake.hcl:5-7
variable "REGISTRY" {
  default = "ghcr.io/refcell/kora"
}
```

No CI job pushes images to this registry.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/.github/workflows/ci.yml`, lines 1-88 (complete CI config, no Docker jobs)
**File**: `/Users/will/dev/nunchi/daeji/docker/docker-bake.hcl`, lines 5-7 (registry), lines 45-53 (kora target, no cache config)
**File**: `/Users/will/dev/nunchi/daeji/docker/Dockerfile`, lines 1-94 (multi-stage build, never built in CI)
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/entrypoint.sh`, 237 lines (never tested in CI)
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/healthcheck.sh`, 162 lines (never tested in CI)

## Impact

- **Silent breakage**: Changes to the Dockerfile, entrypoint.sh, healthcheck.sh, or compose files can break the Docker build without any CI signal. The breakage is only discovered after a 45-minute build on the remote server.
- **Docker-specific bugs undetectable**: The dialable address fix (critical P2P issue), startup barrier mechanism, and DKG ceremony flow are all Docker-specific code paths that are never tested automatically.
- **Slow feedback loop**: Without shared caching, each environment rebuilds from scratch. A Docker build failure could have been caught in CI minutes after a PR is opened, instead of hours later during manual deployment.
- **No container registry**: Pre-built images are not available for quick deployment or rollback. Every deployment requires a full from-source build.

## Root Cause

The CI pipeline was set up for Rust-level validation only. Docker support was added later as a deployment mechanism without corresponding CI integration. The E2E test harness uses an in-process simulated network, not Docker containers.

## Suggested Fix

### 1. Add a Docker image build job

```yaml
docker:
  name: Docker Build
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: docker/setup-buildx-action@v3
    - name: Build Docker image
      run: |
        cd docker
        docker buildx bake --allow=fs.read=.. -f docker-bake.hcl kora-local --load
    - name: Push to registry (main only)
      if: github.ref == 'refs/heads/main'
      run: |
        echo "${{ secrets.GITHUB_TOKEN }}" | docker login ghcr.io -u ${{ github.actor }} --password-stdin
        cd docker
        docker buildx bake --allow=fs.read=.. -f docker-bake.hcl kora --push
```

### 2. Add a compose smoke test job

Start the 4-node devnet, wait for health checks to pass, verify the RPC endpoint responds to `eth_blockNumber`, then tear down.

### 3. Add Docker layer caching with GitHub Actions cache

```hcl
target "kora" {
  cache-from = ["type=gha"]
  cache-to   = ["type=gha,mode=max"]
}
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/.github/workflows/ci.yml` -- Add Docker build job and compose smoke test job
- `/Users/will/dev/nunchi/daeji/docker/docker-bake.hcl` -- Add `cache-from` and `cache-to` for GitHub Actions cache (lines 45-53)

## Related Issues

- `095-docker-build-profile-unused.md` -- BUILD_PROFILE unused (CI should validate that profile selection works)
- `097-docker-observability-not-enabled.md` -- Observability stack (CI should also test the observability profile)
- `099-e2e-comprehensive-test-coverage.md` -- E2E tests (compose smoke test complements the in-process E2E harness)
