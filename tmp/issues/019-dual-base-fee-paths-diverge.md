# Dual Base Fee Computation Paths Can Diverge After Restart

**Category**: Bug
**Severity**: High
**Labels**: `bug`, `correctness`, `consensus`, `recovery`

## Summary

There are two independent code paths that compute the EIP-1559 base fee for block execution, and they can produce different results after a node restart. The proposal/verification path uses an in-memory `block_fees` HashMap, while the finalization replay path reads from the persistent `BlockIndex`. If these two sources disagree after a restart (due to seeding gaps or index incompleteness), the node computes different state roots for the same block in the two paths, triggering a fatal `StateRootMismatch` abort in the finalization pipeline.

## Problem

### Path 1: Proposal and Verification

`RevmApplication::compute_base_fee()` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:215-226` reads from the in-memory `block_fees` HashMap:

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
        None => kora_config::INITIAL_BASE_FEE,  // Fallback when parent not in cache
    }
}
```

The `block_fees` HashMap is populated in two ways:
- During normal operation: `record_block_fees()` at `app.rs:230-232` inserts an entry after each block is built or verified.
- After restart: `seed_block_fees()` at `app.rs:205-210` pre-seeds the cache from recent blocks in the block index.

### Path 2: Finalization Replay

`RevmContextProvider::context()` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs:631-664` reads directly from the persistent `BlockIndex`:

```rust
impl BlockContextProvider for RevmContextProvider {
    fn context(&self, block: &Block) -> BlockContext {
        let base_fee = if block.height == 0 {
            kora_config::INITIAL_BASE_FEE
        } else {
            self.block_index
                .get_block_by_number(block.height - 1)
                .map(|parent| {
                    calculate_base_fee(
                        parent.base_fee_per_gas.unwrap_or(kora_config::INITIAL_BASE_FEE),
                        parent.gas_used,
                        parent.gas_limit,
                        &BaseFeeParams::DEFAULT,
                    )
                })
                .unwrap_or(kora_config::INITIAL_BASE_FEE)  // Different fallback source!
        };
        // ...
    }
}
```

### Divergence Scenario

After a node restart:

1. `seed_block_fees()` pre-seeds the `block_fees` cache from the block index. However, if the block index is incomplete (e.g., the most recent finalized block was not indexed before the crash), the parent's fee data may be missing from the cache.

2. When the node proposes a new block, `compute_base_fee()` looks up the parent digest in `block_fees`. If missing, it returns `INITIAL_BASE_FEE` (7 wei).

3. When the block is later finalized and re-executed via `RevmContextProvider::context()`, the finalization path looks up the parent by block number in the `BlockIndex`. If the parent IS in the index (perhaps indexed during the finalization of an earlier block), it computes a different base fee from the parent's actual gas usage.

4. The two base fees are different. Since the base fee affects transaction execution (gas accounting, fee recipient balance), the state roots diverge.

5. `finalize_block()` at `reporters/src/lib.rs:540-544` detects the mismatch:
```rust
if state_root != block.state_root {
    return Err(FinalizationError::StateRootMismatch {
        expected: block.state_root,
        computed: state_root,
    });
}
```

6. `StateRootMismatch` is non-retryable, causing the node to abort and restart -- hitting the same block again, creating a crash-loop.

### Key difference between the two paths

- **Path 1** keys by consensus digest (a hash): `block_fees.get(&parent_digest)`
- **Path 2** keys by block number: `block_index.get_block_by_number(block.height - 1)`
- **Path 1** falls back to `INITIAL_BASE_FEE` when the parent is not cached
- **Path 2** falls back to `INITIAL_BASE_FEE` when the parent is not indexed
- The two fallback conditions trigger under different circumstances

## Code Reference

**block_fees declaration** (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:114-119`):

```rust
/// Per-block `(gas_used, base_fee_per_gas)` cache, keyed by consensus
/// digest.  Populated when a block is built or verified so that the
/// *next* block can compute its EIP-1559 base fee from the parent's
/// gas usage.  Entries are small (32 + 16 bytes) and the map is bounded
/// by the number of unfinalized blocks.
block_fees: Arc<RwLock<HashMap<ConsensusDigest, (u64, u64)>>>,
```

**seed_block_fees** (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:205-210`):

```rust
pub fn seed_block_fees(&self, entries: &[(ConsensusDigest, u64, u64)]) {
    let mut fees = self.block_fees.write();
    for &(digest, gas_used, base_fee) in entries {
        fees.insert(digest, (gas_used, base_fee));
    }
}
```

## Impact

A node that restarts and hits this divergence enters a crash-loop:

1. Restart
2. `seed_block_fees()` pre-seeds the cache (potentially incomplete)
3. Node proposes or verifies a block using one base fee
4. Finalization re-executes the block using a different base fee
5. State root mismatch detected
6. `std::process::abort()`
7. Go to step 1

The node never stabilizes until the base fee caches are manually corrected or the block index is repaired.

The issue is most likely to trigger when:
- The node crashes during QMDB persistence (finalization partially complete)
- The block index contains blocks that the fee cache does not (or vice versa)
- The network has been running long enough for the base fee to deviate significantly from `INITIAL_BASE_FEE`

## Root Cause

Two separate implementations of base fee computation exist because the finalization path (`RevmContextProvider`) was added later with its own lookup logic. There is no single source of truth for "given a parent, what is the base fee?" The `block_fees` HashMap was designed for fast in-memory lookups during consensus, while the finalization path was designed to work from the persistent block index, but neither is aware of the other.

## Suggested Fix

**Unify into a single `compute_base_fee` method** that both paths use, backed by the persistent block index as the sole source of truth:

```rust
fn compute_base_fee(&self, parent_height: u64) -> u64 {
    if parent_height == 0 {
        return kora_config::INITIAL_BASE_FEE;
    }
    self.block_index
        .get_block_by_number(parent_height - 1)
        .map(|parent| calculate_base_fee(
            parent.base_fee_per_gas.unwrap_or(kora_config::INITIAL_BASE_FEE),
            parent.gas_used,
            parent.gas_limit,
            &BaseFeeParams::DEFAULT,
        ))
        .unwrap_or(kora_config::INITIAL_BASE_FEE)
}
```

This eliminates the `block_fees` HashMap entirely. Benefits:
- Single source of truth for base fee computation
- No cache seeding needed after restart
- Eliminates the memory leak from the unbounded `block_fees` HashMap (issue #010)
- Block index lookups are fast (O(1) by block number)

If the concern is that the block index may not yet contain the parent during fast consensus (before finalization has indexed it), the approach can fall back to the in-memory cache only when the index lookup fails, rather than using the cache as the primary source.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (lines 215-226) -- `compute_base_fee()` uses in-memory cache
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (line 119) -- `block_fees` HashMap declaration
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (lines 205-210) -- `seed_block_fees()` startup seeding
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (lines 230-232) -- `record_block_fees()` cache population
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` (lines 637-651) -- finalization base fee computation

## Related Issues

- `010-block-fees-hashmap-unbounded.md` -- the `block_fees` HashMap that this issue proposes to eliminate also leaks memory unboundedly
- `012-blockhash-opcode-broken-proposal.md` -- another proposal/finalization BlockContext inconsistency (BLOCKHASH opcode)
- `014-triple-block-execution.md` -- the finalization re-execution where the mismatch is detected
- `109-simplex-engine-uses-floor-genesis-on-restart.md` -- related restart recovery issue
