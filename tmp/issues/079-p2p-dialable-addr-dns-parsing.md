# 079: dialable_addr DNS Parsing Broken in Transport Config Builder

**Category:** p2p
**Severity:** high

## Summary

The `parse_network_config()` function in the P2P transport layer parses the `dialable_addr` configuration value using `SocketAddr::parse()`, which only accepts IP:port formats and rejects DNS hostnames. DNS hostnames like Docker container names (`kora-node0:30303`) or Kubernetes service names (`validator-0.validators.default.svc:30303`) fail to parse and return an error. A correct DNS-aware parser (`TransportParsing::parse_ingress()`) already exists in the same crate but is not used by `parse_network_config()`.

## Problem

Kora is a minimal Ethereum-compatible execution client that uses Commonware's P2P transport for inter-node communication. The `dialable_addr` configuration field specifies the address that a node advertises to other nodes for incoming connections. In containerized environments (Docker, Kubernetes), nodes typically discover each other by DNS hostname rather than IP address.

The `parse_network_config()` function handles `dialable_addr` parsing:

**File:** `crates/network/transport/src/ext.rs`, lines 100-107

```rust
let dialable = if let Some(ref dialable_addr) = config.dialable_addr {
    let addr: SocketAddr = dialable_addr
        .parse()
        .map_err(|_| TransportError::InvalidListenAddr(dialable_addr.clone()))?;
    Ingress::Socket(addr)
} else {
    Ingress::Socket(listen_addr)
};
```

`SocketAddr::parse()` only accepts `IP:port` format (e.g., `192.168.1.1:30303` or `[::1]:30303`). It does not accept DNS hostnames like `kora-node0:30303`.

Meanwhile, the same crate already has a DNS-aware parser:

**File:** `crates/network/transport/src/config.rs`, line 184

```rust
pub fn parse_ingress(addr_str: &str) -> Result<Ingress, TransportError> {
```

This function properly handles both IP addresses (returning `Ingress::Socket`) and DNS hostnames (returning `Ingress::Dns`). It is already used for parsing bootstrapper addresses but is not used for `dialable_addr`.

The `parse_network_config()` function is called from both `build_local_transport()` and `build_transport()` (the production code path), so DNS dialable addresses are broken in all deployment modes.

## Code Reference

**File:** `crates/network/transport/src/ext.rs:100-107`
```rust
let dialable = if let Some(ref dialable_addr) = config.dialable_addr {
    let addr: SocketAddr = dialable_addr
        .parse()
        .map_err(|_| TransportError::InvalidListenAddr(dialable_addr.clone()))?;
    Ingress::Socket(addr)
} else {
    Ingress::Socket(listen_addr)
};
```

**File:** `crates/network/transport/src/config.rs:184` (the correct parser that already exists)
```rust
pub fn parse_ingress(addr_str: &str) -> Result<Ingress, TransportError> {
    // Handles both SocketAddr and DNS hostnames
    // Returns Ingress::Socket for IPs, Ingress::Dns for hostnames
}
```

## Impact

- **Docker deployments:** Nodes using container hostnames as `dialable_addr` (e.g., `kora-node0:30303`) will fail to start with `TransportError::InvalidListenAddr`. The current Docker entrypoint works around this by not setting `dialable_addr` or by resolving hostnames to IPs before writing config, but this is fragile and non-obvious.
- **Kubernetes deployments:** Standard Kubernetes service discovery uses DNS names (e.g., `pod-name.service.namespace.svc:port`). These cannot be used as `dialable_addr`.
- **Cloud deployments:** Any environment where nodes discover each other by hostname rather than IP is affected.
- **The workaround is non-obvious:** Operators must resolve DNS to IP before writing configuration, adding complexity and breaking when IPs change (e.g., pod restarts in Kubernetes).

## Root Cause

`parse_network_config()` was written to only handle `SocketAddr` parsing for `dialable_addr`. The more capable `TransportParsing::parse_ingress()` function was added separately (it is used for bootstrapper parsing on line 232 of `config.rs`) but `parse_network_config()` was never updated to use it.

## Suggested Fix

Replace the `SocketAddr`-only parsing with `TransportParsing::parse_ingress()`:

```rust
// Before:
let dialable = if let Some(ref dialable_addr) = config.dialable_addr {
    let addr: SocketAddr = dialable_addr
        .parse()
        .map_err(|_| TransportError::InvalidListenAddr(dialable_addr.clone()))?;
    Ingress::Socket(addr)
} else {
    Ingress::Socket(listen_addr)
};

// After:
let dialable = if let Some(ref dialable_addr) = config.dialable_addr {
    TransportParsing::parse_ingress(dialable_addr)?
} else {
    Ingress::Socket(listen_addr)
};
```

This is a one-line change that reuses the existing DNS-aware parser. The `parse_ingress()` function already handles both IP addresses and DNS hostnames correctly, and its error type is compatible with the function's return type.

## Files to Modify

- `crates/network/transport/src/ext.rs` (lines 100-107) -- replace `SocketAddr` parsing with `TransportParsing::parse_ingress()`

## Related Issues

None.

## Labels

bug, p2p, docker
