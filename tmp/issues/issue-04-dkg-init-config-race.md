# init-config is not idempotent: regenerates DKG keys on every restart, causing permanent consensus deadlock

**Status: Verified** -- All three bugs confirmed by source audit (2026-05-22).

## Summary

The `init-config` Docker Compose service unconditionally regenerates all validator identity keys and BLS threshold shares on every invocation. When combined with the lack of ordering dependencies between `init-config` and the validator services, certain Docker Compose lifecycle operations (`docker compose stop` then `docker compose start`) cause a permanent consensus deadlock. Validators end up holding mismatched DKG key material and can never agree on block proposals again. The only recovery path is a full `docker compose down` / `docker compose up` cycle, which resets the entire chain.

## Problem Description

### The failing operation

Running the following sequence on the Kora devnet causes a permanent, unrecoverable consensus failure:

```bash
cd /opt/kora/docker/compose
docker compose -f devnet.yaml stop
docker compose -f devnet.yaml start
```

After this sequence, the chain produces at most a handful of blocks (observed: 4) and then enters an infinite loop of `invalid nullification` and `invalid finalization` errors. Block production never resumes. The only fix is to tear down and recreate all containers.

### What "works" (but still resets the chain)

```bash
# Option A: restart (stops and starts containers atomically)
docker compose -f devnet.yaml restart

# Option B: down/up (destroys and recreates containers)
docker compose -f devnet.yaml down
docker compose -f devnet.yaml up -d
```

Both options A and B happen to succeed because the init-config service runs to completion before validators finish their startup sequence. However, both still regenerate fresh DKG keys every time, meaning the chain always resets to block 1 with a new group key. Prior chain history is permanently lost.

### Observed symptoms (from `docker compose stop` / `start`)

```
# Node0 rejects messages signed with unknown keys:
invalid nullification peer=4397a442... view=38
invalid finalization peer=b074cff6... view=39

# Node1 enters an infinite nullification loop:
broadcasting nullification floor floor=Finalization(... view: View(14) ...)
```

Nodes never recover from this state. Each validator holds a different `group_public_key` and/or `share.key`, so no threshold quorum can ever be formed.

## Root Cause Analysis

There are three independent bugs that combine to produce this failure, plus two additional issues identified during the audit. All must be fixed.

### Bug 1: `init-config` is not idempotent

The `init-config` service in `docker/compose/devnet.yaml` (lines 83-110) runs two commands unconditionally:

```yaml
# docker/compose/devnet.yaml, lines 83-110
init-config:
  <<: *node-common
  user: root
  entrypoint: ["/bin/bash", "-c"]
  command:
    - |
      echo "[init] Running keygen setup..." && \
      /usr/local/bin/keygen setup \
        --validators=4 \
        --secondary-peers=1 \
        --threshold=3 \
        --chain-id=${CHAIN_ID:-1337} \
        --output-dir=/shared && \
      echo "[init] Running trusted dealer DKG..." && \
      /usr/local/bin/keygen dkg-deal \
        --validators=4 \
        --threshold=3 \
        --output-dir=/shared && \
      echo "[init] Setting permissions..." && \
      chown -R 1000:1000 /shared/node0 /shared/node1 /shared/node2 /shared/node3 /shared/secondary0 && \
      echo "[init] Init complete"
  volumes:
    - shared_config:/shared
    - data_node0:/shared/node0
    - data_node1:/shared/node1
    - data_node2:/shared/node2
    - data_node3:/shared/node3
    - data_secondary0:/shared/secondary0
```

Neither `keygen setup` nor `keygen dkg-deal` check whether keys already exist before generating new ones.

**`keygen setup`** (`bin/keygen/src/setup.rs`) has a *partial* idempotency guard for identity keys only:

```rust
// bin/keygen/src/setup.rs, lines 105-118
let key_path = node_dir.join("validator.key");
let key = if key_path.exists() {
    tracing::info!(node = i, "Loading existing identity key");
    let bytes = fs::read(&key_path)?;
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    ed25519::PrivateKey::from(ed25519_consensus::SigningKey::from(seed))
} else {
    tracing::info!(node = i, "Generating new identity key");
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    fs::write(&key_path, seed)?;
    ed25519::PrivateKey::from(ed25519_consensus::SigningKey::from(seed))
};
```

So `keygen setup` preserves existing `validator.key` files. However, it unconditionally regenerates `peers.json` (line 176), `setup.json` (line 131 for each node), and `genesis.json` -- including a fresh `timestamp` from `SystemTime::now()`:

```rust
// bin/keygen/src/setup.rs, lines 190-199
let genesis = GenesisConfig {
    chain_id: args.chain_id,
    timestamp: std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs(),
    allocations,
};
let genesis_path = args.output_dir.join("genesis.json");
fs::write(&genesis_path, serde_json::to_string_pretty(&genesis)?)?;
```

This means every restart produces a `genesis.json` with a different timestamp, which changes the genesis block identity even when the identity keys are preserved.

**`keygen dkg-deal`** (`bin/keygen/src/dkg_deal.rs`) has **no idempotency guard at all**. It always generates a fresh random polynomial with `OsRng` (line 88), producing new `output.json` and `share.key` files every time:

```rust
// bin/keygen/src/dkg_deal.rs, lines 88-93
let mut rng = rand::rngs::OsRng;

tracing::info!("Generating BLS threshold key shares");
let (public_output, shares) =
    dkg::deal::<MinSig, _, N3f1>(&mut rng, Mode::default(), participants_set)
        .map_err(|e| eyre::eyre!("DKG deal failed: {:?}", e))?;
```

Every call to `dkg::deal()` produces a completely different `group_public_key` and set of secret shares.

**Existing DKG idempotency guard (interactive path only):** The entrypoint script at `docker/scripts/entrypoint.sh` (lines 62-65) correctly guards the *interactive* DKG path:

```bash
# docker/scripts/entrypoint.sh, lines 62-65
if [[ -f "${DATA_DIR}/share.key" && -f "${DATA_DIR}/output.json" ]]; then
    log "DKG already completed (share.key exists)"
    exit 0
fi
```

This guard protects the `dkg` entrypoint mode (interactive ceremony). It is **not reached** by the `init-config` service, which runs `keygen setup` and `keygen dkg-deal` directly via its inline bash command (lines 87-103 of `devnet.yaml`), bypassing the entrypoint entirely. The guard is sufficient for the interactive DKG path but does nothing for the trusted dealer path.

**Note on the Ansible role:** The Ansible role at `ansible/roles/devnet/tasks/main.yml` (lines 10-18) **does** check for existing DKG shares before running init-config:

```yaml
# ansible/roles/devnet/tasks/main.yml, lines 10-18
- name: Check if DKG shares exist
  ansible.builtin.shell: |
    for i in 0 1 2 3; do
      volume="{{ compose_project_name }}_data_node${i}"
      docker volume inspect "$volume" >/dev/null 2>&1 || exit 1
      docker run --rm -v "${volume}:/data" alpine \
        test -f /data/share.key -a -f /data/output.json || exit 1
    done
  register: dkg_check
  ...

- name: Run init-config (trusted dealer DKG)
  ...
  when: dkg_check.rc != 0 and dkg_mode == "trusted"
```

This is a workaround at the orchestration layer. The underlying tools (`keygen setup`, `keygen dkg-deal`) remain non-idempotent.

### Bug 2: `depends_on` is missing AND not enforced by `docker compose start`

The validator services in `docker/compose/devnet.yaml` (lines 199-288) have **no `depends_on` clause** referencing `init-config`:

```yaml
# docker/compose/devnet.yaml, lines 199-219
validator-node0:
  <<: *validator-common
  hostname: node0
  entrypoint: ["/scripts/entrypoint.sh", "validator"]
  volumes:
    - shared_config:/shared:ro
    - data_node0:/data
    - startup_barrier:/barrier
  # NOTE: no depends_on for init-config
```

**Critical nuance:** Even if `depends_on` were added, `docker compose start` does NOT enforce `depends_on` ordering. The `depends_on` directive is only respected by `docker compose up`. From the Docker Compose documentation:

> `docker compose start` starts existing containers for a service. It does **not** re-evaluate `depends_on` or service ordering -- it simply starts the containers in the order they were created.

This means the fix cannot rely solely on `depends_on`. The `init-config` service itself must be idempotent (Bug 1 fix), so that even when `docker compose start` runs it concurrently with validators, it either skips (keys exist) or generates fresh keys before validators read them.

When `docker compose start` is invoked, all services (including `init-config`) start simultaneously. There is a race between:
1. `init-config` writing new `share.key` and `output.json` files to the data volumes.
2. Validator containers reading those files from the same volumes via `entrypoint.sh`.

Some validators may start and read the **old** key files before `init-config` overwrites them. Others may start after `init-config` finishes and read the **new** key files. The result is that different validators hold different BLS key material.

### Bug 3: Barrier files persist across restarts

The startup barrier mechanism in `docker/scripts/entrypoint.sh` (lines 22-48) uses marker files in a shared Docker volume (`startup_barrier`, mounted at `/barrier`):

```bash
# docker/scripts/entrypoint.sh, lines 29-30
touch "${BARRIER_DIR}/node${VALIDATOR_INDEX}.ready"
log "Barrier: marked node${VALIDATOR_INDEX} ready (waiting for ${count} validators)"
```

These `.ready` files survive container restarts because they live in a persistent Docker volume. On the next `docker compose start`, every validator immediately sees 4 stale `.ready` files and proceeds without waiting, defeating the barrier's purpose.

The `devnet-run.sh` script (line 319) correctly clears the barrier before launching validators:

```bash
# docker/scripts/devnet-run.sh, lines 175-180
clear_startup_barrier() {
    local volume="kora-devnet_startup_barrier"
    docker volume inspect "$volume" >/dev/null 2>&1 || return 0
    docker run --rm -v "${volume}:/barrier" alpine \
        sh -c 'rm -f /barrier/*.ready' >/dev/null 2>&1 || true
}
```

But the Ansible deployment role in `ansible/roles/devnet/tasks/main.yml` does **not** clear the barrier. It only clears `/data/runtime` (lines 74-85):

```yaml
# ansible/roles/devnet/tasks/main.yml, lines 74-85
- name: Clear runtime state from data volumes
  ansible.builtin.shell: |
    for volume in \
      {{ compose_project_name }}_data_node0 \
      ...
      docker run --rm -v "${volume}:/data" alpine rm -rf /data/runtime 2>/dev/null || true
    done
  # NOTE: no equivalent task for startup_barrier volume
```

### Bug 4: `.ready` marker written before node is actually ready

The validator entrypoint in `docker/scripts/entrypoint.sh` writes the `.ready` marker at line 97, **before** the barrier wait (line 104) and before the bootstrap peer connectivity check (lines 106-117):

```bash
# docker/scripts/entrypoint.sh, lines 96-117
cp "${SHARED_DIR}/genesis.json" "${DATA_DIR}/" 2>/dev/null || true
touch "${DATA_DIR}/.ready"                          # <-- line 97: marker written

# Wait for all validators to be ready before starting consensus.
wait_for_barrier "$VALIDATOR_COUNT"                  # <-- line 104: barrier wait

if [[ "$IS_BOOTSTRAP" != "true" && -n "$BOOTSTRAP_PEERS" ]]; then
    BOOTSTRAP_HOST=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f1)
    BOOTSTRAP_PORT=$(echo "$BOOTSTRAP_PEERS" | cut -d: -f2)

    log "Waiting for bootstrap peer ${BOOTSTRAP_HOST}:${BOOTSTRAP_PORT}..."
    timeout=120
    while ! nc -z "$BOOTSTRAP_HOST" "$BOOTSTRAP_PORT" 2>/dev/null; do  # <-- line 112
        timeout=$((timeout - 1))
        [[ $timeout -le 0 ]] && error "Timeout waiting for bootstrap peer"
        sleep 1
    done
fi
```

The healthcheck script (`docker/scripts/healthcheck.sh`) in `ready` mode checks the RPC server, which starts after all this initialization. However, `secondary-node0` depends on `validator-node0` with `condition: service_healthy`:

```yaml
# docker/compose/devnet.yaml, lines 293-295
    depends_on:
      validator-node0:
        condition: service_healthy
```

The healthcheck for validators uses `HEALTHCHECK_MODE=ready` which does an RPC call, so the health check itself is accurate. The `.ready` marker at line 97 is not used by the Docker healthcheck. It appears to be an artifact or used by other tooling. The real ordering issue is that the barrier is checked *after* the marker is written, meaning the barrier's `.ready` files (in `/barrier/`) are conflated with the node's `.ready` file (in `/data/`). These are different files in different paths, but the naming is confusing and the `/data/.ready` marker serves no current purpose in the healthcheck flow.

### Bug 5: `genesis.json` unconditionally overwritten with new timestamp

As detailed in Bug 1, `keygen setup` always regenerates `genesis.json` with `SystemTime::now()` at `bin/keygen/src/setup.rs` line 192. This means:

1. Even when identity keys (`validator.key`) are preserved via the existing idempotency guard (lines 105-118), the genesis block changes identity on every restart because the timestamp is different.
2. The entrypoint copies genesis.json from shared config to data dir at `docker/scripts/entrypoint.sh` line 96:
   ```bash
   cp "${SHARED_DIR}/genesis.json" "${DATA_DIR}/" 2>/dev/null || true
   ```
3. The validator loads this genesis on startup at `bin/kora/src/cli.rs` lines 160-162:
   ```rust
   let genesis_path = config.data_dir.join("genesis.json");
   let bootstrap = BootstrapConfig::load(&genesis_path)
       .map_err(|e| eyre::eyre!("Failed to load genesis: {}", e))?;
   ```
4. A different genesis timestamp produces a different genesis block hash, which causes the QMDB state to be re-initialized (see `runner.rs` lines 533-542 where `!has_finalized_history` gates genesis initialization).

### The combined failure mode

Here is the exact sequence of events during `docker compose stop` followed by `docker compose start`:

1. `docker compose stop` sends SIGTERM to all containers. Kora now handles SIGTERM (via `stop_signal: SIGTERM` in the compose file), but the 30-second grace period may still result in SIGKILL if shutdown is slow.

2. All containers exit. Volumes persist: `data_node0..3` (containing old `validator.key`, `share.key`, `output.json`), `startup_barrier` (containing stale `node0.ready` through `node3.ready`).

3. `docker compose start` restarts all services simultaneously, including `init-config`. **`depends_on` is NOT enforced by `start`** -- this is the critical nuance.

4. `init-config` begins generating new keys. `keygen setup` preserves existing `validator.key` files but regenerates `genesis.json` with a new timestamp and overwrites `peers.json`. `keygen dkg-deal` generates an entirely new random polynomial, producing new `output.json` and `share.key` for all 4 nodes.

5. Meanwhile, validator containers start concurrently. The barrier check in `entrypoint.sh` immediately passes (stale `.ready` files). Each validator reads its key files from `/data`.

6. Race outcome: Some validators read old `share.key`/`output.json` (before `init-config` overwrites them). Others read the new files (after `init-config` finishes). Some may read partially-written files.

7. Validators enter consensus with mismatched BLS key material. A validator holding share from group key A rejects proposals signed with shares from group key B. No threshold quorum (3-of-4) can ever be formed because the group keys do not match.

8. The chain enters a permanent deadlock of `invalid nullification` and `invalid finalization` messages.

## Impact

**Severity: High** -- This is a data-loss and availability bug.

- **Permanent consensus deadlock**: `docker compose stop` / `start` produces an unrecoverable cluster. The only fix is `docker compose down -v` (destroying all volumes) followed by a fresh `up`.
- **Silent chain reset on every restart**: Even the "working" restart paths (`restart`, `down`/`up`) silently discard all chain history because `init-config` regenerates keys. Operators may not realize they have lost all prior blocks and state.
- **Ansible deployment gap**: The Ansible `devnet` role does not clear barrier files, so repeated deployments accumulate stale barrier state. If the Ansible role ever needs to stop and restart validators (rather than doing a full teardown), it will hit the same deadlock.
- **Host reboot causes same failure**: On a host reboot, Docker's `unless-stopped` restart policy starts all containers simultaneously, triggering the same race condition as `stop`/`start`.

## Proposed Solution

### 1. Make `init-config` idempotent

Add an idempotency check to the `init-config` service command. If all 4 validators already have matching DKG output, skip key generation entirely:

```yaml
init-config:
  entrypoint: ["/bin/bash", "-c"]
  command:
    - |
      # Check if DKG already completed with consistent keys
      ALL_EXIST=true
      for i in 0 1 2 3; do
        if [[ ! -f "/shared/node${i}/share.key" || ! -f "/shared/node${i}/output.json" ]]; then
          ALL_EXIST=false
          break
        fi
      done

      if [[ "$ALL_EXIST" == "true" ]]; then
        echo "[init] DKG keys already exist, skipping generation"
        exit 0
      fi

      echo "[init] Running keygen setup..." && \
      /usr/local/bin/keygen setup ... && \
      echo "[init] Running trusted dealer DKG..." && \
      /usr/local/bin/keygen dkg-deal ... && \
      ...
```

### 2. Add `depends_on` for init-config on validator services (with caveat)

Ensure validators cannot start until `init-config` has completed:

```yaml
validator-node0:
  depends_on:
    init-config:
      condition: service_completed_successfully
  ...
```

**Caveat:** `depends_on` is only enforced by `docker compose up`, NOT by `docker compose start`. This means `depends_on` alone is insufficient. Fix 1 (idempotency) is the primary defense. `depends_on` provides defense-in-depth for the `up` path only.

### 3. Clear barrier files in the init-config service

Have `init-config` clear stale barrier files as its first action, before key generation:

```yaml
init-config:
  volumes:
    - startup_barrier:/barrier
    ...
  command:
    - |
      # Always clear stale barrier files
      rm -f /barrier/*.ready

      # Then check idempotency and generate keys if needed
      ...
```

### 4. Clear barrier files in Ansible

Add a barrier cleanup task to `ansible/roles/devnet/tasks/main.yml` after the "Clear runtime state" task (after line 85):

```yaml
- name: Clear startup barrier
  ansible.builtin.shell: |
    volume="{{ compose_project_name }}_startup_barrier"
    docker volume inspect "$volume" >/dev/null 2>&1 || exit 0
    docker run --rm -v "${volume}:/barrier" alpine sh -c 'rm -f /barrier/*.ready'
  changed_when: true
```

### 5. Make `genesis.json` generation idempotent

In `bin/keygen/src/setup.rs`, add a check before writing `genesis.json`:

```rust
let genesis_path = args.output_dir.join("genesis.json");
if genesis_path.exists() {
    tracing::info!(path = ?genesis_path, "genesis.json already exists, skipping");
} else {
    let genesis = GenesisConfig {
        chain_id: args.chain_id,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        allocations,
    };
    fs::write(&genesis_path, serde_json::to_string_pretty(&genesis)?)?;
    tracing::info!(path = ?genesis_path, "Wrote genesis configuration");
}
```

## Implementation Details

### Files to modify

| File | Change |
|------|--------|
| `docker/compose/devnet.yaml` (lines 83-110) | Add idempotency guard to `init-config` command; add `depends_on: init-config` to all 4 validator services (lines 199-288); mount `startup_barrier` volume in `init-config`; add barrier cleanup to `init-config` command |
| `bin/keygen/src/dkg_deal.rs` (line 45, `run` fn) | Add idempotency check: skip generation when `output.json` and `share.key` already exist for all nodes |
| `bin/keygen/src/setup.rs` (lines 190-199) | Add idempotency check: skip `genesis.json` generation if file already exists |
| `ansible/roles/devnet/tasks/main.yml` (after line 85) | Add barrier cleanup task after "Clear runtime state" |
| `docker/scripts/devnet-run.sh` | Already correct (calls `clear_startup_barrier` at line 319), no changes needed |

### Detailed changes

#### `docker/compose/devnet.yaml` -- init-config service

Replace the current `init-config` command (lines 83-110) with:

```yaml
init-config:
  <<: *node-common
  user: root
  entrypoint: ["/bin/bash", "-c"]
  command:
    - |
      # Clear stale barrier files from previous runs
      rm -f /barrier/*.ready 2>/dev/null || true

      # Check if DKG already completed for all validators
      DKG_EXISTS=true
      CHECKSUM=""
      for i in 0 1 2 3; do
        if [[ ! -f "/shared/node${i}/share.key" ]] || [[ ! -f "/shared/node${i}/output.json" ]]; then
          DKG_EXISTS=false
          break
        fi
        THIS_SUM=$(sha256sum "/shared/node${i}/output.json" | awk '{print $1}')
        if [[ -z "$CHECKSUM" ]]; then
          CHECKSUM="$THIS_SUM"
        elif [[ "$THIS_SUM" != "$CHECKSUM" ]]; then
          echo "[init] WARNING: output.json mismatch between nodes, regenerating"
          DKG_EXISTS=false
          break
        fi
      done

      if [[ "$DKG_EXISTS" == "true" ]]; then
        echo "[init] DKG keys already exist and are consistent, skipping"
        exit 0
      fi

      echo "[init] Running keygen setup..." && \
      /usr/local/bin/keygen setup \
        --validators=4 \
        --secondary-peers=1 \
        --threshold=3 \
        --chain-id=${CHAIN_ID:-1337} \
        --output-dir=/shared && \
      echo "[init] Running trusted dealer DKG..." && \
      /usr/local/bin/keygen dkg-deal \
        --validators=4 \
        --threshold=3 \
        --output-dir=/shared && \
      echo "[init] Setting permissions..." && \
      chown -R 1000:1000 /shared/node0 /shared/node1 /shared/node2 /shared/node3 /shared/secondary0 && \
      echo "[init] Init complete"
  volumes:
    - shared_config:/shared
    - data_node0:/shared/node0
    - data_node1:/shared/node1
    - data_node2:/shared/node2
    - data_node3:/shared/node3
    - data_secondary0:/shared/secondary0
    - startup_barrier:/barrier
```

#### `docker/compose/devnet.yaml` -- validator services

Add `depends_on` to each validator (example for node0, repeat for nodes 1-3):

```yaml
validator-node0:
  <<: *validator-common
  hostname: node0
  depends_on:
    init-config:
      condition: service_completed_successfully
  entrypoint: ["/scripts/entrypoint.sh", "validator"]
  ...
```

#### `bin/keygen/src/dkg_deal.rs` -- idempotency in Rust code

Add a check at the top of the `run` function (after line 45, before loading participants):

```rust
pub(crate) fn run(args: DkgDealArgs) -> Result<()> {
    // Check if DKG output already exists for all validators
    let all_exist = (0..args.validators).all(|i| {
        let node_dir = args.output_dir.join(format!("node{}", i));
        node_dir.join("share.key").exists() && node_dir.join("output.json").exists()
    });

    if all_exist {
        tracing::info!("DKG output already exists for all {} validators, skipping", args.validators);
        return Ok(());
    }

    // ... existing generation code ...
}
```

#### `bin/keygen/src/setup.rs` -- genesis.json idempotency

Replace lines 190-199 with a conditional write:

```rust
let genesis_path = args.output_dir.join("genesis.json");
if genesis_path.exists() {
    tracing::info!(path = ?genesis_path, "genesis.json already exists, preserving");
} else {
    let genesis = GenesisConfig {
        chain_id: args.chain_id,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        allocations,
    };
    fs::write(&genesis_path, serde_json::to_string_pretty(&genesis)?)?;
    tracing::info!(path = ?genesis_path, "Wrote genesis configuration");
}
```

#### `ansible/roles/devnet/tasks/main.yml` -- barrier cleanup

Add after the "Clear runtime state from data volumes" task (after line 85):

```yaml
- name: Clear startup barrier
  ansible.builtin.shell: |
    volume="{{ compose_project_name }}_startup_barrier"
    docker volume inspect "$volume" >/dev/null 2>&1 || exit 0
    docker run --rm -v "${volume}:/barrier" alpine sh -c 'rm -f /barrier/*.ready'
  changed_when: true
```

### Lifecycle matrix after fix

| Operation | init-config runs? | Keys regenerated? | Barrier cleared? | Result |
|-----------|-------------------|-------------------|------------------|--------|
| First `docker compose up -d` | Yes | Yes (no existing keys) | Yes (init-config clears) | Chain starts from block 1 |
| `docker compose stop` / `start` | Yes (skips, keys exist) | No | Yes (init-config clears) | Chain resets (tmpfs), same keys |
| `docker compose restart` | Yes (skips, keys exist) | No | Yes (init-config clears) | Chain resets (tmpfs), same keys |
| `docker compose down` / `up -d` | Yes (skips, keys exist) | No | Yes (init-config clears) | Chain resets (tmpfs), same keys |
| `docker compose down -v` / `up -d` | Yes | Yes (volumes deleted) | Yes | Fresh chain, new keys |
| Ansible `deploy.yml` | Conditional (line 28) | Only if missing | Yes (new task) | Chain resets, same keys |
| Host reboot | Yes (skips, keys exist) | No | Yes (init-config clears) | Chain resets (tmpfs), same keys |

## Testing Plan

1. **Reproduce the deadlock** (before fix):
   - Deploy a fresh devnet: `docker compose -f devnet.yaml up -d init-config && docker compose -f devnet.yaml up -d`
   - Wait for block production (verify via `curl localhost:8545 -X POST -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'`)
   - Run `docker compose -f devnet.yaml stop` then `docker compose -f devnet.yaml start`
   - Confirm the chain gets permanently stuck (blocks stop advancing within 10 seconds)

2. **Verify idempotency** (after fix):
   - Deploy fresh devnet, wait for blocks
   - Record the `group_public_key` from any node's `/data/output.json`
   - Run `docker compose stop` / `start`
   - Verify: same `group_public_key` in all nodes' `/data/output.json`
   - Verify: block production resumes (chain resets to block 1 due to tmpfs, but consensus is healthy)

3. **Verify barrier cleanup** (after fix):
   - Deploy fresh devnet, wait for blocks
   - Run `docker compose stop`
   - Inspect barrier volume: `docker run --rm -v kora-devnet_startup_barrier:/barrier alpine ls /barrier/`
   - Confirm stale `.ready` files exist
   - Run `docker compose start`
   - Confirm init-config cleared the barrier files before validators started
   - Confirm all validators reached the barrier and proceeded together

4. **Verify genesis.json preservation** (after fix):
   - Deploy fresh devnet, record genesis timestamp from `/shared/genesis.json`
   - Run `docker compose stop` / `start`
   - Verify: genesis timestamp is unchanged
   - Verify: validators load the same genesis block identity

5. **Verify host reboot scenario** (after fix):
   - Deploy fresh devnet, record group key
   - Reboot the host
   - Wait for Docker to auto-start containers
   - Verify same group key, block production resumes

6. **Verify fresh deploy** (after fix):
   - `docker compose down -v` (delete all volumes)
   - `docker compose up -d`
   - Verify new keys are generated, block production starts

7. **Verify Ansible deploy** (after fix):
   - Run `ansible-playbook playbooks/deploy.yml -i inventory/hosts.yml` from the `ansible/` directory
   - Verify barrier is cleared
   - Verify keys are preserved if they already exist

## References

| File | Path | Relevance |
|------|------|-----------|
| Compose devnet | `docker/compose/devnet.yaml` | `init-config` (lines 83-110), validator services (lines 199-288), `secondary-node0` depends_on (lines 293-295) |
| Trusted dealer DKG | `bin/keygen/src/dkg_deal.rs` | No idempotency guard, `OsRng` at line 88, `dkg::deal()` at line 91 |
| Setup command | `bin/keygen/src/setup.rs` | Identity key guard (lines 105-118), genesis.json overwrite (lines 190-199) |
| Entrypoint | `docker/scripts/entrypoint.sh` | DKG idempotency guard for interactive path (lines 62-65), `.ready` marker (line 97), barrier wait (line 104), bootstrap wait (lines 106-117) |
| Healthcheck | `docker/scripts/healthcheck.sh` | `ready` mode checks RPC (lines 13-18), not `.ready` file |
| Devnet runner | `docker/scripts/devnet-run.sh` | `clear_startup_barrier()` (lines 175-180), called at line 319 |
| Ansible role | `ansible/roles/devnet/tasks/main.yml` | DKG existence check (lines 10-18), runtime cleanup (lines 74-85), missing barrier cleanup |
| DKG output | `crates/node/dkg/src/output.rs` | `DkgOutput::exists()` (lines 98-100), `DkgOutput::load()` (lines 66-95) |
| CLI validator | `bin/kora/src/cli.rs` | DKG output check (line 135), genesis load (lines 160-162), threshold scheme load (line 144) |
| Runner | `crates/node/runner/src/runner.rs` | `ProductionRunner::run()` (line 492), genesis init gated by `has_finalized_history` (lines 533-542) |
| Threshold scheme | `crates/node/runner/src/scheme.rs` | `load_threshold_scheme()` referenced in `cli.rs` line 144 |
