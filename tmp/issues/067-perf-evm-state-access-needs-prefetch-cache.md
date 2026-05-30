# EVM State Access Needs Prefetch Cache for Contract-Heavy Workloads

**Category**: performance -- executor
**Severity**: medium

**Labels**: `performance`, `executor`, `storage`

---

## Summary

REVM's `DatabaseRef` trait is synchronous, so every state access during EVM execution goes through a `block_on()` bridge that converts async reads to synchronous calls. Each storage read (`SLOAD`) triggers a full async-to-sync context switch. For contract-heavy workloads (DEX swaps, multi-hop routes), the overhead of hundreds of individual `block_on` calls per block becomes a significant fraction of total block production time. A prefetch cache that pre-loads expected state before EVM execution would eliminate most of this overhead.

---

## Problem

Kora's EVM execution uses two adapter layers that bridge REVM's synchronous `DatabaseRef` trait to the asynchronous QMDB storage backend:

1. **StateDbAdapter** (used with overlay state during consensus): Uses `tokio::task::block_in_place` + `handle.block_on` for each state access. Each `storage_ref` call delegates to a single `block_on` call wrapping the async `storage()` method.

2. **QmdbHandle DatabaseRef impl** (used with direct QMDB access): Uses `futures::executor::block_on` and makes **two** sequential `block_on` calls per `storage_ref` -- one for the account lookup (to get the generation number) and one for the storage value.

For the overlay adapter, the overhead is lower because `block_in_place` is a no-op on `spawn_blocking` threads (tokio 1.28+). But for the QMDB handler, each `SLOAD` requires two full `block_on` roundtrips.

---

## Code Reference

**StateDbAdapter block_on bridge** -- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/adapter.rs:32-40`:
```rust
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current()
        && handle.runtime_flavor() == RuntimeFlavor::MultiThread
    {
        return tokio::task::block_in_place(|| handle.block_on(f));
    }

    futures::executor::block_on(f)
}
```

**StateDbAdapter storage_ref** -- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/adapter.rs:92-98`:
```rust
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match block_on(self.state.storage(&address, &index)) {
            Ok(value) => Ok(value),
            Err(StateDbError::AccountNotFound(_)) => Ok(U256::ZERO),
            Err(e) => Err(e.into()),
        }
    }
```

**QmdbHandle storage_ref with double block_on** -- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs:116-127`:
```rust
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let store = block_on(self.read());            // block_on #1: acquire read handle

        // Get account to find generation
        let generation = match block_on(store.get_account(&address))? {  // block_on #2: account lookup
            Some((_, _, _, generation)) => generation,
            None => return Ok(U256::ZERO),
        };

        let key = StorageKey::new(address, generation, index);
        Ok(block_on(store.get_storage(&key))?.unwrap_or(U256::ZERO))  // block_on #3: storage read
    }
```

**EVM execution is run on spawn_blocking** -- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:355-358`:
```rust
            match tokio::task::spawn_blocking(move || {
                executor.execute(&state, &context, &txs_bytes)
            })
            .await
```

---

## Impact

**For simple transfer workloads** (the current devnet load): The impact is negligible because transfers only access account balances (via `basic_ref`), not storage slots. The overlay adapter batches the three account reads (nonce, balance, code_hash) into a single `block_on` call (line 70-75 of adapter.rs).

**For contract-heavy workloads**:
- A Uniswap V2 swap touches approximately 10 storage slots = ~10-30 `block_on` calls per swap (depending on which adapter)
- A multi-hop route through 3 pools = ~30-90 `block_on` calls
- At 100 swaps per block = ~3,000-9,000 `block_on` calls per block
- The `block_in_place` overhead (context switch + runtime re-entry) adds approximately 1-5 microseconds per call
- Total: 3-45ms per block just from the async-to-sync bridge, comparable to the entire block production time (~7-31ms)

The QMDB handler adapter at `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs:116-127` is particularly expensive because it makes three sequential `block_on` calls per `storage_ref` (read handle acquisition, account lookup for generation, storage value lookup).

---

## Root Cause

REVM's execution engine requires synchronous state access via the `DatabaseRef` trait, but Kora's storage backend (QMDB) is async. The `block_on()` bridge converts each individual access synchronously rather than batching reads ahead of time. There is no caching or prefetching layer between the EVM and the storage backend.

---

## Suggested Fix

Implement a prefetch cache in the adapter that pre-loads expected state before EVM execution begins:

```rust
struct PrefetchCache {
    accounts: HashMap<Address, AccountInfo>,
    storage: HashMap<(Address, U256), U256>,
}

impl StateDbAdapter<S> {
    /// Pre-load state for addresses and storage slots in the access list.
    async fn prefetch(&mut self, access_list: &[(Address, Vec<U256>)]) {
        for (addr, slots) in access_list {
            // Batch account read
            if let Ok(info) = self.state.basic_info(addr).await {
                self.cache.accounts.insert(*addr, info);
            }
            // Batch storage reads
            for slot in slots {
                if let Ok(value) = self.state.storage(addr, slot).await {
                    self.cache.storage.insert((*addr, *slot), value);
                }
            }
        }
    }
}

impl DatabaseRef for StateDbAdapter<S> {
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // Check prefetch cache first (no block_on needed)
        if let Some(&value) = self.cache.storage.get(&(address, index)) {
            return Ok(value);
        }
        // Fall back to block_on for cache misses
        match block_on(self.state.storage(&address, &index)) {
            Ok(value) => Ok(value),
            Err(StateDbError::AccountNotFound(_)) => Ok(U256::ZERO),
            Err(e) => Err(e.into()),
        }
    }
}
```

For EIP-2930 transactions (which include explicit access lists), the prefetch is exact. For legacy transactions, the access list can be estimated from the transaction's `to` address (known contract storage layout) or from a previous simulation run.

An alternative approach is to use REVM's `DatabaseAsync` trait directly (which Kora already partially implements via `QmdbHandle`'s `DatabaseAsyncRef` impl at line 134-203 of the handlers adapter), avoiding the `block_on` bridge entirely. However, this requires changes to the executor pipeline.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/adapter.rs` -- add prefetch cache to `StateDbAdapter`, use it in `DatabaseRef` implementation
- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/lib.rs` -- call `prefetch` before EVM execution with access list data
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` -- (optional) add caching to `QmdbHandle` `DatabaseRef` impl

---

## Related Issues

- `013-eth-call-blocks-async-runtime.md` -- related issue with block_on blocking async workers
- `082-storage-block-on-async-deadlock.md` -- related block_on usage risks
- `036-storage-overlay-code-lookup-linear-scan.md` -- another storage access performance issue
