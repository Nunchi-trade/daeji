# Recovered Blocks Indexed With gas_used: 0

**Category**: RPC / Data Quality
**Severity**: Medium
**Labels**: `bug`, `correctness`, `rpc`, `recovery`

## Summary

When Kora recovers finalized blocks from the archive during startup, `index_recovered_block` creates RPC index entries with `gas_used: 0` for every block. Blocks within the checkpoint replay tail get corrected, but all blocks before the checkpoint permanently show `gasUsed: 0` in RPC responses. This affects `eth_getBlockByNumber`, `eth_feeHistory`, and any gas estimation that references historical blocks.

## Problem

The `index_recovered_block` function in `crates/node/runner/src/runner.rs` (lines 250-275) creates an `IndexedBlock` entry for each recovered block. The block archive stores only header and transaction data -- not execution results. Since gas usage is an execution output (computed by running transactions through the EVM), it is not available from the archive. The function hardcodes `gas_used: 0`.

During recovery, `restore_checkpoint_and_replay_tail` (lines 360-460) re-executes blocks from the QMDB checkpoint to HEAD, and the replay path (`replay_finalized_block`) updates the index with correct gas values. However, blocks before the checkpoint (which can be the vast majority of the archive) are never re-executed and permanently retain `gas_used: 0`.

With the default `DEFAULT_CHECKPOINT_INTERVAL` of 256 blocks, all blocks older than HEAD-256 will have incorrect gas data after restart.

## Code Reference

`crates/node/runner/src/runner.rs` lines 250-275:

```rust
fn index_recovered_block(
    index: &kora_indexer::BlockIndex,
    block: &Block,
    provider: &RevmContextProvider,
) {
    let block_context = provider.context(block);
    let transaction_hashes = block.txs.iter().map(|tx| keccak256(&tx.bytes)).collect();
    let tx_bytes_total: u64 = block.txs.iter().map(|tx| tx.bytes.len() as u64).sum();
    let indexed_block = kora_indexer::IndexedBlock {
        hash: block.id().0,
        number: block.height,
        parent_hash: block.parent.0,
        state_root: block.state_root.0,
        transactions_root: EMPTY_ROOT_HASH,
        receipts_root: EMPTY_ROOT_HASH,
        timestamp: block_context.header.timestamp,
        gas_limit: block_context.header.gas_limit,
        gas_used: 0,                           // <-- always zero
        base_fee_per_gas: block_context.header.base_fee_per_gas,
        mix_hash: block.prevrandao,
        logs_bloom: alloy_primitives::Bloom::ZERO,
        size: 508 + tx_bytes_total,
        transaction_hashes,
    };
    index.insert_block(indexed_block, Vec::new(), Vec::new());
}
```

Note that `logs_bloom` is also hardcoded to `Bloom::ZERO`, `transactions_root` and `receipts_root` are set to `EMPTY_ROOT_HASH`, and receipt/log data (the last two `Vec::new()` arguments) are empty. These are all execution outputs that are unavailable from the archive.

## Impact

- **`eth_getBlockByNumber` returns incorrect data**: The `gasUsed` field is 0 for all blocks before the checkpoint. Wallets, block explorers, and indexers that query historical blocks will show incorrect gas usage.
- **`eth_feeHistory` is incorrect**: Gas usage ratios (`gasUsedRatio`) will be 0.0 for historical blocks, causing gas price estimation algorithms to underestimate required gas prices. Wallets using EIP-1559 fee estimation may set fees too low, leading to stuck transactions.
- **`seed_block_fee_cache` is affected**: The function at lines 222-243 seeds the base fee cache from the block index on startup. It reads `indexed.gas_used` for the last ~5 blocks. If these blocks are from recovery (pre-checkpoint), they have `gas_used: 0`, which can cause incorrect base fee calculations for the first blocks after restart.
- **No receipt/log data**: Recovered blocks have no receipts and no logs. `eth_getTransactionReceipt` and `eth_getLogs` will return empty results for historical transactions after a restart.

## Root Cause

The finalized block archive (`finalized_blocks`) stores only block header and transaction data. Execution outputs (gas used, receipts, logs, bloom filters) are computed during EVM execution and are not persisted in the consensus archive. The `index_recovered_block` function has no way to obtain these values without re-executing the block, which requires the full state at that block's parent height.

## Suggested Fix

There are several approaches, in order of preference:

1. **Store execution summary in the archive**: Extend the `Block` type (or add a companion archive) to include `gas_used`, `receipts_root`, and `logs_bloom` alongside the block data. These fields are small (a few hundred bytes per block) and would make recovery fully accurate.

2. **Store execution results in a separate archive**: Create a parallel archive indexed by block height that stores `gas_used`, per-transaction receipts, and logs. Populate it during normal finalization and read it during recovery.

3. **Mark recovered blocks as incomplete**: Add an `is_partial` flag to `IndexedBlock`. Have the RPC layer either return appropriate error responses or include a warning header for queries on partial blocks.

4. **At minimum, emit warnings**: Log a warning during recovery indicating the height range with missing gas/receipt data, so operators are aware of the limitation.

```rust
// Option 4 (minimal):
if recovered > 0 {
    warn!(
        recovered_blocks = recovered,
        "recovered blocks indexed with gas_used=0 and no receipts; \
         historical RPC queries may return incomplete data"
    );
}
```

## Files to Modify

- `crates/node/runner/src/runner.rs` -- `index_recovered_block` function (lines 250-275)
- For option 1/2: block type or archive configuration in `crates/node/runner/` and `crates/network/marshal/`

## Related Issues

- `019-dual-base-fee-paths-diverge.md` -- incorrect fee data after restart (related root cause)
- `110-seed-block-archive-coverage-not-validated-on-recovery.md` -- another data quality issue in the same recovery path
- `104-recovery-loads-entire-block-archive-into-memory.md` -- the same recovery function, different concern (memory)
