# Nodes Stuck in OOM Restart Loop During Catch-Up After Late Start

**Category**: docker, reliability, performance
**Severity**: high

**Labels**: `bug`, `reliability`, `performance`, `docker`, `storage`, `recovery`

## Summary

Nodes that restart significantly later than the rest of the cluster (e.g., nodes 8 and 9 on the live 10-node devnet, restarted approximately 4 hours after cluster start) become stuck in a perpetual OOM-kill restart loop. During catch-up, the node loads all archived blocks into an in-memory `BTreeMap`, consuming memory far beyond the 4GB Docker limit. Docker kills the node, restarts it, and the cycle repeats -- with the gap growing larger on each iteration, making recovery progressively harder.

## Problem

### Observed behavior on the live devnet

At block height `0x72F47` (471,879) on healthy nodes:
- Nodes 0-7: healthy, producing blocks at approximately 34 blocks/s
- Node 8: `health=starting`, unreachable RPC, multiple restarts
- Node 9: `health=starting`, unreachable RPC, 14 restarts

The cycle is:
1. Node starts, detects `last_committed_digest` exists (indicating a prior run)
2. Archive recovery begins -- `recover_finalized_state` loads ALL archived blocks into a `BTreeMap` in memory
3. QMDB restore from checkpoint, then replay of archived tail
4. Catch-up mode activates (catch-up threshold = 64 blocks, but the gap is approximately 470K blocks)
5. Memory climbs steadily during catch-up block verification
6. Node hits the 4GB Docker memory limit and is OOM-killed
7. Docker restarts the node (`restart: unless-stopped`), cycle repeats from step 1

### Archive recovery loads all blocks into memory

The `recover_finalized_state` function loads every archived block into a `BTreeMap`:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs:288-334
async fn recover_finalized_state<FB, FC>(
    ledger: &LedgerService,
    block_index: &Arc<kora_indexer::BlockIndex>,
    finalized_blocks: &FB,
    finalizations_by_height: &FC,
    provider: &RevmContextProvider,
    data_dir: &Path,
    chain_id: u64,
) -> anyhow::Result<Option<(u64, bool)>>
where
    FB: Archive<Key = ConsensusDigest, Value = Block>,
    FC: Archive<Key = ConsensusDigest, Value = CertArchive>,
{
    // ...
    let mut recovered = 0u64;
    let mut recovered_blocks = BTreeMap::new();
    for (start, end) in block_ranges {
        for height in start..=end {
            let Some(block) = finalized_blocks
                .get(ArchiveId::Index(height))
                .await
                // ...
            else {
                continue;
            };
            index_recovered_block(block_index, &block, provider);
            recovered_blocks.insert(height, block);  // <-- unbounded memory growth
            recovered += 1;
        }
    }
    // ...
}
```

### Small catch-up threshold

The catch-up threshold is 64 blocks:

```rust
// /Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:86
const CATCH_UP_THRESHOLD: u64 = 64;
```

This was designed for small gaps. For a 470K block gap, the node processes the vast majority of blocks in catch-up mode.

### Catch-up mode logic

```rust
// /Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:452-467
fn is_catching_up(&self, block_height: u64) -> bool {
    let recovered = self.recovered_height.load(Ordering::Relaxed);
    if recovered == 0 {
        return false;
    }
    if block_height <= recovered {
        return false;
    }
    let verified = self.last_verified_height.load(Ordering::Relaxed);
    verified < recovered.saturating_add(CATCH_UP_THRESHOLD)
}
```

### Docker memory limit

The compose file sets a 4GB memory limit:

```yaml
# /Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml:55-57
deploy:
  resources:
    limits:
      memory: 4G
```

Steady-state usage is approximately 230MB, so 4GB is approximately 17x steady-state -- but insufficient for loading hundreds of thousands of archived blocks into a `BTreeMap`.

### devnet-run.sh clears progress

If the operator uses `devnet-run.sh` to restart, runtime state is wiped:

```bash
# /Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh:316-319
docker compose -f compose/devnet.yaml stop \
    validator-node0 validator-node1 validator-node2 validator-node3 secondary-node0 >/dev/null 2>&1 || true
clear_runtime_state
```

This means any partial catch-up progress is lost, forcing the node to start from scratch.

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, lines 288-334 (`recover_finalized_state` -- loads all blocks into BTreeMap)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, line 286 (`SNAPSHOT_PREPOPULATE_COUNT = 64`)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, lines 360-461 (`restore_checkpoint_and_replay_tail`)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs`, line 86 (`CATCH_UP_THRESHOLD = 64`)
**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs`, lines 452-467 (`is_catching_up`)
**File**: `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml`, line 56 (memory limit: 4G)
**File**: `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh`, lines 162-173, 316-319 (`clear_runtime_state`)

## Impact

- **Unavailable nodes**: Nodes that restart after a significant delay cannot rejoin the cluster, permanently reducing the effective validator set (e.g., from 10 to 8).
- **Perpetual restart loop**: Each OOM-kill is followed by a Docker restart, which re-enters catch-up, which OOMs again. The gap grows with each restart, making recovery progressively harder.
- **Quorum risk**: The 10-node cluster requires 7 nodes for quorum. With 2 nodes in OOM loops, only 1 more failure puts the cluster at risk of losing quorum entirely.
- **No alerting**: The `HighMemoryUsage` Prometheus alert (defined in `docker/config/alerts.yml:101-108`) would detect this pattern, but the observability stack is not enabled on the live devnet.

## Root Cause

Unbounded memory consumption during catch-up when a node must bridge a large block gap. Multiple subsystems contribute:

1. **`recover_finalized_state`** (runner.rs:288): Loads ALL archived blocks into an in-memory `BTreeMap`. Designed for small gaps (64 blocks per `SNAPSHOT_PREPOPULATE_COUNT`) but the archive can grow to hundreds of thousands of blocks.
2. **Catch-up threshold too small**: `CATCH_UP_THRESHOLD = 64` means the catch-up window only covers 64 blocks beyond the recovery point.
3. **No backpressure on resolver**: The Commonware resolver fetches blocks as fast as peers will serve them, without regard for memory consumption.
4. **Memory limit too low for catch-up workload**: 4GB is sufficient for steady-state but not for loading 470K+ blocks.

## Suggested Fix

### Immediate mitigation

1. **Increase memory limits** for nodes expected to catch up (e.g., 8GB or 12GB).
2. **Stop clearing runtime state on restart**: Remove or make conditional the `clear_runtime_state` call in `devnet-run.sh` so that OOM-restarted nodes retain partial catch-up progress.

### Architectural fixes

3. **Stream-process archived blocks**: Modify `recover_finalized_state` to process blocks in bounded batches rather than loading them all into a `BTreeMap`. Process each batch (e.g., 1000 blocks at a time), apply to QMDB, release memory, then process the next batch.

4. **Add memory-aware backpressure**: Implement a mechanism that slows or pauses block fetching from the resolver when process RSS approaches a configurable threshold (e.g., 80% of the container memory limit).

5. **Implement incremental catch-up**: Process catch-up blocks in bounded batches, checkpoint QMDB between batches, and release memory before processing the next batch. This ensures that if the node is OOM-killed, it can resume from the last checkpoint rather than restarting from the beginning.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- Modify `recover_finalized_state` to stream blocks in batches instead of loading all into BTreeMap (lines 288-334); adjust `SNAPSHOT_PREPOPULATE_COUNT` (line 286)
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- Consider dynamic catch-up threshold (line 86)
- `/Users/will/dev/nunchi/daeji/docker/compose/devnet.yaml` -- Increase memory limit and/or add catch-up-specific limits (line 56)
- `/Users/will/dev/nunchi/daeji/docker/scripts/devnet-run.sh` -- Make `clear_runtime_state` conditional (lines 162-173, 316-319)

## Related Issues

- `094-docker-no-qmdb-backup.md` -- No backup mechanism (each OOM restart loses runtime state)
- `097-docker-observability-not-enabled.md` -- Observability not enabled (alerting would detect OOM loops earlier)
- `093-docker-10node-compose-not-in-vcs.md` -- Unversioned 10-node compose (different memory limits)
