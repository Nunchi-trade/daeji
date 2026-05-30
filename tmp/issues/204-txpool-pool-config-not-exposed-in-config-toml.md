# PoolConfig Not Exposed via config.toml -- All Operators Get Hardcoded Defaults

**Category**: enhancement -- txpool, config
**Severity**: high

## Summary

The `PoolConfig` struct defines configurable parameters for the transaction pool (max pool sizes, gas price floor, TTL values, transaction size limits), but none of these parameters are exposed in the node's `config.toml` or CLI flags. The runner always constructs `PoolConfig::default()`, meaning all deployments -- from local devnets to production validators -- use identical hardcoded pool parameters regardless of scale or requirements.

## Problem

The `PoolConfig` struct has a full builder API (`with_max_pending_txs()`, `with_min_gas_price()`, etc.) and sensible defaults, but this API is never used from the node configuration layer. The runner constructs `PoolConfig::default()` at both transaction validation call sites.

The `NodeConfig` struct (the top-level deserialized configuration) has no `[txpool]` section.

## Code Reference

**Runner always uses defaults** -- File: `crates/node/runner/src/runner.rs`, lines 1113-1118 (gossip handler path):

```rust
let current_state = gossip_ledger.latest_state().await;
let validator = TransactionValidator::new(
    gossip_chain_id,
    current_state,
    PoolConfig::default(),   // <-- hardcoded default
)
.with_pool(gossip_pool.clone());
```

File: `crates/node/runner/src/runner.rs`, lines 1212-1214 (RPC handler path):

```rust
let validator =
    TransactionValidator::new(chain_id, state, PoolConfig::default())  // <-- hardcoded default
        .with_pool(pool);
```

**PoolConfig defaults** -- File: `crates/node/txpool/src/config.rs`, lines 24-37:

```rust
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_pending_txs: 4096,
            max_queued_txs: 1024,
            max_txs_per_sender: 256,
            max_tx_size: 128 * 1024,      // 128 KB
            min_gas_price: 1_000_000_000, // 1 gwei
            replacement_bump_percent: 10,
            pending_ttl_secs: 30 * 60,    // 30 minutes
            queued_ttl_secs: 60 * 60,     // 1 hour
        }
    }
}
```

**NodeConfig has no txpool section** -- File: `crates/node/config/src/node.rs`, lines 20-52:

```rust
pub struct NodeConfig {
    pub chain_id: u64,
    pub data_dir: PathBuf,
    pub worker_threads: usize,
    pub consensus: ConsensusConfig,
    pub network: NetworkConfig,
    pub execution: ExecutionConfig,
    pub rpc: RpcConfig,
    // No txpool field
}
```

## Impact

Operators cannot tune pool behavior for their deployment:

1. **Cannot increase `max_pending_txs`** (default: 4,096) for high-throughput scenarios where the mempool needs to hold more transactions.
2. **Cannot adjust `min_gas_price`** (default: 1 gwei) to set a different fee floor for the network.
3. **Cannot change TTL values** (`pending_ttl_secs`: 30 min, `queued_ttl_secs`: 1 hr) for networks with different block times or finality characteristics.
4. **Cannot adjust `max_tx_size`** (default: 128 KB) for networks that require larger transactions (e.g., large contract deployments).
5. **Cannot tune `max_txs_per_sender`** (default: 256) for deployments where a single sender needs more or fewer concurrent transactions.
6. **Cannot set `replacement_bump_percent`** (default: 10%) for different replacement pricing policies.
7. **All devnet and production deployments use identical pool parameters** regardless of scale, hardware, or network conditions.

## Root Cause

The pool configuration system was built with a complete builder API but never wired into the node configuration deserialization (`NodeConfig`) or CLI argument parsing.

## Suggested Fix

1. Add a `[txpool]` section to `NodeConfig`:

```rust
// In crates/node/config/src/node.rs
pub struct NodeConfig {
    // ... existing fields ...
    #[serde(default)]
    pub txpool: TxPoolConfig,
}
```

2. Create a `TxPoolConfig` that maps to `PoolConfig`:

```toml
[txpool]
max_pending_txs = 8192
max_queued_txs = 2048
max_txs_per_sender = 256
min_gas_price = 1000000000  # 1 gwei
max_tx_size = 131072        # 128 KB
pending_ttl_secs = 1800     # 30 minutes
queued_ttl_secs = 3600      # 1 hour
replacement_bump_percent = 10
```

3. Thread the config through to the runner:

```rust
let pool_config = config.txpool.into_pool_config();
TransactionValidator::new(chain_id, state, pool_config)
```

## Files to Modify

- `crates/node/config/src/node.rs` -- Add `txpool: TxPoolConfig` field to `NodeConfig`
- `crates/node/config/src/lib.rs` -- Add `TxPoolConfig` struct with serde deserialization
- `crates/node/runner/src/runner.rs` -- Replace `PoolConfig::default()` with the deserialized config (lines 1116 and 1213)

## Related Issues

- `199-txpool-no-block-gas-limit-validation.md` -- The `block_gas_limit` should also be added to `PoolConfig` as part of this effort

## Labels

enhancement, config, txpool, good first issue
