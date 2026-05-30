# DKG Ceremony State File Written World-Readable (0644)

**Category**: security
**Severity**: high
**Labels**: `security`, `dkg`, `bug`

## Summary

The DKG state file (`dkg_state.json`) is written using `std::fs::write()`, which creates the file with default umask permissions (typically 0644 on Linux). This means any local user on the machine can read the file. The state file contains hex-encoded signed dealer logs and commitment data from the DKG ceremony. Other secret files like `share.key` and `validator.key` are correctly written with restrictive mode 0600 via a `write_secret_file()` helper, but the state file was overlooked.

## Problem

In `crates/node/dkg/src/state.rs`, the `PersistedDkgState::save()` method writes the DKG ceremony state to disk using `std::fs::write()`. This standard library function does not set explicit file permissions -- it relies on the process's umask, which on most Linux systems defaults to 022, resulting in mode 0644 (owner read-write, group and others read).

The `dkg_state.json` file contains:
- `our_signed_log`: hex-encoded signed dealer log containing commitment data
- `received_logs`: hex-encoded signed dealer logs from all other ceremony participants
- `phase`, `dealer_started`, `dealer_finalized`: ceremony progress metadata
- `session`: ceremony session metadata including `ceremony_id`, `chain_id`, and `round`

While these are signed dealer logs (not raw secret shares), they contain intermediate cryptographic material from the DKG ceremony that could aid a local attacker in reconstructing or biasing the group key if combined with other compromises.

The `write_secret_file()` utility function that correctly sets mode 0600 already exists in both `crates/node/dkg/src/output.rs` (line 118) and `bin/keygen/src/dkg_deal.rs` (line 145), and is used for `share.key`. It was not applied to the state file.

## Code Reference

The problematic save method in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/state.rs` (lines 122-127):

```rust
    /// Save state to disk.
    pub fn save(&self, data_dir: &Path) -> Result<(), DkgError> {
        let path = data_dir.join(Self::STATE_FILE);
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;  // <-- uses default umask (0644)
        Ok(())
    }
```

For comparison, the correctly secured `write_secret_file()` in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` (lines 118-128):

```rust
/// Write `data` to `path` with mode `0600` so key material is never world-readable.
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

**Local privilege escalation / information leakage.** On a multi-user system or a containerized environment with shared volume mounts:

1. Any user on the same machine can read `dkg_state.json` and obtain commitment data and dealer logs from the DKG ceremony.
2. In a container orchestration environment where volumes are shared or accessible to sidecar containers, the DKG state is exposed.
3. The combination of dealer logs from all participants could allow an attacker to reconstruct or bias the group key, particularly if they also compromise one participant's secret share.
4. During crash recovery windows, the state file persists on disk longer than the active ceremony, increasing the exposure window.

## Root Cause

The `save()` method was implemented using the standard `std::fs::write()` convenience function without considering file permissions. The `write_secret_file()` utility that correctly handles permissions was not applied to the DKG state file, likely because the state file was not considered security-sensitive (it contains signed logs rather than raw secret shares). However, the defense-in-depth principle dictates that DKG intermediate material should be treated as sensitive.

## Suggested Fix

Use the same `write_secret_file()` pattern used for `share.key`. The simplest approach is to either import the existing helper or inline the permission-setting logic:

**Before** (`crates/node/dkg/src/state.rs:122-127`):
```rust
pub fn save(&self, data_dir: &Path) -> Result<(), DkgError> {
    let path = data_dir.join(Self::STATE_FILE);
    let content = serde_json::to_string_pretty(self)?;
    std::fs::write(&path, content)?;
    Ok(())
}
```

**After**:
```rust
pub fn save(&self, data_dir: &Path) -> Result<(), DkgError> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;

    let path = data_dir.join(Self::STATE_FILE);
    let content = serde_json::to_string_pretty(self)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    f.write_all(content.as_bytes())?;
    Ok(())
}
```

Alternatively, refactor `write_secret_file()` into a shared utility crate and reuse it across `output.rs`, `dkg_deal.rs`, `setup.rs`, and `state.rs`.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/state.rs` -- `PersistedDkgState::save()` at line 122

## Related Issues

- Local file `017-dkg-no-encryption-at-rest.md` -- DKG key material is not encrypted at rest (broader issue)
- Local file `173-dkg-output-json-default-permissions.md` -- `output.json` also uses default permissions (same class of bug)
- Local file `024-trusted-dealer-no-cleanup.md` -- Trusted dealer does not clean up intermediate files
