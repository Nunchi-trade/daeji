# P2P: Both Production Code Paths Use Local/Devnet Transport Instead of Production Transport

**Category**: bug
**Severity**: medium

## Summary

Both production entry points (`ProductionRunner::run_standalone()` and `LegacyNodeService::run_with_context()`) use `build_local_transport()` which enables `allow_private_ips(true)` and uses faster/more lenient discovery settings intended for local testing. The `build_transport()` method, which uses `discovery::Config::recommended()` with conservative production-suitable settings, is defined but never called from any code path.

## Problem

Kora's transport layer provides two builder methods on `NetworkConfig`:

1. **`build_local_transport()`** -- Uses `TransportConfig::local()` with `allow_private_ips(true)`. Designed for local development and devnet testing with faster discovery and more lenient connection settings.

2. **`build_transport()`** -- Uses `TransportConfig::recommended()` with production-grade conservative settings suitable for deployments on the public internet. Does NOT set `allow_private_ips(true)`.

Both production startup paths use `build_local_transport()`:

**`ProductionRunner::run_standalone()`** at `crates/node/runner/src/runner.rs`, line 911:
```rust
let transport = config
    .network
    .build_local_transport(validator_key, context.child("transport"))
```

**`LegacyNodeService::run_with_context()`** at `crates/node/service/src/service.rs`, line 114:
```rust
let mut transport = self
    .config
    .network
    .build_local_transport(validator_key, context)
```

The `build_transport()` method is defined at `crates/network/transport/src/ext.rs`, lines 65-86, but is never invoked from anywhere in the codebase.

## Code Reference

**Local transport builder (used by both paths)** -- `crates/network/transport/src/ext.rs:42-63`:
```rust
fn build_local_transport<E>(
    &self,
    crypto: ed25519::PrivateKey,
    context: E,
) -> Result<NetworkTransport<ed25519::PublicKey, E>, TransportError>
where
    E: Spawner + BufferPooler + Clock + CryptoRngCore + RNetwork + Resolver + Metrics,
{
    let (listen_addr, dialable, bootstrappers) = parse_network_config(self)?;

    let transport_config = TransportConfig::local(
        crypto,
        DEFAULT_NAMESPACE,
        listen_addr,
        dialable,
        bootstrappers,
        DEFAULT_MAX_MESSAGE_SIZE,
    )
    .with_allow_private_ips(true);  // <-- DEVNET SETTING

    Ok(transport_config.build(context))
}
```

**Production transport builder (never called)** -- `crates/network/transport/src/ext.rs:65-86`:
```rust
fn build_transport<E>(
    &self,
    crypto: ed25519::PrivateKey,
    context: E,
) -> Result<NetworkTransport<ed25519::PublicKey, E>, TransportError>
where
    E: Spawner + BufferPooler + Clock + CryptoRngCore + RNetwork + Resolver + Metrics,
{
    let (listen_addr, dialable, bootstrappers) = parse_network_config(self)?;

    let transport_config = TransportConfig::recommended(
        crypto,
        DEFAULT_NAMESPACE,
        listen_addr,
        dialable,
        bootstrappers,
        DEFAULT_MAX_MESSAGE_SIZE,
    );

    Ok(transport_config.build(context))
}
```

**Production runner usage** -- `crates/node/runner/src/runner.rs:909-912`:
```rust
let transport = config
    .network
    .build_local_transport(validator_key, context.child("transport"))
    .map_err(|e| anyhow::anyhow!("failed to build transport: {}", e))?;
```

**Legacy service usage** -- `crates/node/service/src/service.rs:111-115`:
```rust
let mut transport = self
    .config
    .network
    .build_local_transport(validator_key, context)
    .map_err(|e| eyre::eyre!("failed to build transport: {}", e))?;
```

## Impact

For production deployments on the public internet:

1. **Private IP acceptance**: With `allow_private_ips(true)`, peers can advertise RFC 1918 private IP addresses (e.g., `10.x.x.x`, `192.168.x.x`) as dialable addresses. The node will attempt to connect to these unreachable addresses, wasting connection slots and potentially leaking information about the node's network environment.

2. **Aggressive discovery**: The `local()` discovery config uses shorter timeouts, more aggressive retry intervals, and more lenient connection acceptance. This increases CPU and network overhead compared to the `recommended()` config, and may cause instability under adverse network conditions.

3. **No safe production path**: Since both entry points hardcode `build_local_transport()`, there is no way to run Kora with production transport settings without modifying the source code.

4. **Current mitigation**: For the Docker-based devnet deployment, this is actually the correct behavior (nodes run on a private Docker network). The issue only becomes relevant for public internet deployments.

## Root Cause

The `build_transport()` production method was created alongside `build_local_transport()` but was never wired into either startup path. All deployments to date have been devnet environments where the local transport config is appropriate, so the production path was never needed and the oversight was not discovered.

## Suggested Fix

1. **Add a configuration option** to `NetworkConfig` to select between local and production transport:

```rust
// In kora_config::NetworkConfig:
/// Use local/devnet transport settings (allow_private_ips, faster discovery).
/// Default: true for backward compatibility. Set to false for production.
pub local_mode: bool,
```

2. **Update both startup paths** to select the transport builder based on configuration:

```rust
// In runner.rs:
let transport = if config.network.local_mode {
    config.network.build_local_transport(validator_key, context.child("transport"))
} else {
    config.network.build_transport(validator_key, context.child("transport"))
}?;
```

3. **Make `allow_private_ips` independently configurable** so operators can control it regardless of the discovery config preset.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- line 911, change to use config-driven transport selection
- `crates/node/service/src/service.rs` -- line 114, same change
- `crates/node/config/src/node.rs` -- add `local_mode` field to `NetworkConfig`
- `crates/network/transport/src/ext.rs` -- no changes needed (both builders already exist)

## Related Issues

- `046-p2p-production-runner-uses-local-transport.md` -- original finding covering only the runner path; this extends the scope to include `service.rs`

## Labels

`bug`, `p2p`, `config`, `security`
