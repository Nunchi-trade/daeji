# Block Hash Truncation Uses Arbitrary HashMap Ordering

**Category**: Executor / Correctness
**Severity**: Medium

## Summary

When the `with_recent_block_hashes()` method receives more than 256 block hash entries, it truncates using `HashMap::into_iter().take(256)`. Because `HashMap` iteration order is non-deterministic, this could discard the most recent block hashes and retain older ones instead. Although the upstream caller currently caps output at 256 entries (making this path unreachable), the function's contract is misleading and would produce non-deterministic BLOCKHASH results if a future caller passed more entries.

## Problem

In `crates/node/executor/src/context.rs`, the `with_recent_block_hashes()` method at line 53 truncates an oversized `HashMap<u64, B256>` by calling `.into_iter().take(MAX_BLOCK_HASHES).collect()`. The EVM `BLOCKHASH` opcode (EIP-4399) only supports looking up the 256 most recent block hashes, so the correct behavior is to retain the 256 entries with the highest block numbers (i.e., the most recent blocks). However, `HashMap::into_iter()` yields entries in an arbitrary order determined by the internal hash function, which is randomized (SipHash with per-process random keys due to HashDoS mitigation). This means different validators could retain different subsets of block hashes, causing divergent `BLOCKHASH` opcode results and state root mismatches.

The function is located at `/Users/will/dev/nunchi/daeji/crates/node/executor/src/context.rs`, lines 53-60.

## Code Reference

```rust
// crates/node/executor/src/context.rs:49-60
    /// Set the recent block hashes for BLOCKHASH opcode support.
    ///
    /// Retains at most 256 entries (the EVM BLOCKHASH depth limit).
    #[must_use]
    pub fn with_recent_block_hashes(mut self, hashes: HashMap<u64, B256>) -> Self {
        if hashes.len() > MAX_BLOCK_HASHES {
            self.recent_block_hashes = hashes.into_iter().take(MAX_BLOCK_HASHES).collect();
        } else {
            self.recent_block_hashes = hashes;
        }
        self
    }
```

The constant `MAX_BLOCK_HASHES` is defined at line 9:

```rust
// crates/node/executor/src/context.rs:9
const MAX_BLOCK_HASHES: usize = 256;
```

The existing test at line 130 verifies truncation to 256 but does not verify that the *correct* 256 entries (highest block numbers) are retained:

```rust
// crates/node/executor/src/context.rs:130-138
    #[test]
    fn block_context_with_recent_block_hashes_truncates() {
        let header = Header::default();
        let hashes: HashMap<u64, B256> =
            (0..300).map(|i| (i, B256::repeat_byte(i as u8))).collect();
        assert_eq!(hashes.len(), 300);
        let context =
            BlockContext::new(header, B256::ZERO, B256::ZERO).with_recent_block_hashes(hashes);
        assert_eq!(context.recent_block_hashes.len(), MAX_BLOCK_HASHES);
    }
```

## Impact

- **Potential consensus divergence**: If triggered, different validators would retain different subsets of block hashes due to `HashMap`'s non-deterministic iteration order. Contracts calling `BLOCKHASH` during EVM execution would get different results on different validators, producing different state roots and causing the network to fork.
- **Non-deterministic behavior**: The retained set varies across Rust versions, platforms, process restarts, and ASLR randomization. Two identical nodes with identical inputs could produce different execution results.
- **Currently unreachable but fragile**: The upstream `BlockIndex::recent_block_hashes()` caller already caps output at 256 entries, so this truncation path is not currently reachable. However, the function's public API contract does not document or enforce this assumption, and a future caller providing more than 256 entries would trigger the bug silently.

## Root Cause

The truncation logic uses `HashMap::into_iter().take(N)` which has no ordering guarantee, instead of explicitly selecting the N entries with the highest keys (most recent block numbers). The `HashMap` type in Rust uses randomized hashing (SipHash with per-process random seeds) for HashDoS protection, meaning iteration order is intentionally unpredictable.

## Suggested Fix

Sort by block number descending before taking, or filter to retain entries with the highest block numbers:

```rust
// BEFORE (non-deterministic):
if hashes.len() > MAX_BLOCK_HASHES {
    self.recent_block_hashes = hashes.into_iter().take(MAX_BLOCK_HASHES).collect();
}

// AFTER (deterministic -- retains highest block numbers):
if hashes.len() > MAX_BLOCK_HASHES {
    let min_block = hashes.keys().copied().max()
        .unwrap_or(0)
        .saturating_sub(MAX_BLOCK_HASHES as u64 - 1);
    self.recent_block_hashes = hashes.into_iter()
        .filter(|(k, _)| *k >= min_block)
        .collect();
}
```

Alternatively, change the input type from `HashMap<u64, B256>` to `BTreeMap<u64, B256>` to make the ordering explicit and allow efficient truncation via `split_off()`.

The existing test should also be strengthened to verify that the retained entries are the 256 with the highest block numbers:

```rust
#[test]
fn block_context_with_recent_block_hashes_truncates_to_most_recent() {
    let header = Header::default();
    let hashes: HashMap<u64, B256> =
        (0..300).map(|i| (i, B256::repeat_byte(i as u8))).collect();
    let context =
        BlockContext::new(header, B256::ZERO, B256::ZERO).with_recent_block_hashes(hashes);
    assert_eq!(context.recent_block_hashes.len(), MAX_BLOCK_HASHES);
    // Must retain block numbers 44..=299 (the 256 highest)
    for i in 44..300u64 {
        assert!(context.recent_block_hashes.contains_key(&i), "missing block {}", i);
    }
    for i in 0..44u64 {
        assert!(!context.recent_block_hashes.contains_key(&i), "should not contain block {}", i);
    }
}
```

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/context.rs` (lines 53-60) -- fix truncation to retain highest block numbers

## Related Issues

- `090-executor-receipt-index-alignment.md` -- another executor correctness issue with similar consensus-divergence risk

## Labels

bug, correctness, executor
