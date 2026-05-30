# No Image Versioning or Tagging Strategy -- Deployments Not Traceable or Rollbackable

**Category:** Bug -- Docker/Deployment
**Severity:** Medium

## Summary

Every Docker build produces the tag `kora:local`, overwriting the previous image with no version information. There is no git commit SHA, no semantic version, no build timestamp, and no image registry. This means an operator cannot determine which version of the code is running on a server, cannot roll back to a previous image after a bad deployment (requiring a full ~45-minute rebuild from a known-good commit), and cannot compare binaries between deployments to identify what changed.

## Problem

Kora's Docker build pipeline consists of three components that all hardcode the `kora:local` tag:

1. **`docker-bake.hcl`** (line 63): The `kora-local` build target always produces the tag `kora:local`.
2. **Ansible group vars** (line 7): The `docker_image` variable is set to `kora:local`.
3. **Docker Compose** (line 25): All containers use `image: kora:local` via the `x-node-common` YAML anchor.

When a developer runs `docker buildx bake kora-local`, the new image overwrites the previous `kora:local` tag. Docker retains the previous image as an untagged (dangling) image, but there is no way to identify it or associate it with a specific git commit.

The `docker-bake.hcl` file does define a `GIT_REF_NAME` variable (line 13) and a `docker-metadata-action` target (line 38) that uses it for the production `kora` target, but:
- `GIT_REF_NAME` defaults to `"main"` (not a commit SHA).
- The `docker-metadata-action` target is only used by the production `kora` target, not by `kora-local`.
- No CI/CD pipeline exists to set `GIT_REF_NAME` to a meaningful value.

The build infrastructure also does not embed version information into the binary itself. Running `kora --version` does not report a git SHA or build timestamp.

**File:** `docker/docker-bake.hcl`, line 63
**File:** `ansible/inventory/group_vars/devnet.yml`, line 7
**File:** `docker/compose/devnet.yaml`, line 25

## Code Reference

The build target with hardcoded tag:

```hcl
# docker/docker-bake.hcl:59-67
target "kora-local" {
  context    = ".."
  dockerfile = "docker/Dockerfile"
  platforms  = ["linux/amd64"]
  tags       = ["kora:local"]        # <-- No version tag
  args = {
    BUILD_PROFILE = "release"
  }
}
```

The unused `GIT_REF_NAME` variable (only used by the production target, not `kora-local`):

```hcl
# docker/docker-bake.hcl:13-15
variable "GIT_REF_NAME" {
  default = "main"
}

# docker/docker-bake.hcl:37-39
target "docker-metadata-action" {
  tags = ["${REGISTRY}/kora:${GIT_REF_NAME}"]
}
```

The Ansible and Compose configurations referencing the unversioned tag:

```yaml
# ansible/inventory/group_vars/devnet.yml:7
docker_image: "kora:local"

# docker/compose/devnet.yaml:24-25 (x-node-common anchor)
x-node-common: &node-common
  image: kora:local
```

## Impact

1. **No deployment traceability.** When debugging a production issue on the devnet, an operator cannot determine which git commit is running. The image tag `kora:local` contains no version information. The operator must rely on `docker inspect` to check the image creation timestamp and correlate it with git history, which is error-prone.
2. **No rollback capability.** After a bad deployment, the previous `kora:local` image has been overwritten. The only rollback path is to check out a known-good git commit and rebuild the entire image, which takes approximately 45 minutes under QEMU emulation on the devnet server. During this time, the devnet runs the broken code.
3. **Non-reproducible builds.** Multiple operators building at different times from different branches all produce images tagged `kora:local`. It is impossible to determine which operator's build is running on the server.
4. **No image provenance.** Without a registry or content-addressable tagging, there is no chain of custody from source code to running binary. This makes security auditing and incident response significantly harder.

## Root Cause

The Docker build configuration was set up for local development use only. The `kora-local` target was designed for single-developer, single-machine builds where versioning is not a concern. No CI/CD pipeline or image registry was integrated to provide automated versioning, and the existing `GIT_REF_NAME` variable was not plumbed through to the local build target.

## Suggested Fix

1. **Tag images with the git commit SHA** in the `kora-local` target:

   ```hcl
   # BEFORE (docker/docker-bake.hcl:59-67)
   target "kora-local" {
     context    = ".."
     dockerfile = "docker/Dockerfile"
     platforms  = ["linux/amd64"]
     tags       = ["kora:local"]
     args = {
       BUILD_PROFILE = "release"
     }
   }

   # AFTER
   variable "GIT_SHA" {
     default = ""
   }

   target "kora-local" {
     context    = ".."
     dockerfile = "docker/Dockerfile"
     platforms  = ["linux/amd64"]
     tags       = compact(["kora:local", GIT_SHA != "" ? "kora:${GIT_SHA}" : ""])
     args = {
       BUILD_PROFILE = "release"
       GIT_SHA       = GIT_SHA
     }
   }
   ```

2. **Pass the git SHA at build time:**

   ```bash
   # In Justfile or build script
   cd docker && GIT_SHA=$(git rev-parse --short HEAD) docker buildx bake --allow=fs.read=.. --load kora-local
   ```

3. **Embed version info in the binary** via Cargo build script or environment variable:

   ```dockerfile
   # docker/Dockerfile (builder stage)
   ARG GIT_SHA=""
   ENV GIT_SHA=${GIT_SHA}
   RUN cargo build --release -p kora -p keygen
   ```

   Then in the Kora binary's CLI setup, read `GIT_SHA` from a compile-time environment variable to include in `--version` output.

4. **Retain the previous image** before building by re-tagging:

   ```bash
   # Before building, save the current image
   docker tag kora:local kora:previous 2>/dev/null || true
   ```

## Files to Modify

- `docker/docker-bake.hcl` -- add `GIT_SHA` variable, include commit SHA in tags for `kora-local` target
- `ansible/inventory/group_vars/devnet.yml` -- optionally change `docker_image` to support versioned tags
- `docker/compose/devnet.yaml` -- optionally parameterize the image tag via an environment variable

## Related Issues

- [093 - 10-node compose file not in version control](/Users/will/dev/nunchi/daeji/tmp/kora/issues/093-docker-10node-compose-not-in-vcs.md) -- another deployment reproducibility issue: the production compose file is not tracked
- [095 - Docker build profile variable is unused](/Users/will/dev/nunchi/daeji/tmp/kora/issues/095-docker-build-profile-unused.md) -- the `BUILD_PROFILE` variable in docker-bake.hcl is defined but not used by the Dockerfile, another gap in build configuration

## Labels

`bug`, `docker`, `ci`, `enhancement`
