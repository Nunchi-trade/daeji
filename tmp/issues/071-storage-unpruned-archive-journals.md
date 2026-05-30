# 071: Disk Exhaustion via Unpruned Archive Journals (~1 GB/day)

**Category:** storage / reliability
**Severity:** high

## Summary

Kora initializes two prunable archive stores (`finalized_blocks` and `finalizations_by_height`) for consensus data, but no code path ever invokes `prune()` on them. Journal files grow without bound on disk. On a 10-node devnet producing ~33 blocks/s, each node accumulates approximately 1 GB of archive data per day with empty blocks, and far more under load. This will eventually exhaust disk space and crash the node.

## Problem

Kora is a minimal Ethereum-compatible execution client built on Commonware's Simplex BFT consensus. As part of consensus, two archives store finalized block data and finalization certificates on disk using Commonware's journaled storage. These archives are initialized as *prunable* (via `init_prunable_checkpointed`), meaning they are designed to support periodic deletion of old data. However, no code path ever calls the `prune()` method on either archive.

The archives are initialized in the validator runner at startup:

**File:** `crates/node/runner/src/runner.rs`, lines 988-1006

```rust
let finalizations_by_height =
    ArchiveInitializer::init_prunable_checkpointed::<_, ConsensusDigest, CertArchive>(
        context.child("finalizations_by_height"),
        finalizations_prefix,
        (),
        checkpoint_interval,
    )
    .await
    .context("init finalizations archive")?;

let finalized_blocks =
    ArchiveInitializer::init_prunable_checkpointed::<_, ConsensusDigest, Block>(
        context.child("finalized_blocks"),
        blocks_prefix,
        block_cfg,
        checkpoint_interval,
    )
    .await
    .context("init blocks archive")?;
```

The `prune()` trait implementations exist in the marshal archive wrappers, but are never called from any production code path:

**File:** `crates/network/marshal/src/archive.rs`, lines 232-234 (Certificates) and 269-271 (Blocks)

```rust
// Certificates::prune (line 232)
async fn prune(&mut self, min: Height) -> Result<(), Self::Error> {
    self.inner.prune(min.get()).await
}

// Blocks::prune (line 269)
async fn prune(&mut self, min: Height) -> Result<(), Self::Error> {
    self.inner.prune(min.get()).await
}
```

A codebase-wide search for `.prune(` in the main `crates/` directory confirms that the archive `prune()` methods are only defined in `crates/network/marshal/src/archive.rs` but never called. The only `prune()` calls in the codebase are for the transaction mempool (in `crates/node/ledger/src/lib.rs` and `crates/node/txpool/src/pool.rs`), which is a completely separate subsystem.

## Code Reference

**File:** `crates/node/runner/src/runner.rs:988-1006`
```rust
let finalizations_by_height =
    ArchiveInitializer::init_prunable_checkpointed::<_, ConsensusDigest, CertArchive>(
        context.child("finalizations_by_height"),
        finalizations_prefix,
        (),
        checkpoint_interval,
    )
    .await
    .context("init finalizations archive")?;

let finalized_blocks =
    ArchiveInitializer::init_prunable_checkpointed::<_, ConsensusDigest, Block>(
        context.child("finalized_blocks"),
        blocks_prefix,
        block_cfg,
        checkpoint_interval,
    )
    .await
    .context("init blocks archive")?;
```

**File:** `crates/network/marshal/src/archive.rs:232-234`
```rust
async fn prune(&mut self, min: Height) -> Result<(), Self::Error> {
    self.inner.prune(min.get()).await
}
```

## Impact

**Disk exhaustion leading to node crash.** Measured on a 10-node devnet producing ~28 blocks/s with empty blocks:

| Metric | Value |
|--------|-------|
| Per-block journal size | ~489 bytes |
| Disk growth per node (empty blocks) | ~1.09 GB/day |
| Disk growth (10-node cluster, empty) | ~10.95 GB/day |
| Time to fill 391 GB disk (empty blocks) | ~36 days |
| Time to fill 391 GB disk (light load) | ~3.6 days |
| Time to fill 391 GB disk (heavy load) | ~8.6 hours |

Once the disk fills, the node crashes and cannot restart until journal files are manually cleaned up. Each journal entry also contributes approximately 8 KiB/block to an in-memory index, causing steady memory pressure (~15.4 MiB/min at 33 blocks/s).

In a production deployment running under sustained load, this could cause node failures within hours.

## Root Cause

The Commonware marshal owns the archive handles and is responsible for invoking `prune()`. Either the marshal does not call `prune()` at all internally, or Kora's configuration does not enable pruning behavior in the marshal. Since the archives are owned by the marshal actor, Kora's `FinalizedReporter` (which processes finalized blocks on the application side) cannot call `prune()` directly without architectural changes to share the archive handles.

## Suggested Fix

**Option A (upstream):** File an issue or PR on Commonware to make the marshal call `prune()` after finalization acknowledgments accumulate past a configurable retention window.

**Option B (Kora-side workaround):** Refactor `FinalizedReporter` to hold `Arc` handles to both archives and call `prune()` directly after processing each finalized block:

```rust
// Before: no pruning occurs

// After: prune archives after processing each finalized block
const ARCHIVE_RETENTION_BLOCKS: u64 = 10_000; // ~5 min at 33 blocks/s

if block.height > ARCHIVE_RETENTION_BLOCKS {
    let min = block.height - ARCHIVE_RETENTION_BLOCKS;
    if let Err(e) = finalized_blocks.prune(Height::new(min)).await {
        warn!(error = %e, "failed to prune finalized_blocks archive");
    }
    if let Err(e) = finalizations.prune(Height::new(min)).await {
        warn!(error = %e, "failed to prune finalizations archive");
    }
}
```

This requires threading the archive handles from `runner.rs` into the reporter via shared ownership (`Arc<Mutex<...>>`) or a separate prune handle. The retention window (10,000 blocks) should align with `DEFAULT_PRUNABLE_ITEMS_PER_SECTION` (256) since pruning operates at section granularity.

## Files to Modify

- `crates/node/runner/src/runner.rs` (lines 988-1006) -- archive initialization; need to share handles for pruning
- `crates/network/marshal/src/archive.rs` (lines 232-234, 269-271) -- `prune()` trait implementations exist but are never called

## Related Issues

- `072-storage-state-root-not-mpt.md` -- another storage architecture concern (non-MPT state root)

## Labels

bug, reliability, storage, performance
