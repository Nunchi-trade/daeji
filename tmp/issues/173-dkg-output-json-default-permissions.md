# output.json Written With Default Permissions (World-Readable)

**Category**: security
**Severity**: medium
**Labels**: `security`, `dkg`, `good first issue`

## Summary

Both the interactive DKG path (`DkgOutput::save()`) and the trusted dealer path (`dkg_deal::run()`) write `output.json` using `std::fs::write()`, which creates the file with default umask permissions (typically 0644 on Linux). While `output.json` contains only public data (group public key, polynomial, participant keys), it is written in the same directory as `share.key`, which is correctly secured with mode 0600. This inconsistency creates a confusing security posture and leaks network topology information (participant count, threshold, and public keys of all validators).

## Problem

Two code paths write `output.json` with default permissions:

1. **Interactive DKG**: `crates/node/dkg/src/output.rs` at line 61 uses `std::fs::write()`:
   ```rust
   let output_path = data_dir.join("output.json");
   std::fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;
   ```
   Immediately followed by `share.key` written with 0600 at line 67:
   ```rust
   let share_path = data_dir.join("share.key");
   write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;
   ```

2. **Trusted dealer**: `bin/keygen/src/dkg_deal.rs` at line 128 uses `fs::write()`:
   ```rust
   let output_path = node_dir.join("output.json");
   fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;
   ```
   Followed by `share.key` with 0600 at line 132:
   ```rust
   let share_path = node_dir.join("share.key");
   write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;
   ```

In both cases, the `write_secret_file()` function (which uses `OpenOptionsExt::mode(0o600)`) is available and used for `share.key` but not for `output.json`.

## Code Reference

Interactive DKG in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` (lines 60-68):

```rust
        let output_path = data_dir.join("output.json");
        std::fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;

        let share_json =
            ShareJson { index: self.share_index, secret: hex::encode(&self.share_secret) };

        let share_path = data_dir.join("share.key");
        write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;
```

Trusted dealer in `/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs` (lines 127-132):

```rust
        let output_path = node_dir.join("output.json");
        fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;

        let share_json = ShareJson { index: share.index.get(), secret: hex::encode(&share_bytes) };
        let share_path = node_dir.join("share.key");
        write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;
```

`write_secret_file()` in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` (lines 118-128):

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

## Impact

**Information leakage of network topology.** The `output.json` file contains:

- `group_public_key`: The BLS group public key (public, but reveals the network's signing identity)
- `public_polynomial`: The sharing polynomial (public, but needed for threshold verification)
- `threshold`: The quorum requirement (reveals the fault tolerance of the network)
- `participants`: The number of validators (reveals network size)
- `participant_keys`: All validator ed25519 public keys (reveals the full validator set)

While this data is public by nature (validators advertise their keys in the P2P network), having it available in a single readable file in a known location makes reconnaissance trivial. More importantly, the inconsistent permission model -- where `share.key` is 0600 but `output.json` right next to it is 0644 -- may mislead operators into thinking all DKG output is protected.

## Root Cause

The `output.json` file was treated as "public data" and written with the standard `std::fs::write()` convenience function. The developer applied `write_secret_file()` only to `share.key` (which contains the actual secret share). However, the distinction between "public" and "secret" files is not clearly documented, and the inconsistent permission model is confusing.

## Suggested Fix

Write `output.json` with explicit permissions (0640 or 0644) to make the security intent clear. Using 0640 provides defense-in-depth without breaking any functionality:

**Before** (both `output.rs:61` and `dkg_deal.rs:128`):
```rust
std::fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;
```

**After**:
```rust
use std::os::unix::fs::OpenOptionsExt;
let mut f = std::fs::OpenOptions::new()
    .write(true)
    .create(true)
    .truncate(true)
    .mode(0o640)  // explicit: owner rw, group r, others none
    .open(&output_path)?;
std::io::Write::write_all(&mut f, serde_json::to_string_pretty(&output_json)?.as_bytes())?;
```

Alternatively, if the data is intentionally public, add a code comment explaining the permission choice:

```rust
// output.json contains only public data (group key, polynomial, participant keys).
// Permissions are intentionally permissive (default umask) -- see share.key for secrets.
std::fs::write(&output_path, serde_json::to_string_pretty(&output_json)?)?;
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` -- `DkgOutput::save()` at line 61
- `/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs` -- `run()` at line 128

## Related Issues

- Local file `166-dkg-state-file-world-readable.md` -- `dkg_state.json` also written with default permissions (same class of issue, but with more sensitive data)
- Local file `017-dkg-no-encryption-at-rest.md` -- DKG key material not encrypted at rest (broader security concern)
