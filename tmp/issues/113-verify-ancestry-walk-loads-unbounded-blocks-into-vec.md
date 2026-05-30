# Verify Ancestry Walk Collects Unbounded Blocks Into Vec

**Category**: Consensus / Memory
**Severity**: Medium
**Labels**: `bug`, `performance`, `reliability`, `consensus`

## Summary

The `verify` method in `RevmApplication` collects all unverified ancestor blocks into a `Vec<Block>` in memory before beginning verification. If the node is far behind the network (e.g., after a network partition or slow restart), this can load thousands of full blocks into memory at once, causing a memory spike proportional to the gap between the node's verified height and the network tip.

## Problem

The `verify` method in `crates/node/runner/src/app.rs` (lines 851-899) is called by the simplex consensus engine to verify a proposed block and its ancestry chain. It walks the ancestry stream (which yields blocks from newest to oldest) and collects every block that has not yet been verified (i.e., has no known state root in the ledger) into a `Vec`. The walk continues until it finds a block with a known state root or exhausts the ancestry stream.

There is no upper bound on how many blocks are collected. During normal operation the gap is typically 1-2 blocks (just the newly proposed block and perhaps its parent). But during catch-up scenarios -- after a network partition, slow restart, or when a node was offline for an extended period -- the gap can be thousands or tens of thousands of blocks.

## Code Reference

`crates/node/runner/src/app.rs` lines 867-877:

```rust
let mut blocks_to_verify = Vec::new();
let mut verified_parent_timestamp: Option<u64> = None;
while let Some(block) = ancestry.next().await {
    let digest = block.commitment();
    // Stop if we've already verified this block
    if self.ledger.query_state_root(digest).await.is_some() {
        verified_parent_timestamp = Some(block.timestamp);
        break;
    }
    blocks_to_verify.push(block);
}
let ancestry_elapsed = start.elapsed();
```

After collection, the blocks are verified in reverse order (oldest to newest) at lines 894-899:

```rust
let mut parent_ts = verified_parent_timestamp;
for block in blocks_to_verify.into_iter().rev() {
    if !self.verify_block(&block, parent_ts, now_secs).await {
        return false;
    }
    parent_ts = Some(block.timestamp);
}
```

## Impact

- **Memory spike during catch-up**: At ~10 KB per block (header + transactions), a gap of 100,000 blocks requires ~1 GB of RAM just for the ancestry Vec. On the devnet configuration of 4 GB per node, this can cause OOM.
- **Verification delay**: All blocks must be collected before any verification begins. With a large gap, the node spends significant time downloading and buffering blocks before starting any work. A streaming approach would allow verification to begin sooner.
- **Amplified by catch-up frequency**: If multiple proposals arrive while the node is catching up, each triggers a separate `verify()` call with its own ancestry walk, potentially multiplying the memory usage.

## Root Cause

The blocks must be collected into a Vec because the ancestry stream yields blocks in newest-to-oldest order, but verification must proceed oldest-to-newest (to validate timestamp monotonicity and parent chain). The current implementation buffers the entire stream, reverses it, then verifies sequentially. There is no depth limit on the ancestry walk.

## Suggested Fix

**Option 1: Cap the ancestry walk depth.** Add a maximum depth (e.g., `CATCH_UP_THRESHOLD` or a configurable limit) to the ancestry walk. If the gap exceeds the limit, return `false` and let the resolver/catch-up mechanism handle the gap:

Before:
```rust
while let Some(block) = ancestry.next().await {
    let digest = block.commitment();
    if self.ledger.query_state_root(digest).await.is_some() {
        verified_parent_timestamp = Some(block.timestamp);
        break;
    }
    blocks_to_verify.push(block);
}
```

After:
```rust
const MAX_ANCESTRY_DEPTH: usize = 256;
while let Some(block) = ancestry.next().await {
    let digest = block.commitment();
    if self.ledger.query_state_root(digest).await.is_some() {
        verified_parent_timestamp = Some(block.timestamp);
        break;
    }
    blocks_to_verify.push(block);
    if blocks_to_verify.len() >= MAX_ANCESTRY_DEPTH {
        warn!(
            depth = blocks_to_verify.len(),
            "ancestry walk exceeded max depth, deferring to catch-up"
        );
        return false;
    }
}
```

**Option 2: Stream-and-verify with bounded buffer.** Use a fixed-size sliding window: collect blocks in chunks, verify each chunk, then discard it before loading the next. This requires storing verified block digests temporarily but avoids loading the entire chain at once.

**Option 3: Two-pass approach.** First pass: walk the ancestry collecting only heights and digests (not full blocks). Second pass: load and verify blocks one at a time in reverse order. This trades an extra ancestry traversal for bounded memory.

Option 1 is simplest and safest, as it aligns with the existing catch-up mechanism that handles large gaps.

## Files to Modify

- `crates/node/runner/src/app.rs` -- `verify` method (lines 851-899)

## Related Issues

- `104-recovery-loads-entire-block-archive-into-memory.md` -- similar unbounded memory allocation during startup recovery
- `027-catch-up-mode-persists-indefinitely.md` -- catch-up mode behavior when node is far behind
- `008-catch-up-silent-state-divergence.md` -- catch-up path correctness concerns
