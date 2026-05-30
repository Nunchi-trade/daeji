# 030: Stale Storage Slots Not Cleaned on Account Self-Destruct -- Unbounded Disk Growth

**Category**: bug
**Severity**: medium
**Labels**: bug, storage, performance

---

## Summary

When an EVM account is self-destructed and recreated (via the `SELFDESTRUCT` opcode or `CREATE2` to the same address), QMDB increments the account's "generation" counter to logically isolate old storage entries from new ones. While this correctly prevents stale reads (old storage values are never returned for the new account), the old storage entries with the previous generation remain on disk indefinitely. Over time, repeated self-destruct/recreate cycles on the same address accumulate orphaned storage entries that can never be accessed but consume disk space.

---

## Problem

QMDB uses a generation-based key scheme for storage: each storage slot is keyed as `StorageKey(address, generation, slot)`. When an account is self-destructed, the generation counter is incremented (lines 281-285 of `store.rs`), which means all subsequent storage reads use a new key prefix and never see old values.

The correctness of this scheme is verified across multiple layers:

1. **Overlay layer** (`overlay.rs` line 124): Returns `U256::ZERO` for any storage read on a self-destructed account, bypassing the base store entirely.
2. **QMDB store** (`store.rs` lines 281-285): Increments the account's `generation` on self-destruct, making old `StorageKey(addr, old_gen, slot)` entries inaccessible.
3. **ChangeSet merge** (`changes.rs` lines 81-83): Clears accumulated storage changes when the `selfdestructed` flag is set.

However, the old `(address, old_gen, slot)` entries are never deleted from the backing store. They remain on disk consuming space but are unreachable via the current generation.

**File**: `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs`

---

## Code Reference

The generation increment on self-destruct/recreate in `store.rs` (lines 270-313):

```rust
// crates/storage/qmdb/src/store.rs:280-288
// Increment generation on recreate or selfdestruct to invalidate old storage.
let new_gen = if update.created || update.selfdestructed {
    current_gen.saturating_add(1)
} else {
    current_gen
};

if update.selfdestructed {
    batches.accounts.push((*address, None));
}
```

Storage key construction with generation (lines 305-312 of `store.rs`):

```rust
// crates/storage/qmdb/src/store.rs:304-312
// Add storage changes
for (slot, value) in &update.storage {
    let key = StorageKey::new(*address, new_gen, *slot);
    if value.is_zero() {
        batches.storage.push((key, None));
    } else {
        batches.storage.push((key, Some(*value)));
    }
}
```

The overlay's self-destruct handling (lines 123-126 of `overlay.rs`):

```rust
// crates/storage/overlay/src/overlay.rs:123-126
if let Some(update) = changes.accounts.get(&address) {
    if update.selfdestructed {
        return Ok(U256::ZERO);
    }
```

The ChangeSet merge clearing storage on self-destruct (lines 81-83 of `changes.rs`):

```rust
// crates/storage/qmdb/src/changes.rs:81-83
if selfdestructed {
    self.storage.clear();
}
```

The self-destructed address collection in the executor (lines 450-456 of `revm.rs`):

```rust
// crates/node/executor/src/revm.rs:450-456
// Collect addresses that were selfdestructed in this transaction.
// Their storage entries in QMDB become orphaned and need future GC.
for (address, account) in &evm_state {
    if account.is_selfdestructed() {
        outcome.selfdestructed_addresses.push(*address);
    }
}
```

---

## Impact

1. **EVM correctness is NOT affected**: The generation-based isolation ensures that storage reads for a new account at `(address, new_gen, slot)` never return old values from `(address, old_gen, slot)`. All validators use the same generation-increment mechanism, so there is no state divergence.

2. **Unbounded disk growth**: Each self-destruct/recreate cycle on the same address leaves behind all storage entries under the old generation. If an address had N storage slots and was self-destructed K times, the disk retains N * K orphaned entries (minus any from the most recent generation which are still live).

3. **Potential for malicious exploitation**: A contract could be designed to repeatedly self-destruct and recreate via `CREATE2` with many storage slots to intentionally inflate disk usage. Each cycle would leave thousands of orphaned entries. While the gas cost of this attack is non-trivial, the disk impact is permanent since there is no garbage collection.

4. **Practical rarity**: Self-destruct is deprecated in recent Ethereum EIPs (EIP-6049, EIP-6780 in Dencun), and most contracts do not self-destruct. The practical risk is low for typical usage, which is why this issue is rated medium severity.

---

## Root Cause

QMDB uses a generation counter to logically isolate storage across self-destruct/recreate cycles. When the generation is incremented, old entries become unreachable but are not deleted from the backing store. There is no garbage collection mechanism to identify and remove entries from previous generations.

---

## Suggested Fix

Add a background garbage collection mechanism to clean up orphaned storage entries:

1. **Propagate self-destructed addresses to the persistence layer**: The executor already collects `selfdestructed_addresses` (lines 450-456 of `revm.rs`). This list should be passed through to the QMDB persistence layer.

2. **Background GC task**: After a block with self-destructed accounts is finalized, enumerate and delete storage entries under old generation keys:

```rust
// After selfdestruct finalization:
for address in &selfdestructed_addresses {
    let current_gen = store.get_generation(address);
    // Delete all storage entries with generation < current_gen
    if current_gen > 0 {
        store.delete_storage_by_generation(address, current_gen - 1);
    }
}
```

3. **Lazy GC alternative**: If enumerating storage entries is expensive, add a periodic background scan that identifies storage entries with generations below the current generation and removes them in batches.

4. **Track orphaned entry count as a metric**: Expose a gauge metric so operators can monitor disk growth from orphaned entries.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs` -- Add `delete_storage_by_generation()` method and GC integration (around line 313)
- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` -- Ensure `selfdestructed_addresses` is propagated to the persistence layer (already collected at line 450)
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- Wire the GC task into the finalization reporter pipeline

---

## Related Issues

- `022-database-commit-swallows-errors.md` (storage layer reliability)
