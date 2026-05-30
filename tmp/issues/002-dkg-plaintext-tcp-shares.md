# 002: DKG Ceremony Transmits Secret Key Shares Over Plaintext TCP

**Category:** security / dkg
**Severity:** critical
**Labels:** security, bug, dkg, p2p

---

## Summary

The interactive DKG (Distributed Key Generation) ceremony sends threshold secret shares -- the most sensitive cryptographic material in the system -- over raw, unencrypted, unauthenticated TCP connections. A network attacker who can observe or intercept traffic during the ceremony can reconstruct the full group secret key and forge arbitrary threshold signatures, completely compromising the chain's consensus integrity.

## Problem

The `DkgCeremony` runner at `crates/node/dkg/src/ceremony.rs:70` instantiates `DkgNetwork` (the plaintext TCP transport) for all ceremony communications. The `DkgNetwork::send_to()` method at `crates/node/dkg/src/network.rs:60-98` establishes plain TCP connections without any encryption or authentication:

1. **No encryption**: Messages including secret polynomial shares are sent as plaintext bytes over TCP. Any network observer (man-in-the-middle, compromised switch, cloud provider) can read every DKG share in transit.

2. **No sender authentication**: The sender identity is self-reported by writing the sender's 32-byte Ed25519 public key into the message envelope at line 70 (`self.config.my_public_key().write(&mut envelope)`). The receiver reads this public key at lines 129-141 using `commonware_codec::ReadExt::read()`, which is only deserialization -- there is no cryptographic signature verification. An attacker can impersonate any participant by writing a forged public key.

3. **No integrity protection**: Without authentication or integrity checks, an attacker can inject biased shares to control the resulting group key.

A production-grade `DkgTransport` exists at `crates/node/dkg/src/transport.rs` that uses commonware's authenticated, encrypted P2P channels. The file's own module-level documentation states: "This module provides authenticated, encrypted channels for DKG ceremony messages using the commonware-p2p authenticated discovery network." However, the `DkgCeremony` runner uses `DkgNetwork` (the plaintext one), not `DkgTransport`. The secure transport is defined but never wired into the ceremony.

## Code Reference

**Plaintext TCP connection in `DkgNetwork::send_to()` -- `crates/node/dkg/src/network.rs:60-98`:**

```rust
pub fn send_to(&self, to: &ed25519::PublicKey, msg: &ProtocolMessage) -> Result<(), DkgError> {
    let addr = self
        .peer_addrs
        .get(to)
        .ok_or_else(|| DkgError::Network(format!("Unknown peer: {:?}", to)))?;

    let payload = msg.to_bytes();
    let mut envelope = Vec::new();

    // Write our public key -- self-reported, no authentication
    self.config.my_public_key().write(&mut envelope);
    // Write payload length
    (payload.len() as u32).to_le_bytes().iter().for_each(|b| envelope.push(*b));
    // Write payload
    envelope.extend_from_slice(&payload);

    // ...

    match TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5)) {
        Ok(mut stream) => {
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .map_err(|e| DkgError::Network(format!("Set timeout: {}", e)))?;
            stream
                .write_all(&envelope)  // Plaintext TCP, no TLS, no authentication
                .map_err(|e| DkgError::Network(format!("Write: {}", e)))?;
            // ...
        }
        // ...
    }
}
```

**Unauthenticated receiver in `poll_incoming()` -- `crates/node/dkg/src/network.rs:128-141`:**

```rust
// Read public key (32 bytes for ed25519)
let mut pk_bytes = [0u8; 32];
if stream.read_exact(&mut pk_bytes).is_err() {
    warn!(%addr, "Failed to read sender public key");
    continue;
}

let from = match commonware_codec::ReadExt::read(&mut pk_bytes.as_slice()) {
    Ok(pk) => pk,    // Only deserialization, no signature verification
    Err(e) => {
        warn!(%addr, ?e, "Failed to decode public key");
        continue;
    }
};
```

**Ceremony instantiates plaintext network -- `crates/node/dkg/src/ceremony.rs:70`:**

```rust
// Initialize network
let network = DkgNetwork::new(self.config.clone())?;
```

**Secure transport exists but is unused -- `crates/node/dkg/src/transport.rs:1-5`:**

```rust
//! Production DKG transport using commonware-p2p authenticated discovery.
//!
//! This module provides authenticated, encrypted channels for DKG ceremony messages
//! using the commonware-p2p authenticated discovery network.
```

## Impact

If the DKG ceremony is run over any network that is not physically isolated (e.g., across data centers, over the internet, or even within a shared cloud VPC), an attacker can:

1. **Reconstruct the group BLS secret key**: By observing the polynomial shares exchanged during the ceremony, an attacker with access to a threshold number of shares can reconstruct the group secret key.
2. **Forge finality certificates**: With the group secret key, the attacker can sign arbitrary blocks, enabling double-spend attacks.
3. **Completely compromise consensus integrity**: The attacker can produce valid threshold signatures for any message, defeating the entire BFT consensus mechanism.
4. **Impersonate participants**: By writing a forged public key in the envelope, the attacker can inject biased shares or suppress legitimate shares.

The trusted dealer mode (`dkg_deal` in `bin/keygen/src/dkg_deal.rs`) avoids the network exposure by generating all shares in a single process, but introduces its own single-point-of-compromise problem.

## Root Cause

The `DkgCeremony` was built with `DkgNetwork` (a simple TCP wrapper at `crates/node/dkg/src/network.rs`) for development convenience. The secure `DkgTransport` (at `crates/node/dkg/src/transport.rs`) was added later but was never plumbed into the ceremony runner at `crates/node/dkg/src/ceremony.rs`.

## Suggested Fix

1. **Replace `DkgNetwork` with `DkgTransport`** in `ceremony.rs`. The `DkgTransport` already implements authenticated, encrypted channels using commonware's discovery layer.

2. **If both transports must coexist**, gate `DkgNetwork` behind a `--insecure-dkg` CLI flag that prints a prominent warning, and make `DkgTransport` the default:

```rust
// In ceremony.rs
let network: Box<dyn DkgTransportTrait> = if insecure_mode {
    warn!("INSECURE DKG: using plaintext TCP -- DO NOT USE IN PRODUCTION");
    Box::new(DkgNetwork::new(self.config.clone())?)
} else {
    Box::new(DkgTransport::new(context, self.config.clone())?)
};
```

3. **Add a deprecation warning** on `DkgNetwork`:

```rust
#[deprecated(note = "Use DkgTransport for authenticated, encrypted DKG channels")]
pub struct DkgNetwork { /* ... */ }
```

## Files to Modify

- `crates/node/dkg/src/ceremony.rs` -- Line 70: Replace `DkgNetwork::new()` with `DkgTransport` initialization
- `crates/node/dkg/src/network.rs` -- Add `#[deprecated]` annotation
- `bin/kora/src/cli.rs` -- Add `--insecure-dkg` flag if both transports are kept

## Related Issues

- [006 -- P2P Ports Exposed on Public Internet](./006-p2p-ports-public-internet.md) -- Related network security concern
