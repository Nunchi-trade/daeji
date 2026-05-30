# extract_changes() Copies Bytecode for All Touched Accounts, Not Just Newly Created Ones

**Category**: Executor / Performance
**Severity**: Low

## Summary

The `extract_changes()` function in the REVM executor unconditionally copies bytecode for every touched account that has code, even when the code has not changed (i.e., the account was only called, not created). For contract accounts, `Bytecode::bytes().to_vec()` performs a full heap copy of up to 24KB (the EIP-170 contract size limit). With many contract calls per block at 33 blocks/s, this produces significant unnecessary allocation pressure on the execution hot path.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`, the `extract_changes()` function at lines 710-744 iterates over all touched accounts in the REVM execution state and extracts changes for the `ChangeSet`. At line 728, bytecode is unconditionally copied for every touched account that has code:

```rust
// crates/node/executor/src/revm.rs:710-744
fn extract_changes(state: &EvmState) -> ChangeSet {
    let mut changes = ChangeSet::new();

    for (address, account) in state {
        // Skip untouched accounts
        if !account.is_touched() {
            continue;
        }

        // Extract storage changes (skip read-only SLOAD slots)
        let storage: BTreeMap<U256, U256> = account
            .storage
            .iter()
            .filter(|(_, v)| v.is_changed())
            .map(|(k, v): (&U256, &EvmStorageSlot)| (*k, v.present_value()))
            .collect();

        // Extract code if present
        let code = account.info.code.as_ref().map(|c: &Bytecode| c.bytes().to_vec());  // <-- UNCONDITIONAL COPY

        let update = AccountUpdate {
            created: account.is_created(),
            selfdestructed: account.is_selfdestructed(),
            nonce: account.info.nonce,
            balance: account.info.balance,
            code_hash: account.info.code_hash,
            code,
            storage,
        };

        changes.insert(*address, update);
    }

    changes
}
```

The `AccountUpdate` struct (in `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/changes.rs:53-69`) stores code as `Option<Vec<u8>>`:

```rust
// crates/storage/qmdb/src/changes.rs:53-69
pub struct AccountUpdate {
    /// Whether account was created in this change.
    pub created: bool,
    /// Whether account was selfdestructed.
    pub selfdestructed: bool,
    /// Current nonce.
    pub nonce: u64,
    /// Current balance.
    pub balance: U256,
    /// Code hash.
    pub code_hash: B256,
    /// New code bytes (if code was deployed).
    pub code: Option<Vec<u8>>,
    /// Storage slot changes.
    pub storage: BTreeMap<U256, U256>,
}
```

The same unconditional copy also exists in the `DatabaseCommit::commit()` method in `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` at line 231:

```rust
// crates/storage/handlers/src/adapter.rs:231
let code = account.info.code.as_ref().map(|c| c.bytes().to_vec());
```

## Impact

- **Unnecessary allocations on the hot path**: Every contract interaction triggers a bytecode copy, even though code only changes when a contract is created or self-destructed. In typical EVM execution, contracts are called (not created) in >99% of cases.
- **Memory pressure**: For blocks with many contract calls, the cumulative allocation is significant. Example: 50 contract calls x 12KB average bytecode size = 600KB of unnecessary copies per block. At 33 blocks/s, that is approximately 20MB/s of short-lived allocations that the allocator must handle and the garbage collector must reclaim.
- **Allocator fragmentation**: The allocated `Vec<u8>` objects are short-lived (created in `extract_changes()`, consumed during state commit, then freed) and variable-sized (100 bytes to 24KB), contributing to heap fragmentation.

## Root Cause

The code does not distinguish between accounts whose code changed (newly created or self-destructed) and accounts that were merely called. REVM populates the `account.info.code` field for all loaded accounts (it caches the bytecode after loading it via `code_by_hash_ref()`), so `account.info.code` is `Some(...)` for all contract accounts regardless of whether the code was modified. The `extract_changes()` function treats this as "there is code to store" rather than checking whether the code actually changed.

## Suggested Fix

**Option A (recommended -- simplest and most effective)**:

Only extract code for newly created accounts, since code is immutable once deployed (it can only change via `CREATE`):

```rust
// BEFORE (crates/node/executor/src/revm.rs:728):
let code = account.info.code.as_ref().map(|c: &Bytecode| c.bytes().to_vec());

// AFTER:
let code = if account.is_created() {
    account.info.code.as_ref().map(|c: &Bytecode| c.bytes().to_vec())
} else {
    None
};
```

This is safe because the `AccountUpdate::merge()` method in `crates/storage/qmdb/src/changes.rs:71-100` already handles the case where `code` is `None` correctly -- it only overwrites code when `code_hash` has changed or `code` is `Some`:

```rust
// crates/storage/qmdb/src/changes.rs:89-92
if self.code_hash != code_hash || code.is_some() {
    self.code = code;
}
self.code_hash = code_hash;
```

The same fix should be applied to `DatabaseCommit::commit()` in `crates/storage/handlers/src/adapter.rs:231`:

```rust
// BEFORE (crates/storage/handlers/src/adapter.rs:231):
let code = account.info.code.as_ref().map(|c| c.bytes().to_vec());

// AFTER:
let code = if account.is_created() {
    account.info.code.as_ref().map(|c| c.bytes().to_vec())
} else {
    None
};
```

**Option B** -- Change `AccountUpdate.code` from `Option<Vec<u8>>` to `Option<Bytes>` to avoid the heap copy by cloning the inner `Arc`:

```rust
let code = account.info.code.as_ref().map(|c: &Bytecode| c.bytes().clone());  // Arc increment, no copy
```

This requires changing the `code` field type in `crates/storage/qmdb/src/changes.rs:66` from `Option<Vec<u8>>` to `Option<Bytes>` and updating all downstream consumers. More invasive than Option A.

**Recommendation**: Option A is simpler, has no type-level changes, and handles the 99%+ case (existing contract calls). The savings are proportional to the number of contract calls per block.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` (line 728) -- `extract_changes()` function
- `/Users/will/dev/nunchi/daeji/crates/storage/handlers/src/adapter.rs` (line 231) -- `DatabaseCommit::commit()` method
- `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/changes.rs` (lines 53-69, 71-100) -- `AccountUpdate` struct and `merge()` method (only if Option B is chosen)

## Related Issues

- None currently identified.

## Labels

performance, executor
