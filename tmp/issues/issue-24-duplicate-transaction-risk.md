# Excluded transaction set only walks unpersisted snapshots -- finalized transactions can be re-proposed

## Summary

When building a new block proposal, the `collect_pending_tx_ids` method in `RevmApplication` (and its equivalent in `kora_consensus::proposal`) constructs an "excluded" set of transaction IDs that should not be re-proposed. This excluded set is built by walking the snapshot chain from the current parent backward, collecting tx IDs from each snapshot. However, the walk **stops at the first persisted snapshot** -- meaning any transactions included in already-persisted (finalized) blocks that are still in the mempool will not appear in the excluded set. If the mempool has not yet pruned those transactions (e.g., due to a race between finalization pruning and proposal building), they can be re-proposed in a new block.

## Priority

**P1 -- Operational Reliability**

In practice, the finalization reporter prunes the mempool when blocks are finalized (see `reporters/src/lib.rs` line 154), which usually removes transactions before the next proposal. However, there is a timing window where proposal building races with finalization pruning, and the excluded set provides no safety net for this race. The risk is low under normal operation but increases under load or when a node falls behind in finalization processing.

## Problem Description

### The excluded set construction

**File**: `crates/node/runner/src/app.rs`, lines 276-296

```rust
fn collect_pending_tx_ids(
    &self,
    snapshots: &InMemorySnapshotStore<OverlayState<QmdbState>>,
    from: ConsensusDigest,
) -> BTreeSet<kora_consensus::TxId> {
    let mut excluded = BTreeSet::new();
    let mut current = Some(from);

    while let Some(digest) = current {
        if snapshots.is_persisted(&digest) {
            break;                              // <-- STOPS HERE
        }
        let Some(snapshot) = snapshots.get(&digest) else {
            break;
        };
        excluded.extend(snapshot.tx_ids.iter().copied());
        current = snapshot.parent;
    }

    excluded
}
```

The method walks the chain of snapshots from `from` (the parent of the block being proposed) backward toward the genesis. At each step, it checks `snapshots.is_persisted(&digest)`. Once it hits a persisted snapshot, it stops and returns the collected set.

The identical pattern exists in the consensus crate:

**File**: `crates/node/consensus/src/proposal.rs`, lines 176-192

```rust
fn collect_pending_tx_ids(&self, from: Digest) -> Result<BTreeSet<TxId>, ConsensusError> {
    let mut excluded = BTreeSet::new();
    let mut current = Some(from);

    while let Some(digest) = current {
        if self.snapshots.is_persisted(&digest) {
            break;
        }

        let snapshot =
            self.snapshots.get(&digest).ok_or(ConsensusError::SnapshotNotFound(digest))?;
        excluded.extend(snapshot.tx_ids.iter().copied());
        current = snapshot.parent;
    }

    Ok(excluded)
}
```

And in the e2e test harness:

**File**: `crates/e2e/src/harness.rs`, lines 799-819

### Why this is correct in the common case

The design assumption is:
1. When a block is finalized, the `FinalizedReporter` calls `state.prune_mempool(&block.txs)` (file `crates/node/reporters/src/lib.rs`, line 154)
2. This removes the finalized transactions from the mempool via `Mempool::prune()` (file `crates/node/txpool/src/pool.rs`, lines 628-674)
3. Therefore, by the time we propose the next block, the finalized transactions are already gone from the mempool, so they cannot appear in `mempool.build()` output regardless of the excluded set

### The gap: timing race between finalization and proposal

The race condition:

```
Timeline:
  T1: Block N is finalized by consensus
  T2: FinalizedReporter::report() is called asynchronously
  T3: Leader is asked to propose Block N+2 (parent = Block N+1, which references N)
  T4: collect_pending_tx_ids walks from N+1, hits N (persisted), stops
  T5: mempool.build() is called -- transactions from Block N may still be in mempool
  T6: FinalizedReporter finishes processing and calls prune_mempool() for Block N
```

If the proposal at T3-T5 happens before the prune at T6, transactions from Block N could be:
- Present in the mempool (not yet pruned)
- NOT in the excluded set (because Block N is already marked as persisted)
- Selected by `mempool.build()` for inclusion in the new proposal

### What happens if a transaction is re-proposed

If a duplicate transaction makes it into a proposed block:
1. During execution, the EVM will reject it (nonce already used), so it will be skipped
2. This wastes block space and gas computation
3. It does not cause a consensus safety violation (the state root will still be deterministic)
4. However, it degrades throughput and indicates a correctness gap in the deduplication logic

### The `prune()` mechanism itself is correct

**File**: `crates/node/txpool/src/pool.rs`, lines 628-674

The `prune()` method correctly removes transactions by matching on `TxId` and advances sender nonces:

```rust
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();
    let mut confirmed_by_sender: HashMap<Address, u64> = HashMap::new();
    for id in tx_ids {
        let Some(hash) = inner.by_id.get(id) else { continue; };
        if let Some(tx) = inner.by_hash.get(hash) {
            confirmed_by_sender
                .entry(tx.sender)
                .and_modify(|nonce| *nonce = (*nonce).max(tx.nonce))
                .or_insert(tx.nonce);
        }
    }
    // ... removes all txs with nonce <= confirmed_nonce per sender ...
}
```

The issue is not with pruning itself, but with the gap between persistence marking and pruning execution.

## Fix

### Option A: Include the persisted snapshot's tx_ids in the excluded set

The simplest fix: instead of stopping *before* the persisted snapshot, include its tx_ids too:

```rust
fn collect_pending_tx_ids(
    &self,
    snapshots: &InMemorySnapshotStore<OverlayState<QmdbState>>,
    from: ConsensusDigest,
) -> BTreeSet<kora_consensus::TxId> {
    let mut excluded = BTreeSet::new();
    let mut current = Some(from);

    while let Some(digest) = current {
        let is_persisted = snapshots.is_persisted(&digest);
        if let Some(snapshot) = snapshots.get(&digest) {
            excluded.extend(snapshot.tx_ids.iter().copied());
            current = snapshot.parent;
        } else {
            break;
        }
        if is_persisted {
            break;  // Include persisted snapshot's txs, then stop
        }
    }

    excluded
}
```

This adds one extra snapshot's worth of tx_ids to the excluded set (the most recently finalized block), which closes the race window. The overhead is negligible since it is at most one additional snapshot.

Apply the same change in `crates/node/consensus/src/proposal.rs` and `crates/e2e/src/harness.rs`. Note that in `proposal.rs` the function returns `Result<BTreeSet<TxId>, ConsensusError>`, so the fix must preserve that return type and the `Ok(excluded)` return.

### Option B: Ensure pruning happens before proposal building

Make `build_block` explicitly check that the mempool has been pruned for the parent chain before calling `mempool.build()`. This is more invasive and harder to guarantee atomically.

### Recommended approach

Option A is simpler, has no performance impact, and provides defense-in-depth alongside the existing prune-on-finalize mechanism.

## Effort Estimate

**2-4 hours**:
- 30 minutes to implement the fix in all three locations
- 1 hour to add unit tests verifying the excluded set includes the persisted snapshot's txs
- 1-2 hours to run the e2e test suite and verify no regressions

## Affected Files

| File | Lines | Change |
|------|-------|--------|
| `crates/node/runner/src/app.rs` | 276-296 | Include persisted snapshot tx_ids before breaking |
| `crates/node/consensus/src/proposal.rs` | 176-192 | Same change (preserving `Result` return type) |
| `crates/e2e/src/harness.rs` | 799-819 | Same change |

## Testing Checklist

- [ ] Unit test: verify excluded set includes tx_ids from the first persisted ancestor
- [ ] Unit test: verify excluded set still stops after the persisted ancestor (does not walk infinitely)
- [ ] E2e test: submit transactions, finalize block, immediately propose next block -- verify no duplicates
- [ ] E2e test: verify block building under high load does not re-propose finalized transactions
