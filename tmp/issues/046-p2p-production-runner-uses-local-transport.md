# 046: Production Runner Always Uses Local Transport Config With Private IPs Enabled

**Category:** bug / security
**Severity:** medium

**Labels:** `bug`, `security`, `p2p`, `config`

## Summary

The `ProductionRunner::run_standalone()` method, which is the primary entry point for running a validator node, always calls `build_local_transport()` instead of `build_transport()`. The "local" variant uses `discovery::Config::local()` and explicitly enables `allow_private_ips = true`. The production-grade `build_transport()` method (which uses `discovery::Config::recommended()`) is defined in the codebase but is never called from any validator startup path.

## Problem

The `run_standalone()` method at `crates/node/runner/src/runner.rs:889-937` is the main entry point for starting a validator. On line 910-911, it calls `build_local_transport()`:

```rust
pub fn run_standalone(self, config: kora_config::NodeConfig) -> Result<(), RunnerError> {
    // ...
    executor.start(|context| async move {
        let validator_key = config
            .validator_key()
            .map_err(|e| anyhow::anyhow!("failed to load validator key: {}", e))?;

        let transport = config
            .network
            .build_local_transport(validator_key, context.child("transport"))
            .map_err(|e| anyhow::anyhow!("failed to build transport: {}", e))?;
        // ...
    })
}
```

The `build_local_transport()` implementation at `crates/network/transport/src/ext.rs:42-63` explicitly enables private IPs:

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
    .with_allow_private_ips(true);   // <-- Always allows private IPs

    Ok(transport_config.build(context))
}
```

Meanwhile, the production transport builder at `crates/network/transport/src/ext.rs:65-86` uses `TransportConfig::recommended()` and does not enable private IPs, but this method is never called:

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

## Impact

For a production deployment on the public internet:

1. **Private IP address acceptance**: Peers can advertise private IP addresses (10.x.x.x, 192.168.x.x, 172.16-31.x.x) as their dialable addresses. An attacker could register as a peer with a private IP, causing other validators to waste connection attempts to unreachable addresses or to connect to unintended hosts on the local network.

2. **Discovery behavior**: The `local` discovery config (from commonware's `discovery::Config::local()`) typically has shorter timeouts and more aggressive re-discovery intervals, increasing CPU and network overhead compared to production-tuned settings in `discovery::Config::recommended()`.

3. **Security posture**: The "local" configuration was designed for development environments where all nodes are on the same machine or LAN. Using it in production means the node's network layer operates with development-grade security settings.

Note: The current devnet deployment uses Docker containers on a single host, so `allow_private_ips = true` is required there. However, this setting should not be the default for the `run_standalone()` path used in real deployments.

## Root Cause

The production transport builder (`build_transport`) was created but never wired into the main startup path. Only the local/devnet builder is used because initial deployments have all been devnet environments on a single host.

## Suggested Fix

**Option 1 (recommended):** Add a `--network-mode production|local` CLI flag (defaulting to `production` for the `validator` subcommand). Use `build_transport()` for production mode and `build_local_transport()` for local mode:

```rust
let transport = if config.network.is_local_mode() {
    config.network.build_local_transport(validator_key, context.child("transport"))
} else {
    config.network.build_transport(validator_key, context.child("transport"))
}
.map_err(|e| anyhow::anyhow!("failed to build transport: {}", e))?;
```

**Option 2:** Make `allow_private_ips` configurable via `NetworkConfig` in the TOML config file:

```toml
[network]
allow_private_ips = false  # default for production
```

Then apply it in both transport builders rather than hardcoding `true` in `build_local_transport`.

**Option 3:** At minimum, detect the environment automatically -- if `dialable_addr` is a private IP, use local config; otherwise use production config:

```rust
let is_private = listen_addr.ip().is_private() || listen_addr.ip().is_loopback();
let transport_config = if is_private {
    TransportConfig::local(...)
} else {
    TransportConfig::recommended(...)
};
```

## Files to Modify

- `crates/node/runner/src/runner.rs` -- `run_standalone()` at line 909-912 (change from `build_local_transport` to conditional)
- `crates/network/transport/src/ext.rs` -- Both `build_local_transport()` (line 42) and `build_transport()` (line 65)
- `crates/node/config/src/network.rs` -- Add `allow_private_ips` field to `NetworkConfig`

## Related Issues

- `045-p2p-uniform-rate-quota-all-channels.md` -- Rate quota configuration also lacks production vs. local distinction
