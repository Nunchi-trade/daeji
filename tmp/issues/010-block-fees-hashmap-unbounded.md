# 010: block_fees HashMap Grows Without Bound -- Unbounded Memory Leak

**Category:** bug / consensus / executor
**Severity:** high
**Labels:** bug, performance, reliability, consensus, executor

---

## Summary

The `block_fees` cache in `RevmApplication` is a `HashMap<ConsensusDigest, (u64, u64)>` that stores `(gas_used, base_fee)` for every block that is built or verified. Entries are inserted via `record_block_fees()` but are **never removed**. The code comment on lines 114-118 claims "the map is bounded by the number of unfinalized blocks," but this is incorrect -- entries for finalized blocks persist indefinitely. At 33 blocks/second, the cache grows ~5.5 MB/hour or ~924 MB/week.

## Problem

The `block_fees` field at `crates/node/runner/src/app.rs:119` is declared as an unbounded `HashMap`:

```rust
block_fees: Arc<RwLock<HashMap<ConsensusDigest, (u64, u64)>>>,
```

The comment on lines 114-118 directly above the field states:

```rust
/// Per-block `(gas_used, base_fee_per_gas)` cache, keyed by consensus
/// digest.  Populated when a block is built or verified so that the
/// *next* block can compute its EIP-1559 base fee from the parent's
/// gas usage.  Entries are small (32 + 16 bytes) and the map is bounded
/// by the number of unfinalized blocks.
```

The claim that "the map is bounded by the number of unfinalized blocks" is incorrect. No code removes entries when blocks are finalized. The only operations on the map are:

1. **Insert** via `record_block_fees()` at line 230-232 -- called after every successful `build_block()` (line 676 in the build path) and `verify_block()` (line 676 in the verify path):

```rust
fn record_block_fees(&self, digest: ConsensusDigest, gas_used: u64, base_fee: u64) {
    self.block_fees.write().insert(digest, (gas_used, base_fee));
}
```

2. **Read** via `compute_base_fee()` at lines 215-226 -- looks up the parent block's fee data to derive the next block's EIP-1559 base fee:

```rust
fn compute_base_fee(&self, parent_digest: ConsensusDigest) -> u64 {
    let fees = self.block_fees.read();
    match fees.get(&parent_digest) {
        Some(&(parent_gas_used, parent_base_fee)) => calculate_base_fee(
            parent_base_fee,
            parent_gas_used,
            self.gas_limit,
            &BaseFeeParams::DEFAULT,
        ),
        None => kora_config::INITIAL_BASE_FEE,
    }
}
```

3. **Seed on startup** via `seed_block_fees()` at lines 205-210 -- pre-populates the cache from the block index after restart.

There is no `remove()`, `retain()`, or `clear()` call anywhere for this map. The `FinalizedReporter` at `crates/node/reporters/src/lib.rs` does not notify the application layer to prune old entries.

## Code Reference

See code blocks above. Key locations:

- `crates/node/runner/src/app.rs:114-119` -- `block_fees` field declaration with incorrect bounding comment
- `crates/node/runner/src/app.rs:230-232` -- `record_block_fees()` inserts without bounds check
- `crates/node/runner/src/app.rs:215-226` -- `compute_base_fee()` reads from the cache (only needs parent)
- `crates/node/runner/src/app.rs:205-210` -- `seed_block_fees()` pre-populates on startup

## Impact

At 33 blocks/second, this adds ~33 entries/second. Each entry is approximately 80 bytes (32-byte `ConsensusDigest` key + 16-byte `(u64, u64)` value tuple + HashMap overhead including the hash and pointer). This accumulates to:

- ~5.5 MB/hour
- ~132 MB/day
- ~924 MB/week

On the live devnet where 8 of 10 nodes are at their 4 GB memory ceiling, this leak contributes significantly to the memory pressure. After one week of continuous operation, the `block_fees` cache alone consumes nearly 1 GB -- a quarter of the container's memory limit.

The leak is particularly insidious because:
1. It is slow enough to avoid immediate detection during short test runs.
2. It accumulates to a significant fraction of available memory over operational timescales.
3. Its growth rate is proportional to throughput -- faster chains leak faster.

## Root Cause

The `record_block_fees()` function is called during both `build_block()` and `verify_block()`, but no corresponding cleanup is performed when blocks are finalized. The `compute_base_fee()` function only needs the **parent** block's fee data, meaning only the most recent ~64 entries (the `MAX_PROPOSAL_LAG` window at line 63) are ever queried. All older entries are dead weight.

## Suggested Fix

**Option 1 -- LRU cache** (simplest and most effective):

Replace `HashMap` with a bounded LRU cache capped at `MAX_PROPOSAL_LAG * 2` (~128 entries). Only recent block fees are needed for base fee computation:

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

block_fees: Arc<RwLock<LruCache<ConsensusDigest, (u64, u64)>>>,

// In new():
block_fees: Arc::new(RwLock::new(
    LruCache::new(NonZeroUsize::new(128).unwrap())
)),
```

**Option 2 -- Finalization pruning**:

Add a method to `RevmApplication` that the `FinalizedReporter` calls after each finalization to remove entries for finalized blocks:

```rust
fn prune_block_fees_below(&self, finalized_height: u64) {
    // Remove entries for blocks at or below the finalized height.
    // Requires mapping digests to heights, which may need an auxiliary index.
}
```

**Option 3 -- Periodic sweep**:

Every N finalized blocks, compare the `block_fees` keys against the snapshot store's active set and remove entries not in the active set.

Option 1 is recommended because it is self-bounding, requires no coordination with the finalization pipeline, and the LRU eviction naturally keeps the most recently needed entries.

## Files to Modify

- `crates/node/runner/src/app.rs:119` -- Replace `HashMap` with bounded `LruCache`
- `crates/node/runner/src/app.rs:149-162` -- Update `new()` to initialize LRU cache
- `crates/node/runner/src/app.rs:230-232` -- `record_block_fees()` will automatically evict old entries with LRU

## Related Issues

- [005 -- Memory Exhaustion](./005-memory-exhaustion-devnet.md) -- This leak is one of multiple contributing factors to the devnet OOM crisis
