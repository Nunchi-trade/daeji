# 026: parent_hash Always B256::ZERO in EVM Block Execution Context

**Category**: bug
**Severity**: high
**Labels**: bug, executor, correctness

---

## Summary

The `parent_hash` field in the EVM block execution context (`BlockContext`) is hardcoded to `B256::ZERO` for every block. This means the block header used during EVM execution has an incorrect `parent_hash`, which breaks any EVM contract logic that inspects the current block's parent hash via the block header. The `BLOCKHASH` opcode for accessing historical block hashes works correctly through a separate mechanism, and blocks served via RPC also have correct parent hashes through the indexer, so the impact is limited to in-EVM header inspection.

---

## Problem

When constructing the EVM block context for both block building and block verification, the `block_context()` method in `app.rs` creates a `Header` using `..Default::default()`, which sets `parent_hash` to `B256::ZERO`. It then passes `B256::ZERO` explicitly as the `parent_hash` argument to `BlockContext::new()`.

This affects two code paths:
1. **Block building** (`build_block` at line 344 of `app.rs`): calls `self.block_context()` which returns a context with zero parent hash.
2. **Block verification** (`verify_block` at line 590 of `app.rs`): calls `self.block_context()` which returns the same zero parent hash.

The RPC-facing `IndexedBlock` data is populated separately and does correctly set `parent_hash`, so `eth_getBlockByNumber` returns the correct value. The issue is confined to the EVM execution environment.

**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs`

---

## Code Reference

The `block_context()` method (lines 234-251 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:234-251
fn block_context(
    &self,
    height: u64,
    timestamp: u64,
    prevrandao: B256,
    parent_digest: ConsensusDigest,
) -> BlockContext {
    let base_fee = self.compute_base_fee(parent_digest);
    let header = Header {
        number: height,
        timestamp,
        gas_limit: self.gas_limit,
        beneficiary: self.fee_recipient,
        base_fee_per_gas: Some(base_fee),
        ..Default::default()          // <-- parent_hash defaults to B256::ZERO
    };
    BlockContext::new(header, B256::ZERO, prevrandao)  // <-- parent_hash = B256::ZERO
}
```

The `BlockContext` struct (lines 14-27 of `context.rs`):

```rust
// crates/node/executor/src/context.rs:14-27
pub struct BlockContext {
    /// Block header.
    pub header: Header,
    /// Parent block hash.
    pub parent_hash: B256,
    /// Previous block's randomness (prevrandao).
    pub prevrandao: B256,
    /// Blob base fee for Cancun+ (EIP-4844).
    pub blob_base_fee: Option<u128>,
    /// Recent block hashes keyed by block number for the BLOCKHASH opcode.
    pub recent_block_hashes: HashMap<u64, B256>,
}
```

Usage in `build_block` (line 344 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:344
let context = self.block_context(height, timestamp, prevrandao, parent_digest);
```

Usage in `verify_block` (line 590 of `app.rs`):

```rust
// crates/node/runner/src/app.rs:589-590
let context =
    self.block_context(block.height, block.timestamp, block.prevrandao, parent_digest);
```

---

## Impact

1. **EVM header inspection**: Any Solidity contract that accesses `block.parenthash` (which maps to the `BLOCKHASH` opcode applied to `block.number - 1`, or to the header's `parent_hash` field depending on the EVM implementation) may see incorrect results. Specifically, if the REVM implementation uses `BlockContext.parent_hash` or `Header.parent_hash` to serve the `BLOCKHASH(block.number - 1)` request, it will return zero instead of the actual parent hash.

2. **BLOCKHASH opcode for older blocks**: The `BLOCKHASH` opcode for blocks older than the immediate parent works correctly because it is served by the `recent_block_hashes` map, which is populated with correct hashes from the block index.

3. **RPC correctness**: Not affected. The `IndexedBlock` used for RPC responses correctly populates `parent_hash` from `block.parent.0`.

4. **No consensus divergence**: All validators use the same zero value, so blocks produced by any validator are deterministically identical. This bug does not cause disagreement between validators.

5. **Cross-chain bridge risk**: Any bridge contract that verifies block headers or relies on parent hash chaining within the EVM will see incorrect data.

---

## Root Cause

The `block_context()` method does not look up the parent block's Ethereum-style hash from the block index. The parent block's consensus digest is available (as `parent_digest`), but it is not the same as the Ethereum-style block hash. The method would need to look up the parent's `IndexedBlock` from the `BlockIndex` to obtain the correct Ethereum-style hash.

---

## Suggested Fix

Pass a reference to the `BlockIndex` into `block_context()` and look up the parent block's hash:

**Before:**
```rust
fn block_context(
    &self,
    height: u64,
    timestamp: u64,
    prevrandao: B256,
    parent_digest: ConsensusDigest,
) -> BlockContext {
    let base_fee = self.compute_base_fee(parent_digest);
    let header = Header {
        number: height,
        timestamp,
        gas_limit: self.gas_limit,
        beneficiary: self.fee_recipient,
        base_fee_per_gas: Some(base_fee),
        ..Default::default()
    };
    BlockContext::new(header, B256::ZERO, prevrandao)
}
```

**After:**
```rust
fn block_context(
    &self,
    height: u64,
    timestamp: u64,
    prevrandao: B256,
    parent_digest: ConsensusDigest,
    block_index: &BlockIndex,
) -> BlockContext {
    let base_fee = self.compute_base_fee(parent_digest);
    let parent_hash = if height <= 1 {
        B256::ZERO  // Genesis has no parent
    } else {
        block_index
            .get_block_by_number(height - 1)
            .map(|b| b.hash)
            .unwrap_or(B256::ZERO)
    };
    let header = Header {
        parent_hash,
        number: height,
        timestamp,
        gas_limit: self.gas_limit,
        beneficiary: self.fee_recipient,
        base_fee_per_gas: Some(base_fee),
        ..Default::default()
    };
    BlockContext::new(header, parent_hash, prevrandao)
}
```

This requires adding a `block_index: Arc<BlockIndex>` field to `RevmApplication` and threading it through from the runner.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- Update `block_context()` to look up parent hash from block index (line 234), update callers at lines 344 and 590
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- Add `block_index: Arc<BlockIndex>` field to `RevmApplication`

---

## Related Issues

- None
