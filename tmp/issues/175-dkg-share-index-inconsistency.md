# Trusted Dealer and Interactive DKG Share Index Inconsistency

**Category**: correctness
**Severity**: medium
**Labels**: `correctness`, `dkg`, `bug`

## Summary

The trusted dealer DKG path uses `share.index.get()` (which returns the inner value of a `NonZeroU32`, inherently 1-indexed) to write the share index into `share.key`. The interactive DKG path uses `usize::from(share.index) as u32` (which for `NonZeroU32` also produces the 1-indexed inner value). The project's memory document states "Both DKG modes produce 0-indexed share indices" and "validator_index should equal share_index directly (no subtraction needed)." If the downstream consumer (the threshold signing scheme) expects 0-indexed share indices but receives 1-indexed values, validators would use incorrect shares and produce invalid threshold signatures.

## Problem

There are two code paths that produce DKG share indices for persistence:

1. **Trusted dealer** (`bin/keygen/src/dkg_deal.rs` line 130): Uses `share.index.get()` on a `NonZeroU32`. The `.get()` method returns the inner `u32` value, which for `NonZeroU32` is always >= 1. This means the first share has index 1, not 0.

   ```rust
   let share_json = ShareJson { index: share.index.get(), secret: hex::encode(&share_bytes) };
   ```

2. **Interactive DKG** (`crates/node/dkg/src/protocol.rs` line 873): Uses `usize::from(share.index) as u32`. The `From<NonZeroU32>` for `usize` returns the inner value, which is also 1-indexed.

   ```rust
   share_index: usize::from(share.index) as u32,
   ```

The downstream consumer is `load_threshold_scheme()` at `crates/node/runner/src/scheme.rs` (lines 22-54), which loads the share via `Share::read_cfg()` and passes it directly to `bls12381_threshold::Scheme::signer()`:

```rust
    let share = Share::read_cfg(&mut output.share_secret.as_slice(), &())
        .map_err(|e| anyhow::anyhow!("failed to decode share: {:?}", e))?;

    let scheme =
        bls12381_threshold::Scheme::signer(SIMPLEX_NAMESPACE, participants_set, group_poly, share)
            .ok_or_else(|| anyhow::anyhow!("failed to create signer: share public key mismatch"))?;
```

The `Share` type from commonware preserves the original index from serialization. The `Scheme::signer()` method matches the share's public key against the participant set -- this is based on the public key, not the index, so it works regardless of indexing convention. However, the `share_index` field written to `output.json` via `DkgOutput.share_index` is used at `cli.rs:210` for `validator_index`:

```rust
        let validator_index = dkg_output.share_index;
        if validator_index >= validator_count {
            return Err(eyre::eyre!(
                "DKG share_index ({validator_index}) must be less than participant count ({validator_count})"
            ));
        }
```

If `share_index` is 1-indexed (values 1 through N) and `validator_count` is N, then the last validator has `share_index == N` which equals `validator_count`, failing this bounds check.

## Code Reference

Trusted dealer share index in `/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs` (line 130):

```rust
        let share_json = ShareJson { index: share.index.get(), secret: hex::encode(&share_bytes) };
```

Interactive DKG share index in `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` (line 873):

```rust
            share_index: usize::from(share.index) as u32,
```

Scheme loading in `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs` (lines 46-51):

```rust
    let share = Share::read_cfg(&mut output.share_secret.as_slice(), &())
        .map_err(|e| anyhow::anyhow!("failed to decode share: {:?}", e))?;

    let scheme =
        bls12381_threshold::Scheme::signer(SIMPLEX_NAMESPACE, participants_set, group_poly, share)
            .ok_or_else(|| anyhow::anyhow!("failed to create signer: share public key mismatch"))?;
```

Validator startup bounds check in `/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs` (lines 210-215):

```rust
        let validator_index = dkg_output.share_index;
        if validator_index >= validator_count {
            return Err(eyre::eyre!(
                "DKG share_index ({validator_index}) must be less than participant count ({validator_count})"
            ));
        }
```

## Impact

**Potential consensus failure or startup crash due to share index misalignment.**

1. **Startup crash**: If the commonware DKG library produces shares with 1-indexed `NonZeroU32` indices, the last validator in a set of N validators has `share_index == N`. The bounds check at `cli.rs:211` (`validator_index >= validator_count`) would reject this, preventing the last validator from starting.

2. **Silent index mismatch**: If the bounds check passes but the index is off-by-one from what the simplex consensus expects, the validator's leader schedule position would be wrong, causing it to propose at incorrect times or fail to produce valid threshold signatures when it should be signing.

3. **Cross-mode inconsistency**: If one deployment uses the trusted dealer and another uses interactive DKG, and they produce different index conventions, validators cannot be interchangeably started with shares from either mode.

Note: The project memory states these produce 0-indexed values, which may indicate a fix was applied at the commonware library level (where `NonZeroU32(1)` maps to logical index 0 via a convention), or that the documentation is aspirational rather than current.

## Root Cause

`NonZeroU32::get()` returns the inner u32 value, which is >= 1 by definition (NonZeroU32 cannot hold 0). The documented convention of 0-indexed shares may not match the actual values produced by the commonware DKG library's `Share.index` field. The two code paths (`get()` vs `usize::from()`) happen to produce the same values but for different reasons, and neither explicitly converts from 1-based to 0-based indexing.

## Suggested Fix

1. **Add an explicit test** that validates the share index from both DKG paths matches what the downstream consumers expect:

```rust
#[test]
fn share_index_is_zero_based() {
    // After running trusted dealer DKG with 4 participants:
    // share indices should be 0, 1, 2, 3 (not 1, 2, 3, 4)
    for (i, share) in shares.iter().enumerate() {
        assert_eq!(share.index.get() as usize, i,
            "Share index should be 0-based, got {} for participant {}", share.index.get(), i);
    }
}
```

2. **Add an explicit conversion function** that documents the indexing convention:

```rust
/// Convert a commonware DKG share index to a 0-based validator index.
/// The commonware library uses NonZeroU32 (1-indexed), but our convention is 0-indexed.
fn share_index_to_validator_index(share_index: NonZeroU32) -> u32 {
    share_index.get() - 1  // Convert from 1-based to 0-based
}
```

3. **Add a startup assertion** in `scheme.rs` that verifies the loaded share index is within the expected range:

```rust
assert!(dkg_output.share_index < dkg_output.participants as u32,
    "Share index {} must be less than participant count {}",
    dkg_output.share_index, dkg_output.participants);
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/bin/keygen/src/dkg_deal.rs` -- Verify share index convention at line 130
- `/Users/will/dev/nunchi/daeji/crates/node/dkg/src/protocol.rs` -- Verify share index convention at line 873
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/scheme.rs` -- Add share index validation
- `/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs` -- Verify bounds check at line 211 is consistent with indexing convention

## Related Issues

- Local file `178-no-dkg-output-integrity-check-at-startup.md` -- No integrity check on DKG output at startup (related verification gap)
