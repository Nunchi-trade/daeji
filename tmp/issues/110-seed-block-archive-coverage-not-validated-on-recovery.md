# Seed and Block Archive Coverage Not Cross-Validated on Recovery

**Category**: Recovery / Consistency
**Severity**: Medium
**Labels**: `bug`, `correctness`, `reliability`, `recovery`

## Summary

During startup recovery, Kora loads finalization certificates (containing consensus seeds) and finalized blocks from two independent archives. There is no validation that these two archives cover the same height range, are contiguous, or are mutually consistent. If one archive was pruned more aggressively than the other, the node silently restarts with missing or mismatched data, leading to incorrect prevrandao values and potentially divergent leader election.

## Problem

The `recover_finalized_state` function in `crates/node/runner/src/runner.rs` (lines 288-358) iterates over two independent archives:

1. **Finalization archive** (`finalizations_by_height`): Contains consensus certificates with seed values used for the `PREVRANDAO` opcode and leader election. Iterated at lines 304-316.
2. **Block archive** (`finalized_blocks`): Contains the full block data (header + transactions). Iterated at lines 320-334.

Each archive independently reports its own `ranges()`. The function iterates both without any cross-validation:
- It does not check that both archives cover the same height range.
- It does not check for gaps within either archive.
- It does not verify that every finalized block has a corresponding finalization certificate (and vice versa).

## Code Reference

`crates/node/runner/src/runner.rs` lines 300-334:

```rust
{
    let block_ranges: Vec<_> = finalized_blocks.ranges().collect();
    let finalization_ranges: Vec<_> = finalizations_by_height.ranges().collect();

    for (start, end) in finalization_ranges {
        for height in start..=end {
            if let Some(finalization) = finalizations_by_height
                .get(ArchiveId::Index(height))
                .await
                .with_context(|| format!("load finalization at height {height}"))?
            {
                ledger
                    .set_seed(finalization.proposal.payload, seed_hash(finalization.seed()))
                    .await;
            }
        }
    }

    let mut recovered = 0u64;
    let mut recovered_blocks = BTreeMap::new();
    for (start, end) in block_ranges {
        for height in start..=end {
            let Some(block) = finalized_blocks
                .get(ArchiveId::Index(height))
                .await
                .with_context(|| format!("load finalized block at height {height}"))?
            else {
                continue;
            };

            index_recovered_block(block_index, &block, provider);
            recovered_blocks.insert(height, block);
            recovered += 1;
        }
    }
```

Note that the finalization loop uses `if let Some(finalization) = ...` (silently skipping missing entries) and the block loop uses `else { continue; }` (also silently skipping). Neither emits a warning for missing data.

## Impact

- **Incorrect PREVRANDAO**: If finalization certificates are missing for some heights but the corresponding blocks exist, `ledger.set_seed()` is never called for those heights. When blocks at those heights are later used in `get_prevrandao` lookups, they return `B256::ZERO` instead of the correct seed hash. Smart contracts relying on `PREVRANDAO` for randomness (e.g., NFT minting, lottery contracts) will get deterministic zero values.
- **Divergent leader election**: The simplex consensus engine uses `Random` elector, which derives the next leader from the finalization seed. If a restarting node has different seed state than the rest of the network (due to missing certificates), it will compute different leader assignments, failing to notarize proposals it sees as illegitimate and proposing at wrong views.
- **Silent divergence**: No errors, warnings, or panics are emitted. The node appears healthy but operates with incorrect state, which may only manifest as elevated nullification rates or consensus stalls hours later.

## Root Cause

The two archives are treated as independent data sources. Each is iterated in isolation and missing entries are silently skipped. There is no post-recovery consistency check.

## Suggested Fix

After loading both archives, validate that:
1. The block and finalization archives have overlapping coverage for at least the recent recovery window (last `DEFAULT_CHECKPOINT_INTERVAL + SNAPSHOT_PREPOPULATE_COUNT` blocks).
2. Within the overlapping range, every block height has a corresponding finalization certificate.
3. Emit an error log and return an error (or panic) if the coverage is inconsistent, rather than silently proceeding.

```rust
// After collecting ranges:
let block_max = block_ranges.last().map(|(_, end)| *end).unwrap_or(0);
let finalization_max = finalization_ranges.last().map(|(_, end)| *end).unwrap_or(0);

if block_max != finalization_max {
    warn!(
        block_max,
        finalization_max,
        "archive coverage mismatch: block and finalization archives \
         have different head heights"
    );
}

// For the recovery window, verify both archives are present:
let min_required = block_max.saturating_sub(checkpoint_interval + SNAPSHOT_PREPOPULATE_COUNT);
for height in min_required..=block_max {
    let has_block = finalized_blocks.get(ArchiveId::Index(height)).await?.is_some();
    let has_cert = finalizations_by_height.get(ArchiveId::Index(height)).await?.is_some();
    if has_block != has_cert {
        return Err(anyhow::anyhow!(
            "archive inconsistency at height {height}: block={has_block}, cert={has_cert}"
        ));
    }
}
```

## Files to Modify

- `crates/node/runner/src/runner.rs` -- `recover_finalized_state` function (lines 288-358)

## Related Issues

- `104-recovery-loads-entire-block-archive-into-memory.md` -- the same recovery function, different concern (memory)
- `054-recovery-crash-during-replay-inconsistent-marker.md` -- another recovery path consistency issue
- `111-index-recovered-block-records-gas-used-zero.md` -- data quality issue in the same recovery path
