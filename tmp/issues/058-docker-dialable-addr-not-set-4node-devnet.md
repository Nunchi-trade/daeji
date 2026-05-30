# dialable_addr Not Set in Docker Devnet -- Nodes Advertise 0.0.0.0:30303

**Category:** docker, p2p
**Severity:** high

## Summary

The Docker devnet (both the 4-node `docker/compose/devnet.yaml` and the entrypoint script) does not configure the `dialable_addr` network setting. Without it, each node advertises `0.0.0.0:30303` as its reachable address to peers. This is not a routable address, and on certain Docker networking configurations (overlay networks, Swarm, cross-host deployments), it causes a partial P2P mesh, leading to high consensus nullification rates. This issue was previously observed and fixed on the 10-node remote devnet, but the fix was not propagated back to the repository's Docker configuration.

## Problem

Kora validators form a P2P mesh using commonware's transport layer. Each node needs to advertise a routable address so that other nodes can connect to it. This is the `dialable_addr` field in `NetworkConfig`.

The `NetworkConfig` struct (in `crates/node/config/src/network.rs`, line 18) defines:

```rust
pub dialable_addr: Option<String>,
```

When this is `None` (the default), the transport layer falls back to the `listen_addr`, which defaults to `0.0.0.0:30303`. This address means "listen on all interfaces" but is not meaningful as a destination address -- other containers cannot connect to `0.0.0.0:30303` on a remote host.

The Docker entrypoint (`docker/scripts/entrypoint.sh`) does not set `dialable_addr` anywhere. There is no `config.toml` generation (see issue 055), no CLI flag for it, and no environment variable to control it:

```bash
exec /usr/local/bin/kora validator \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    $GOSSIP_FLAG \
    "$@"
```

The Docker compose file (`docker/compose/devnet.yaml`) sets hostnames for each validator (e.g., line 225: `hostname: node0`), but this hostname is only used by the Docker DNS resolver -- it is not automatically picked up by the Kora transport layer.

## Code Reference

File: `crates/node/config/src/network.rs`, lines 12-18 (the network config with optional dialable_addr):

```rust
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,

    /// External address for NAT traversal (if different from listen_addr).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialable_addr: Option<String>,
```

File: `crates/node/config/src/network.rs`, lines 34-35 (default is None):

```rust
            listen_addr: DEFAULT_LISTEN_ADDR.to_string(),
            dialable_addr: None,
```

File: `docker/scripts/entrypoint.sh`, lines 200-205 (no dialable_addr configuration):

```bash
exec /usr/local/bin/kora validator \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    $GOSSIP_FLAG \
    "$@"
```

File: `docker/compose/devnet.yaml`, lines 223-226 (hostname is set but not used for dialable_addr):

```yaml
  validator-node0:
    <<: *validator-common
    hostname: node0
    depends_on:
```

## Impact

1. **Partial P2P mesh**: On Docker's default bridge network, this may work by luck because Docker DNS resolves container names and the transport layer might infer the source address from incoming connections. However, on overlay networks, Docker Swarm, Kubernetes, or multi-host deployments, nodes advertising `0.0.0.0:30303` result in a partial mesh where some peer connections fail.

2. **High nullification rate**: This was directly observed on the 10-node remote devnet: without `dialable_addr`, the P2P mesh was partial (not all 9 peer connections established per node), leading to 65% consensus nullification -- effectively unusable. The fix (setting `dialable_addr` to `$(hostname):30303` in the entrypoint) brought nullification to 0%.

3. **"Works on my machine" problem**: The 4-node local devnet happens to work on Docker Desktop's default bridge network because the transport layer can often infer correct addresses from connection metadata. An operator who tests locally and then deploys to a production Docker environment may encounter complete consensus failure without understanding why.

4. **Bootstrap peer connectivity**: The `bootstrappers` in `peers.json` map public keys to addresses. Nodes that cannot resolve each other's advertised addresses will fail to bootstrap, potentially preventing the node from joining the network at all.

## Root Cause

The fix for the P2P dialable address issue was applied directly to the 10-node remote devnet's configuration (which lives outside the repository at `/opt/kora/docker/compose/devnet-10node.yaml` on the remote server), but was not propagated back to the repository's Docker entrypoint or compose file.

## Suggested Fix

**Option 1 (recommended)**: Auto-detect the container hostname in the entrypoint and set `dialable_addr` via a generated config file:

Add to `docker/scripts/entrypoint.sh`, in the `validator` case, before the `exec` command:

```bash
# Generate config.toml with dialable_addr set to the container hostname.
# Without this, nodes advertise 0.0.0.0:30303 which is not routable
# from other containers.
DIALABLE_ADDR="${KORA_DIALABLE_ADDR:-$(hostname):30303}"
CONFIG_FILE="${DATA_DIR}/config.toml"
cat > "$CONFIG_FILE" <<EOF
[network]
dialable_addr = "${DIALABLE_ADDR}"
EOF

exec /usr/local/bin/kora validator \
    --config "$CONFIG_FILE" \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    $GOSSIP_FLAG \
    "$@"
```

**Option 2**: Set the `KORA_DIALABLE_ADDR` environment variable explicitly in the compose file for each validator:

```yaml
validator-node0:
  environment:
    - KORA_DIALABLE_ADDR=node0:30303
```

**Option 3**: Modify the `NetworkConfig` transport builder (in the `kora_transport` crate) to automatically use the system hostname when `dialable_addr` is `None` and `listen_addr` starts with `0.0.0.0`. This would fix the issue at the application level rather than the Docker level.

## Files to Modify

- `docker/scripts/entrypoint.sh` -- generate config.toml with `dialable_addr` (primary fix)
- `docker/compose/devnet.yaml` -- optionally add `KORA_DIALABLE_ADDR` environment variables

## Related Issues

- `055-docker-no-config-toml-in-entrypoint.md` -- the entrypoint does not generate a config file at all, which is the root cause of why `dialable_addr` cannot be set
- `059-metrics-missing-finalized-height-view-nullification.md` -- metrics are needed to detect the nullification spike caused by partial mesh

## Labels

bug, p2p, docker, reliability
