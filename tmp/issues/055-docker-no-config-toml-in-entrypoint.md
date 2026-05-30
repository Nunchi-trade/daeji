# Docker Entrypoint Does Not Generate config.toml for Runtime Tuning

**Category:** docker, enhancement
**Severity:** medium

## Summary

The Docker entrypoint script starts the Kora validator with CLI flags for `--data-dir`, `--peers`, and `--chain-id`, but never generates or passes a `config.toml` file. This means the node always uses compiled-in defaults for all configuration parameters that are not exposed as CLI flags -- including consensus timeouts, block codec limits, gas limit, RPC listen address, network listen address, and the `dialable_addr` setting. Operators cannot tune these parameters without rebuilding the Docker image or manually mounting a config file.

## Problem

Kora is an EVM execution client whose runtime behavior is governed by `NodeConfig` (defined in `crates/node/config/src/node.rs`). This struct contains nested configuration for consensus (`ConsensusConfig`), networking (`NetworkConfig`), execution (`ExecutionConfig`), and RPC (`RpcConfig`). When no `--config` flag is passed, `NodeConfig::load(None)` returns `NodeConfig::default()`, which uses all compiled-in defaults.

The Docker entrypoint (`docker/scripts/entrypoint.sh`) launches the validator at lines 200-205:

```bash
exec /usr/local/bin/kora validator \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    $GOSSIP_FLAG \
    "$@"
```

There is no `--config` flag. The binary's CLI definition (`bin/kora/src/cli.rs`, lines 19-20) supports a `--config` flag:

```rust
#[arg(short, long, value_name = "FILE", global = true)]
pub config: Option<PathBuf>,
```

But the entrypoint never uses it, and no `config.toml` is generated from environment variables.

## Code Reference

File: `docker/scripts/entrypoint.sh`, lines 200-205 (the validator launch command):

```bash
exec /usr/local/bin/kora validator \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    --chain-id "$CHAIN_ID" \
    $GOSSIP_FLAG \
    "$@"
```

File: `crates/node/config/src/node.rs`, lines 54-66 (the defaults that always apply):

```rust
impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            chain_id: DEFAULT_CHAIN_ID,
            data_dir: PathBuf::from(DEFAULT_DATA_DIR),
            worker_threads: default_worker_threads(),
            consensus: ConsensusConfig::default(),
            network: NetworkConfig::default(),
            execution: ExecutionConfig::default(),
            rpc: RpcConfig::default(),
        }
    }
}
```

Key consensus parameters that cannot be tuned via Docker (from `crates/node/config/src/consensus.rs`):
- `leader_timeout_secs`: default 1s (line 35)
- `certification_timeout_secs`: default 2s (line 43)
- `timeout_retry_secs`: default 1s (line 50)
- `fetch_timeout_secs`: default 5s (line 53)
- `activity_timeout_views`: default 20 (line 56)

## Impact

1. **Geo-distributed deployments**: A deployment spanning multiple data centers requires higher consensus timeouts (e.g., 3-5 seconds for leader timeout) to account for cross-region latency. With the current entrypoint, the only option is to rebuild the Docker image with different defaults.

2. **High-throughput chains**: Different gas limits or block codec limits may be needed for specific workloads. These are configurable via `NodeConfig` but not exposed through the Docker entrypoint.

3. **Missing `dialable_addr`**: The `NetworkConfig` has a `dialable_addr: Option<String>` field (in `crates/node/config/src/network.rs`, line 18) that is critical for Docker networking (see issue 058). Without a config file, this field is always `None`, causing nodes to advertise `0.0.0.0:30303` to peers instead of their container hostname.

4. **Operator workflow**: Operators must either (a) manually create a config file and mount it as a Docker volume (undocumented), (b) pass it via the `"$@"` catch-all at the end of the exec command (fragile, requires knowing internal CLI structure), or (c) rebuild the image.

## Root Cause

The entrypoint was designed for a single-purpose devnet where compiled-in defaults are sufficient. As the deployment matrix expanded (different node counts, network topologies, hardware), configuration flexibility was not added to the entrypoint.

## Suggested Fix

Generate a `config.toml` in the entrypoint from environment variables, then pass it via `--config`:

**Add before the `exec` command in the validator case** (`docker/scripts/entrypoint.sh`):

```bash
# Generate config.toml from environment variables
CONFIG_FILE="${DATA_DIR}/config.toml"
cat > "$CONFIG_FILE" <<EOF
chain_id = ${CHAIN_ID:-1337}

[consensus.simplex]
leader_timeout_secs = ${KORA_LEADER_TIMEOUT:-1}
certification_timeout_secs = ${KORA_CERT_TIMEOUT:-2}
timeout_retry_secs = ${KORA_TIMEOUT_RETRY:-1}

[network]
listen_addr = "${KORA_LISTEN_ADDR:-0.0.0.0:30303}"
$([ -n "${KORA_DIALABLE_ADDR}" ] && echo "dialable_addr = \"${KORA_DIALABLE_ADDR}\"")

[rpc]
http_addr = "${KORA_RPC_ADDR:-0.0.0.0:8545}"
EOF

exec /usr/local/bin/kora validator \
    --config "$CONFIG_FILE" \
    --data-dir "$DATA_DIR" \
    --peers "${SHARED_DIR}/peers.json" \
    $GOSSIP_FLAG \
    "$@"
```

This allows operators to tune any parameter by adding environment variables in the Docker compose file:

```yaml
environment:
  - KORA_LEADER_TIMEOUT=3
  - KORA_CERT_TIMEOUT=5
  - KORA_DIALABLE_ADDR=validator-node0:30303
```

## Files to Modify

- `docker/scripts/entrypoint.sh` -- add config.toml generation in the `validator` case (around line 199)

## Related Issues

- `058-docker-dialable-addr-not-set-4node-devnet.md` -- the `dialable_addr` setting requires a config file, which the entrypoint does not generate
- `051-config-default-chain-id-collides-mainnet.md` -- chain ID is currently only set via CLI flag; a config file would provide a more robust default

## Labels

enhancement, docker, config
