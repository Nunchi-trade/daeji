# Recovery Startup Loads Entire Block Archive Into Memory

**Category**: Recovery / Memory
**Severity**: High
**Labels**: `bug`, `reliability`, `performance`, `recovery`

## Summary

When a Kora validator restarts, the `recover_finalized_state` function iterates over every block ever finalized and loads them all into a `BTreeMap<u64, Block>` in memory. For a node that has been running for hours or days at 30+ blocks/second, this means loading hundreds of thousands of full blocks (each containing transaction data) into RAM, likely causing an out-of-memory crash on memory-constrained nodes.

## Problem

The function `recover_finalized_state` in `crates/node/runner/src/runner.rs` (lines 288-358) unconditionally iterates over all ranges returned by `finalized_blocks.ranges()` and loads every single block into a `BTreeMap<u64, Block>` in memory. There is no upper bound on the number of blocks loaded, and no filtering to only load the blocks that downstream recovery steps actually need.

At Kora's production throughput of ~30 blocks/second, a node running for 24 hours accumulates ~2.6 million blocks. Even at ~1 KB per empty block, this amounts to multiple gigabytes of memory. With transactions included, memory usage can be far higher. The current devnet configuration allocates only 4 GB RAM per node.

## Code Reference

`crates/node/runner/src/runner.rs` lines 318-334:

```rust
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

The same function also loads all finalization certificates into the seed cache (lines 304-316):

```rust
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
```

## Impact

- **OOM on restart**: Nodes that have been running for extended periods will exhaust memory during restart, entering a crash-restart loop. This is especially severe on the 4 GB-per-node devnet configuration.
- **Restart time**: Even if memory is sufficient, loading millions of blocks from disk is extremely slow, delaying time-to-participation by minutes or longer.
- **Wasted work**: The entire archive is loaded even though downstream consumers have bounded needs. The `recovered_blocks` BTreeMap is passed to `restore_checkpoint_and_replay_tail` (line 337), which only reads blocks from the checkpoint height to HEAD -- typically the last 256 blocks (the `DEFAULT_CHECKPOINT_INTERVAL`).

## Root Cause

The recovery loop has no upper bound on iteration. It loads every block the archive contains, regardless of how many blocks downstream recovery actually needs. The three downstream consumers have bounded requirements:

1. `seed_block_fee_cache` -- needs only the last ~5 blocks for EIP-1559 base fee recovery.
2. `prepopulate_snapshot_cache` -- needs at most 64 blocks (`SNAPSHOT_PREPOPULATE_COUNT`).
3. `restore_checkpoint_and_replay_tail` -- needs blocks from the checkpoint to HEAD, at most `DEFAULT_CHECKPOINT_INTERVAL` (256) blocks.

The maximum required window is therefore roughly `max(5, 64, 256) = 256` blocks before HEAD.

## Suggested Fix

Compute the minimum required height and only load blocks from that height onward. The finalization/seed loop should also be bounded since seeds beyond the recent window are not needed for correct operation.

Before:
```rust
let mut recovered_blocks = BTreeMap::new();
for (start, end) in block_ranges {
    for height in start..=end {
        // loads ALL blocks
    }
}
```

After:
```rust
// Determine the archive head height from the last range.
let archive_head = block_ranges.last().map(|(_, end)| *end).unwrap_or(0);
// Only load blocks within the recovery window.
let recovery_window = DEFAULT_CHECKPOINT_INTERVAL + SNAPSHOT_PREPOPULATE_COUNT + 16; // margin
let min_height = archive_head.saturating_sub(recovery_window);

let mut recovered_blocks = BTreeMap::new();
for (start, end) in block_ranges {
    let effective_start = start.max(min_height);
    if effective_start > end {
        continue; // skip ranges entirely before the window
    }
    for height in effective_start..=end {
        // load block ...
    }
}
```

The same approach should be applied to the finalization/seed loop (lines 304-316), bounding it to the same recovery window.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- function `recover_finalized_state` (lines 288-358)

## Related Issues

- `005-memory-exhaustion-devnet.md` -- runtime memory exhaustion (different root cause: runtime growth, not startup)
- `098-docker-oom-restart-loop.md` -- OOM restart loop in Docker (this issue is one concrete trigger)
- `113-verify-ancestry-walk-loads-unbounded-blocks-into-vec.md` -- similar unbounded memory pattern in the consensus verification path
