# EVM: BLOCKHASH opcode always returns zero, breaking dependent smart contracts

**Severity:** Medium

## Summary

The EVM `BLOCKHASH` opcode (0x40) always returns `B256::ZERO` in Kora, deviating from the Ethereum EVM specification. Any smart contract that calls `blockhash(blockNumber)` in Solidity will receive a zero hash regardless of the block number requested. This silently breaks contracts that rely on block hashes for randomness, commit-reveal schemes, governance snapshot verification, or any other on-chain logic that depends on the historical block hash.

## Background

### What is Kora?

Kora is an EVM-compatible blockchain built with a modular architecture. It uses [REVM](https://github.com/bluealloy/revm) as its EVM execution engine, configured with the Cancun hardfork spec (`SpecId::CANCUN`). The chain runs a custom consensus layer (Simplex via Commonware) and stores state in QMDB, a custom state database.

### How EVM execution works in Kora

When a block is executed, the `RevmExecutor` (in `crates/node/executor/src/revm.rs`) wraps the state database in a `StateDbAdapter` (in `crates/node/executor/src/adapter.rs`). This adapter implements REVM's `DatabaseRef` trait, which is the interface REVM uses to read blockchain state during EVM execution. The `DatabaseRef` trait has four methods:

- `basic_ref()` (line 48) -- account lookups (nonce, balance, code hash)
- `code_by_hash_ref()` (line 60) -- contract bytecode retrieval
- `storage_ref()` (line 68) -- storage slot reads
- `block_hash_ref()` (line 76) -- historical block hash lookups

The first three are fully implemented by delegating to the `StateDbRead` trait via a `block_on()` async-to-sync bridge function (lines 15-23). The fourth is stubbed.

### State read path during execution

State reads flow through this layered architecture:

```
REVM EVM Execution
    -> StateDbAdapter (sync DatabaseRef)
        -> block_on() (async-to-sync bridge)
            -> OverlayState<QmdbState> (check pending changes first)
                -> QmdbState (QMDB on disk, if not in overlay)
```

The `OverlayState` (in `crates/storage/overlay/src/overlay.rs`) wraps a base `QmdbState` with a `ChangeSet` of pending modifications. For `block_hash`, the overlay will need to pass through to the underlying storage since block hashes are not part of the change set.

## The Code

**File:** `crates/node/executor/src/adapter.rs`, lines 76-79

```rust
fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
    // Block hash lookups not supported yet
    Ok(B256::ZERO)
}
```

The `_number` parameter is underscore-prefixed, confirming it is intentionally unused. The comment acknowledges this is a known gap. The method unconditionally returns `B256::ZERO` for every block number.

## Root Cause

The `StateDbRead` trait (defined in `crates/storage/traits/src/state.rs`, lines 13-48) has no `block_hash` method:

```rust
// crates/storage/traits/src/state.rs:13-48
pub trait StateDbRead: Clone + Send + Sync + 'static {
    fn nonce(&self, address: &Address) -> impl Future<Output = Result<u64, StateDbError>> + Send;
    fn balance(&self, address: &Address) -> impl Future<Output = Result<U256, StateDbError>> + Send;
    fn code_hash(&self, address: &Address) -> impl Future<Output = Result<B256, StateDbError>> + Send;
    fn code(&self, code_hash: &B256) -> impl Future<Output = Result<Bytes, StateDbError>> + Send;
    fn storage(&self, address: &Address, slot: &U256) -> impl Future<Output = Result<U256, StateDbError>> + Send;
    fn exists(&self, address: &Address) -> impl Future<Output = Result<bool, StateDbError>> + Send {
        // default implementation checks nonce > 0 || balance != 0
    }
}
```

There are five data accessors -- nonce, balance, code_hash, code, and storage -- plus a default `exists` method, but no block_hash. The underlying QMDB state database does not maintain a block hash ring buffer either. Since there is no data source for historical block hashes anywhere in the storage layer, the adapter has nothing to delegate to, so it returns zero.

Note that during execution, state reads flow through the `StateDbAdapter` which bridges REVM's sync `DatabaseRef` interface to the async `StateDbRead` trait using a `block_on()` helper (lines 15-23 of `adapter.rs`). This helper uses `tokio::task::block_in_place` on multi-threaded runtimes or `futures::executor::block_on` otherwise. The `block_hash` implementation will need to use the same bridging pattern.

## What Breaks

Contracts that use `blockhash()` for:

1. **Randomness schemes** -- `blockhash(block.number - 1)` is commonly used as a source of pseudo-randomness in lotteries, games, and NFT minting. These will always get zero, making outcomes deterministic and predictable.

2. **Commit-reveal protocols** -- Some commit-reveal implementations use the block hash of the commit block as part of the reveal verification. With a zero hash, reveals cannot be validated correctly.

3. **DeFi oracle designs** -- Certain oracle patterns use block hashes to verify that price data was submitted in a specific block. Zero hashes break these integrity checks.

4. **On-chain governance** -- Snapshot verification mechanisms that reference a historical block hash to prove a governance checkpoint will fail to verify.

5. **Cross-contract verification** -- Any contract that checks `blockhash(n) != 0` as a validity assertion (e.g., to confirm a block number is within the last 256 blocks) will always fail.

## What Still Works

This issue does **not** affect:

- Basic ETH transfers
- ERC-20 / ERC-721 token operations
- DEX swaps and liquidity pool interactions
- Contract deployments
- Storage reads/writes
- Any contract logic that does not call the `BLOCKHASH` opcode

Most DeFi activity (Uniswap-style AMMs, lending protocols, staking) does not depend on `blockhash()` and will function correctly.

## EVM Specification Reference

The Ethereum Yellow Paper (Appendix H) defines:

> `BLOCKHASH` (opcode 0x40) returns the hash of one of the 256 most recent complete blocks. If the requested block number is not within the range `[currentBlock - 256, currentBlock - 1]`, the opcode returns 0.

Kora must maintain a ring buffer of the last 256 block hashes and serve them through the `DatabaseRef` interface to be EVM-compliant.

## Proposed Fix

### Files to modify

| File | Change |
|------|--------|
| `crates/storage/traits/src/state.rs` | Add `block_hash` method to `StateDbRead` trait |
| `crates/node/executor/src/adapter.rs` | Wire `block_hash_ref` to delegate to `StateDbRead::block_hash` |
| `crates/storage/overlay/src/overlay.rs` | Implement `block_hash` on `OverlayState<S>` (pass through to base) |
| `crates/storage/handlers/src/state.rs` | Implement `block_hash` on `QmdbHandle<A,S,C>` (read from ring buffer; `QmdbState` is a type alias for a concrete `QmdbHandle`) |
| `crates/node/runner/src/app.rs` | Populate ring buffer during block finalization in `verify_block` (line 166) and `build_block` (line 84) |

### 1. Add `block_hash` to `StateDbRead`

In `crates/storage/traits/src/state.rs`, add a method to the `StateDbRead` trait (after line 35, before the `exists` default method):

```rust
pub trait StateDbRead: Clone + Send + Sync + 'static {
    // ... existing methods (nonce, balance, code_hash, code, storage) ...

    /// Get the hash of a recent block by number.
    /// Returns the hash if the block is within the last 256 blocks, or B256::ZERO otherwise.
    fn block_hash(&self, number: u64) -> impl Future<Output = Result<B256, StateDbError>> + Send;

    /// Check if an account exists.
    fn exists(...) { ... }
}
```

Every existing implementor of `StateDbRead` must implement this new method. Search for all implementors:
- `QmdbHandle<A, S, C>` in `crates/storage/handlers/src/state.rs` (the production implementation; `QmdbState` in `crates/storage/qmdb-ledger/src/ledger.rs` is a type alias for a concrete `QmdbHandle`)
- `OverlayState<S>` in `crates/storage/overlay/src/overlay.rs`
- `MockStateDb` in `crates/node/executor/src/revm.rs` (line 674)
- `MockStateDb` in `crates/node/executor/tests/executor.rs` (line 54)
- `MockState` and `MissingAccountState` in `crates/node/rpc/src/indexed_provider.rs` (lines 399, 424)
- `MockStateDb` in `crates/storage/overlay/src/overlay.rs` (line 191, test-only)

### 2. Maintain a ring buffer of last 256 block hashes

Store the most recent 256 block hashes, matching the EVM spec. This could be:

- **Option A:** A dedicated in-memory `VecDeque<(u64, B256)>` in the ledger service (`crates/node/ledger/`), populated during finalization. Simpler but lost on restart (would need to be rebuilt from the block archive).
- **Option B:** Stored in QMDB as a special system storage area (e.g., under a reserved address like `0x0000...0001`). Persistent but adds write overhead per block.

Option A is recommended for simplicity. The ring buffer should be accessible to `QmdbState` so that the `block_hash` trait method can read from it.

### 3. Wire through `StateDbAdapter`

In `crates/node/executor/src/adapter.rs`, replace lines 76-79:

```rust
// BEFORE (current code):
fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
    // Block hash lookups not supported yet
    Ok(B256::ZERO)
}

// AFTER:
fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
    Ok(block_on(self.state.block_hash(number))?)
}
```

This follows the same `block_on()` bridging pattern used by `basic_ref()` (line 49), `code_by_hash_ref()` (line 64), and `storage_ref()` (line 69).

### 4. Populate during block finalization

When a block is finalized (in the runner/app layer), insert its hash into the ring buffer. In `crates/node/runner/src/app.rs`:

- `build_block()` (line 84): After computing `block_digest` at line 150, store `(block.height, block_digest_hash)` in the ring buffer before returning.
- `verify_block()` (line 166): After verifying the state root match (line 210-218), before inserting the snapshot (line 223), store the block hash.

The `BlockContext` constructed at line 68-78 (`block_context()` method) passes the current block's height and parent hash. The ring buffer must be populated so that by the time the next block executes, the previous block's hash is available.

### 5. Update MockStateDb in tests

The `MockStateDb` in `crates/node/executor/src/revm.rs` (around line 674) must implement the new method:

```rust
async fn block_hash(&self, _number: u64) -> Result<B256, StateDbError> {
    Ok(B256::ZERO)
}
```

## Verification Checklist

1. Deploy a Solidity contract that reads `blockhash(block.number - 1)` and stores the result
2. Execute a transaction that calls this contract
3. Verify the stored value is non-zero and matches the actual parent block hash
4. Verify that requesting a block older than 256 blocks returns zero (per EVM spec)
5. Verify that requesting the current block number returns zero (per EVM spec)
6. Verify that requesting a future block number returns zero (per EVM spec)
7. Run `cargo test` across the workspace to confirm no regressions (the `MockStateDb` and all `StateDbRead` implementors must compile)
8. Verify that basic transactions (ETH transfers, contract deployments) still work correctly after the change
