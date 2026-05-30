# Kora Configuration System

## What Is Kora?

Kora is an EVM-compatible blockchain built on the Commonware consensus framework. It uses BLS12-381 threshold signatures for finalization and ed25519 for P2P identity. The system runs a Simplex-based consensus engine, executes EVM transactions via REVM, and manages state through QMDB. Validators participate in a Distributed Key Generation (DKG) ceremony to establish threshold signing keys, then run a consensus protocol to finalize blocks.

## How Kora Is Configured

Kora uses a layered configuration system:

1. **Configuration file** (TOML or JSON) -- loaded via `--config <path>` CLI flag
2. **CLI arguments** -- override specific config file values (e.g., `--chain-id`, `--data-dir`)
3. **Environment variables** -- used primarily in Docker deployments for runtime overrides
4. **Code constants** -- hardcoded defaults compiled into the binary, not modifiable at runtime
5. **Docker Compose file** -- orchestrates multi-validator devnets with per-container environment

Configuration is loaded at startup by the `NodeConfig::load()` function. If no config file is provided, all values fall back to compiled defaults. There is no hot-reload mechanism; all changes require a process restart.

---

## Configuration Sources

### CLI Arguments

Defined in `bin/kora/src/cli.rs` using the `clap` crate:

| Argument | Type | Scope | Description |
|----------|------|-------|-------------|
| `--config, -c` | `PathBuf` | global | Path to TOML/JSON config file |
| `--verbose, -v` | `bool` | global | Enable verbose logging |
| `--chain-id` | `u64` | global | Override chain ID from config |
| `--data-dir` | `PathBuf` | global | Override data directory from config |
| `--peers` | `PathBuf` | dkg/validator/secondary | Path to `peers.json` with participant info |
| `--force-restart` | `bool` | dkg | Force restart DKG, ignoring persisted state |

### Subcommands

| Command | Description |
|---------|-------------|
| `kora dkg` | Run DKG ceremony to generate threshold key shares |
| `kora validator` | Run a consensus validator node |
| `kora secondary` | Run a non-voting secondary (follower) peer |
| (none) | Run in legacy mode |

### Environment Variables

| Variable | Default | Where Used | Description |
|----------|---------|------------|-------------|
| `RUST_LOG` | `info` | Docker compose, tracing subscriber | Log level filter (e.g., `debug`, `info,kora_runner=trace`) |
| `CHAIN_ID` | `1337` | Docker compose, entrypoint.sh | Chain identifier passed to `--chain-id` |
| `KORA_RUNTIME_DIR` | `/runtime` (Docker) | Validator containers | Override directory for Commonware runtime storage (journals) |
| `VALIDATOR_INDEX` | `0` | Docker compose | Identifies which validator identity to use |
| `IS_BOOTSTRAP` | `false` | Docker compose, entrypoint.sh | Whether this node is the initial bootstrap peer |
| `BOOTSTRAP_PEERS` | `""` | Docker compose, entrypoint.sh | Comma-separated list of `host:port` bootstrap addresses |
| `DATA_DIR` | `/data` | entrypoint.sh | Persistent storage directory inside container |
| `SHARED_DIR` | `/shared` | entrypoint.sh | Shared configuration volume mount point |
| `HEALTHCHECK_MODE` | `ready` | Docker compose, healthcheck.sh | Healthcheck behavior: `p2p`, `ready`, or `dkg` |
| `COMPOSE_PROFILES` | (unset) | devnet-run.sh | Set to `none` to skip observability stack |

### Configuration File Format

Supported formats: TOML (default) and JSON (detected by `.json` extension).

Example TOML:

```toml
chain_id = 1337
data_dir = "/var/lib/kora"

[consensus]
threshold = 3
validator_key = "/data/validator.key"
participants = [
  "aabbccdd...",
  "11223344..."
]

[network]
listen_addr = "0.0.0.0:30303"
dialable_addr = "1.2.3.4:30303"
bootstrap_peers = ["pubkey_hex@host:port"]

[execution]
gas_limit = 250000000
block_time = 2

[rpc]
http_addr = "0.0.0.0:8545"
ws_addr = "0.0.0.0:8546"
```

---

## Configurable Parameters by Subsystem

### Node (Top-Level)

| Parameter | Config Key | Default | Description |
|-----------|-----------|---------|-------------|
| Chain ID | `chain_id` | `1` | EVM chain identifier; affects transaction signing and replay protection |
| Data Directory | `data_dir` | `/var/lib/kora` | Root directory for all persistent state (keys, DKG output, journals) |

Source: `crates/node/config/src/node.rs`

---

### Consensus (Simplex Engine)

These are defined as compile-time constants in `crates/node/runner/src/runner.rs`. They are NOT exposed in the configuration file.

| Parameter | Constant Name | Default | Description |
|-----------|--------------|---------|-------------|
| Leader timeout | `CONSENSUS_LEADER_TIMEOUT` | 2 seconds | How long to wait for a leader to propose before triggering nullification |
| Certification timeout | `CONSENSUS_CERTIFICATION_TIMEOUT` | 4 seconds | How long to wait for block certification (notarization) |
| Timeout retry | `CONSENSUS_TIMEOUT_RETRY` | 1 second | Interval between nullification retry attempts |
| Fetch timeout | `CONSENSUS_FETCH_TIMEOUT` | 1 second | Timeout for fetching missing blocks from peers |
| Activity timeout | `CONSENSUS_ACTIVITY_TIMEOUT` | 256 views | Views of inactivity before a validator is considered inactive |
| Skip timeout | `CONSENSUS_SKIP_TIMEOUT` | 32 views | Views before skipping a stalled validator |
| Fetch concurrent | (inline) | 32 | Maximum concurrent block fetch requests |
| Mailbox size | `MAILBOX_SIZE` (from kora_simplex) | 1024 | Internal channel buffer for consensus messages |
| Replay buffer | (inline) | 16 MiB | Buffer for journal replay on startup |
| Write buffer | (inline) | 16 MiB | Buffer for writing to journals |
| Epoch length | `EPOCH_LENGTH` | `u64::MAX` | Effectively infinite; no epoch rotation |
| Signature threads | `SIGNATURE_THREADS` | 2 | Thread pool size for signature verification |
| Forwarding policy | (inline) | `SilentLeader` | How proposals are forwarded among validators |
| Partition prefix | `PARTITION_PREFIX` | `"kora"` | Storage namespace for consensus journals |

**Default Configuration (from `kora_simplex::DefaultConfig`):**

These are the library defaults, which the production runner overrides:

| Parameter | Default (library) | Production Override |
|-----------|------------------|-------------------|
| Leader timeout | 1 second | 2 seconds |
| Certification timeout | 2 seconds | 4 seconds |
| Timeout retry | 5 seconds | 1 second |
| Fetch timeout | 1 second | 1 second |
| Activity timeout | 20 views | 256 views |
| Skip timeout | 10 views | 32 views |
| Fetch concurrent | 8 | 32 |

Source: `crates/node/simplex/src/config.rs`, `crates/node/runner/src/runner.rs`

---

### Consensus (Config File)

| Parameter | Config Key | Default | Description |
|-----------|-----------|---------|-------------|
| Validator key path | `consensus.validator_key` | `{data_dir}/validator.key` | Path to 32-byte ed25519 private key file |
| Threshold | `consensus.threshold` | `2` | Minimum signers for threshold BLS signature (t of n) |
| Participants | `consensus.participants` | `[]` | Hex-encoded ed25519 public keys of all validators |

Source: `crates/node/config/src/consensus.rs`

---

### Transaction Pool

Defined in `crates/node/txpool/src/config.rs`. The pool uses code-level defaults and is NOT configurable via file or environment.

| Parameter | Builder Method | Default | Description |
|-----------|--------------|---------|-------------|
| Max pending transactions | `with_max_pending_txs` | 4,096 | Maximum executable (ready) transactions in pool |
| Max queued transactions | `with_max_queued_txs` | 1,024 | Maximum future-nonce transactions in pool |
| Max transactions per sender | `with_max_txs_per_sender` | 256 | Maximum transactions from a single address |
| Max transaction size | `with_max_tx_size` | 131,072 (128 KiB) | Maximum raw transaction byte size |
| Min gas price | `with_min_gas_price` | 0 | Minimum effective gas price for acceptance |
| Replacement bump percent | `with_replacement_bump_percent` | 10 | % fee increase required to replace a pending tx |

**Validation rules applied by `TransactionValidator`:**

- Chain ID must match node's chain ID
- Effective gas price must be >= `min_gas_price`
- Gas limit must cover intrinsic gas (21000 + calldata + access list + create costs)
- Nonce must be >= state nonce
- Nonce must be <= state nonce + `max_txs_per_sender` (prevents far-future nonces)
- Balance must cover `gas_limit * max_fee_per_gas + value`
- Transaction size must be <= `max_tx_size`

Source: `crates/node/txpool/src/config.rs`, `crates/node/txpool/src/validator.rs`

---

### Block Production

| Parameter | Constant Name | Default | Description |
|-----------|--------------|---------|-------------|
| Max transactions per block | `BLOCK_CODEC_MAX_TXS` | 10,000 | Maximum number of transactions that can be included in one block |
| Max block payload size | `BLOCK_CODEC_MAX_TX_BYTES` | 8 MiB | Maximum total bytes of all transactions in one block |

These are compile-time constants in `crates/node/runner/src/runner.rs` and are used both for block production (`mempool.build(max_txs, ...)`) and codec limits.

The `RevmApplication` is initialized with `block_cfg.max_txs` and a configurable `gas_limit`.

---

### RPC Server

**From the config file** (`crates/node/config/src/rpc.rs`):

| Parameter | Config Key | Default | Description |
|-----------|-----------|---------|-------------|
| HTTP/JSON-RPC address | `rpc.http_addr` | `0.0.0.0:8545` | Address for the JSON-RPC endpoint |
| WebSocket address | `rpc.ws_addr` | `0.0.0.0:8546` | Address for WebSocket subscriptions |

**From the RPC server module** (`crates/node/rpc/src/config.rs`):

| Parameter | Default | Description |
|-----------|---------|-------------|
| HTTP endpoint | `127.0.0.1:8545` | HTTP status/JSON-RPC address |
| JSON-RPC endpoint | `127.0.0.1:8545` | JSON-RPC server address |
| Chain ID | `1` | Returned in `eth_chainId` responses |
| Max connections | 100 | Maximum concurrent WebSocket/HTTP connections |
| Requests per second | 100 | Rate limit per client |
| Burst size | 200 | Burst allowance for rate limiting |
| CORS allowed origins | `["http://localhost:3000"]` | Origins permitted for cross-origin requests |
| CORS allowed methods | `["GET", "POST", "OPTIONS"]` | HTTP methods allowed |
| CORS allowed headers | `["Content-Type"]` | Request headers allowed |
| CORS max age | 3,600 seconds | Preflight cache duration |

**Note:** The rate limiting configuration is defined in the `RateLimitConfig` struct but **is not currently wired** into the actual RPC server middleware. The config exists for future use.

Source: `crates/node/rpc/src/config.rs`

---

### Networking (P2P)

**From the config file** (`crates/node/config/src/network.rs`):

| Parameter | Config Key | Default | Description |
|-----------|-----------|---------|-------------|
| Listen address | `network.listen_addr` | `0.0.0.0:30303` | Socket address for incoming P2P connections |
| Dialable address | `network.dialable_addr` | None (uses listen_addr) | External address for NAT traversal |
| Bootstrap peers | `network.bootstrap_peers` | `[]` | Initial peers as `pubkey_hex@host:port` |

**From transport constants** (`crates/network/transport/src/config.rs`):

| Parameter | Constant | Default | Description |
|-----------|----------|---------|-------------|
| Max message size | `DEFAULT_MAX_MESSAGE_SIZE` | 1 MiB (1,048,576 bytes) | Maximum P2P message payload |
| Channel backlog | `DEFAULT_BACKLOG` | 256 | Size of internal message queues |
| Namespace | `DEFAULT_NAMESPACE` | `_COMMONWARE_KORA_NETWORK` | Protocol identifier for peer discovery |

The production runner overrides the backlog to 2048 for local transport builds.

Source: `crates/network/transport/src/config.rs`, `crates/network/transport/src/ext.rs`

---

### Storage

| Parameter | Source | Default | Description |
|-----------|--------|---------|-------------|
| Runtime directory | `KORA_RUNTIME_DIR` env var | `{data_dir}/runtime` | Directory for Commonware runtime journals (consensus WAL) |
| Data directory | `data_dir` config | `/var/lib/kora` | Root persistent storage |
| QMDB partition | Code | `{partition_prefix}-qmdb` | State database storage partition |
| Finalized blocks archive | Code | `{partition_prefix}-finalized-blocks` | Archive of finalized block data |
| Finalizations archive | Code | `{partition_prefix}-finalizations-by-height` | Archive of finalization certificates |

**Docker tmpfs configuration:**

In the Docker Compose devnet, validators mount a tmpfs at `/runtime` with 1 GiB size limit. This avoids Docker volume fsync latency for consensus journals, which are write-heavy.

```yaml
tmpfs:
  - /runtime:size=1g,mode=1777
```

The `KORA_RUNTIME_DIR` environment variable defaults to `/runtime` inside containers, directing journal writes to this tmpfs. This means **consensus journal state is ephemeral** in devnet; only the `/data` volume persists DKG shares and validator keys across restarts.

**Buffer pool configuration** (`crates/node/simplex/src/pool.rs`):

| Parameter | Default | Description |
|-----------|---------|-------------|
| Page size | 65,535 bytes (~64 KiB) | Size of each buffer page |
| Pool capacity | 10,000 pages | Total buffer pool size (~625 MiB max) |

Source: `crates/node/runner/src/runner.rs`, `docker/compose/devnet.yaml`

---

### EVM / Execution

**From the config file** (`crates/node/config/src/execution.rs`):

| Parameter | Config Key | Default | Description |
|-----------|-----------|---------|-------------|
| Block gas limit | `execution.gas_limit` | 250,000,000 | Maximum gas per block |
| Block time | `execution.block_time` | 2 seconds | Target block production interval |

**From the executor config** (`crates/node/executor/src/config.rs`):

| Parameter | Default | Description |
|-----------|---------|-------------|
| Chain ID | 1 | EVM chain ID for execution context |
| Spec ID | `CANCUN` | EVM hardfork specification level |
| Gas limit min | 5,000 | Minimum allowed gas limit |
| Gas limit max | `u64::MAX` | Maximum allowed gas limit |
| Gas limit delta divisor | 1,024 | Max change per block: `parent_limit / divisor` |
| Base fee elasticity multiplier | 2 | EIP-1559 elasticity multiplier |
| Base fee max change denominator | 8 | EIP-1559 max base fee change per block |

**Current production behavior:**

In the production runner, `base_fee_per_gas` is hardcoded to `Some(0)` in block contexts. EIP-1559 fee logic is structurally present but effectively disabled (base fee always zero).

Source: `crates/node/executor/src/config.rs`, `crates/node/config/src/execution.rs`

---

### DKG (Distributed Key Generation)

Configured programmatically via `DkgConfig` struct (`crates/node/dkg/src/config.rs`):

| Parameter | Source | Default | Description |
|-----------|--------|---------|-------------|
| Identity key | `validator.key` file | Generated if missing | ed25519 private key for P2P authentication |
| Validator index | Derived from peers.json | N/A | Position in the participant set |
| Participants | peers.json | N/A | All validator public keys |
| Threshold | peers.json / config | 3 (devnet) | Minimum signers (t of n) |
| Chain ID | `--chain-id` / config | 1337 (devnet) | Domain separation for DKG protocol |
| Data directory | `--data-dir` / config | `/data` (Docker) | Where to store `share.key` and `output.json` |
| Listen address | `network.listen_addr` | `0.0.0.0:30303` | P2P socket for DKG ceremony |
| Bootstrap peers | peers.json | N/A | Initial peer connections |
| Timeout | Code | 300 seconds (5 minutes) | Total DKG ceremony timeout |

**DKG modes:**

- **Trusted dealer** (`keygen dkg-deal`): Single process generates all shares. Fast, suitable for development. No network required.
- **Interactive** (`kora dkg`): All participants run a P2P DKG ceremony. Required for production security.

**Outputs:**
- `share.key`: This validator's BLS12-381 secret key share
- `output.json`: Public DKG output (group public key, verification keys)

Source: `crates/node/dkg/src/config.rs`, `bin/kora/src/cli.rs`

---

### Logging

| Parameter | Source | Default | Description |
|-----------|--------|---------|-------------|
| Log filter | `RUST_LOG` env var | `info` | Standard `tracing` filter directive |
| Format | Code | Full format with timestamps | Uses `tracing_subscriber::fmt` |

**Recommended log levels for debugging:**

```
RUST_LOG=info                        # Normal operation
RUST_LOG=debug                       # Block production/verification timing
RUST_LOG=kora_runner=debug           # Runner-level diagnostics
RUST_LOG=kora_runner=trace           # Mempool drain details
RUST_LOG=kora_txpool=debug           # Transaction pool operations
RUST_LOG=commonware_consensus=debug  # Consensus protocol messages
```

---

### Metrics

| Parameter | Source | Default | Description |
|-----------|--------|---------|-------------|
| Metrics address | Code | `0.0.0.0:9002` | Prometheus metrics endpoint |
| Metrics path | Code | `/metrics` | HTTP path for scraping |
| Format | Code | OpenMetrics text | `application/openmetrics-text` |

In the devnet compose, Prometheus scrapes ports 9000-9003 (mapped from internal 9002).

---

## Parameters That SHOULD Be Configurable But Are Hardcoded

### Mempool Size Limits

The `PoolConfig` defaults (4096 pending, 1024 queued) are not exposed in any config file or environment variable. They are set at compile time. Under heavy load, the pool simply logs warnings when limits are exceeded but does **not actively evict** transactions. Operators cannot tune pool capacity without code changes.

### Transaction TTL

There is no time-to-live mechanism for pooled transactions. Once accepted, a transaction remains in the pool indefinitely until it is either included in a block or its nonce is consumed. There is no periodic cleanup of stale transactions.

### Block Hash History Depth

The EVM execution context sets `block_hash` to `B256::ZERO` in the `BlockContext`. There is no maintained history of previous block hashes. The `BLOCKHASH` opcode will return zero for all queries. This is hardcoded and not configurable.

### Base Fee

The base fee per gas is hardcoded to `0` in block production. While the `BaseFeeParams` struct exists with proper EIP-1559 parameters, the production runner bypasses it entirely.

### RPC Port

The production validator hardcodes the RPC bind address to `0.0.0.0:8545`. While `RpcConfig` in the config file has an `http_addr` field, the validator runner ignores it and uses its own hardcoded address.

### Metrics Port

Similarly hardcoded to `0.0.0.0:9002` in the validator runner.

### Consensus Timeouts

All consensus timing parameters (leader timeout, certification timeout, etc.) are compile-time constants. Operators cannot tune them without rebuilding the binary. This is particularly problematic for networks with different latency profiles.

---

## Configuration Validation

**Current behavior: crash at startup.**

The system provides no graceful validation layer. Invalid configurations manifest as:

- **Parse errors**: `ConfigError::TomlParse` or `ConfigError::JsonParse` -- returned immediately if the config file has syntax errors
- **Missing files**: `ConfigError::Read` -- returned if the config file path is specified but does not exist
- **Invalid key length**: `ConfigError::InvalidKeyLength` -- if `validator.key` is not exactly 32 bytes
- **Invalid participant keys**: `ConfigError::InvalidParticipantKeyLength` / `ConfigError::InvalidParticipantKey` -- if consensus participants have malformed keys
- **Invalid listen address**: `TransportError::InvalidListenAddr` -- if `network.listen_addr` cannot be parsed as a socket address
- **Missing DKG output**: Hard error if `kora validator` is run without prior DKG ceremony

**What is NOT validated:**
- Gas limit of 0 (will produce empty blocks silently)
- Threshold > number of participants (will cause consensus liveness failure)
- Bootstrap peers that are unreachable (will timeout in entrypoint.sh after 120s)
- Chain ID mismatch between validators (will silently reject each other's transactions)

---

## Recommended Configurations

### Development (Fast Iteration)

```toml
chain_id = 1337
data_dir = "/tmp/kora-dev"

[network]
listen_addr = "127.0.0.1:30303"

[execution]
gas_limit = 250000000
block_time = 2

[rpc]
http_addr = "127.0.0.1:8545"
```

Environment:
```bash
export RUST_LOG=info,kora_runner=debug
export KORA_RUNTIME_DIR=/tmp/kora-runtime  # tmpfs for speed
```

Use trusted dealer DKG (`keygen dkg-deal`) for instant key generation.

### Testing (Stress Test Settings)

For load testing with high transaction throughput:

```toml
chain_id = 1337
data_dir = "/data"

[execution]
gas_limit = 500000000    # Higher gas limit allows more txs per block
block_time = 1           # Faster target (though actual rate depends on consensus)

[network]
listen_addr = "0.0.0.0:30303"
```

Environment:
```bash
export RUST_LOG=warn,kora_runner=info  # Reduce log noise under load
export KORA_RUNTIME_DIR=/dev/shm/kora  # RAM-backed storage for journals
```

Code-level adjustments (require rebuild):
- Increase `BLOCK_CODEC_MAX_TXS` beyond 10,000 for extreme throughput tests
- Increase `max_pending_txs` in `PoolConfig` to 16,384 or higher
- Decrease `CONSENSUS_LEADER_TIMEOUT` to 1s for faster empty block production

### Production (Hardened Values)

```toml
chain_id = <your-production-chain-id>
data_dir = "/var/lib/kora"

[consensus]
threshold = 3  # For 4 validators (tolerates 1 failure)
validator_key = "/var/lib/kora/validator.key"
participants = [
  "<validator_0_pubkey_hex>",
  "<validator_1_pubkey_hex>",
  "<validator_2_pubkey_hex>",
  "<validator_3_pubkey_hex>"
]

[network]
listen_addr = "0.0.0.0:30303"
dialable_addr = "<public_ip>:30303"
bootstrap_peers = ["<pubkey>@<peer_host>:30303"]

[execution]
gas_limit = 250000000
block_time = 2

[rpc]
http_addr = "0.0.0.0:8545"
```

Environment:
```bash
export RUST_LOG=info
# Do NOT set KORA_RUNTIME_DIR -- let it default to {data_dir}/runtime
# for persistence across restarts
```

Production recommendations:
- Use interactive DKG ceremony (`kora dkg`) -- never trusted dealer in production
- Back up `validator.key` and `share.key` securely
- Place `data_dir` on fast SSD with proper fsync support
- Do NOT use tmpfs for production runtime storage (data loss on restart/crash)
- Ensure all validators use identical `chain_id` and `participants` lists
- Run behind a reverse proxy that enforces rate limiting (since built-in rate limiting is not wired)
- Set `dialable_addr` when behind NAT/firewall

---

## Environment Variable Reference Table

| Variable | Required | Default | Used By | Description |
|----------|----------|---------|---------|-------------|
| `RUST_LOG` | No | `info` | All binaries | tracing filter directive |
| `CHAIN_ID` | No | `1337` | Docker entrypoint | Passed as `--chain-id` to kora binary |
| `KORA_RUNTIME_DIR` | No | `{data_dir}/runtime` | Validator runtime | Override Commonware storage directory |
| `VALIDATOR_INDEX` | Yes (Docker) | `0` | Docker entrypoint | Identifies validator in multi-node setup |
| `IS_BOOTSTRAP` | No | `false` | Docker entrypoint | Skip waiting for bootstrap peer |
| `BOOTSTRAP_PEERS` | No | `""` | Docker entrypoint | Bootstrap peer `host:port` |
| `DATA_DIR` | No | `/data` | Docker entrypoint | Container data directory |
| `SHARED_DIR` | No | `/shared` | Docker entrypoint | Shared config volume path |
| `HEALTHCHECK_MODE` | No | `ready` | healthcheck.sh | `p2p`, `ready`, or `dkg` |
| `COMPOSE_PROFILES` | No | (unset) | devnet-run.sh | Set to `none` to skip observability |
| `GF_SECURITY_ADMIN_USER` | No | `admin` | Grafana | Grafana admin username |
| `GF_SECURITY_ADMIN_PASSWORD` | No | `admin` | Grafana | Grafana admin password |

---

## Docker Compose Devnet Port Mapping

| Service | Internal Port | External Port | Purpose |
|---------|--------------|---------------|---------|
| validator-node0 | 30303 | 30400 | P2P |
| validator-node0 | 8545 | 8545 | JSON-RPC |
| validator-node0 | 9002 | 9000 | Metrics |
| validator-node1 | 30303 | 30401 | P2P |
| validator-node1 | 8545 | 8546 | JSON-RPC |
| validator-node1 | 9002 | 9001 | Metrics |
| validator-node2 | 30303 | 30402 | P2P |
| validator-node2 | 8545 | 8547 | JSON-RPC |
| validator-node2 | 9002 | 9002 | Metrics |
| validator-node3 | 30303 | 30403 | P2P |
| validator-node3 | 8545 | 8548 | JSON-RPC |
| validator-node3 | 9002 | 9003 | Metrics |
| secondary-node0 | 30303 | 30500 | P2P |
| prometheus | 9090 | 9090 | Prometheus UI |
| grafana | 3000 | 3000 | Grafana dashboards |
| loki | 3100 | 3100 | Log aggregation |

---

## Key Source Files

| File | Purpose |
|------|---------|
| `crates/node/config/src/node.rs` | Top-level `NodeConfig` struct |
| `crates/node/config/src/network.rs` | Network configuration |
| `crates/node/config/src/consensus.rs` | Consensus configuration |
| `crates/node/config/src/execution.rs` | Execution configuration |
| `crates/node/config/src/rpc.rs` | RPC configuration |
| `crates/node/runner/src/runner.rs` | Production runner with hardcoded constants |
| `crates/node/simplex/src/config.rs` | Consensus engine defaults |
| `crates/node/txpool/src/config.rs` | Transaction pool configuration |
| `crates/node/txpool/src/validator.rs` | Transaction validation rules |
| `crates/node/executor/src/config.rs` | EVM execution parameters |
| `crates/node/rpc/src/config.rs` | RPC server parameters |
| `crates/node/dkg/src/config.rs` | DKG ceremony configuration |
| `crates/network/transport/src/config.rs` | P2P transport constants |
| `bin/kora/src/cli.rs` | CLI argument definitions |
| `docker/scripts/entrypoint.sh` | Container startup logic |
| `docker/compose/devnet.yaml` | Multi-validator orchestration |
