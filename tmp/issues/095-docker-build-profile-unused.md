# BUILD_PROFILE Arg Declared in docker-bake.hcl but Unused in Dockerfile

**Category**: docker, ci
**Severity**: low

**Labels**: `bug`, `docker`, `ci`, `good first issue`

## Summary

The `docker-bake.hcl` file declares a `BUILD_PROFILE` variable and passes it to the Dockerfile for three build targets (`kora`, `kora-local`, `kora-dev`), but the Dockerfile never declares the corresponding `ARG` and unconditionally hardcodes `--release` in all `cargo` commands. This means the `kora-dev` target, which is intended to produce a fast debug build, silently produces an identical release build. Additionally, the base Docker image uses an unpinned nightly Rust toolchain tag, making builds non-reproducible.

## Problem

### BUILD_PROFILE passed but never received

The `docker-bake.hcl` defines the variable and passes it to three targets:

```hcl
# /Users/will/dev/nunchi/daeji/docker/docker-bake.hcl:17-19
variable "BUILD_PROFILE" {
  default = "release"
}
```

```hcl
# kora target (lines 50-52): passes BUILD_PROFILE from variable
args = {
  BUILD_PROFILE = BUILD_PROFILE
}

# kora-local target (lines 64-66): hardcodes "release"
args = {
  BUILD_PROFILE = "release"
}

# kora-dev target (lines 78-80): sets "dev" -- supposed to be debug build
args = {
  BUILD_PROFILE = "dev"
}
```

### Dockerfile ignores the arg

The Dockerfile never declares `ARG BUILD_PROFILE` and hardcodes `--release` everywhere:

```dockerfile
# /Users/will/dev/nunchi/daeji/docker/Dockerfile:35
RUN cargo chef cook --release --recipe-path recipe.json

# /Users/will/dev/nunchi/daeji/docker/Dockerfile:41
RUN cargo build --release -p kora -p keygen -p loadgen
```

Binary copy paths are also hardcoded to the `release` directory:

```dockerfile
# /Users/will/dev/nunchi/daeji/docker/Dockerfile:63-65
COPY --from=builder /app/target/release/kora /usr/local/bin/
COPY --from=builder /app/target/release/keygen /usr/local/bin/
COPY --from=builder /app/target/release/loadgen /usr/local/bin/
```

### Unpinned nightly toolchain

The base image uses an unpinned nightly tag:

```dockerfile
# /Users/will/dev/nunchi/daeji/docker/Dockerfile:9
FROM rustlang/rust:nightly-bookworm AS chef
```

This means any nightly Rust release can change the compiler version between builds without any code change.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/docker/docker-bake.hcl`, lines 17-19 (variable declaration), lines 50-52 (`kora` target), lines 64-66 (`kora-local` target), lines 78-80 (`kora-dev` target)
**File**: `/Users/will/dev/nunchi/daeji/docker/Dockerfile`, line 9 (unpinned nightly), line 35 (`cargo chef cook --release`), line 41 (`cargo build --release`), lines 63-65 (binary copy from `target/release/`)

## Impact

- **Misleading build targets**: Running `docker buildx bake kora-dev` produces a release build identical to `docker buildx bake kora`. Developers expecting a debug build with debug symbols get a fully optimized release build instead.
- **Wasted build time for development**: The `kora-dev` target's purpose is fast iteration builds. Without the `dev` profile, it compiles with full optimizations and LTO, taking approximately 45 minutes under QEMU instead of the approximately 15 minutes a debug build would take.
- **No debug builds available**: Debug symbols and `RUST_BACKTRACE`-friendly binaries cannot be built through the Docker pipeline, making production debugging harder.
- **Non-reproducible builds**: The unpinned nightly image can change between builds, potentially introducing breaking changes or subtle behavior differences.

## Root Cause

The `BUILD_PROFILE` variable was added to `docker-bake.hcl` as a planned feature, but the corresponding `ARG` declaration and variable substitution were never added to the Dockerfile. The Dockerfile was written first with hardcoded `--release` and the bake file was added later without updating the Dockerfile to use the variable.

## Suggested Fix

### 1. Add `ARG BUILD_PROFILE` to the Dockerfile and use it

```dockerfile
# Before:
RUN cargo chef cook --release --recipe-path recipe.json
RUN cargo build --release -p kora -p keygen -p loadgen

# After:
ARG BUILD_PROFILE=release
RUN cargo chef cook --profile ${BUILD_PROFILE} --recipe-path recipe.json
RUN cargo build --profile ${BUILD_PROFILE} -p kora -p keygen -p loadgen
```

**Important caveat**: Cargo uses the directory name `debug` for the `dev` profile, but `release` for the `release` profile. The `COPY` step needs to handle this mapping:

```dockerfile
# Determine output directory based on profile
ARG BUILD_PROFILE=release
# Cargo outputs to target/debug for --profile dev, target/release for --profile release
RUN if [ "${BUILD_PROFILE}" = "dev" ]; then \
      ln -s /app/target/debug /app/target/profile-out; \
    else \
      ln -s /app/target/${BUILD_PROFILE} /app/target/profile-out; \
    fi

COPY --from=builder /app/target/profile-out/kora /usr/local/bin/
COPY --from=builder /app/target/profile-out/keygen /usr/local/bin/
COPY --from=builder /app/target/profile-out/loadgen /usr/local/bin/
```

### 2. Pin the nightly toolchain

```dockerfile
# Before:
FROM rustlang/rust:nightly-bookworm AS chef

# After:
FROM rustlang/rust:nightly-2026-05-15-bookworm AS chef
```

### 3. Add CI validation

Add a CI job that builds with the `kora-dev` target to verify the dev profile works (see issue 096).

## Files to Modify

- `/Users/will/dev/nunchi/daeji/docker/Dockerfile` -- Add `ARG BUILD_PROFILE`, use it in cargo commands (lines 35, 41), handle output directory mapping (lines 63-65), pin nightly tag (line 9)
- `/Users/will/dev/nunchi/daeji/docker/docker-bake.hcl` -- No changes needed (already correctly configured)

## Related Issues

- `096-ci-docker-build-smoke-test.md` -- CI should validate that both release and dev profile builds work
