# Validator Private Key Loaded From Disk Twice on Startup

**Category**: Config / Security
**Severity**: Low
**Labels**: `bug`, `security`, `config`, `good first issue`

## Summary

During validator startup, the Ed25519 private key is read from disk twice: once in `run_standalone` to build the P2P transport, and again inside `run()` to derive the validator's public key. This doubles the exposure window for side-channel attacks on key material and introduces a TOCTOU (time-of-check-time-of-use) risk if the key file changes between reads.

## Problem

The `ProductionRunner::run_standalone` method in `crates/node/runner/src/runner.rs` calls `config.validator_key()` at line 905-907, which reads the 32-byte seed from `{data_dir}/validator.key` on disk and derives an Ed25519 private key. This key is then moved into `build_local_transport()` at line 911. Later, inside `run()` at line 1303-1305, `config.validator_key()` is called again, reading the same file from disk a second time to derive the public key via `commonware_cryptography::Signer::public_key(&validator_key)`.

The `validator_key()` method (defined in `crates/node/config/src/node.rs` line 132-175) reads bytes from disk every time it is called -- it has no caching.

## Code Reference

First read -- `crates/node/runner/src/runner.rs` lines 904-912:

```rust
executor.start(|context| async move {
    let validator_key = config
        .validator_key()
        .map_err(|e| anyhow::anyhow!("failed to load validator key: {}", e))?;

    let transport = config
        .network
        .build_local_transport(validator_key, context.child("transport"))
        .map_err(|e| anyhow::anyhow!("failed to build transport: {}", e))?;
```

Second read -- `crates/node/runner/src/runner.rs` lines 1303-1306:

```rust
let validator_key = config
    .validator_key()
    .map_err(|e| anyhow::anyhow!("failed to load validator key: {}", e))?;
let my_pk = commonware_cryptography::Signer::public_key(&validator_key);
```

The `validator_key()` method -- `crates/node/config/src/node.rs` lines 132-150:

```rust
pub fn validator_key(
    &self,
) -> Result<commonware_cryptography::ed25519::PrivateKey, ConfigError> {
    let key_path = self
        .consensus
        .validator_key
        .clone()
        .unwrap_or_else(|| self.data_dir.join("validator.key"));

    // Try to load existing key
    match std::fs::read(&key_path) {
        Ok(key_bytes) => {
            if key_bytes.len() != 32 {
                return Err(ConfigError::InvalidKeyLength(key_bytes.len()));
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&key_bytes);
            Ok(private_key_from_seed(seed))
        }
        // ...
```

## Impact

- **TOCTOU risk**: If the key file is deleted, corrupted, or replaced between the two reads (possible with Docker volume mounts, external tooling, or hot-key-rotation scripts), the second read either fails or silently reads a different key. This would cause the node to join consensus with a public key that does not match its P2P transport identity, leading to authentication failures or identity confusion.
- **Security exposure**: The private key material is deserialized from disk twice, doubling the window during which key bytes exist in user-space buffers. While the practical risk is low on modern operating systems, it violates the principle of minimizing key material exposure.
- **Unnecessary I/O**: Two filesystem reads where one suffices.

## Root Cause

The key is moved (consumed) by `build_local_transport()` at line 911 because the transport config takes ownership of the `ed25519::PrivateKey`. After that point, `run_standalone` no longer has access to the key, so `run()` must reload it. The fix is to derive the public key before the move, or to clone the key before passing it to the transport builder.

## Suggested Fix

Derive the public key from the first key load before it is moved into the transport, then pass the public key (or a clone of the key) into `run()` so that `run()` never needs to re-read the file.

Before (`run_standalone`, lines 904-915):
```rust
executor.start(|context| async move {
    let validator_key = config
        .validator_key()
        .map_err(|e| anyhow::anyhow!("failed to load validator key: {}", e))?;

    let transport = config
        .network
        .build_local_transport(validator_key, context.child("transport"))
        .map_err(|e| anyhow::anyhow!("failed to build transport: {}", e))?;

    let ctx =
        kora_service::NodeRunContext::new(context, std::sync::Arc::new(config), transport);
```

After:
```rust
executor.start(|context| async move {
    let validator_key = config
        .validator_key()
        .map_err(|e| anyhow::anyhow!("failed to load validator key: {}", e))?;
    let my_pk = commonware_cryptography::Signer::public_key(&validator_key);

    let transport = config
        .network
        .build_local_transport(validator_key, context.child("transport"))
        .map_err(|e| anyhow::anyhow!("failed to build transport: {}", e))?;

    let ctx =
        kora_service::NodeRunContext::new(context, std::sync::Arc::new(config), transport)
            .with_public_key(my_pk);
```

Then remove the second `config.validator_key()` call at lines 1303-1306 in `run()` and use the public key from the context instead.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- `run_standalone` (lines 904-915) and `run` (lines 1303-1306)
- Possibly `crates/node/service/` -- `NodeRunContext` may need a field to carry the public key

## Related Issues

- `018-dkg-no-memory-zeroization.md` -- both involve key material handling hygiene
- `050-config-validator-key-auto-generated-silently.md` -- validator key management
