# 050: Validator Key Silently Auto-Generated When Key File Is Missing

**Category:** bug / security
**Severity:** medium

**Labels:** `bug`, `security`, `config`, `reliability`

## Summary

When the validator key file does not exist at the configured path, the `validator_key()` function silently generates a new random Ed25519 key and writes it to disk. This is convenient for first-time development setup but dangerous in production: an operator who misconfigures the key path gets a brand-new identity that does not match the DKG participants list. The node then fails much later with an unhelpful "public key not found in participants" error, far removed from the actual cause.

## Problem

The `NodeConfig::validator_key()` method at `crates/node/config/src/node.rs:133-183` handles a missing key file by generating a new random key without any warning, confirmation, or flag check:

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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Generate new key -- no warning or confirmation!
            let mut seed = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);

            // Ensure parent directory exists
            if let Some(parent) = key_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| ConfigError::CreateDir {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }

            // Write key to disk with restrictive permissions (0600)
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&key_path)
                    .map_err(|e| ConfigError::Write { path: key_path.clone(), source: e })?;
                std::io::Write::write_all(&mut f, &seed)
                    .map_err(|e| ConfigError::Write { path: key_path.clone(), source: e })?;
            }

            Ok(private_key_from_seed(seed))
        }
        Err(e) => Err(ConfigError::Read { path: key_path, source: e }),
    }
}
```

This function is called at `crates/node/runner/src/runner.rs:905-907`:

```rust
let validator_key = config
    .validator_key()
    .map_err(|e| anyhow::anyhow!("failed to load validator key: {}", e))?;
```

The generated key is used for P2P identity and consensus participation. Since the key is brand new, it will not match any entry in the DKG participants list, causing the node to fail during consensus setup.

## Impact

In production deployments, this silent auto-generation causes several problems:

1. **Misconfigured path**: An operator who sets `data_dir` or `consensus.validator_key` incorrectly in their config file gets a new validator identity without any warning. The node starts up and connects to peers, but then fails during DKG or consensus with an error like "public key not found in participants."

2. **Debugging difficulty**: The error message at failure time does not reference the key generation. The operator must manually discover that a new key was generated at the wrong location and that it doesn't match their expected identity.

3. **Ephemeral storage**: If the data directory is on ephemeral storage (e.g., a tmpfs, an unperisted Docker volume, or a Kubernetes pod with no PVC), the key is regenerated on every restart, creating a different identity each time. The node can never participate in consensus because its identity keeps changing.

4. **Security**: Auto-generating cryptographic keys without explicit operator action violates the principle of least surprise and makes it harder to audit key management practices.

## Root Cause

The auto-generation behavior was designed for developer convenience during early development (so a first-time developer can run `kora validator` without first running a key generation step). However, this convenience behavior is applied unconditionally to all environments, including production.

## Suggested Fix

**Option 1 (recommended):** Make auto-generation opt-in via a `--generate-key` CLI flag or `kora init` subcommand. In `validator_key()`, if the file does not exist and auto-generation was not explicitly requested, return a clear error:

```rust
Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
    return Err(ConfigError::KeyNotFound {
        path: key_path.display().to_string(),
        hint: "Run 'kora init' to generate a new key, or set 'consensus.validator_key' \
               in your config to point to an existing key file.".to_string(),
    });
}
```

**Option 2:** Log a prominent warning when auto-generating, so operators notice it in production:

```rust
Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
    tracing::warn!(
        path = %key_path.display(),
        "Validator key not found. Generating new key. \
         This is only appropriate for development. In production, \
         provide an existing key file via 'consensus.validator_key'."
    );
    // ... generate key ...
}
```

**Option 3:** Only auto-generate in the DKG and setup subcommands (where generating a new key makes sense). The `validator` subcommand should require an existing key.

## Files to Modify

- `crates/node/config/src/node.rs` -- `NodeConfig::validator_key()` at line 133-183 (add error on missing file or at least a warning)
- `bin/kora/src/cli.rs` -- CLI entry point (add `--generate-key` flag or `init` subcommand)

## Related Issues

None directly related in the current issue set.
