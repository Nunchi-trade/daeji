# No Integrity Check on DKG Output Files at Validator Startup

**Category**: reliability
**Severity**: medium
**Labels**: `reliability`, `dkg`, `correctness`, `enhancement`

## Summary

When a validator starts, it loads `output.json` and `share.key` from disk and constructs the BLS threshold signing scheme. There is no integrity verification that the secret share actually matches the group public key, that the share index is consistent with the participant list, or that the files have not been tampered with. If either file is corrupted (disk error, accidental overwrite) or tampered with (local privilege escalation), the validator will start successfully but produce invalid threshold signatures, causing silent consensus failures that are extremely difficult to diagnose.

## Problem

The validator startup path in `bin/kora/src/cli.rs` loads DKG output at lines 169-173:

```rust
        let dkg_output = kora_dkg::DkgOutput::load(&config.data_dir)?;
        tracing::info!(share_index = dkg_output.share_index, "Loaded DKG output");

        let scheme = load_threshold_scheme(&config.data_dir)
            .map_err(|e| eyre::eyre!("Failed to load threshold scheme: {}", e))?;
        tracing::info!("Loaded threshold signing scheme");
```

The `load_threshold_scheme()` function in `crates/node/runner/src/scheme.rs` (lines 22-54) performs partial verification:
- It loads the participant keys and creates a `Set<ed25519::PublicKey>`
- It deserializes the `Sharing<MinSig>` polynomial
- It deserializes the `Share`
- It calls `Scheme::signer()` which checks that the share's **public key** matches one of the participants

What it does **not** verify:
1. **Share-to-polynomial consistency**: The share secret key, when evaluated against the group polynomial, should produce the share's public key. This is not checked.
2. **Group key consistency**: The group public key in `output.json` should equal the polynomial evaluation at index 0. This is not checked.
3. **Share index validity**: The share index stored in `share.key` should correspond to this validator's position. The `share_index` from `output.json` is used as `validator_index` at `cli.rs:210`, but there is no check that this matches the share's internal index.
4. **File integrity**: There is no signature, MAC, or checksum on either file.

## Code Reference

Validator startup in `/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs` (lines 163-174):

```rust
        if !kora_dkg::DkgOutput::exists(&config.data_dir) {
            return Err(eyre::eyre!(
                "DKG output not found. Run 'kora dkg' first to generate threshold shares."
            ));
        }

        let dkg_output = kora_dkg::DkgOutput::load(&config.data_dir)?;
        tracing::info!(share_index = dkg_output.share_index, "Loaded DKG output");

        let scheme = load_threshold_scheme(&config.data_dir)
            .map_err(|e| eyre::eyre!("Failed to load threshold scheme: {}", e))?;
        tracing::info!("Loaded threshold signing scheme");
```

`load_threshold_scheme()` in `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs` (lines 22-54):

```rust
pub fn load_threshold_scheme(data_dir: &Path) -> anyhow::Result<ThresholdScheme> {
    let output = DkgOutput::load(data_dir)?;

    let participants: Vec<ed25519::PublicKey> = output
        .participant_keys
        .iter()
        .map(|k| {
            ed25519::PublicKey::read(&mut k.as_slice())
                .map_err(|e| anyhow::anyhow!("failed to decode participant key: {:?}", e))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let n = participants.len();
    let n_cfg =
        NonZeroU32::new(n as u32).ok_or_else(|| anyhow::anyhow!("participants cannot be empty"))?;

    let participants_set = Set::from_iter_dedup(participants);

    let group_poly = Sharing::<MinSig>::read_cfg(
        &mut output.public_polynomial.as_slice(),
        &(n_cfg, ModeVersion::v0()),
    )
    .map_err(|e| anyhow::anyhow!("failed to decode public polynomial: {:?}", e))?;

    let share = Share::read_cfg(&mut output.share_secret.as_slice(), &())
        .map_err(|e| anyhow::anyhow!("failed to decode share: {:?}", e))?;

    let scheme =
        bls12381_threshold::Scheme::signer(SIMPLEX_NAMESPACE, participants_set, group_poly, share)
            .ok_or_else(|| anyhow::anyhow!("failed to create signer: share public key mismatch"))?;

    Ok(scheme)
}
```

`DkgOutput::load()` in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` (lines 76-109):

```rust
    pub fn load(data_dir: &Path) -> Result<Self, DkgError> {
        let output_path = data_dir.join("output.json");
        let output_str = std::fs::read_to_string(&output_path)?;
        let output: OutputJson = serde_json::from_str(&output_str)
            .map_err(|e| DkgError::Serialization(e.to_string()))?;

        let share_path = data_dir.join("share.key");
        let share_str = std::fs::read_to_string(&share_path)?;
        let share: ShareJson =
            serde_json::from_str(&share_str).map_err(|e| DkgError::Serialization(e.to_string()))?;

        // ... hex decoding ...

        // Always compute the correct quorum from N3f1 rather than trusting
        // the persisted threshold value, which may be wrong in old output files.
        let correct_threshold = N3f1::quorum(output.participants);

        Ok(Self {
            // ... fields populated from deserialized JSON ...
        })
    }
```

## Impact

**Silent consensus failure from corrupted or tampered DKG files.** The validator starts and appears healthy, but:

1. **Invalid threshold signatures**: If the share secret is corrupted, the validator produces threshold partial signatures that cannot be combined with other validators' signatures. When this validator is the leader, the block is proposed but never certified (other validators cannot combine the corrupted partial signature). This manifests as nullified blocks only when the corrupted validator leads, making it intermittent and hard to diagnose.

2. **Wrong leader schedule**: If `share_index` in `output.json` is corrupted, the validator thinks it is at a different position in the leader rotation. It may attempt to propose blocks at the wrong time, or miss its actual leader slot.

3. **Group key mismatch**: If `output.json` is overwritten with a different ceremony's output (e.g., from a different chain or an older ceremony), the validator uses the wrong group key. All threshold operations fail, but the error messages reference cryptographic verification failures rather than file corruption.

4. **Difficult debugging**: Because the startup succeeds with no warnings, operators must deduce the corruption from consensus-level symptoms (increased nullification, failed certifications) without any hint that the root cause is corrupted DKG files.

## Root Cause

The startup path assumes DKG output files are correct and uncorrupted. There is no self-check that verifies the loaded share produces valid signatures for the group key. The `Scheme::signer()` call performs a public-key match (ensuring the share's public key is in the participant set) but does not perform a full round-trip signature verification.

## Suggested Fix

Add a self-check at startup that verifies the share can produce valid partial signatures:

```rust
// After loading the threshold scheme in cli.rs:
fn verify_dkg_integrity(scheme: &ThresholdScheme, dkg_output: &DkgOutput) -> eyre::Result<()> {
    use commonware_cryptography::bls12381::primitives::variant::MinSig;

    // 1. Verify share produces valid signatures for the group key
    let test_msg = b"kora_startup_self_check";
    let partial_sig = scheme.sign(test_msg);
    // Note: Full verification requires combining partial signatures from
    // threshold participants, which is not possible with a single share.
    // However, we can verify the partial signature is well-formed.

    // 2. Log key fingerprints for operator cross-verification
    tracing::info!(
        share_index = dkg_output.share_index,
        group_key = hex::encode(&dkg_output.group_public_key[..8]),
        polynomial_len = dkg_output.public_polynomial.len(),
        participants = dkg_output.participants,
        threshold = dkg_output.threshold,
        "DKG integrity check passed"
    );

    Ok(())
}
```

At minimum, log prominent startup information that allows operators to visually cross-verify across nodes:

```rust
tracing::info!(
    share_index = dkg_output.share_index,
    group_key = %hex::encode(&dkg_output.group_public_key),
    participants = dkg_output.participants,
    "DKG output loaded -- verify group_key matches across all validators"
);
```

For stronger guarantees, add a file integrity check:

```rust
// Compute and verify a checksum over output.json + share.key
let mut hasher = Sha256::new();
hasher.update(&output_bytes);
hasher.update(&share_bytes);
let checksum = hasher.finalize();
tracing::info!(checksum = %hex::encode(&checksum[..8]), "DKG file checksum");
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs` -- Add integrity check after DKG loading at line 174
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs` -- Optionally add polynomial consistency check in `load_threshold_scheme()`
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/output.rs` -- Optionally add self-check method to `DkgOutput`

## Related Issues

- Local file `175-dkg-share-index-inconsistency.md` -- Share index may be 1-indexed vs expected 0-indexed (related verification gap)
- Local file `166-dkg-state-file-world-readable.md` -- DKG state file is world-readable (tampering vector)
- Local file `173-dkg-output-json-default-permissions.md` -- `output.json` is world-readable (tampering vector for the file this issue checks)
