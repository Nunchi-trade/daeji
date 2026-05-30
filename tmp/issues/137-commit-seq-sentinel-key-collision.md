# Commit Sequence Sentinel Key Can Collide With Real Ethereum Accounts

## Category
bug -- storage / data integrity

## Severity
high

## Summary
The `COMMIT_SEQ_ACCOUNT_KEY` sentinel address (`0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFfe`) is used to store a cross-partition consistency marker in the QMDB accounts partition. This address occupies the same key space as real Ethereum addresses. While extremely unlikely, a CREATE2 deployment could produce this address, causing the commit sequence marker and account state to corrupt each other. The comment in the source code incorrectly claims the address is derived from `keccak256(b"__QMDB_COMMIT_SEQ__")`, but the actual value is an arbitrary constant.

## Problem
The `QmdbStore` uses sentinel keys in each of the three QMDB partitions to track a cross-partition commit sequence number. These markers are written on every block commit and read on startup to detect partial commits caused by crashes. The sentinel for the accounts partition is:

**File:** `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs`, lines 13-20:
```rust
/// Sentinel address used to store the commit sequence number in the accounts partition.
///
/// Derived from the first 20 bytes of keccak256(b"__QMDB_COMMIT_SEQ__").
/// This is a preimage-resistant address that will not collide with any real Ethereum account.
pub const COMMIT_SEQ_ACCOUNT_KEY: Address = Address::new([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFE,
]);
```

The comment says the value is "Derived from the first 20 bytes of keccak256(b'__QMDB_COMMIT_SEQ__')", but the actual keccak256 of that string is `0x7a3c...` (entirely different). The value `0xFFFF...FFFE` is an arbitrary manually-chosen constant.

The storage partition sentinel uses a composite key with `generation = u64::MAX` and `slot = U256::MAX`, providing stronger collision resistance:

```rust
pub const COMMIT_SEQ_STORAGE_KEY: StorageKey =
    StorageKey::new(COMMIT_SEQ_ACCOUNT_KEY, u64::MAX, U256::MAX);
```

The code partition sentinel uses a 32-byte key `0xFFFF...FFFE`:

```rust
pub const COMMIT_SEQ_CODE_KEY: B256 = B256::new([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
]);
```

The sentinel values are written on every block commit via `apply_batches()` at lines 334-366:

```rust
pub async fn apply_batches(&mut self, batches: StoreBatches) -> Result<(), QmdbError> {
    let next_seq = self.commit_seq.saturating_add(1);
    let stores = self.stores_mut()?;

    let mut account_ops = batches.accounts;
    account_ops.push((COMMIT_SEQ_ACCOUNT_KEY, Some(encode_commit_seq_account(next_seq))));

    let mut storage_ops = batches.storage;
    storage_ops.push((COMMIT_SEQ_STORAGE_KEY, Some(U256::from(next_seq))));

    let mut code_ops = batches.code;
    code_ops.push((COMMIT_SEQ_CODE_KEY, Some(encode_commit_seq_code(next_seq))));
    // ... write batches to each partition ...
}
```

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs`, lines 13-35 (sentinel key definitions):
```rust
pub const COMMIT_SEQ_ACCOUNT_KEY: Address = Address::new([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFE,
]);

pub const COMMIT_SEQ_STORAGE_KEY: StorageKey =
    StorageKey::new(COMMIT_SEQ_ACCOUNT_KEY, u64::MAX, U256::MAX);

pub const COMMIT_SEQ_CODE_KEY: B256 = B256::new([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
]);
```

`/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs`, lines 37-41 (sentinel encoding):
```rust
fn encode_commit_seq_account(seq: u64) -> [u8; AccountEncoding::SIZE] {
    AccountEncoding::encode(seq, U256::ZERO, B256::ZERO, 0)
}
```

## Impact
- **Account data corruption (theoretical):** If a contract is deployed at address `0xFFFF...FFFE` (via CREATE2 with the right salt), the commit sequence marker will overwrite the account's nonce/balance/code_hash on every block. Conversely, the account's state will corrupt the commit sequence marker, causing false partition-inconsistency errors on restart.
- **Code hash collision (theoretical):** The code sentinel `0xFFFF...FFFE` is not a valid keccak256 output (the probability of any real code having this exact hash is negligible), so the code partition sentinel is safe in practice.
- **Incorrect documentation:** The comment claiming keccak256 derivation is factually wrong, which could mislead future developers into believing the address has preimage resistance when it is actually an arbitrary constant.
- **CREATE2 mining:** An attacker with sufficient compute could use CREATE2 to mine a deployment at the sentinel address. While the difficulty is comparable to finding a vanity address (2^160 search space), the incentive exists because it would brick the node's consistency checking mechanism.

## Root Cause
The sentinel address occupies the same key space as real Ethereum addresses, with no namespace separation. The QMDB partitions do not have a metadata namespace distinct from the EVM state namespace. The misleading keccak256 comment suggests the original intent was to use a collision-resistant derivation, but the implementation uses a manually chosen constant instead.

## Suggested Fix
**Option 1 (cleanest): Separate metadata namespace.** Use a distinct storage mechanism (e.g., a separate file, a metadata partition, or a reserved key prefix) for commit sequence tracking, keeping it entirely outside the EVM address space.

**Option 2: Use actual keccak256 derivation.** Replace the sentinel with the real first 20 bytes of `keccak256(b"__QMDB_COMMIT_SEQ__")`. This is still in the EVM address space but is genuinely preimage-resistant, making CREATE2 mining infeasible.

**Option 3 (minimal): Fix the comment.** At minimum, correct the misleading comment to state that the value is an arbitrary constant:

```rust
/// Sentinel address used to store the commit sequence number in the accounts partition.
///
/// This is a manually chosen constant address (`0xFFFF...FFFE`) that is
/// extremely unlikely to collide with any real Ethereum account, but is
/// NOT derived from a hash function. A separate metadata namespace would
/// be the ideal long-term solution.
```

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs` -- fix the sentinel key derivation or documentation, and consider a separate metadata namespace

## Related Issues
- `001-qmdb-non-atomic-cross-partition-writes.md` -- the sentinel keys are the consistency mechanism for cross-partition writes; a collision undermines this mechanism
- `135-qmdb-genesis-init-no-restart-guard.md` -- genesis marker (if added) would need a similar sentinel key approach

## Labels
bug, correctness, storage, security
