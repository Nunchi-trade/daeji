# 141: AccountUpdate Merge Loses Created/Selfdestructed Flag Interaction -- Ghost Storage Slots

**Category**: bug
**Severity**: high
**Component**: storage / correctness

## Summary

When `AccountUpdate::merge()` combines two updates where the account was first created and then selfdestructed within the same merged changeset, both `created=true` and `selfdestructed=true` end up set simultaneously. The downstream `build_batches()` method only increments the storage generation counter by 1 regardless of how many lifecycle transitions occurred. This can cause storage slots written between the CREATE2 and SELFDESTRUCT to survive as ghost entries under the new generation, producing incorrect state reads for future deployments at the same address.

## Problem

The `AccountUpdate::merge()` method in `crates/storage/qmdb/src/changes.rs` handles the `created` and `selfdestructed` flags as independent booleans. When `other.created` is true, it clears storage and sets `self.created = true`. When `other.selfdestructed` is true, it clears storage and overwrites `self.selfdestructed` with the new value. However, when a create is followed by a selfdestruct in subsequent merged updates, both flags end up true simultaneously:

1. First merge: `other.created = true` -> sets `self.created = true`, clears storage
2. Between merges: storage slots are written to `self.storage`
3. Second merge: `other.selfdestructed = true` -> sets `self.selfdestructed = true`, clears storage

After both merges, `self.created == true` AND `self.selfdestructed == true`.

The `build_batches()` method in `crates/storage/qmdb/src/store.rs` (line 281) uses a single condition:

```rust
let new_gen = if update.created || update.selfdestructed {
    current_gen.saturating_add(1)
} else {
    current_gen
};
```

This adds only 1 to the generation, but two lifecycle transitions occurred (create + selfdestruct), each of which should bump the generation to invalidate old storage. With only a single bump, the old storage from before the CREATE (keyed at `current_gen`) and any storage written between create and selfdestruct (keyed at `current_gen + 1` due to the create) may collide with future deployments.

**File**: `crates/storage/qmdb/src/changes.rs` (lines 73-99), `crates/storage/qmdb/src/store.rs` (lines 281-285)

## Code Reference

```rust
// crates/storage/qmdb/src/changes.rs:73-99
impl AccountUpdate {
    /// Merge another update into this one.
    pub fn merge(&mut self, other: Self) {
        let Self { created, selfdestructed, nonce, balance, code_hash, code, storage } = other;

        if created {
            self.storage.clear();
            self.created = true;
        }

        if selfdestructed {
            self.storage.clear();
        }

        self.selfdestructed = selfdestructed;  // <-- does NOT clear self.created
        self.nonce = nonce;
        self.balance = balance;

        if self.code_hash != code_hash || code.is_some() {
            self.code = code;
        }
        self.code_hash = code_hash;

        if !selfdestructed {
            for (slot, value) in storage {
                self.storage.insert(slot, value);
            }
        }
    }
}
```

```rust
// crates/storage/qmdb/src/store.rs:281-285
// Increment generation on recreate or selfdestruct to invalidate old storage.
let new_gen = if update.created || update.selfdestructed {
    current_gen.saturating_add(1)  // <-- only +1 even if both flags are true
} else {
    current_gen
};
```

## Impact

**Scenario**: A contract factory uses CREATE2 to deploy a contract, which then calls SELFDESTRUCT within the same block (or within a merged changeset spanning multiple unfinalized blocks).

1. Before the block: account at address X has generation `G`, with storage slots keyed at generation `G`.
2. CREATE2 deploys to address X: `created = true`, storage cleared, new storage written at generation `G+1`.
3. SELFDESTRUCT on address X: `selfdestructed = true`, storage cleared.
4. After merge: both `created = true` and `selfdestructed = true`. Generation bumps from `G` to `G+1` (only one bump).
5. Future CREATE2 at address X: starts with generation `G+1` -- the same generation that was used for storage between create and selfdestruct. Ghost storage slots from step 2 may still exist in QMDB under generation `G+1`, causing incorrect state reads.

This is a correctness bug that affects the EVM's CREATE2 + SELFDESTRUCT interaction, which is used by patterns like metamorphic contracts.

## Root Cause

The merge logic treats `created` and `selfdestructed` as independent boolean flags, but the downstream `build_batches()` collapses both flags into a single generation increment. The merge does not clear `self.created` when `selfdestructed` is applied, and `build_batches()` does not count the number of lifecycle transitions.

## Suggested Fix

**Option A**: Clear `created` on selfdestruct during merge (simplest fix):

```rust
// In AccountUpdate::merge():
if selfdestructed {
    self.storage.clear();
    self.created = false;  // selfdestruct supersedes create
}
```

**Option B**: Count generation bumps in `build_batches()`:

```rust
let mut gen_bumps = 0u64;
if update.created { gen_bumps += 1; }
if update.selfdestructed { gen_bumps += 1; }
let new_gen = current_gen.saturating_add(gen_bumps);
```

Option A is preferred because it correctly models the semantic: if an account is created and then selfdestructed in the same changeset, the net effect is equivalent to a selfdestruct (the account does not exist at the end of the block).

## Files to Modify

- `crates/storage/qmdb/src/changes.rs` -- fix `AccountUpdate::merge()` to handle created+selfdestructed interaction
- `crates/storage/qmdb/src/store.rs` -- optionally fix `build_batches()` to count generation bumps separately

## Related Issues

- [030-stale-storage-slots-unbounded-disk.md](./030-stale-storage-slots-unbounded-disk.md) -- stale storage slots on account recreation, related root cause

## Labels

`bug`, `correctness`, `storage`
