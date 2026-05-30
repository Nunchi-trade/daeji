# `devnet-run.sh` Phase 1.5 Re-Runs Full `init-config` When Only Shares Are Missing

**Category:** Bug -- Docker/Deployment
**Severity:** Medium

## Summary

When DKG shares are missing but peer configuration (keys, `peers.json`, genesis) already exists, the `devnet-run.sh` script's Phase 1.5 (trusted dealer path) re-runs the full `init-config` Docker Compose service, which regenerates BOTH the peer configuration AND the DKG shares from scratch. This makes it impossible to perform a shares-only regeneration (re-keying) while keeping existing validator identities, because the script treats configuration and shares as an indivisible unit. An operator attempting to re-key the DKG inadvertently overwrites all validator keys and genesis, breaking continuity with the existing chain state.

## Problem

The `devnet-run.sh` script (`docker/scripts/devnet-run.sh`) manages the devnet lifecycle in phases. Phase 1 (line 214) checks whether peer configuration exists; Phase 1.5 (line 296 for the trusted dealer path) checks whether DKG shares exist.

When `SHARES_EXIST` is false (shares are missing or were manually deleted for re-keying) but `CONFIG_EXISTS` is true (peer configuration from a previous run still exists), Phase 1.5 runs the full `init-config` service:

```bash
# docker/scripts/devnet-run.sh:295-308
else
    if [[ "$SHARES_EXIST" != "true" ]]; then
        echo ""
        print_phase "1.5/3" "Trusted Dealer DKG"

        if run_with_spinner "Generating threshold shares..." docker compose -f compose/devnet.yaml run --rm init-config; then
```

The `init-config` service (`docker/compose/devnet.yaml`, lines 102-134) is defined as a Docker Compose service that runs both `keygen setup` (which generates validator keys, peers.json, and genesis) AND `keygen dkg-deal` (which generates threshold shares):

```bash
# init-config service command (docker/compose/devnet.yaml:106-126)
if [ -f /shared/node0/share.key ] && [ -f /shared/node0/output.json ]; then
    echo "[init] DKG already completed, skipping"
    exit 0
fi
echo "[init] Running keygen setup..." && \
/usr/local/bin/keygen setup \
  --validators=4 \
  --secondary-peers=1 \
  --chain-id=${CHAIN_ID:-1337} \
  --output-dir=/shared && \
echo "[init] Running trusted dealer DKG..." && \
/usr/local/bin/keygen dkg-deal \
  --validators=4 \
  --output-dir=/shared && \
```

The `init-config` service has a guard that checks for `share.key`, but by the time Phase 1.5 executes, the shares have already been deleted by the `clear_dkg_outputs` function (called at line 208 when `check_dkg_outputs` fails):

```bash
# docker/scripts/devnet-run.sh:205-209
if check_dkg_outputs; then
    SHARES_EXIST=true
else
    clear_dkg_outputs    # <-- Deletes share.key, output.json, dkg_state.json
fi
```

So the guard inside `init-config` (`if [ -f /shared/node0/share.key ]`) never triggers, and the full setup + DKG runs unconditionally, overwriting the existing peer configuration.

**File:** `docker/scripts/devnet-run.sh`, lines 296-305
**File:** `docker/compose/devnet.yaml`, lines 102-134

## Code Reference

Phase 1.5 of the trusted dealer path in `devnet-run.sh`:

```bash
# docker/scripts/devnet-run.sh:295-308
else
    if [[ "$SHARES_EXIST" != "true" ]]; then
        echo ""
        print_phase "1.5/3" "Trusted Dealer DKG"

        if run_with_spinner "Generating threshold shares..." docker compose -f compose/devnet.yaml run --rm init-config; then
            print_success "Threshold shares generated"
        else
            print_error "Trusted dealer DKG failed"
            exit 1
        fi
    else
        print_skip "Threshold shares exist"
    fi
fi
```

The `clear_dkg_outputs` function that runs before Phase 1.5 when shares are invalid:

```bash
# docker/scripts/devnet-run.sh:153-160
clear_dkg_outputs() {
    for i in 0 1 2 3; do
        local volume="kora-devnet_data_node${i}"
        docker volume inspect "$volume" >/dev/null 2>&1 || continue
        docker run --rm -v "${volume}:/data" alpine \
            rm -f /data/share.key /data/output.json /data/dkg_state.json >/dev/null 2>&1 || true
    done
}
```

The `init-config` service that is called (runs BOTH setup AND dkg-deal):

```yaml
# docker/compose/devnet.yaml:102-126
  init-config:
    <<: *node-common
    user: root
    entrypoint: ["/bin/bash", "-c"]
    command:
      - |
        if [ -f /shared/node0/share.key ] && [ -f /shared/node0/output.json ]; then
            echo "[init] DKG already completed, skipping"
            exit 0
        fi
        echo "[init] Clearing startup barrier from previous runs..." && \
        rm -f /barrier/*.ready && \
        echo "[init] Running keygen setup..." && \
        /usr/local/bin/keygen setup \
          --validators=4 \
          --secondary-peers=1 \
          --chain-id=${CHAIN_ID:-1337} \
          --output-dir=/shared && \
        echo "[init] Running trusted dealer DKG..." && \
        /usr/local/bin/keygen dkg-deal \
          --validators=4 \
          --output-dir=/shared && \
        echo "[init] Setting permissions..." && \
        chown -R 1000:1000 /shared/node0 /shared/node1 /shared/node2 /shared/node3 /shared/secondary0 /barrier && \
        echo "[init] Init complete"
```

## Impact

1. **Re-keying is impossible without losing validator identities.** An operator who deletes shares to trigger DKG re-keying inadvertently regenerates all validator keys and the genesis block. The new keys do not match the existing chain state, making the restarted validators unable to participate in consensus on the existing chain.
2. **Existing chain state becomes orphaned.** When `keygen setup` runs, it generates new validator keys and a new genesis. The QMDB data from the previous run is now associated with keys that no longer exist. The chain must effectively be restarted from genesis.
3. **Misleading spinner message.** The spinner says "Generating threshold shares..." (line 300) but the operation actually regenerates the entire configuration, misleading the operator about the scope of changes.
4. **No way to recover.** Once `init-config` completes, the previous validator keys are overwritten in the shared volumes. There is no backup or confirmation step.

## Root Cause

The `devnet-run.sh` script treats shares and peer configuration as a single unit. The `init-config` service bundles `keygen setup` and `keygen dkg-deal` into one command with a single guard on `share.key`. When only shares need to be regenerated (re-keying scenario), there is no separate code path that runs only `keygen dkg-deal` while preserving the existing peer configuration.

## Suggested Fix

1. **Add a conditional path** in Phase 1.5 that distinguishes between "no config at all" and "config exists but shares are missing":

   ```bash
   # BEFORE (docker/scripts/devnet-run.sh:296-305)
   if [[ "$SHARES_EXIST" != "true" ]]; then
       echo ""
       print_phase "1.5/3" "Trusted Dealer DKG"
       if run_with_spinner "Generating threshold shares..." docker compose -f compose/devnet.yaml run --rm init-config; then

   # AFTER
   if [[ "$SHARES_EXIST" != "true" ]]; then
       echo ""
       print_phase "1.5/3" "Trusted Dealer DKG"
       if [[ "$CONFIG_EXISTS" == "true" ]]; then
           # Config exists but shares are missing: only regenerate shares
           if run_with_spinner "Regenerating threshold shares (keeping existing config)..." \
               docker compose -f compose/devnet.yaml run --rm --entrypoint "/bin/bash" init-config -c \
               '/usr/local/bin/keygen dkg-deal --validators=4 --output-dir=/shared && \
                chown -R 1000:1000 /shared/node0 /shared/node1 /shared/node2 /shared/node3'; then
   ```

2. **Or create a dedicated `dkg-deal-only` service** in the compose file:

   ```yaml
   # docker/compose/devnet.yaml (new service)
   dkg-deal-only:
     <<: *node-common
     user: root
     entrypoint: ["/bin/bash", "-c"]
     command:
       - |
         echo "[dkg] Running trusted dealer DKG (shares only)..." && \
         /usr/local/bin/keygen dkg-deal \
           --validators=4 \
           --output-dir=/shared && \
         chown -R 1000:1000 /shared/node0 /shared/node1 /shared/node2 /shared/node3 && \
         echo "[dkg] Shares generated"
     volumes:
       - shared_config:/shared
       - data_node0:/shared/node0
       - data_node1:/shared/node1
       - data_node2:/shared/node2
       - data_node3:/shared/node3
   ```

3. **Fix the guard in `init-config`** to also check whether `peers.json` exists (in addition to `share.key`) so it does not regenerate configuration that already exists:

   ```bash
   # BEFORE
   if [ -f /shared/node0/share.key ] && [ -f /shared/node0/output.json ]; then

   # AFTER
   if [ -f /shared/node0/share.key ] && [ -f /shared/node0/output.json ]; then
       echo "[init] DKG already completed, skipping"
       exit 0
   elif [ -f /shared/peers.json ] && [ -f /shared/node0/validator.key ]; then
       echo "[init] Config exists, only running DKG..."
       /usr/local/bin/keygen dkg-deal --validators=4 --output-dir=/shared
       # ... (chown, etc.)
       exit 0
   fi
   ```

## Files to Modify

- `docker/scripts/devnet-run.sh` -- add conditional logic in Phase 1.5 (lines 296-305) to distinguish between full setup and shares-only regeneration
- `docker/compose/devnet.yaml` -- optionally add a `dkg-deal-only` service, or modify the `init-config` command to detect existing configuration

## Related Issues

- [056 - devnet-run.sh always clears state before starting](/Users/will/dev/nunchi/daeji/tmp/kora/issues/056-docker-devnet-run-always-clears-state.md) -- related issue where devnet-run.sh clears runtime state (not DKG config) on every start
- [024 - Trusted dealer DKG does not clean up intermediate artifacts](/Users/will/dev/nunchi/daeji/tmp/kora/issues/024-trusted-dealer-no-cleanup.md) -- the trusted dealer DKG process leaves intermediate artifacts that are not cleaned up

## Labels

`bug`, `docker`, `dkg`, `config`, `reliability`
