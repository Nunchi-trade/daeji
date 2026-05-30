# DKG share_index is 0-indexed but validator startup assumed 1-indexed

**Severity:** P1 / High
**Component:** `bin/kora/src/cli.rs`, `bin/keygen/src/dkg_deal.rs`, `crates/node/dkg/src/output.rs`
**Status:** Fixed (2026-05-22)

---

## Summary

The trusted dealer DKG (`keygen dkg-deal`) produces 0-indexed share indices (0, 1, 2, 3 for 4 validators), but the validator startup code in `cli.rs` assumed shares were 1-indexed and performed `checked_sub(1)` to convert to a 0-based validator index. This caused any validator receiving share_index=0 to crash on startup with:

```
Error: DKG share_index is 0 but must be >= 1 (1-indexed)
Location: bin/kora/src/cli.rs:174:28
```

---

## Root Cause

In `dkg_deal.rs:129`, the trusted dealer writes share indices using:

```rust
let share_json = ShareJson { index: share.index.get(), secret: hex::encode(&share_bytes) };
```

The `share.index.get()` returns the raw BLS library index, which is 0-based. With 4 validators, the shares are assigned indices {0, 1, 2, 3} based on the sorted order of participant public keys (via `Set`), NOT the node number order.

In `cli.rs:170-174`, the validator startup code did:

```rust
// share_index from DKG is 1-indexed; convert to 0-based for leader election.
let validator_index = dkg_output
    .share_index
    .checked_sub(1)
    .ok_or_else(|| eyre::eyre!("DKG share_index is 0 but must be >= 1 (1-indexed)"))?;
```

The comment was wrong — shares are 0-indexed, not 1-indexed.

---

## Impact

- One validator (whichever gets share_index=0) crash-loops on startup
- The remaining 3 validators start but with incorrect validator indices (off by 1), causing wrong leader election assignments
- In a 4-validator cluster with 3/4 quorum requirement, losing one validator means zero margin for any other failure

### Observed on remote devnet (65.21.232.29):

| Node | share_index | validator_index (buggy) | validator_index (correct) |
|------|-------------|------------------------|--------------------------|
| node0 | 2 | 1 | 2 |
| node1 | 3 | 2 | 3 |
| node2 | 0 | CRASH | 0 |
| node3 | 1 | 0 | 1 |

---

## Fix Applied

**`cli.rs`**: Replaced `checked_sub(1)` with direct use of `share_index` plus bounds validation:

```rust
let validator_index = dkg_output.share_index;
if validator_index >= validator_count {
    return Err(eyre::eyre!(
        "DKG share_index ({validator_index}) must be less than participant count ({validator_count})"
    ));
}
```

**`output.rs`**: Updated doc comment from "1-indexed" to "0-indexed".

Both the trusted dealer (`dkg_deal.rs`) and interactive DKG (`protocol.rs:874`) produce 0-indexed shares, so this fix applies to both code paths.

---

## Files Modified

| File | Change |
|------|--------|
| `bin/kora/src/cli.rs` | Removed `checked_sub(1)`, use `share_index` directly with bounds check |
| `crates/node/dkg/src/output.rs` | Updated doc comment: "1-indexed" → "0-indexed" |
