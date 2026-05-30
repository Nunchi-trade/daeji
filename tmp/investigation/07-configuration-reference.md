# Configuration Reference

Complete list of all configurable parameters in the Kora codebase with defaults, types, and source locations.

---

## Node Configuration (`crates/node/config/src/`)

| Parameter | Default | Type | File:Line | Notes |
|-----------|---------|------|-----------|-------|
| `chain_id` | 1 | u64 | node.rs:10,20 | Override in config file |
| `data_dir` | /var/lib/kora | PathBuf | node.rs:13,24 | Validator key + state storage |
| `consensus.validator_key` | None | Option<PathBuf> | consensus.rs:20 | Auto-generated if missing |
| `consensus.threshold` | 2 | u32 | consensus.rs:13,23 | Threshold for DKG |
| `consensus.participants` | [] | Vec<Vec<u8>> | consensus.rs:32 | Hex-encoded public keys |
| `execution.gas_limit` | 250,000,000 | u64 | execution.rs:6,16 | Per-block gas limit |
| `execution.block_time` | 2 | u64 | execution.rs:9,20 | Target block time (seconds) |
| `network.listen_addr` | 0.0.0.0:30303 | String | network.rs:6,13 | P2P listen address |
| `network.dialable_addr` | None | Option<String> | network.rs:18 | NAT traversal |
| `network.bootstrap_peers` | [] | Vec<String> | network.rs:22 | Bootstrap addresses |
| `rpc.http_addr` | 0.0.0.0:8545 | String | rpc.rs:6,16 | HTTP JSON-RPC |
| `rpc.ws_addr` | 0.0.0.0:8546 | String | rpc.rs:9,20 | WebSocket RPC |

---

## Runner Constants (`crates/node/runner/src/runner.rs`)

### Block Codec

| Constant | Value | Line | Purpose |
|----------|-------|------|---------|
| `BLOCK_CODEC_MAX_TXS` | 10,000 | 44 | Max transactions per block |
| `BLOCK_CODEC_MAX_TX_BYTES` | 8 MiB | 47 | Max block size in bytes |

### Consensus Timeouts

| Constant | Value | Line | Simplex Default | Purpose |
|----------|-------|------|-----------------|---------|
| `CONSENSUS_LEADER_TIMEOUT` | 2s | 48 | 1s | Leader proposal timeout |
| `CONSENSUS_CERTIFICATION_TIMEOUT` | 4s | 49 | 2s | 2/3+ certification timeout |
| `CONSENSUS_TIMEOUT_RETRY` | 1s | 50 | 5s | Retry interval |
| `CONSENSUS_FETCH_TIMEOUT` | 1s | 51 | 1s | Block fetch timeout |
| `CONSENSUS_ACTIVITY_TIMEOUT` | 256 views | 52 | 20 views | Inactivity timeout |
| `CONSENSUS_SKIP_TIMEOUT` | 32 views | 53 | 10 views | Skip advance timeout |

### Worker Configuration

| Constant | Value | Line | Purpose |
|----------|-------|------|---------|
| `SIGNATURE_THREADS` | 2 | 54 | BLS signature verification threads |
| `EPOCH_LENGTH` | u64::MAX | 55 | Epoch length (effectively infinite) |
| `PARTITION_PREFIX` | "kora" | 56 | Storage partition name |
| `MAILBOX_SIZE` | 1024 | (simplex default) | Consensus mailbox capacity |

### Storage

| Constant | Value | Line | Purpose |
|----------|-------|------|---------|
| `RUNTIME_DIR_ENV` | "KORA_RUNTIME_DIR" | 57 | Env var to override runtime dir |
| Runtime dir default | `{data_dir}/runtime` | 74-76 | Commonware journal storage |

---

## Executor Configuration (`crates/node/executor/src/config.rs`)

| Parameter | Default | Type | Line | Purpose |
|-----------|---------|------|------|---------|
| `chain_id` | Required | u64 | 52 | Must match network |
| `spec_id` | CANCUN | SpecId | 54,66 | EVM hardfork version |
| `gas_limit_bounds.min` | 5,000 | u64 | 19 | Minimum gas limit |
| `gas_limit_bounds.max` | u64::MAX | u64 | 19 | Maximum gas limit |
| `gas_limit_bounds.max_delta_divisor` | 1,024 | u64 | 19 | Max change rate |
| `base_fee_params.elasticity_multiplier` | 2 | u64 | 39 | EIP-1559 elasticity |
| `base_fee_params.max_change_denominator` | 8 | u64 | 39 | EIP-1559 max change |

---

## Transaction Pool Configuration (`crates/node/txpool/src/config.rs`)

| Parameter | Default | Type | Line | Purpose |
|-----------|---------|------|------|---------|
| `max_pending_txs` | 4,096 | usize | 23 | Global executable tx limit |
| `max_queued_txs` | 1,024 | usize | 24 | Global future-nonce limit |
| `max_txs_per_sender` | 256 | usize | 25 | Per-sender transaction cap |
| `max_tx_size` | 128 KiB | usize | 26 | Max transaction size |
| `min_gas_price` | 0 | u128 | 27 | Minimum gas price |
| `replacement_bump_percent` | 10% | u8 | 28 | Required gas increase for replacement |

**Note:** These defaults are used by `TransactionValidator` even though `TransactionPool` is not wired.

---

## RPC Server Configuration (`crates/node/rpc/src/config.rs`)

| Parameter | Default | Type | Line | Purpose |
|-----------|---------|------|------|---------|
| `http_addr` | 127.0.0.1:8545 | SocketAddr | 65 | HTTP RPC bind |
| `jsonrpc_addr` | 127.0.0.1:8545 | SocketAddr | 66 | jsonrpsee bind |
| `chain_id` | 1 | u64 | 67 | Chain ID for responses |
| `max_connections` | 100 | u32 | 31 | Max concurrent connections |
| `cors.allowed_origins` | ["http://localhost:3000"] | Vec<String> | 92 | CORS origins |
| `cors.allowed_methods` | ["GET","POST","OPTIONS"] | Vec<String> | 93 | CORS methods |
| `cors.max_age` | 3,600 | u64 | 95 | Preflight cache (seconds) |
| `rate_limit.requests_per_second` | 100 | u64 | 141 | **NOT ENFORCED** |
| `rate_limit.burst_size` | 200 | u64 | 141 | **NOT ENFORCED** |

---

## Simplex Defaults (`crates/node/simplex/src/config.rs`)

These are the Simplex library defaults. Runner overrides several of them (see Runner Constants above).

| Parameter | Default | Type | Line | Runner Override |
|-----------|---------|------|------|-----------------|
| `DEFAULT_MAILBOX_SIZE` | 1,024 | usize | 17 | Same |
| `DEFAULT_REPLAY_BUFFER` | 1 MiB | usize | 20 | 16 MiB |
| `DEFAULT_WRITE_BUFFER` | 1 MiB | usize | 23 | 16 MiB |
| `DEFAULT_LEADER_TIMEOUT` | 1s | Duration | 26 | 2s |
| `DEFAULT_NOTARIZATION_TIMEOUT` | 2s | Duration | 29 | 4s |
| `DEFAULT_NULLIFY_RETRY` | 5s | Duration | 32 | 1s |
| `DEFAULT_FETCH_TIMEOUT` | 1s | Duration | 35 | 1s |
| `DEFAULT_ACTIVITY_TIMEOUT` | 20 views | ViewDelta | 38 | 256 views |
| `DEFAULT_SKIP_TIMEOUT` | 10 views | ViewDelta | 41 | 32 views |
| `DEFAULT_FETCH_CONCURRENT` | 8 | usize | 44 | 32 |

---

## P2P Network Configuration (`crates/network/transport/src/config.rs`)

| Parameter | Default | Line | Purpose |
|-----------|---------|------|---------|
| `DEFAULT_MAX_MESSAGE_SIZE` | 1 MiB | 12 | Max P2P message size |
| Channel quota | 1000 msg/s | builder.rs:21-24 | Per-channel rate limit |
| Channel backlog | 256 | config.rs:15 | Message queue size |
| Resolver mailbox | 1024 | marshal/peers.rs:32 | Resolver queue size |
| Resolver initial delay | 200ms | marshal/peers.rs:33 | Initial fetch delay |
| Resolver timeout | 200ms | marshal/peers.rs:34 | Fetch timeout |
| Fetch retry timeout | 100ms | marshal/peers.rs:35 | Retry interval |

---

## Docker Environment Variables

| Variable | Default | Used By | Purpose |
|----------|---------|---------|---------|
| `RUST_LOG` | info | All nodes | Log level filter |
| `CHAIN_ID` | 1337 | All nodes | Network chain ID |
| `KORA_RUNTIME_DIR` | /runtime | All nodes | Commonware journal storage |
| `HEALTHCHECK_MODE` | ready | Validators | Health check mode |
| `IS_BOOTSTRAP` | varies | Node 0 | Whether node is bootstrap |
| `BOOTSTRAP_PEERS` | varies | Nodes 1-3 | Bootstrap peer addresses |
| `GF_SECURITY_ADMIN_USER` | admin | Grafana | Dashboard admin username |
| `GF_SECURITY_ADMIN_PASSWORD` | admin | Grafana | Dashboard admin password |
| `GF_AUTH_ANONYMOUS_ENABLED` | true | Grafana | Allow anonymous access |
| `GF_AUTH_ANONYMOUS_ORG_ROLE` | Viewer | Grafana | Anonymous role |

---

## Tuning Recommendations

| Parameter | Current | Suggested | Rationale |
|-----------|---------|-----------|-----------|
| `CONSENSUS_ACTIVITY_TIMEOUT` | 256 views | 64 views | Faster recovery from persistent stalls |
| `BLOCK_CODEC_MAX_TXS` | 10,000 | 1,000 | Reduce block build time, limit ECDSA recovery |
| `max_pending_txs` | 4,096 | 4,096 | Good default (needs TransactionPool wired) |
| `max_txs_per_sender` | 256 | 64 | Reduce impact of single-sender flooding |
| Prometheus scrape | 15s | 10s | Better resolution for consensus events |
| `CONSENSUS_LEADER_TIMEOUT` | 2s | 1s | Faster consensus under normal conditions |
| `SIGNATURE_THREADS` | 2 | 4 | More verification parallelism |
