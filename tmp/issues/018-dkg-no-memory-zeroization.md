# No Memory Zeroization of Secret Key Material

**Category**: Security
**Severity**: High
**Labels**: `security`, `dkg`, `config`

## Summary

Secret key material (ed25519 seeds, BLS threshold shares) is stored in plain Rust arrays (`[u8; 32]`) and `Vec<u8>` buffers that are never explicitly zeroized when they go out of scope. The `zeroize` crate is not used anywhere in the codebase. This means that after key material is logically no longer needed, the raw bytes remain in process memory until the memory is overwritten by unrelated data, creating a window for extraction via memory dumps, core files, swap, or cold boot attacks.

## Problem

Secret key material passes through several code paths where it is stored in stack-allocated arrays or heap-allocated vectors, and in none of these paths is the data securely erased when it goes out of scope.

### 1. Validator key loading (`node.rs`)

At `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs:148-150`, the 32-byte ed25519 seed is copied into a stack array, used to create a private key, and then the array is dropped without zeroization:

```rust
let mut seed = [0u8; 32];
seed.copy_from_slice(&key_bytes);
Ok(private_key_from_seed(seed))
// `seed` dropped here -- 32 bytes of key material remain on stack
// until overwritten by unrelated function calls
```

### 2. Validator key generation (`node.rs`)

At `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs:154-155`, a freshly generated seed is similarly left on the stack:

```rust
let mut seed = [0u8; 32];
rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
// ... seed written to disk, then used for private_key_from_seed ...
// seed dropped without zeroization
```

### 3. Keygen setup (`setup.rs`)

At `/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs:122-126`:

```rust
let mut seed = [0u8; 32];
rand::rngs::OsRng.fill_bytes(&mut seed);
write_secret_file(&key_path, &seed)?;
private_key_from_seed(seed)
// seed still on stack after function returns, never zeroized
```

The same pattern repeats for loading existing keys at `setup.rs:118-120`:

```rust
let bytes = fs::read(&key_path)?;  // Vec<u8> on heap
let mut seed = [0u8; 32];
seed.copy_from_slice(&bytes);
// `bytes` Vec dropped -- heap memory freed but not zeroed
// `seed` on stack, also not zeroed
```

### 4. DKG protocol (`protocol.rs`)

At `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs:431-440`, the DKG dealer phase handles private key material through several intermediate buffers:

```rust
pub fn start_dealer(&mut self) -> Result<(), DkgError> {
    let mut rng = rand::rngs::OsRng;
    let (dealer, pub_msg, priv_msgs) = Dealer::<MinSig, ed25519::PrivateKey>::start::<N3f1>(
        &mut rng,
        self.info.clone(),
        self.config.identity_key.clone(),  // identity key cloned into intermediate buffer
        None,
    )
    .map_err(|e| DkgError::Crypto(format!("Failed to start dealer: {:?}", e)))?;
    // ... private shares in priv_msgs are not zeroized when dropped
```

### 5. Threshold scheme loading (`scheme.rs`)

At `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs:22-54`, the BLS share secret is loaded from disk and decoded through several intermediate buffers:

```rust
pub fn load_threshold_scheme(data_dir: &Path) -> anyhow::Result<ThresholdScheme> {
    let output = DkgOutput::load(data_dir)?;
    // output.share_secret is Vec<u8> containing the raw share -- never zeroized
    // ...
    let share = Share::read_cfg(&mut output.share_secret.as_slice(), &())
        .map_err(|e| anyhow::anyhow!("failed to decode share: {:?}", e))?;
    // output.share_secret Vec dropped here without zeroization
```

## Code Reference

All code snippets above are taken from the current codebase. The `zeroize` crate does not appear in any `Cargo.toml` in the workspace:

A search for "zeroize" in the codebase yields zero results, confirming it is not used anywhere.

## Impact

Memory dumps, core files, swap files, or cold boot attacks can recover secret key material from process memory even after it is logically no longer needed:

- **Process crash with core dump**: If a crash occurs while key material is on the stack or heap, the core dump will contain the raw secret bytes. Core dumps are disabled in the devnet Docker compose configuration (`ulimits: core: 0`), but this is not guaranteed in all deployment environments.
- **Swap-to-disk under memory pressure**: No `mlock()` is applied to pages containing key material, so the OS can swap them to disk at any time. The swap file persists across reboots.
- **VM live migration**: In cloud environments, memory pages are transferred over the network during live migration, exposing key material to the hypervisor and network infrastructure.
- **Heap reuse**: After `Vec<u8>` buffers containing secrets are dropped, the allocator may reuse that memory for other data, but until it does, the secret bytes remain readable. A subsequent buffer overflow in unrelated code could read the old key material.

The window of exposure is particularly long for:
- The `config.identity_key` field in `DkgConfig` and `DkgParticipant`, which holds the ed25519 private key for the lifetime of the DKG ceremony
- The `share_secret` field in `DkgOutput`, which is held until the output is saved to disk

## Root Cause

The `zeroize` crate was not integrated into the project. All key material is handled using plain Rust arrays (`[u8; 32]`) and vectors (`Vec<u8>`) without any secure cleanup mechanism. Rust's standard library provides no automatic memory sanitization on drop.

## Suggested Fix

1. Add `zeroize` as a dependency to `kora-config`, `keygen`, `kora-dkg`, and `kora-runner`:

```toml
[dependencies]
zeroize = { version = "1", features = ["zeroize_derive"] }
```

2. Use `Zeroizing<[u8; 32]>` for all seed buffers:

```rust
use zeroize::Zeroizing;

// Before:
let mut seed = [0u8; 32];
seed.copy_from_slice(&key_bytes);
Ok(private_key_from_seed(seed))

// After:
let mut seed = Zeroizing::new([0u8; 32]);
seed.copy_from_slice(&key_bytes);
Ok(private_key_from_seed(*seed))
// seed automatically zeroized on drop
```

3. Use `Zeroizing<Vec<u8>>` for key file reads:

```rust
// Before:
let bytes = fs::read(&key_path)?;

// After:
let bytes = Zeroizing::new(fs::read(&key_path)?);
```

4. Apply `#[zeroize(drop)]` derive macro on structs that hold secret material:

```rust
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DkgOutput {
    // ...
    pub share_secret: Vec<u8>,
    // ...
}
```

5. Consider `mlock()` for key material pages to prevent swapping to disk (via the `memsec` or `region` crate).

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs` (lines 148-155) -- ed25519 seed not zeroized (both load and generate paths)
- `/Users/will/dev/nunchi/daeji/crates/node/config/Cargo.toml` -- add `zeroize` dependency
- `/Users/will/dev/nunchi/daeji/bin/keygen/src/setup.rs` (lines 118-127) -- generated and loaded seeds not zeroized
- `/Users/will/dev/nunchi/daeji/bin/keygen/Cargo.toml` -- add `zeroize` dependency
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (lines 431-440) -- DKG dealer start handles identity key clones
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` -- `DkgOutput` struct holds `share_secret: Vec<u8>`
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs` (lines 22-54) -- loaded threshold scheme passes through intermediate buffers

## Related Issues

- `017-dkg-no-encryption-at-rest.md` -- secret keys are also stored as plaintext on disk (complementary to this in-memory issue)
- `166-dkg-state-file-world-readable.md` -- DKG state file has overly permissive permissions
- `004-curve25519-dalek-ng-timing-sidechannel.md` -- related cryptographic security concern
