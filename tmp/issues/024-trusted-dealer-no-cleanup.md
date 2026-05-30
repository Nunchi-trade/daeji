# 024: Trusted Dealer DKG Mode Has No Post-Ceremony Cleanup of Shared Secrets

**Category**: security
**Severity**: high
**Labels**: security, dkg, docker

---

## Summary

The trusted dealer DKG mode generates all BLS12-381 threshold shares in a single process on a single machine and writes them to a shared filesystem. After distribution, neither the in-process memory containing all shares nor the filesystem copies are securely erased. A compromise of the dealer machine or the shared volume yields the complete group secret key, enabling an attacker to forge threshold signatures and fabricate finality certificates.

---

## Problem

Kora supports two DKG modes: interactive (production) and trusted dealer (development). The trusted dealer mode (`bin/keygen/src/dkg_deal.rs`) is designed for fast devnet bootstrapping. It works as follows:

1. The `dkg::deal()` function generates all secret shares in a single process (line 92-94).
2. The `shares` variable and `public_output` contain the complete group key material in memory.
3. Each share is written to a shared filesystem path (`/shared/nodeN/share.key`) via `write_secret_file()` (line 132).
4. The process exits without zeroing the `shares` variable or cleaning up the filesystem.

In the Docker devnet deployment, all shares exist simultaneously in the `shared_config` Docker volume, which is accessible to all containers and persists across container restarts.

**File**: `/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs`

---

## Code Reference

The share generation and distribution code (lines 89-141 of `dkg_deal.rs`):

```rust
// bin/keygen/src/dkg_deal.rs:89-94
let mut rng = rand::rngs::OsRng;

tracing::info!("Generating BLS threshold key shares");
let (public_output, shares) =
    dkg::deal::<MinSig, _, N3f1>(&mut rng, Mode::default(), participants_set)
        .map_err(|e| eyre::eyre!("DKG deal failed: {:?}", e))?;
```

```rust
// bin/keygen/src/dkg_deal.rs:111-135
for (i, pk) in participants.iter().enumerate() {
    let share =
        shares.get_value(pk).ok_or_else(|| eyre::eyre!("Missing share for node{}", i))?;

    let mut share_bytes = Vec::new();
    share.write(&mut share_bytes);

    let node_dir = args.output_dir.join(format!("node{}", i));

    // ... writes output.json ...

    let share_json = ShareJson { index: share.index.get(), secret: hex::encode(&share_bytes) };
    let share_path = node_dir.join("share.key");
    write_secret_file(&share_path, serde_json::to_string_pretty(&share_json)?.as_bytes())?;

    tracing::info!(node = i, "Wrote DKG output and share");
}

tracing::info!("Trusted dealer DKG complete");
// Process exits here -- no cleanup of `shares`, `public_output`, or filesystem copies
```

The module-level doc comment (lines 1-4) acknowledges the security limitation:

```rust
// bin/keygen/src/dkg_deal.rs:1-4
//! Trusted dealer DKG for devnet.
//!
//! Generates all BLS12-381 threshold shares using a single trusted dealer.
//! This is NOT secure for production but allows testing the validator workflow.
```

The `write_secret_file` function does set file permissions to 0600 (line 151), which prevents other users from reading the file, but does not address the shared volume concern:

```rust
// bin/keygen/src/dkg_deal.rs:145-155
fn write_secret_file(path: &std::path::Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)?
}
```

---

## Impact

The dealer machine (or the shared Docker volume), if compromised during or after the ceremony, gives the attacker the complete group secret key. This enables:

1. **Forging threshold signatures**: The attacker can produce valid BLS12-381 threshold signatures without the cooperation of any legitimate validator.
2. **Fabricating finality certificates**: Consensus finality in Kora relies on threshold signatures over block commitments. An attacker with the group key can finalize arbitrary blocks.
3. **Complete consensus takeover**: With the ability to forge finality certificates, the attacker can rewrite the chain history or censor transactions.

In the Docker devnet setup specifically:
- All shares exist simultaneously in the `shared_config` Docker volume.
- The volume persists across container restarts (`docker volume ls` lists it).
- Any container with access to the volume can read all shares.
- The share data remains on disk indefinitely until the volume is explicitly removed.

---

## Root Cause

The trusted dealer mode was designed for development convenience. The shared filesystem approach eliminates the need for network-based share distribution but concentrates all secrets in one location. No cleanup or memory zeroization steps were implemented because the mode was intended only for local testing.

---

## Suggested Fix

1. **Add memory zeroization**: Use the `zeroize` crate to clear the `shares` and `public_output` variables before process exit:

```rust
use zeroize::Zeroize;

// After all shares are written:
drop(shares);  // or shares.zeroize() if the type implements Zeroize
drop(public_output);
```

2. **Add runtime warning**: Print a prominent warning when trusted dealer mode is used:

```rust
tracing::warn!(
    "=== TRUSTED DEALER MODE ===\n\
     All threshold shares are generated in a single process.\n\
     This is NOT secure for production. The shared filesystem \
     contains the complete group secret key."
);
```

3. **Mount shared directory as tmpfs**: In the Docker compose configuration, use a tmpfs mount for the shared volume so shares are never written to persistent disk:

```yaml
volumes:
  shared_config:
    driver: local
    driver_opts:
      type: tmpfs
      o: size=10m
```

4. **Post-distribution cleanup**: Add a cleanup step that securely overwrites and removes the share files after all validators have confirmed receipt:

```rust
for i in 0..args.validators {
    let share_path = args.output_dir.join(format!("node{}/share.key", i));
    // Overwrite with zeros before deletion
    if let Ok(mut f) = fs::OpenOptions::new().write(true).open(&share_path) {
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = f.write_all(&vec![0u8; size as usize]);
        let _ = f.sync_all();
    }
    let _ = fs::remove_file(&share_path);
}
```

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs` -- Add memory zeroization, runtime warning, and post-distribution cleanup
- `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml` -- Use tmpfs for the shared_config volume

---

## Related Issues

- `021-dkg-legacy-messages-replay.md` (DKG protocol security)
