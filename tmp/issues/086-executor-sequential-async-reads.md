# StateDbAdapter basic_ref() Issues Three Sequential Reads Instead of Concurrent

**Category**: Executor / Performance
**Severity**: Low

## Summary

The `basic_ref()` method in the executor's `StateDbAdapter` makes three sequential async reads (nonce, balance, code_hash) inside a single `block_on()` call. These could potentially be issued concurrently with `tokio::join!` to reduce latency when falling through to the QMDB disk-backed layer. However, the common case (overlay cache hit) resolves all three reads from the same in-memory `BTreeMap` with no I/O, so the benefit is marginal and should be benchmarked before implementing.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/node/executor/src/adapter.rs`, the `basic_ref()` implementation at lines 67-82 makes three sequential `await` calls:

```rust
// crates/node/executor/src/adapter.rs:67-82
fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
    // Batch all three reads into a single block_on call to reduce the
    // overhead of the async-to-sync bridge (block_in_place + handle.block_on).
    match block_on(async {
        let nonce = self.state.nonce(&address).await?;      // read 1 (sequential)
        let balance = self.state.balance(&address).await?;    // read 2 (sequential)
        let code_hash = self.state.code_hash(&address).await?; // read 3 (sequential)
        Ok::<_, StateDbError>((nonce, balance, code_hash))
    }) {
        Ok((nonce, balance, code_hash)) => {
            Ok(Some(AccountInfo { nonce, balance, code_hash, code: None, account_id: None }))
        }
        Err(StateDbError::AccountNotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
```

The `block_on()` helper (lines 32-40) correctly uses `tokio::task::block_in_place` + `Handle::block_on` when a Tokio multi-thread runtime is available, falling back to `futures::executor::block_on()` for non-Tokio contexts:

```rust
// crates/node/executor/src/adapter.rs:32-40
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current()
        && handle.runtime_flavor() == RuntimeFlavor::MultiThread
    {
        return tokio::task::block_in_place(|| handle.block_on(f));
    }

    futures::executor::block_on(f)
}
```

The three reads are batched into a single `block_on()` call (avoiding three separate `block_in_place` transitions), but they execute sequentially within that future.

Additionally, `code_by_hash_ref()` (lines 84-90) and `storage_ref()` (lines 92-98) each incur a separate `block_on()` call:

```rust
// crates/node/executor/src/adapter.rs:84-90
fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
    if code_hash == KECCAK256_EMPTY || code_hash == B256::ZERO {
        return Ok(Bytecode::default());
    }
    let bytes = block_on(self.state.code(&code_hash))?;
    Ok(Bytecode::new_raw(bytes))
}

// crates/node/executor/src/adapter.rs:92-98
fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
    match block_on(self.state.storage(&address, &index)) {
        Ok(value) => Ok(value),
        Err(StateDbError::AccountNotFound(_)) => Ok(U256::ZERO),
        Err(e) => Err(e.into()),
    }
}
```

## Impact

- **Latency on the execution hot path**: `basic_ref()` is called for every account touched during EVM execution. Each block may touch dozens or hundreds of accounts. The EVM runs inside `tokio::task::spawn_blocking` (via `crates/node/runner/src/app.rs:355` and `crates/node/consensus/src/proposal.rs:183`).
- **Underutilized I/O concurrency**: If the underlying `StateDb` supports concurrent reads (e.g., QMDB has separate partitions for accounts, storage, and code), sequential reads leave I/O bandwidth underutilized.

**Important caveats** (why this is low severity):

1. **Overlay cache dominates the hot path**: The `OverlayState` (used in production) resolves nonce, balance, and code_hash from the same in-memory `BTreeMap::get()` call with no I/O. The sequential reads only become a bottleneck when falling through to the QMDB base layer (cold accounts not modified in recent unfinalized blocks).
2. **QMDB accounts partition is shared**: Even when falling through to QMDB, all three fields (nonce, balance, code_hash) live in the same accounts partition. The partition may serialize concurrent reads internally, negating the benefit of `tokio::join!`.
3. **`tokio::join!` overhead**: For the overlay case, wrapping three synchronous `BTreeMap` lookups in `tokio::join!` adds unnecessary Future state machine overhead (each future must be polled independently even though all resolve immediately).

This should be benchmarked before implementing.

## Root Cause

REVM's `DatabaseRef` trait is synchronous, requiring the async-to-sync bridge (`block_on`). Within the bridge, the three state reads were written as simple sequential `await`s because this is the most straightforward pattern. There was no compelling reason to complicate the code with `tokio::join!` unless profiling shows a measurable benefit.

## Suggested Fix

Use `tokio::join!` to issue all three reads concurrently within the existing `block_on`:

```rust
// BEFORE (sequential):
match block_on(async {
    let nonce = self.state.nonce(&address).await?;
    let balance = self.state.balance(&address).await?;
    let code_hash = self.state.code_hash(&address).await?;
    Ok::<_, StateDbError>((nonce, balance, code_hash))
})

// AFTER (concurrent):
match block_on(async {
    let (nonce, balance, code_hash) = tokio::join!(
        self.state.nonce(&address),
        self.state.balance(&address),
        self.state.code_hash(&address),
    );
    Ok::<_, StateDbError>((nonce?, balance?, code_hash?))
})
```

**Recommendation**: Benchmark before and after to confirm a measurable improvement. If the overlay cache hit rate is >99% for typical workloads, the benefit will be negligible and the added complexity is not justified.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/adapter.rs` (lines 67-82) -- `basic_ref()` method

## Related Issues

- `085-storage-two-disk-reads-per-lookup.md` -- another read amplification issue in the storage layer; both address I/O inefficiency on the execution hot path

## Labels

performance, executor
