# RPC: Rate limiting config exists but is never applied to servers

**Severity:** Medium (devnet), High (production)
**Component:** `kora-rpc`
**Labels:** `bug`, `rpc`, `security`, `denial-of-service`

## Summary

The RPC crate defines a complete `RateLimitConfig` struct with sensible defaults (100 requests/second, burst size 200), stores it in `RpcServerConfig`, and even provides a builder method `with_rate_limit()`. However, neither the `RpcServer` nor the `JsonRpcServer` ever reads or applies this configuration. The `from_config` constructor copies `cors` and `max_connections` from the config but silently drops `rate_limit`. No Tower `RateLimitLayer` or jsonrpsee rate-limiting middleware is constructed anywhere in the server startup path. The rate limiting infrastructure is dead code.

## Background

Kora is an EVM-compatible blockchain built on Commonware consensus primitives. Each validator node runs an RPC server (implemented in `kora-rpc`) that exposes Ethereum-standard JSON-RPC methods (`eth_*`, `net_*`, `web3_*`) plus Kora-specific methods (`kora_*`). The RPC server is the primary external interface -- wallets, dApps, block explorers, and the load generator (`bin/loadgen`) all interact with the chain through this endpoint.

The RPC architecture consists of two server components:

1. **HTTP status server** (Axum) -- Serves `/status` and `/health` endpoints for monitoring and health checks. Binds to `http_addr` (typically the JSON-RPC port + 1).
2. **JSON-RPC server** (jsonrpsee) -- Serves the full Ethereum JSON-RPC API. Binds to `jsonrpc_addr` (typically port 8545).

Both are started concurrently in `RpcServer::start()` via `tokio::spawn`. The production runner instantiates the RPC server in `crates/node/runner/src/runner.rs` at line 432 using `RpcServer::with_state_provider()`, bypassing `from_config` entirely and never setting any rate limit.

### What IS enforced

`max_connections` is correctly wired for both servers. The HTTP server uses a `ConcurrencyLimitLayer` from Tower:

```rust
// crates/node/rpc/src/server.rs, line 219
.layer(ConcurrencyLimitLayer::new(max_connections as usize))
```

The JSON-RPC server uses jsonrpsee's built-in connection limit:

```rust
// crates/node/rpc/src/server.rs, line 239
let server = match Server::builder()
    .max_connections(max_connections)
    .build(jsonrpc_addr)
    .await
```

These cap the number of **concurrent connections** but do NOT limit the **request rate per connection**. A single persistent connection can issue unlimited JSON-RPC calls per second.

## The Dead Code

### RateLimitConfig definition

In `crates/node/rpc/src/config.rs`, lines 130-150:

```rust
/// Rate limiting configuration.
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    /// Maximum requests per second per client.
    pub requests_per_second: u64,
    /// Burst size for rate limiting.
    pub burst_size: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self { requests_per_second: 100, burst_size: 200 }
    }
}

impl RateLimitConfig {
    /// Disable rate limiting.
    pub const fn disabled() -> Self {
        Self { requests_per_second: u64::MAX, burst_size: u64::MAX }
    }
}
```

This is a well-designed config: 100 requests per second with a burst allowance of 200 covers normal dApp usage while preventing abuse. The `disabled()` constructor provides an escape hatch for testing. The struct is stored in `RpcServerConfig` at line 17:

```rust
pub struct RpcServerConfig {
    // ...
    /// Rate limiting configuration.
    pub rate_limit: RateLimitConfig,
    /// Maximum number of concurrent connections.
    pub max_connections: u32,
}
```

### Builder method exists but has no effect

In `crates/node/rpc/src/config.rs`, lines 48-52:

```rust
/// Set rate limit.
#[must_use]
pub const fn with_rate_limit(mut self, requests_per_second: u64) -> Self {
    self.rate_limit.requests_per_second = requests_per_second;
    self
}
```

This modifies a config field that is never read by any server constructor. The README (`crates/node/rpc/README.md`, line 102) even documents the builder:

```rust
.with_rate_limit(100)  // requests per second
```

A user following the documentation would believe rate limiting is active.

### from_config drops rate_limit

In `crates/node/rpc/src/server.rs`, lines 185-197, the `from_config` constructor copies every field from `RpcServerConfig` except `rate_limit`:

```rust
pub fn from_config(state: NodeState, config: RpcServerConfig, state_provider: S) -> Self {
    Self {
        state,
        http_addr: config.http_addr,
        jsonrpc_addr: config.jsonrpc_addr,
        chain_id: config.chain_id,
        tx_submit: None,
        state_provider,
        cors_config: config.cors,           // <-- copied
        max_connections: config.max_connections, // <-- copied
        peer_count: 0,
        // rate_limit: NOT copied, NOT stored
    }
}
```

The `RpcServer` struct itself (lines 72-82) has no `rate_limit` field:

```rust
pub struct RpcServer<S: StateProvider = NoopStateProvider> {
    state: NodeState,
    http_addr: SocketAddr,
    jsonrpc_addr: SocketAddr,
    chain_id: u64,
    tx_submit: Option<TxSubmitCallback>,
    state_provider: S,
    cors_config: CorsConfig,
    max_connections: u32,          // <-- stored and used
    peer_count: u64,
    // no rate_limit field
}
```

### start() constructs no rate-limiting middleware

In `crates/node/rpc/src/server.rs`, lines 202-285, the `start()` method builds both servers. Neither server pipeline includes any rate-limiting layer:

**HTTP server (lines 214-235):**
```rust
let app = Router::new()
    .route("/status", get(status_handler))
    .route("/health", get(health_handler))
    .layer(cors_layer)                                      // CORS: yes
    .layer(ConcurrencyLimitLayer::new(max_connections as usize))  // Connection limit: yes
    .with_state(node_state);                                // Rate limit: NO
```

**JSON-RPC server (lines 237-282):**
```rust
let server = match Server::builder()
    .max_connections(max_connections)   // Connection limit: yes
    // .set_rpc_middleware(...)         // Rate limit: NO
    .build(jsonrpc_addr)
    .await
```

The `tower` dependency in `Cargo.toml` has the `"limit"` feature enabled (line 16), which provides `tower::limit::RateLimitLayer`. This layer is available but never imported or used -- only `ConcurrencyLimitLayer` is imported from it (line 7).

### JsonRpcServer has the same gap

The standalone `JsonRpcServer` (lines 323-415) also lacks rate limiting. It stores `max_connections` but has no `rate_limit` field, no `with_rate_limit` builder method, and its `start()` method (line 391) constructs no rate-limiting middleware.

### Production runner bypasses config entirely

In `crates/node/runner/src/runner.rs`, lines 432-440, the production runner creates the RPC server directly via `with_state_provider()` rather than `from_config()`:

```rust
let rpc = kora_rpc::RpcServer::with_state_provider(
    node_state.clone(),
    *addr,
    self.chain_id,
    indexed_provider,
)
.with_tx_submit(tx_submit)
.with_peer_count(self.scheme.participants().len().saturating_sub(1) as u64);
drop(rpc.start());
```

No `.with_rate_limit()` call, no `RpcServerConfig`, no rate limiting of any kind. Even if `from_config` were fixed to propagate the rate limit, the production code path does not use it.

## Impact

### Denial-of-service via RPC saturation

A single client maintaining one persistent HTTP/WebSocket connection can send unlimited JSON-RPC requests per second. Under adversarial conditions:

1. **CPU saturation** -- Methods like `eth_call`, `eth_estimateGas`, and `eth_getBlockByNumber` execute EVM code or query state. At high call rates, these saturate the CPU on the RPC-handling Tokio runtime.

2. **State lock contention** -- The `StateProvider` implementation (`IndexedStateProvider`) holds locks on QMDB state and the indexer database. High-frequency `eth_getBalance`, `eth_getStorageAt`, and `eth_getTransactionReceipt` calls create read lock contention that can block write paths used by consensus.

3. **Consensus degradation on validators** -- In the current Kora devnet architecture, validators run their RPC server on the same process as the consensus engine. An RPC flood steals Tokio task budget from consensus-critical tasks (vote handling, block execution, certificate propagation). This can cause leader timeouts, increased nullification rates, and chain stalls.

4. **Amplification via multi-validator targeting** -- The devnet exposes 4 RPC endpoints (ports 8545-8548). An attacker can target all 4 simultaneously, degrading consensus across the entire validator set.

### Distinction from max_connections

`max_connections` limits how many clients can connect simultaneously. It does NOT limit how fast a connected client can issue requests. With `max_connections: 100` and no rate limit:

```
Scenario A (rate limited):    100 clients * 100 req/s = 10,000 req/s max
Scenario B (current, no RL):  100 clients * unlimited req/s = unbounded
```

Even a single client with one connection can issue thousands of requests per second via HTTP pipelining or WebSocket multiplexing.

### Impact on specific RPC methods

| Method | Cost | Risk |
|--------|------|------|
| `eth_call` | EVM execution against state | CPU-bound, blocks executor threads |
| `eth_estimateGas` | Binary search over EVM executions | 10-20x cost of `eth_call` |
| `eth_getBlockByNumber(true)` | Full block with transaction details | Memory allocation for large blocks |
| `eth_sendRawTransaction` | Validation + mempool insertion | Creates lock contention on mempool |
| `eth_getStorageAt` | QMDB read | Read lock on state database |
| `eth_blockNumber` | In-memory read | Low cost individually, but floods log pipeline |

## Why This Happened

The `RateLimitConfig` appears to have been added as part of a config infrastructure pass that also added `CorsConfig` and the builder pattern on `RpcServerConfig`. The CORS configuration was fully wired (`build_cors_layer()` at line 37, `cors_layer` applied at line 218), and `max_connections` was wired to both servers. But the rate limiting was left at the config-definition stage without the corresponding middleware integration.

The `RpcServer` struct was likely written before `RpcServerConfig`, using individual builder methods (`with_cors`, `with_max_connections`) rather than a single config object. When `from_config` was added later, it mapped the fields that had corresponding struct members but had no `rate_limit` field to map to.

## Proposed Fix

### Option A: Tower RateLimitLayer for the HTTP server (minimal)

The `tower` crate's `limit` feature (already enabled in `Cargo.toml`) provides `RateLimitLayer`. Add it to the HTTP server's middleware stack:

```rust
// In crates/node/rpc/src/server.rs

use tower::limit::RateLimitLayer;

// In start(), after building cors_layer:
let rate_limit_layer = RateLimitLayer::new(
    self.rate_limit.requests_per_second as u64,
    Duration::from_secs(1),
);

let app = Router::new()
    .route("/status", get(status_handler))
    .route("/health", get(health_handler))
    .layer(cors_layer)
    .layer(rate_limit_layer)  // <-- add rate limiting
    .layer(ConcurrencyLimitLayer::new(max_connections as usize))
    .with_state(node_state);
```

Note: Tower's `RateLimitLayer` is a global rate limiter (across all connections), not per-client. This is a reasonable starting point but does not provide per-IP fairness.

### Option B: jsonrpsee RPC middleware for the JSON-RPC server (recommended)

jsonrpsee 0.24 supports `RpcServiceBuilder` with Tower-compatible middleware. This allows rate limiting directly on the JSON-RPC layer:

```rust
use jsonrpsee::server::middleware::rpc::RpcServiceBuilder;

let rpc_middleware = RpcServiceBuilder::new()
    .layer_fn(|service| RateLimitedRpcService::new(service, rate_config));

let server = Server::builder()
    .max_connections(max_connections)
    .set_rpc_middleware(rpc_middleware)
    .build(jsonrpc_addr)
    .await;
```

This approach applies rate limiting at the RPC method call level, which is more precise than HTTP-level limiting (one HTTP request can batch multiple JSON-RPC calls).

### Option C: Per-IP rate limiting with Governor (production-grade)

For production, per-IP rate limiting prevents a single abusive client from consuming the global rate budget. The `tower-governor` crate or a custom middleware using the `governor` crate (which implements the GCRA algorithm used by `RateLimitConfig`'s `burst_size` field) provides this:

```rust
use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;

let quota = Quota::per_second(NonZeroU32::new(config.requests_per_second as u32).unwrap())
    .allow_burst(NonZeroU32::new(config.burst_size as u32).unwrap());
```

This would require adding `governor` or `tower-governor` as a dependency.

### Struct changes required

Regardless of which option is chosen, the `RpcServer` struct needs a `rate_limit` field:

```rust
pub struct RpcServer<S: StateProvider = NoopStateProvider> {
    // ... existing fields ...
    rate_limit: RateLimitConfig,  // <-- add this
}
```

And `from_config` needs to propagate it:

```rust
pub fn from_config(state: NodeState, config: RpcServerConfig, state_provider: S) -> Self {
    Self {
        // ... existing fields ...
        cors_config: config.cors,
        max_connections: config.max_connections,
        rate_limit: config.rate_limit,  // <-- add this
        peer_count: 0,
    }
}
```

A `with_rate_limit` builder method should be added to `RpcServer` (not just `RpcServerConfig`):

```rust
#[must_use]
pub fn with_rate_limit(mut self, config: RateLimitConfig) -> Self {
    self.rate_limit = config;
    self
}
```

### Per-IP vs global rate limiting

The existing `RateLimitConfig` has two fields: `requests_per_second` and `burst_size`. Consider adding a `scope` field to `RateLimitConfig` in `crates/node/rpc/src/config.rs`:

```rust
pub struct RateLimitConfig {
    pub requests_per_second: u64,
    pub burst_size: u64,
    pub scope: RateLimitScope,
}

pub enum RateLimitScope {
    /// Single global rate limit shared across all clients.
    Global,
    /// Per-IP rate limit (each source IP gets its own bucket).
    PerIp,
    /// Per-connection rate limit.
    PerConnection,
}
```

For the devnet, `Global` is sufficient. For production with public-facing RPC, `PerIp` prevents a single client from monopolizing the rate budget. jsonrpsee's built-in `set_rpc_middleware` supports per-connection middleware naturally.

## Files to Modify

| File | Change | Key Lines |
|------|--------|-----------|
| `crates/node/rpc/src/server.rs` | Add `rate_limit` field to `RpcServer` struct; propagate in `from_config`; add `with_rate_limit` builder; apply `RateLimitLayer` or jsonrpsee middleware in `start()` | 72-82 (struct), 185-197 (from_config), 202-285 (start) |
| `crates/node/rpc/src/server.rs` | Apply rate limiting to `JsonRpcServer` as well | 323-415 |
| `crates/node/rpc/src/config.rs` | Optionally add `RateLimitScope` enum; ensure `burst_size` is used (not just `requests_per_second`) | 130-150 |
| `crates/node/runner/src/runner.rs` | Pass rate limit config when constructing the RPC server, either via `from_config` or `with_rate_limit` builder | 432-440 |
| `crates/node/rpc/Cargo.toml` | Possibly add `governor` or `tower-governor` dependency for per-IP rate limiting | 16-17 |

## Testing Plan

### 1. Unit test: config propagation

Verify that `RateLimitConfig` flows from `RpcServerConfig` through `from_config` to the `RpcServer` struct:

```rust
#[test]
fn from_config_propagates_rate_limit() {
    let config = RpcServerConfig::default().with_rate_limit(500);
    let server = RpcServer::from_config(
        NodeState::new("test".to_string()),
        config,
        NoopStateProvider,
    );
    assert_eq!(server.rate_limit.requests_per_second, 500);
    assert_eq!(server.rate_limit.burst_size, 200);
}
```

### 2. Integration test: rate limit enforcement

Start a `JsonRpcServer` with a low rate limit (e.g., 5 req/s) and send requests exceeding the limit:

```rust
#[tokio::test]
async fn rate_limit_returns_429() {
    let server = JsonRpcServer::with_state_provider(addr, 1, NoopStateProvider)
        .with_rate_limit(RateLimitConfig { requests_per_second: 5, burst_size: 5 })
        .start()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let body = r#"{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}"#;

    // Send 10 requests rapidly
    let mut rejected = 0;
    for _ in 0..10 {
        let resp = client.post(&format!("http://{}", addr))
            .body(body)
            .send()
            .await
            .unwrap();
        if resp.status() == 429 {
            rejected += 1;
        }
    }

    assert!(rejected > 0, "Some requests should have been rate-limited");
}
```

### 3. Load test: RPC performance under rate limiting

Using the existing load generator (`bin/loadgen`), verify that the chain remains healthy when RPC rate limiting is active:

```bash
# Start devnet with rate limiting enabled (default 100 req/s)
just trusted-devnet

# Run loadgen at moderate rate (should succeed)
cargo run --release --bin loadgen -- \
  --total-txs 1000 --accounts 5 --concurrency 10 \
  --rpc-url http://127.0.0.1:8545

# Run loadgen at high rate (should see some 429s but chain stays healthy)
cargo run --release --bin loadgen -- \
  --total-txs 10000 --accounts 20 --concurrency 200 \
  --rpc-url http://127.0.0.1:8545
```

Expected: The chain does not stall under high RPC load. Some loadgen requests receive 429 responses, but the overall success rate for submitted transactions remains high because rate limiting prevents CPU saturation.

### 4. Verify max_connections still works

Ensure the existing `max_connections` enforcement is not broken by the addition of rate limiting:

```bash
# Open 101 simultaneous connections to a server with max_connections=100
# The 101st should be rejected
```

### 5. Verify disabled rate limiting

```rust
#[test]
fn disabled_rate_limit_allows_unlimited() {
    let config = RateLimitConfig::disabled();
    // requests_per_second = u64::MAX should effectively disable limiting
    assert_eq!(config.requests_per_second, u64::MAX);
    assert_eq!(config.burst_size, u64::MAX);
}
```

## Verification Steps

After implementing the fix:

```bash
# 1. Verify rate limit config is propagated
cargo test -p kora-rpc -- rate_limit

# 2. Verify no regressions in existing tests
cargo test -p kora-rpc

# 3. Verify the RPC server starts with rate limiting
cd docker && just trusted-devnet
sleep 10

# 4. Check that normal requests succeed
curl -s http://127.0.0.1:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
# Expected: {"jsonrpc":"2.0","result":"0x...","id":1}

# 5. Flood the RPC and verify rate limiting kicks in
for i in $(seq 1 500); do
  curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8545 -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' &
done | sort | uniq -c
# Expected: mix of 200 and 429 responses

# 6. Verify the chain is still healthy after the flood
curl -s http://127.0.0.1:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
# Expected: block number continues advancing
```

## Relationship to Other Issues

- **Issue #15 (Docker production readiness):** Production deployments with public-facing RPC are especially vulnerable without rate limiting. Rate limiting is a prerequisite for exposing RPC endpoints to the internet.
- **Issue #16 (P2P connection health monitoring):** Without rate limiting, an RPC flood can degrade P2P message handling (both share the same Tokio runtime), making P2P health metrics report false degradation that is actually RPC-induced.
- **Issue #17 (Nonce validation gap):** High-rate RPC submissions exacerbate the nonce validation gap by creating more concurrent `eth_sendRawTransaction` calls that race through the validation-to-mempool path.

## Related Files

| File | Description |
|------|-------------|
| `crates/node/rpc/src/config.rs` | `RateLimitConfig` definition (L130-150), `RpcServerConfig` with `rate_limit` field (L17), `with_rate_limit` builder (L48-52), defaults (L139-142) |
| `crates/node/rpc/src/server.rs` | `RpcServer` struct missing `rate_limit` field (L72-82), `from_config` dropping `rate_limit` (L185-197), `start()` with no rate-limiting middleware (L202-285), `JsonRpcServer` also missing rate limiting (L323-415) |
| `crates/node/rpc/Cargo.toml` | `tower` dependency with `"limit"` feature already enabled (L16), provides `RateLimitLayer` but only `ConcurrencyLimitLayer` is used |
| `crates/node/rpc/README.md` | Documents `with_rate_limit(100)` as if it works (L102) |
| `crates/node/runner/src/runner.rs` | Production RPC server construction bypasses `from_config`, no rate limit set (L432-440) |
