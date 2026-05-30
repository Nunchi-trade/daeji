# Hardcoded Configuration Parameters Require Rebuild to Change

## Summary

Many critical runtime parameters in Kora are compiled directly into the binary as Rust `const` values or code-level defaults. Changing any of them -- consensus timeouts, block limits, transaction pool sizes, RPC bind addresses, metrics ports, or the base fee -- requires modifying source code, rebuilding the binary, and coordinating a full redeployment across all validators. This is operationally expensive and makes it impossible to tune the network for different environments (testnet vs. production, high-latency vs. low-latency) without maintaining separate builds.

The configuration infrastructure already exists: `NodeConfig` loads from TOML/JSON files, and there are `ConsensusConfig`, `ExecutionConfig`, `RpcConfig`, and `NetworkConfig` sections. But many of the most important parameters bypass this system entirely.

## Severity

**Medium-High.** No data loss or security vulnerability, but this creates a significant operational burden. A network experiencing consensus timeouts under load cannot be tuned without a synchronized binary rollout. An operator running multiple validators on the same host cannot assign different RPC or metrics ports without patching the code. These are the kinds of knobs that need to be adjustable in any production deployment.

---

## Hardcoded Parameters

### 1. Consensus Timeouts

**File:** `crates/node/runner/src/runner.rs`, lines 48-53

```rust
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_secs(2);
const CONSENSUS_CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(4);
const CONSENSUS_TIMEOUT_RETRY: Duration = Duration::from_secs(1);
const CONSENSUS_FETCH_TIMEOUT: Duration = Duration::from_secs(1);
const CONSENSUS_ACTIVITY_TIMEOUT: ViewDelta = ViewDelta::new(256);
const CONSENSUS_SKIP_TIMEOUT: ViewDelta = ViewDelta::new(32);
```

These constants are fed directly into `simplex::Config` at lines 563-568:

```rust
leader_timeout: CONSENSUS_LEADER_TIMEOUT,
certification_timeout: CONSENSUS_CERTIFICATION_TIMEOUT,
timeout_retry: CONSENSUS_TIMEOUT_RETRY,
fetch_timeout: CONSENSUS_FETCH_TIMEOUT,
activity_timeout: CONSENSUS_ACTIVITY_TIMEOUT,
skip_timeout: CONSENSUS_SKIP_TIMEOUT,
```

**Why this matters:** Consensus timeout tuning is fundamental to any BFT system. A validator set spanning multiple regions needs longer leader timeouts to account for network latency. A local devnet wants shorter timeouts for faster iteration. A testnet under adversarial conditions may need longer activity timeouts to avoid unnecessary view changes. Today, all of these scenarios require a rebuild.

The existing `ConsensusConfig` struct (`crates/node/config/src/consensus.rs`) has fields for `validator_key`, `threshold`, and `participants`, but no timeout fields at all. The timeout values exist only as compile-time constants in the runner.

### 2. Block Production Limits

**File:** `crates/node/runner/src/runner.rs`, lines 44-47

```rust
const BLOCK_CODEC_MAX_TXS: usize = 10_000;
const BLOCK_CODEC_MAX_TX_BYTES: usize = 8 * 1024 * 1024; // 8 MiB
```

These are used to construct `BlockCfg` at line 85-87:

```rust
const fn block_codec_cfg() -> BlockCfg {
    BlockCfg { max_txs: BLOCK_CODEC_MAX_TXS, tx: TxCfg { max_tx_bytes: BLOCK_CODEC_MAX_TX_BYTES } }
}
```

**Why this matters:** The maximum transactions per block and the maximum serialized block size directly control throughput and network bandwidth requirements. A high-throughput deployment might want to increase these limits. A resource-constrained testnet might want to lower them. The comment on `BLOCK_CODEC_MAX_TX_BYTES` itself acknowledges the value was sized for "a devnet stress batch of 10k signed transfers," suggesting it was not designed as a permanent production value.

### 3. Transaction Pool Sizes

**File:** `crates/node/txpool/src/config.rs`, lines 20-31

```rust
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_pending_txs: 4096,
            max_queued_txs: 1024,
            max_txs_per_sender: 256,
            max_tx_size: 128 * 1024, // 128 KB
            min_gas_price: 0,
            replacement_bump_percent: 10,
        }
    }
}
```

`PoolConfig` has a builder API (`with_max_pending_txs()`, etc.) and is a well-structured configuration object. However, it is never wired into `NodeConfig`. The runner constructs it via `PoolConfig::default()` (line 413 of `runner.rs`), and there is no way to set these values from a config file or environment variable.

**Why this matters:** Pool sizing directly affects memory usage and mempool behavior under load. An operator seeing mempool evictions at `max_pending_txs=4096` has no way to increase it without editing Rust source. The `min_gas_price=0` default means the mempool accepts zero-fee transactions, which is fine for a testnet but would be a spam vector in production. Operators need to set a minimum gas price without rebuilding.

### 4. RPC Port

**File:** `bin/kora/src/cli.rs`, line 161

```rust
let rpc_addr: std::net::SocketAddr = "0.0.0.0:8545".parse()?;
```

The `RpcConfig` struct in `crates/node/config/src/rpc.rs` already defines `http_addr` as a configurable field with `DEFAULT_HTTP_ADDR = "0.0.0.0:8545"`. The config file infrastructure exists and can parse custom addresses. But the CLI ignores the config value entirely and hardcodes the address. The `config.rpc.http_addr` field is never read during production startup.

**Why this matters:** Running multiple validators on the same machine (common in testing and staging) requires different ports. Docker deployments may need to bind to specific interfaces. The config field exists, it is just not connected.

### 5. Metrics Port

**File:** `bin/kora/src/cli.rs`, line 164

```rust
let metrics_addr: std::net::SocketAddr = "0.0.0.0:9002".parse()?;
```

Same pattern as the RPC port. The `ProductionRunner` struct accepts `metrics_addr` as an `Option<SocketAddr>`, but the value is always hardcoded in the CLI. There is no field for it in `NodeConfig` and no way to set it from configuration.

**Why this matters:** Prometheus scrape targets need predictable, configurable ports. Multi-validator hosts need unique ports per process.

### 6. Base Fee Per Gas

**File:** `crates/node/runner/src/runner.rs`, lines 117-127

```rust
impl BlockContextProvider for RevmContextProvider {
    fn context(&self, block: &Block) -> BlockContext {
        let header = Header {
            // ...
            base_fee_per_gas: Some(0),
            ..Default::default()
        };
        BlockContext::new(header, B256::ZERO, block.prevrandao)
    }
}
```

The `base_fee_per_gas` is unconditionally set to `Some(0)`. Alloy's `Header` type supports the full EIP-1559 fee structure, and the EVM execution pipeline (revm) can enforce base fee mechanics. But the fee is hardcoded to zero, meaning all EIP-1559 transaction pricing is bypassed. There is no way to enable a fee market without modifying this line and rebuilding.

**Why this matters:** A base fee of zero means there is no economic cost to submitting transactions. This is acceptable for testnets but creates a spam vulnerability in any deployment that accepts public traffic. At minimum, operators should be able to set a non-zero base fee from configuration.

### 7. Block Hash History Depth

**File:** `crates/node/runner/src/runner.rs`, line 126

```rust
BlockContext::new(header, B256::ZERO, block.prevrandao)
```

The second argument to `BlockContext::new` is the parent block hash, which is always `B256::ZERO`. The `StateDbAdapter` also returns zero for all block hash lookups. This means the `BLOCKHASH` EVM opcode always returns zero, regardless of the block number queried.

This is documented separately in issue #9, but it is worth noting here because the depth of block hash history (how many recent block hashes to maintain) is the kind of parameter that should be configurable rather than hardcoded to zero.

---

## Configuration Validation Gaps

Beyond the hardcoded values, several configuration combinations that are guaranteed to cause runtime failures are accepted silently at startup.

### Gas limit of 0

Setting `execution.gas_limit = 0` in the config file is accepted without error. The validator starts, produces blocks, but every transaction fails because no gas is available. The blocks are empty. There is no startup warning.

### Threshold greater than number of participants

Setting `consensus.threshold = 5` with only 3 participants configured is accepted without error. The validator starts, joins the network, but consensus can never finalize a block because the threshold is unreachable. The node appears to be running but no progress is made. This is a liveness failure that should be caught at startup.

### Unreachable bootstrap peers

If the configured bootstrap peer addresses are unreachable (wrong IP, firewall, process not running), the P2P layer attempts to connect with a 120-second timeout. There is no fast-fail mechanism and no clear log message distinguishing "peer not yet started" from "peer address is wrong." In a misconfigured deployment, the operator waits two minutes before seeing any indication of a problem.

### Chain ID mismatch between validators

If validators in the same network are configured with different chain IDs, they will silently reject each other's transactions. Each validator signs transactions and blocks with its own chain ID. The other validators see invalid signatures and drop the messages. There is no explicit error about chain ID disagreement; it just looks like the network is not making progress.

---

## Proposed Fix

### Phase 1: Move constants to configuration

1. **Extend `ConsensusConfig`** (`crates/node/config/src/consensus.rs`) with timeout fields:

   ```rust
   pub struct ConsensusConfig {
       // existing fields...
       pub leader_timeout_ms: u64,          // default: 2000
       pub certification_timeout_ms: u64,   // default: 4000
       pub timeout_retry_ms: u64,           // default: 1000
       pub fetch_timeout_ms: u64,           // default: 1000
       pub activity_timeout_views: u64,     // default: 256
       pub skip_timeout_views: u64,         // default: 32
   }
   ```

2. **Add block production config** to `ExecutionConfig` or a new `BlockConfig` section in `NodeConfig`:

   ```rust
   pub struct BlockConfig {
       pub max_txs: usize,             // default: 10_000
       pub max_tx_bytes: usize,        // default: 8_388_608 (8 MiB)
   }
   ```

3. **Add pool config** as a new section in `NodeConfig` (`crates/node/config/src/node.rs`):

   ```rust
   pub struct NodeConfig {
       // existing fields...
       pub pool: PoolConfig,
   }
   ```

   This requires making `PoolConfig` serializable (`Serialize`/`Deserialize` derives) and giving it serde defaults.

4. **Add metrics config** to `NodeConfig`:

   ```rust
   pub struct MetricsConfig {
       pub addr: String,               // default: "0.0.0.0:9002"
       pub enabled: bool,              // default: true
   }
   ```

5. **Add base fee config** to `ExecutionConfig`:

   ```rust
   pub struct ExecutionConfig {
       // existing fields...
       pub base_fee_per_gas: u64,      // default: 0
   }
   ```

### Phase 2: Wire config to runtime

6. **Update `runner.rs`:** Remove the `const` declarations at lines 44-53. Read values from `config.consensus.*` and `config.execution.*` when constructing `simplex::Config`, `BlockCfg`, and `RevmContextProvider`.

7. **Update `cli.rs`:** Replace hardcoded `"0.0.0.0:8545"` and `"0.0.0.0:9002"` with reads from `config.rpc.http_addr` and `config.metrics.addr`.

8. **Wire `PoolConfig`:** In `runner.rs` line 413 where `PoolConfig::default()` is used inside the RPC transaction submission callback, read from `config.pool` instead.

### Phase 3: Add startup validation

9. **Add a `validate()` method to `NodeConfig`** that checks:
   - `execution.gas_limit > 0`
   - `consensus.threshold <= participants.len()` (when participants are configured)
   - `consensus.threshold > 0`
   - All timeout values are positive
   - All pool size values are positive
   - `rpc.http_addr` and `metrics.addr` parse as valid `SocketAddr`
   - Warn (but do not fail) if `consensus.participants` is empty (solo mode)

10. **Call `config.validate()` at startup** in `cli.rs` before constructing `ProductionRunner`. Return a clear error with the specific invalid field.

### Phase 4: Environment variable overrides

11. **Add env var support** following the existing `KORA_RUNTIME_DIR` pattern. Suggested mapping:
    - `KORA_RPC_ADDR` -> `rpc.http_addr`
    - `KORA_METRICS_ADDR` -> `metrics.addr`
    - `KORA_GAS_LIMIT` -> `execution.gas_limit`
    - `KORA_BASE_FEE` -> `execution.base_fee_per_gas`
    - `KORA_CONSENSUS_LEADER_TIMEOUT_MS` -> `consensus.leader_timeout_ms`
    - etc.

   Env vars should override config file values, following the standard precedence: defaults < config file < env vars < CLI flags.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/config/src/consensus.rs` | Add timeout fields with serde defaults |
| `crates/node/config/src/execution.rs` | Add `base_fee_per_gas` and block limit fields |
| `crates/node/config/src/node.rs` | Add `pool` and `metrics` config sections; add `validate()` method |
| `crates/node/config/src/lib.rs` | Export new config types |
| `crates/node/txpool/src/config.rs` | Add `Serialize`/`Deserialize` derives to `PoolConfig` |
| `crates/node/runner/src/runner.rs` | Remove `const` declarations; read values from config; call `config.validate()` |
| `bin/kora/src/cli.rs` | Read RPC and metrics addresses from config instead of hardcoding |
| `crates/node/rpc/src/server.rs` | No changes needed (already accepts `SocketAddr` as a parameter) |

---

## Testing

### Unit tests

- **Config validation rejects `gas_limit = 0`:** Construct a `NodeConfig` with `execution.gas_limit = 0`, call `validate()`, assert error with clear message.
- **Config validation rejects `threshold > participants.len()`:** Construct with `threshold = 5` and 3 participants, assert error.
- **Config validation rejects `threshold = 0`:** Assert error.
- **Config validation accepts valid config:** Construct a well-formed config, assert `validate()` returns `Ok`.
- **Serde roundtrip with new fields:** Serialize a `ConsensusConfig` with timeout values to TOML/JSON, deserialize, assert equality.
- **Defaults match current hardcoded values:** Assert that `ConsensusConfig::default().leader_timeout_ms == 2000`, etc. This ensures the migration does not accidentally change behavior.

### Integration tests

- **Config file overrides defaults:** Write a TOML file with `consensus.leader_timeout_ms = 5000`, load via `NodeConfig::load()`, assert the value is 5000 and all other fields are defaults.
- **Env vars override config file:** Set `KORA_GAS_LIMIT=500000000`, load a config file with `gas_limit = 250000000`, assert the final value is 500000000.
- **RPC binds to configured address:** Start a node with `rpc.http_addr = "127.0.0.1:9999"`, assert the RPC server is listening on port 9999.
- **Metrics binds to configured address:** Start a node with `metrics.addr = "127.0.0.1:9999"`, assert the metrics endpoint is reachable on that port.

### Backward compatibility

- **Default config produces identical behavior:** A node started with no config file (all defaults) must behave identically to the current hardcoded behavior. This is the most important test -- the migration must be transparent to existing deployments.

---

## Additional Context

The `KORA_RUNTIME_DIR` environment variable (added in commit `a3b6768`) already establishes the pattern for runtime-configurable paths. This issue proposes extending that approach to the rest of the hardcoded parameters.

The `PoolConfig` struct already has a complete builder API (`with_max_pending_txs()`, `with_max_queued_txs()`, etc.) and thorough unit tests. The only gap is that it does not derive `Serialize`/`Deserialize` and is not reachable from `NodeConfig`.

The `RpcConfig` struct already defines `http_addr` with a default of `"0.0.0.0:8545"`. The CLI just needs to read `config.rpc.http_addr` instead of hardcoding the same value. This is likely a one-line change.
