# Every SLOAD Opcode Triggers Two Disk Reads Due to Account Generation Lookup

**Category**: Storage / Performance
**Severity**: Medium

## Summary

Every EVM `SLOAD` (storage read) operation triggers two disk reads: one to fetch the full account record to extract the `generation` number, and a second to read the actual storage slot using a key that includes the generation. The generation number rarely changes (only on `CREATE`/`SELFDESTRUCT`), making it an ideal candidate for in-memory caching. Storage-heavy contracts (DEX AMMs, lending protocols, token balances) issue many `SLOAD` operations per transaction, so this double-read penalty is a significant I/O bottleneck.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs`, both the synchronous `storage_ref()` (lines 116-127) and async `storage_async_ref()` (lines 180-195) implementations follow the same two-read pattern:

1. Read the full account record from QMDB to extract the `generation` number
2. Construct a `StorageKey` that includes `(address, generation, slot_index)`
3. Read the actual storage slot value using the constructed key

This means every `SLOAD` opcode during EVM execution triggers two separate QMDB lookups.

The synchronous path (`DatabaseRef for QmdbHandle`):

```rust
// crates/storage/handlers/src/adapter.rs:116-127
fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
    let store = block_on(self.read());

    // Get account to find generation
    let generation = match block_on(store.get_account(&address))? {  // READ 1: full account
        Some((_, _, _, generation)) => generation,
        None => return Ok(U256::ZERO),
    };

    let key = StorageKey::new(address, generation, index);
    Ok(block_on(store.get_storage(&key))?.unwrap_or(U256::ZERO))    // READ 2: storage slot
}
```

The async path (`DatabaseAsyncRef for QmdbHandle`):

```rust
// crates/storage/handlers/src/adapter.rs:180-195
fn storage_async_ref(
    &self,
    address: Address,
    index: U256,
) -> impl std::future::Future<Output = Result<U256, Self::Error>> + Send {
    let handle = self.clone();
    async move {
        let store = handle.read().await;
        let generation = match store.get_account(&address).await? {  // READ 1: full account
            Some((_, _, _, generation)) => generation,
            None => return Ok(U256::ZERO),
        };
        let key = StorageKey::new(address, generation, index);
        Ok(store.get_storage(&key).await?.unwrap_or(U256::ZERO))    // READ 2: storage slot
    }
}
```

## Impact

- **2x I/O amplification on every SLOAD**: Every storage read costs two disk lookups instead of one. The `SLOAD` opcode is one of the most frequently executed operations in smart contracts.
- **Throughput bottleneck for DeFi workloads**: Storage-heavy contracts (Uniswap V3 pools, Aave lending, ERC-20 token balances) issue dozens of `SLOAD` operations per transaction. A single Uniswap swap may trigger 10+ storage reads, meaning 20+ disk reads instead of 10+.
- **Quantified overhead**: At 33 blocks/s with typical DeFi workloads of ~50 storage reads per block, this represents approximately 1,650 unnecessary disk reads per second. Under high-throughput conditions with hundreds of contract calls per block, the overhead could reach tens of thousands of unnecessary reads per second.
- **Cache-friendly optimization opportunity**: The `generation` number for a given address changes only on `CREATE` or `SELFDESTRUCT`, which are rare operations (typically <0.1% of transactions). An in-memory cache of `Address -> generation` would have a near-100% hit rate and eliminate the first read in almost all cases.

## Root Cause

The QMDB storage key design uses a `(address, generation, slot_index)` composite key to handle account recreation. When an account is self-destructed and then recreated at the same address (e.g., via `SELFDESTRUCT` followed by `CREATE2`), a new generation number invalidates all old storage entries without needing to delete them individually. This is correct for write semantics, but the generation number is stored only in the account record, forcing an account lookup before every storage read.

There is no in-memory cache of `Address -> generation` mappings in `QmdbHandle`.

## Suggested Fix

**Option A (recommended)** -- Add a per-block cache of `Address -> generation` within `QmdbHandle`:

```rust
// In QmdbHandle (or a wrapper):
struct QmdbHandle<A, S, C> {
    // ... existing fields ...
    generation_cache: parking_lot::RwLock<HashMap<Address, u64>>,
}

fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
    let store = block_on(self.read());

    // Check cache first
    let generation = if let Some(&gen) = self.generation_cache.read().get(&address) {
        gen
    } else {
        // Cache miss: read from QMDB and populate cache
        let gen = match block_on(store.get_account(&address))? {
            Some((_, _, _, generation)) => generation,
            None => return Ok(U256::ZERO),
        };
        self.generation_cache.write().insert(address, gen);
        gen
    };

    let key = StorageKey::new(address, generation, index);
    Ok(block_on(store.get_storage(&key))?.unwrap_or(U256::ZERO))
}
```

Invalidate the cache entry when `created || selfdestructed` during `DatabaseCommit::commit()`.

**Option B** -- Pre-populate the generation from `basic_ref()`. Since REVM always calls `basic_ref()` for an account before accessing its storage (to check if the account exists and load its code hash), the generation can be cached as a side effect of `basic_ref()`:

```rust
fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
    let store = block_on(self.read());
    match block_on(store.get_account(&address))? {
        Some((nonce, balance, code_hash, generation)) => {
            // Cache generation for subsequent storage_ref calls
            self.generation_cache.write().insert(address, generation);
            Ok(Some(AccountInfo { nonce, balance, code_hash, code: None, account_id: None }))
        }
        None => Ok(None),
    }
}
```

This approach requires no additional reads and piggybacks on the already-required `basic_ref()` call.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (lines 116-127) -- `storage_ref()` method
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (lines 180-195) -- `storage_async_ref()` method
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (lines 91-103) -- `basic_ref()` method (if using Option B)
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (lines 205-258) -- `DatabaseCommit::commit()` for cache invalidation

## Related Issues

- `086-executor-sequential-async-reads.md` -- another storage read optimization; both address I/O amplification in the execution hot path

## Labels

performance, storage
