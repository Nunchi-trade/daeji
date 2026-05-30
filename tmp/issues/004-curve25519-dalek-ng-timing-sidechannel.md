# 004: curve25519-dalek-ng Had Unpatched Timing Side-Channel (RUSTSEC-2024-0344) -- RESOLVED

**Category:** security / dependencies
**Severity:** critical (was critical, now **resolved**)
**Labels:** security, dependencies

---

## Summary

**This issue has been resolved by the commonware upgrade (merged 2026-05-28, branch `fix/commonware-resolver-upgrade`).** Previously, the `ed25519-consensus` crate (version 2.1.0) depended on `curve25519-dalek-ng` 4.1.1, an unmaintained fork that did not contain the timing side-channel fix from RUSTSEC-2024-0344. The vulnerability allowed LLVM to insert conditional branches on mask values in `Scalar29::sub` and `Scalar52::sub`, creating timing variability that could leak private key material. Kora now uses `commonware-cryptography::ed25519` for all Ed25519 operations, which depends on the patched `curve25519-dalek` (>= 4.1.3).

## Problem (Historical)

The `ed25519-consensus` crate was used by Kora for:
- Validator identity key generation in `bin/keygen/src/setup.rs`
- Validator key management in `crates/node/config/src/node.rs`

This crate depended on `curve25519-dalek-ng` 4.1.1, an unmaintained fork of `curve25519-dalek` that had **not received** the timing side-channel fix from RUSTSEC-2024-0344. The vulnerability was in the `Scalar29::sub` and `Scalar52::sub` functions, where LLVM could insert conditional branches on mask values instead of constant-time operations, creating timing variability that could leak information about private key material through side-channel analysis.

## Resolution

The commonware upgrade replaced all `ed25519-consensus` usage with `commonware-cryptography::ed25519`. Neither `ed25519-consensus` nor `curve25519-dalek-ng` appear in the current `Cargo.lock`.

**`bin/keygen/Cargo.toml` -- no longer lists `ed25519-consensus`:**

```toml
[dependencies]
kora-config.workspace = true
kora-domain.workspace = true
kora-dkg.workspace = true

commonware-cryptography.workspace = true
commonware-codec.workspace = true
commonware-utils.workspace = true
# No ed25519-consensus dependency
```

**`crates/node/config/Cargo.toml` -- no longer lists `ed25519-consensus`:**

```toml
[dependencies]
# Cryptography
commonware-codec.workspace = true
commonware-cryptography.workspace = true
rand.workspace = true
# No ed25519-consensus dependency
```

**`crates/node/config/src/node.rs:194-197` -- now uses `commonware_cryptography::ed25519::PrivateKey`:**

```rust
fn private_key_from_seed(seed: [u8; 32]) -> commonware_cryptography::ed25519::PrivateKey {
    commonware_cryptography::ed25519::PrivateKey::read(&mut seed.as_slice())
        .expect("32-byte ed25519 seed should decode")
}
```

**`bin/keygen/src/setup.rs:87-89` -- now uses `ed25519::PrivateKey` from commonware-cryptography:**

```rust
fn private_key_from_seed(seed: [u8; 32]) -> ed25519::PrivateKey {
    ed25519::PrivateKey::read(&mut seed.as_slice()).expect("32-byte ed25519 seed should decode")
}
```

## Impact (Historical)

The timing leak affected Ed25519 signature operations used in:
- Validator identity key management (`crates/node/config/src/node.rs`)
- Key generation (`bin/keygen/src/setup.rs`)
- Any signature verification through `ed25519-consensus`

In theory, an attacker with precise timing measurements of validator signing operations could extract bits of the private key, eventually reconstructing it. This would allow the attacker to impersonate a validator on the P2P network.

## Root Cause (Historical)

The `ed25519-consensus` crate was chosen early in development and pinned `curve25519-dalek-ng` as a dependency. The `-ng` fork was abandoned by its maintainer, and the RUSTSEC-2024-0344 fix was only applied to the mainline `curve25519-dalek` crate.

## Files (Current State)

- `bin/keygen/Cargo.toml` -- Uses `commonware-cryptography` (no `ed25519-consensus`)
- `crates/node/config/Cargo.toml` -- Uses `commonware-cryptography` (no `ed25519-consensus`)
- `crates/node/config/src/node.rs:194-197` -- `private_key_from_seed()` uses `commonware_cryptography::ed25519::PrivateKey`
- `bin/keygen/src/setup.rs:87-89` -- `private_key_from_seed()` uses `ed25519::PrivateKey` from commonware

## Related Issues

None -- this issue is fully resolved.
