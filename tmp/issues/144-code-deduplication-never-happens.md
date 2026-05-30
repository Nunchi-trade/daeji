# 144: Contract Bytecode Deduplication Never Happens -- Same Code Stored Per Deployment

**Category**: performance
**Severity**: medium
**Component**: storage / executor

## Summary

When building QMDB write batches from a changeset, contract bytecode is unconditionally added to the code partition batch for every deployment, even if the same bytecode (identified by its keccak256 hash) already exists in the store. While the code is keyed by its content hash (making duplicate writes idempotent from a correctness standpoint), each redundant write allocates a full copy of the bytecode (up to 24,576 bytes per EIP-170), triggers a Merkle tree update in the QMDB code partition, and consumes journal space. On chains with many factory-deployed contracts (ERC-20 tokens, proxies, etc.), this causes significant wasted I/O.

## Problem

In `crates/storage/qmdb/src/store.rs`, the `build_batches()` method processes each account update and pushes any associated code to the batch without checking whether that code hash already exists:

```rust
// crates/storage/qmdb/src/store.rs:298-301
// Add code if present
if let Some(ref code) = update.code {
    batches.code.push((update.code_hash, Some(code.clone())));
}
```

The `code.clone()` allocates a full copy of the bytecode for every deployment. For a popular ERC-20 template deployed 10,000 times, this means 10,000 x ~24 KB = 240 MB of redundant bytecode copies flowing through the batch pipeline, each triggering a Merkle tree insert in the code partition.

The upstream code that populates the changeset is in `crates/storage/handlers/src/adapter.rs` (line 231):

```rust
let code = account.info.code.as_ref().map(|c| c.bytes().to_vec());
```

This unconditionally converts REVM's code bytes into a `Vec<u8>` for every account with code, regardless of whether the code was newly deployed or already existed.

**Files**: `crates/storage/qmdb/src/store.rs` (lines 298-301), `crates/storage/handlers/src/adapter.rs` (line 231)

## Code Reference

```rust
// crates/storage/qmdb/src/store.rs:266-313
pub async fn build_batches(&self, changes: &ChangeSet) -> Result<StoreBatches, QmdbError> {
    let stores = self.stores()?;
    let mut batches = StoreBatches::new();

    for (address, update) in &changes.accounts {
        // Get current account to check generation
        let current_gen = match stores.accounts.get(address).await {
            Ok(Some(bytes)) => {
                AccountEncoding::decode(&bytes).map(|(_, _, _, g)| g).unwrap_or(0)
            }
            Ok(None) => 0,
            Err(e) => return Err(QmdbError::Storage(e.to_string())),
        };

        // ... generation logic ...

        if update.selfdestructed {
            batches.accounts.push((*address, None));
        } else {
            // ... account encoding ...

            // Add code if present -- NO DEDUPLICATION CHECK
            if let Some(ref code) = update.code {
                batches.code.push((update.code_hash, Some(code.clone())));
            }
        }
        // ... storage changes ...
    }

    Ok(batches)
}
```

## Impact

1. **Wasted I/O bandwidth**: Every contract deployment writes bytecode to the code partition, even if the same code hash was already stored. For factory patterns (Uniswap pairs, proxy clones), the same bytecode is written thousands of times.
2. **Unnecessary Merkle tree churn**: Each redundant write triggers a Merkle tree update in the QMDB code partition, recomputing hashes up the tree despite the value being identical.
3. **Journal space consumption**: The QMDB journal records every write, so redundant code writes consume journal space that must eventually be compacted.
4. **Memory pressure**: `code.clone()` allocates up to 24 KB per deployment on the hot path, contributing to memory pressure during block execution.

On a chain with high contract deployment activity (e.g., 100 factory deployments per block, 33 blocks/second), this amounts to ~80 MB/s of redundant bytecode allocation and I/O.

## Root Cause

The batch building code unconditionally pushes code entries without checking whether the code hash already exists in the store. The REVM adapter also unconditionally extracts code bytes from every touched account, rather than only newly-deployed ones.

## Suggested Fix

**Option A**: Check for existing code before adding to batch:

```rust
// In build_batches():
if let Some(ref code) = update.code {
    // Only write code if it doesn't already exist in the store
    match stores.code.get(&update.code_hash).await {
        Ok(None) => {
            batches.code.push((update.code_hash, Some(code.clone())));
        }
        Ok(Some(_)) => {
            // Code already exists, skip redundant write
        }
        Err(e) => return Err(QmdbError::Storage(e.to_string())),
    }
}
```

**Option B**: Maintain an in-memory `HashSet<B256>` of known code hashes (cheaper than a store lookup per deployment):

```rust
// In QmdbStore:
known_code_hashes: HashSet<B256>,

// In build_batches():
if let Some(ref code) = update.code {
    if self.known_code_hashes.insert(update.code_hash) {
        // First time seeing this code hash -- write it
        batches.code.push((update.code_hash, Some(code.clone())));
    }
}
```

Option B is preferred for performance since it avoids a disk read on every deployment.

## Files to Modify

- `crates/storage/qmdb/src/store.rs` -- add existence check in `build_batches()` before pushing code entries
- `crates/storage/handlers/src/adapter.rs` -- (optional) only populate `code` field for newly-created accounts

## Related Issues

None.

## Labels

`performance`, `storage`
