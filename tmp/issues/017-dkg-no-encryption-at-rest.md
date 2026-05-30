# Secret Keys Stored as Plaintext on Disk -- No Encryption at Rest

**Category**: Security
**Severity**: High
**Labels**: `security`, `dkg`, `config`

## Summary

All secret key material in the Kora node is stored on disk as unencrypted plaintext. The only protection is Unix file permissions set to `0600` (owner read/write only). Three types of secrets are affected: ed25519 validator identity keys, BLS threshold shares, and DKG ceremony state containing threshold polynomial information. Additionally, the DKG state file (`dkg_state.json`) uses `std::fs::write()` which inherits the process umask, potentially creating a world-readable file containing sensitive ceremony data.

## Problem

The Kora node stores three categories of secret key material on disk without any encryption:

### 1. Validator Identity Keys (`validator.key`)

Raw 32-byte ed25519 seed written directly to a file.

**Generated in keygen** (`/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs:121-127`):

```rust
} else {
    tracing::info!(node = i, "Generating new identity key");
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    write_secret_file(&key_path, &seed)?;  // 0600 permissions
    private_key_from_seed(seed)
};
```

**Auto-generated at runtime** (`/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs:152-177`):

```rust
Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
    let mut seed = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
    // ...
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)  // Owner-only, but plaintext
            .open(&key_path)
            .map_err(/* ... */)?;
        std::io::Write::write_all(&mut f, &seed)
            .map_err(/* ... */)?;
    }
    Ok(private_key_from_seed(seed))
}
```

### 2. BLS Threshold Shares (`share.key`)

JSON-encoded hex string containing the BLS secret share.

**Written by trusted dealer** (`/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs:130-132`):

```rust
let share_json = ShareJson { index: share.index.get(), secret: hex::encode(&share_bytes) };
let share_path = node_dir.join("share.key");
write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;
```

**Written by interactive DKG** (`/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs:63-68`):

```rust
let share_json =
    ShareJson { index: self.share_index, secret: hex::encode(&self.share_secret) };
let share_path = data_dir.join("share.key");
write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;
```

Both paths correctly use `write_secret_file` with `0600` permissions.

### 3. DKG Ceremony State (`dkg_state.json`) -- Permission Inconsistency

The DKG state file contains serialized dealer logs with threshold polynomial information.

**Written with default permissions** (`/Users/will/dev/nunchi/daeji/crates/node/dkg/src/state.rs:122-126`):

```rust
pub fn save(&self, data_dir: &Path) -> Result<(), DkgError> {
    let path = data_dir.join(Self::STATE_FILE);
    let content = serde_json::to_string_pretty(self)?;
    std::fs::write(&path, content)?;  // Uses std::fs::write -- inherits umask!
    Ok(())
}
```

`std::fs::write()` creates files with mode `0666 & ~umask`. With the common default umask of `0022`, this yields `0644` (world-readable). This is inconsistent with the `0600` permissions used for `validator.key` and `share.key`.

### 4. DKG Output (`output.json`) -- Default Permissions

**Written with default permissions** (`/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs:60-61`):

```rust
let output_path = data_dir.join("output.json");
std::fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;  // Default perms
```

While `output.json` contains the public polynomial (not secret), applying restrictive permissions is a defense-in-depth measure.

## Code Reference

**write_secret_file helper in keygen** (`/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs:217-228`):

```rust
fn write_secret_file(path: &std::path::Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .wrap_err_with(|| format!("Failed to create secret file {}", path.display()))?;
    f.write_all(data).wrap_err_with(|| format!("Failed to write secret file {}", path.display()))
}
```

**write_secret_file helper in DKG output** (`/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs:117-128`):

```rust
fn write_secret_file(path: &Path, data: &[u8]) -> Result<(), DkgError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?;
    Ok(())
}
```

Note: `write_secret_file` is duplicated in three places (`setup.rs`, `dkg_deal.rs`, and `output.rs`) rather than shared.

## Impact

- **Any process running as the same Unix user** can read all key material (validator keys, BLS shares).
- **Root users** (including the Docker host in containerized deployments) have full access to all secrets.
- **Disk images, backups, and VM snapshots** expose all secrets in plaintext, making key rotation after infrastructure changes critical but not enforced.
- **In Docker environments**, the container's overlay2 filesystem layer stores keys in plaintext, accessible to anyone with access to the Docker storage directory.
- **The DKG state file** (`dkg_state.json`) is potentially world-readable due to the umask issue, exposing ceremony metadata and signed dealer logs to any local user.
- **If a single validator's key files are compromised**, the attacker gains that validator's consensus identity (ed25519 key) and BLS threshold share, enabling impersonation and participation in threshold signatures.

## Root Cause

No key encryption scheme was implemented. The system relies solely on filesystem permissions for secret protection. The `dkg_state.json` permission inconsistency is an oversight where `std::fs::write()` was used instead of the existing `write_secret_file()` helper.

## Suggested Fix

### Immediate fix: Permission consistency

Change `dkg_state.json` to use `write_secret_file` instead of `std::fs::write`:

```rust
pub fn save(&self, data_dir: &Path) -> Result<(), DkgError> {
    let path = data_dir.join(Self::STATE_FILE);
    let content = serde_json::to_string_pretty(self)?;
    write_secret_file(&path, content.as_bytes())?;  // Use 0600 permissions
    Ok(())
}
```

Also fix `output.json` at `output.rs:60-61` to use restrictive permissions.

### Medium-term: Passphrase-based encryption

Encrypt key files with AES-256-GCM using a key derived from a passphrase via Argon2id. The passphrase can be provided via environment variable (`KORA_KEY_PASSPHRASE`) or read from a file path specified in the config:

```rust
pub fn write_encrypted_key(path: &Path, data: &[u8], passphrase: &str) -> Result<()> {
    let salt = rand::rngs::OsRng.gen::<[u8; 16]>();
    let key = argon2id_derive(passphrase, &salt);
    let nonce = rand::rngs::OsRng.gen::<[u8; 12]>();
    let ciphertext = aes256gcm_encrypt(&key, &nonce, data);
    let mut output = Vec::new();
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    write_secret_file(path, &output)
}
```

### Code deduplication

Consolidate the three copies of `write_secret_file` into a shared utility crate (e.g., `kora-utils` or `kora-config`).

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/state.rs` (line 125) -- `save()` uses `std::fs::write()` instead of `write_secret_file()`
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` (line 61) -- `output.json` uses `std::fs::write()` with default permissions
- `/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs` (line 217) -- duplicated `write_secret_file` helper
- `/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs` (line 144) -- duplicated `write_secret_file` helper
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` (line 117) -- duplicated `write_secret_file` helper
- `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs` (lines 166-177) -- inline `write_secret_file` logic, also plaintext

## Related Issues

- `018-dkg-no-memory-zeroization.md` -- secret key material is also not zeroized in memory
- `166-dkg-state-file-world-readable.md` -- specific focus on the `dkg_state.json` permission issue
- `173-dkg-output-json-default-permissions.md` -- `output.json` default permissions
- `024-trusted-dealer-no-cleanup.md` -- trusted dealer does not clean up sensitive intermediate state
