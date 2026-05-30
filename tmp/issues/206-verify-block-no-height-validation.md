# No Block Height Validation in verify_block -- Height Can Skip or Repeat

**Category**: bug -- consensus
**Severity**: medium

## Summary

The `verify_block()` method in `RevmApplication` validates timestamp monotonicity, future timestamp drift, and state root correctness, but does not validate that `block.height == parent.height + 1`. A block with a skipped height (e.g., `parent.height + 5`) or a repeated height (e.g., `parent.height`) would pass verification as long as the state root is correct. While the commonware simplex consensus engine likely enforces height contiguity at the protocol level, the application layer does not independently verify it, creating a defense-in-depth gap.

## Problem

The `verify_block()` method performs the following checks:
- Timestamp monotonicity: `block.timestamp >= parent_timestamp` (lines 507-518)
- Future timestamp drift: `block.timestamp <= now + MAX_FUTURE_TIMESTAMP_DRIFT` (lines 522-533)
- State root correctness: re-executes the block and compares state roots (lines 645-673)

It does NOT check:
- Height contiguity: `block.height == parent.height + 1`
- Height uniqueness: that the height has not been seen before with a different digest

Height skips could cause subtle issues in downstream components:
- `BlockIndex::prune_before()` uses height-based pruning and assumes contiguous heights
- Checkpoint interval logic uses `height.is_multiple_of()`, which would behave incorrectly with gaps
- External monitoring and RPC queries assume contiguous block numbers

## Code Reference

File: `crates/node/runner/src/app.rs`, lines 469-725 (the `verify_block` method)

```rust
async fn verify_block(
    &self,
    block: &Block,
    parent_timestamp: Option<u64>,
    now_secs: u64,
) -> bool {
    let start = Instant::now();
    let digest = block.commitment();
    let parent_digest = block.parent();

    // ... already-verified early return (lines 479-496) ...

    // Timestamp validation (lines 498-534)
    if !self.is_catching_up(block.height) {
        if let Some(parent_ts) = parent_timestamp
            && block.timestamp < parent_ts
        {
            // reject: timestamp moved backwards
        }
        let max_allowed = now_secs.saturating_add(MAX_FUTURE_TIMESTAMP_DRIFT);
        if block.timestamp > max_allowed {
            // reject: timestamp too far in future
        }
    }

    // Parent snapshot lookup (lines 536-586)
    let parent_snapshot = match self.ledger.parent_snapshot(parent_digest).await {
        // ...
    };

    // EVM execution and state root verification (lines 589-673)
    // ...

    // NO height validation anywhere in this method
    // Missing: if block.height != parent.height + 1 { return false; }

    true
}
```

The `verify()` method that calls `verify_block()` (lines 851-914) collects blocks from the ancestry stream and verifies them oldest-to-newest, but does not check height contiguity between consecutive blocks in the verification chain either.

## Impact

1. **Defense-in-depth gap**: If the consensus engine has a bug that allows height skips or repeats, the application layer would silently accept them. Height monotonicity is a fundamental invariant of blockchain state machines.
2. **Downstream component confusion**: Components that assume contiguous heights (block indexer, pruning logic, checkpoint intervals) would behave incorrectly with gaps.
3. **External tooling breakage**: Block explorers, monitoring dashboards, and RPC clients that iterate through blocks by number would encounter missing or duplicate entries.

The practical risk is low because the commonware simplex consensus engine almost certainly enforces height contiguity at the view/round level. However, defense-in-depth is a standard practice in blockchain implementations -- the application layer should independently verify all invariants it depends on.

## Root Cause

The `verify_block()` method was designed to verify the state transition (timestamp, execution, state root) but did not include structural validation of the block height. The implementers likely relied on the consensus engine's guarantees for height ordering.

## Suggested Fix

Add a height contiguity check at the beginning of `verify_block()`, after the already-verified early return and before the timestamp validation. The parent's height is not directly available in `verify_block()`, but it is available from the `Block` struct (the parent block was passed to the ancestry stream).

However, since `verify_block()` receives the block and a `parent_timestamp` but not the full parent block, the check needs the parent height. The simplest approach is to also pass `parent_height`:

**Option A**: Add parent height to verify_block's signature and check it:

```rust
async fn verify_block(
    &self,
    block: &Block,
    parent_height: u64,
    parent_timestamp: Option<u64>,
    now_secs: u64,
) -> bool {
    // ...
    if block.height != parent_height + 1 {
        warn!(
            ?digest,
            block_height = block.height,
            expected_height = parent_height + 1,
            "verify_block: height not contiguous"
        );
        return false;
    }
    // ...
}
```

**Option B**: Look up the parent block's height from the block index or the parent snapshot in the snapshot store.

## Files to Modify

- `crates/node/runner/src/app.rs` -- Add height contiguity check in `verify_block()` (around line 498, before timestamp validation), and update the `verify()` method to pass parent height

## Related Issues

- None

## Labels

bug, correctness, consensus
