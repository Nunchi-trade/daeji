# Overlay Code Lookup Is O(N) Linear Scan Over All Changed Accounts

**Category**: Performance -- Storage
**Severity**: Medium
**Labels**: `performance`, `storage`, `executor`

## Summary

The `OverlayState::code()` method performs a linear scan over all accounts in the overlay's change set to find one whose `code_hash` matches the requested hash. There is no hash-based index for code lookups, making each lookup O(N) where N is the number of changed accounts. This method is called during EVM execution for opcodes like `EXTCODECOPY`, `EXTCODESIZE`, and `EXTCODEHASH`.

## Problem

Kora is an EVM execution client that layers pending state changes on top of a persistent state database (QMDB). The `OverlayState<S>` struct in `crates/storage/overlay/src/overlay.rs` wraps a base state database with an in-memory `ChangeSet` that records accounts modified during block execution.

The `ChangeSet` stores account changes in a `BTreeMap<Address, AccountUpdate>` keyed by address (`crates/storage/qmdb/src/changes.rs:9-12`). The `code()` method on `OverlayState` is queried by `code_hash` (a `B256`), which requires a reverse lookup -- searching all account updates to find one with a matching `code_hash`. Since there is no secondary index mapping code hashes to code bytes, every `code()` call iterates over the entire `BTreeMap`.

The `AccountUpdate` struct stores contract bytecode in an `Option<Vec<u8>>` field (`crates/storage/qmdb/src/changes.rs:66`). When a match is found, the code bytes are cloned into a new `Bytes` allocation.

## Code Reference

`crates/storage/overlay/src/overlay.rs:94-111`:
```rust
fn code(
    &self,
    code_hash: &B256,
) -> impl std::future::Future<Output = Result<Bytes, StateDbError>> + Send {
    let code_hash = *code_hash;
    let base = self.base.clone();
    let changes = Arc::clone(&self.changes);
    async move {
        for update in changes.accounts.values() {  // O(N) scan over ALL accounts
            if update.code_hash == code_hash
                && let Some(code) = &update.code
            {
                return Ok(Bytes::from(code.clone()));
            }
        }
        base.code(&code_hash).await  // Falls through to QMDB on miss
    }
}
```

The `ChangeSet` and `AccountUpdate` structures in `crates/storage/qmdb/src/changes.rs:8-69`:
```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Account changes keyed by address.
    pub accounts: BTreeMap<Address, AccountUpdate>,
}

// ...

pub struct AccountUpdate {
    pub created: bool,
    pub selfdestructed: bool,
    pub nonce: u64,
    pub balance: U256,
    pub code_hash: B256,
    pub code: Option<Vec<u8>>,        // Contract bytecode
    pub storage: BTreeMap<U256, U256>,
}
```

## Impact

During block execution with many touched accounts (e.g., DeFi transactions interacting with multiple contracts), each code lookup scans the entire change set. Concrete example: if a block touches 200 accounts and the EVM makes 50 code lookups (for `EXTCODECOPY`, `EXTCODESIZE`, `EXTCODEHASH`, and `DELEGATECALL`), that is 10,000 comparisons per block. At 34 blocks per second on the devnet, this produces 340,000 comparisons per second.

For blocks with heavy contract interaction (DEX swaps with multi-hop routes, flash loans touching many contracts), the change set can grow larger and the number of code lookups increases proportionally, making the O(N) behavior a measurable overhead on the execution hot path.

Additionally, `code.clone()` on every hit allocates a new heap buffer for the contract bytecode, which can be significant for large contracts (up to 24KB per EIP-170).

## Root Cause

The `ChangeSet` data structure was designed for account-address-keyed access, but `code()` is queried by `code_hash`, which requires a reverse lookup. No secondary index exists for code_hash-to-code mapping.

## Suggested Fix

Add a `HashMap<B256, Vec<u8>>` (or `BTreeMap<B256, Vec<u8>>`) index to the `ChangeSet` structure that maps code hashes to code bytes. Populate it when accounts with new code are inserted into the change set:

**In `crates/storage/qmdb/src/changes.rs`**, add a field to `ChangeSet`:
```rust
pub struct ChangeSet {
    pub accounts: BTreeMap<Address, AccountUpdate>,
    pub code_by_hash: BTreeMap<B256, Vec<u8>>,  // NEW: secondary index
}
```

Update `ChangeSet::insert()` and `ChangeSet::merge()` to populate `code_by_hash` when an `AccountUpdate` contains `code: Some(...)`.

**In `crates/storage/overlay/src/overlay.rs`**, replace the linear scan:

**Before:**
```rust
for update in changes.accounts.values() {
    if update.code_hash == code_hash
        && let Some(code) = &update.code
    {
        return Ok(Bytes::from(code.clone()));
    }
}
```

**After:**
```rust
if let Some(code) = changes.code_by_hash.get(&code_hash) {
    return Ok(Bytes::from(code.clone()));
}
```

This changes the lookup from O(N) to O(log N) with `BTreeMap` or O(1) with `HashMap`.

## Files to Modify

- `crates/storage/qmdb/src/changes.rs` -- Add `code_by_hash` field to `ChangeSet`, update `merge()` and `insert()` to populate it
- `crates/storage/overlay/src/overlay.rs` -- Update `code()` method (lines 94-111) to use the new index

## Related Issues

- [039-storage-overlay-changeset-cloned-on-commit.md](039-storage-overlay-changeset-cloned-on-commit.md) -- The `ChangeSet` is fully cloned during `commit()` and `compute_root()`, so adding a secondary index increases the clone cost slightly. Both issues should be considered together.
