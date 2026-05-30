# Issue #28: No Garbage Collection for Dead Storage After SELFDESTRUCT

**Severity**: Low-Medium (P3)
**Component**: `crates/storage/backend`, `crates/storage/qmdb`
**Type**: Resource Leak / Disk Usage

## Summary

When a contract executes the `SELFDESTRUCT` opcode, its storage entries in the QMDB storage partition become permanently unreachable but are never deleted from disk. The generation-based invalidation scheme correctly prevents stale data from being read, but the orphaned entries accumulate indefinitely. On long-running chains with significant contract churn (factory patterns, ephemeral DeFi vaults, NFT minting contracts), this will cause monotonically growing disk usage with no upper bound.

## Background: How Storage Keying Works

QMDB splits state into three partitions: accounts, storage, and code. The storage partition uses 60-byte composite keys to associate each storage slot with a specific "generation" of its owning account:

```
StorageKey = address (20 bytes) + generation (8 bytes) + slot (32 bytes) = 60 bytes
```

This layout is defined in `crates/storage/qmdb/src/encoding.rs`:

```rust
pub struct StorageKey {
    pub address: Address,
    pub generation: u64,
    pub slot: U256,
}

impl StorageKey {
    pub fn to_bytes(&self) -> [u8; 60] {
        let mut buf = [0u8; 60];
        buf[0..20].copy_from_slice(self.address.as_slice());
        buf[20..28].copy_from_slice(&self.generation.to_be_bytes());
        buf[28..60].copy_from_slice(&self.slot.to_be_bytes::<32>());
        buf
    }
}
```

The account partition stores the current generation as the last 8 bytes of the 80-byte account encoding (`crates/storage/qmdb/src/encoding.rs`):

```rust
impl AccountEncoding {
    pub const SIZE: usize = 80;  // nonce(8) + balance(32) + code_hash(32) + generation(8)

    pub fn encode(nonce: u64, balance: U256, code_hash: B256, generation: u64) -> [u8; 80] {
        let mut buf = [0u8; 80];
        buf[0..8].copy_from_slice(&nonce.to_be_bytes());
        buf[8..40].copy_from_slice(&balance.to_be_bytes::<32>());
        buf[40..72].copy_from_slice(code_hash.as_slice());
        buf[72..80].copy_from_slice(&generation.to_be_bytes());
        buf
    }
}
```

## The Generation Bump Mechanism

When a `SELFDESTRUCT` or account recreation occurs, `QmdbStore::build_batches` (in `crates/storage/qmdb/src/store.rs`) increments the generation counter:

```rust
// Increment generation on recreate or selfdestruct to invalidate old storage.
let new_gen = if update.created || update.selfdestructed {
    current_gen.saturating_add(1)
} else {
    current_gen
};
```

All subsequent storage reads and writes for that address use the new generation in their `StorageKey`. Since the old generation number is no longer referenced by anything, the old storage entries become unreachable -- they exist in the QMDB storage partition but no code path will ever construct a key that matches them.

This is an elegant design: it avoids the need to enumerate and delete potentially thousands of storage slots during the `SELFDESTRUCT` operation itself, which would be prohibitively expensive in a consensus-critical path. The tradeoff is that the old data remains on disk.

## The Problem

There is no background process, compaction routine, or any other mechanism that ever removes these orphaned storage entries. The `StorageStore` in `crates/storage/backend/src/storage.rs` only supports reads (`QmdbGettable`) and batch writes (`QmdbBatchable`). There is no scanning, iteration, or deletion-by-prefix capability exposed. The `CommonwareBackend` in `crates/storage/backend/src/backend.rs` has no awareness of generation lifecycle at all.

The result is a one-way ratchet on disk usage: storage space is allocated when contracts write slots, but the corresponding disk space is never reclaimed when those contracts are destroyed.

### Concrete scenario

Consider a factory contract that deploys child contracts, each writing 100 storage slots. If the children are later selfdestructed:

1. Child deployed at address `0xABC...` with generation `0` -- writes 100 storage entries keyed as `(0xABC..., 0, slot_i)`
2. Child selfdestructs -- account entry deleted, generation would be `1` if recreated
3. A new contract is deployed at `0xABC...` with generation `1` -- writes 100 new entries keyed as `(0xABC..., 1, slot_i)`
4. The 100 entries from step 1 remain in the storage partition forever with keys `(0xABC..., 0, slot_i)`

If this cycle repeats N times, the storage partition holds `100 * N` unreachable entries for this single address.

## Impact

- **Disk usage**: Grows without bound on chains with contract churn. Each orphaned slot consumes at least 92 bytes in the storage partition (60-byte key + 32-byte value), plus QMDB overhead (journal entries, Merkle tree nodes). A contract with 10,000 slots that selfdestructs leaves behind roughly 1 MB of dead data, not counting Merkle metadata.

- **Merkle tree bloat**: The orphaned entries remain as leaves in the storage partition's MMR (Merkle Mountain Range). This increases proof sizes and the time required to compute `storage.root()`, even though the entries are semantically dead.

- **Backup and sync cost**: Full node snapshots and state syncs transfer all data in the storage partition, including unreachable entries. This inflates transfer sizes for no benefit.

- **No immediate operational risk**: The generation mechanism guarantees correctness -- orphaned entries cannot be read by any contract. The issue is purely one of wasted resources.

## Proposed Fix

### 1. Add a background garbage collection task

Introduce a `StorageGc` component that runs outside the consensus-critical path:

```rust
// crates/storage/backend/src/gc.rs (new file)

pub struct StorageGc {
    accounts: AccountStore,
    storage: StorageStore,
    batch_size: usize,
    interval: Duration,
}

impl StorageGc {
    /// Scan for accounts where the current generation is > 0,
    /// then probe the storage partition for entries with old generation numbers.
    pub async fn run_cycle(&mut self) -> Result<GcStats, BackendError> {
        // 1. Iterate accounts partition for entries with generation > 0
        // 2. For each such account, probe storage keys with generation < current_gen
        // 3. Batch-delete orphaned entries
        // 4. Return stats (entries cleaned, bytes reclaimed)
        todo!()
    }
}
```

### 2. Expose iteration/scanning on the storage partition

The current `StorageStore` only supports point reads via `QmdbGettable::get()`. GC requires the ability to scan keys by prefix (address) or iterate ranges. This likely requires changes to the underlying `commonware-storage` layer or a new trait:

```rust
// In crates/storage/qmdb/src/traits.rs
pub trait QmdbScannable {
    type Key;
    type Value;
    type Error: std::error::Error;

    /// Iterate all keys matching a prefix.
    async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Self::Key, Self::Value)>, Self::Error>;
}
```

This is the most significant prerequisite -- if `commonware-storage`'s QMDB does not support range iteration, this feature may require upstream changes.

### 3. Track generation history in the accounts partition

Currently, when an account selfdestructs, its entry is deleted from the accounts partition entirely (`batches.accounts.push((*address, None))`). This means we lose knowledge of which addresses have ever had elevated generations. Two possible approaches:

- **Tombstone approach**: Instead of deleting the account entry on selfdestruct, write a tombstone value that preserves the generation number. The GC can then scan for tombstones.
- **Separate GC index**: Maintain a lightweight index (address -> last known generation) that the GC consults. This avoids changing the accounts partition schema.

### 4. Spawn the GC task from the runner

In `crates/node/runner/src/runner.rs`, spawn the GC as a background task alongside other observer tasks:

```rust
// After state initialization, before consensus engine start
if gc_enabled {
    let gc = StorageGc::new(
        state.accounts_store(),
        state.storage_store(),
        GcConfig { batch_size: 1000, interval: Duration::from_secs(300) },
    );
    context.spawn(async move {
        loop {
            tokio::time::sleep(gc.interval).await;
            match gc.run_cycle().await {
                Ok(stats) => info!(entries = stats.cleaned, bytes = stats.reclaimed, "storage GC cycle"),
                Err(e) => warn!(error = %e, "storage GC cycle failed"),
            }
        }
    });
}
```

### 5. Export GC metrics

Track and expose GC activity through the existing Commonware metrics infrastructure:

- `kora_storage_gc_entries_cleaned_total` (counter)
- `kora_storage_gc_bytes_reclaimed_total` (counter)
- `kora_storage_gc_cycle_duration_seconds` (histogram)
- `kora_storage_gc_orphaned_entries_remaining` (gauge)

## Files to Modify

| File | Change |
|------|--------|
| `crates/storage/backend/src/backend.rs` | Add GC scanning logic, expose store references for GC |
| `crates/storage/backend/src/storage.rs` | Add prefix scanning and batch deletion capabilities |
| `crates/storage/backend/src/accounts.rs` | Track generation history or support tombstone reads |
| `crates/storage/backend/src/gc.rs` (new) | Core GC implementation |
| `crates/storage/backend/src/lib.rs` | Export GC module |
| `crates/storage/qmdb/src/traits.rs` | Add `QmdbScannable` trait if needed |
| `crates/storage/qmdb/src/store.rs` | Support GC-related queries (generation lookups) |
| `crates/node/runner/src/runner.rs` | Spawn background GC task |

## Testing Plan

### Unit tests

1. **Orphan detection**: Create a contract, write storage slots at generation 0, simulate selfdestruct (bump generation to 1), verify the old storage entries are still present in the storage partition via raw key lookup.

2. **GC cleans orphans**: After the orphan detection test setup, run a GC cycle and verify that entries with generation 0 are removed from the storage partition.

3. **GC preserves live storage**: Write storage at generation 0 for an account whose current generation is 0. Run GC. Verify the entries are NOT deleted.

4. **GC handles re-creation**: Address at generation 2 means generations 0 and 1 are dead. Verify GC cleans both.

5. **Batch size limits**: Verify GC respects its configured batch size and does not attempt to delete all orphans in a single operation.

### Integration tests

6. **State root invariance**: Run a full execution sequence (deploy, write, selfdestruct, redeploy), compute state root, run GC, recompute state root. The roots must be identical -- orphaned entries do not contribute to the authenticated state root because they are unreachable through the current generation, but deleting them from the underlying QMDB partition will change the Merkle root. This needs careful design: either GC must not modify the Merkle tree, or the state root computation must be adjusted.

    > **Note**: This is actually the hardest part of the design. The QMDB storage partition maintains an MMR whose root is part of the composite `StateRoot`. Deleting entries from the partition WILL change the Merkle root. This means either:
    > - (a) GC is only safe after a state root checkpoint, and the new root after GC becomes the canonical root, or
    > - (b) Deleted entries are replaced with zero-value tombstones that preserve the Merkle structure, or
    > - (c) The state root is computed differently to exclude generation-stale entries.
    >
    > This constraint may significantly complicate the implementation and deserves its own design discussion.

7. **Concurrent safety**: Verify that GC running concurrently with block execution does not cause data races or inconsistent reads. The `StoreSlot` pattern in `crates/storage/backend/src/types.rs` wraps an `Option<T>` that is moved via `take()`/`restore()` during batch writes, so GC would need its own store instance or coordinate through this existing ownership protocol to avoid conflicts.

### Property tests

8. **GC never deletes reachable storage**: For any sequence of (deploy, write, selfdestruct, redeploy) operations, GC must never delete a storage entry whose generation matches the current account generation. Use `proptest` or `quickcheck` to generate random operation sequences.

## Open Questions

1. **Merkle root impact**: As noted in testing item 6, deleting orphaned entries from the storage partition will change the MMR root. How should this be handled? Is a "GC epoch" concept needed where the post-GC root becomes canonical?

2. **Iteration support**: Does `commonware-storage`'s QMDB implementation support key-range iteration? If not, this feature is blocked on upstream work. An alternative would be maintaining a separate index of addresses with elevated generations, but this adds its own storage overhead.

3. **Coordination with consensus**: Should GC only run on non-leader nodes to avoid impacting block production latency? Or is it safe to run on all nodes given sufficient batching?

4. **Code partition**: The same generation-based orphaning could theoretically apply to the code partition if a selfdestructed contract's code hash is unique. However, code is keyed by hash (`B256`), not by address+generation, so multiple contracts can share the same code. Code GC would require reference counting, which is a separate concern.

5. **Priority relative to other issues**: Given this is P3, it should be addressed after crash-consistency (#8), transaction gossip (#12), and other higher-priority items. However, for chains planning to run for months or years, this should be on the roadmap.
