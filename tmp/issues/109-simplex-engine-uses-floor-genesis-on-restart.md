# Simplex Engine and Marshal Both Use Genesis Floor/Start on Restart

**Category**: Consensus / Restart Resilience
**Severity**: High
**Labels**: `bug`, `reliability`, `consensus`, `recovery`

## Summary

When a Kora validator restarts after having finalized thousands of blocks, both the simplex consensus engine and the marshal are initialized with `Floor::Genesis` / `Start::Genesis` unconditionally. This forces the engine to replay all historical views from view 0 and causes the marshal to re-process the entire finalized archive from genesis, wasting significant time and resources before the node can participate in current consensus.

## Problem

Two components are misconfigured on restart:

**1. Simplex engine floor** (`crates/node/runner/src/runner.rs` line 1437): The simplex engine is always initialized with `simplex::Floor::Genesis(...)`, telling it the lowest acceptable view is 0. After a restart where the node previously finalized block N, the engine must walk through all views from 0 to the network's current view before it can participate. While the marshal/resolver handles actual data replay, the engine still processes every historical view, sending stale votes that the network will ignore.

**2. Marshal start position** (`crates/node/runner/src/runner.rs` line 1375): The marshal is always initialized with `Start::Genesis(ledger.genesis_block())`. After the node has been running for an extended period and the archive's stored genesis block digest no longer matches the marshal's internal expectations (due to archive pruning or rotation), the marshal can panic on restart with a genesis anchor mismatch.

The project's MEMORY.md documents a previous fix for the marshal (`Start::Floor(finalization)` when `recovered_head_height` is `Some`), but that fix has not been applied to the current codebase on the main branch.

## Code Reference

Simplex engine configuration -- `crates/node/runner/src/runner.rs` lines 1424-1452:

```rust
let engine = simplex::Engine::new(
    scratch_context.child("engine"),
    simplex::Config {
        scheme: self.scheme.clone(),
        elector: Random,
        blocker: transport.oracle.clone(),
        automaton: marshaled.clone(),
        relay: marshaled,
        reporter,
        strategy,
        partition: self.partition_prefix.clone(),
        mailbox_size: NZUsize!(MAILBOX_SIZE),
        epoch: Epoch::zero(),
        floor: simplex::Floor::Genesis(ledger.genesis_block().commitment()),
        // ...
    },
);
```

Marshal initialization -- `crates/node/runner/src/runner.rs` lines 1369-1380:

```rust
let (actor, marshal_mailbox, _last_processed_height) =
    kora_marshal::ActorInitializer::init_with_strategy::<_, Block, _, _, _, Exact, _>(
        scratch_context.clone(),
        finalizations_by_height,
        finalized_blocks,
        scheme_provider,
        commonware_consensus::marshal::Start::Genesis(ledger.genesis_block()),
        page_cache.clone(),
        block_cfg,
        strategy.clone(),
    )
    .await;
```

The `recovered_head_height` variable is available at this point (computed at line 1148), but is only used for the application and snapshot cache -- not for the marshal or simplex engine floor.

## Impact

- **Slow restart**: After a long run, the simplex engine replays potentially millions of historical views before reaching the current network view. At 30 blocks/s, a 24-hour run accumulates ~2.6M views. This can delay time-to-participation by minutes.
- **Wasted bandwidth**: The engine sends stale notarize/finalize votes for historical views. These are harmless (peers ignore them) but waste network bandwidth and CPU on both the restarting node and its peers.
- **Marshal panic risk**: If the archive has been pruned or the genesis block in the archive has a different digest than expected (due to archive rotation), `Start::Genesis` causes the marshal to panic with an anchor mismatch. This was observed and documented in the project's recovery fixes.
- **Crash-restart loop**: The marshal panic on restart is fatal and the node will crash-loop indefinitely until manual intervention (archive wipe or code fix).

## Root Cause

The `run()` method unconditionally uses genesis-based initialization for both the marshal and the simplex engine, without checking whether the node has finalized blocks from a previous run. The `recovered_head_height` variable (line 1148) provides the information needed to make this decision, but it is not used to set the marshal start or engine floor.

## Suggested Fix

When `recovered_head_height` is `Some`, use `Start::Floor` for the marshal and `Floor::Finalization` for the simplex engine, based on the last finalization from the archive.

Before (marshal):
```rust
commonware_consensus::marshal::Start::Genesis(ledger.genesis_block()),
```

After (marshal):
```rust
if recovered_head_height.is_some() {
    let last_idx = finalizations_by_height.last_index();
    let last_finalization = finalizations_by_height.get(ArchiveId::Index(last_idx)).await?.unwrap();
    commonware_consensus::marshal::Start::Floor(last_finalization)
} else {
    commonware_consensus::marshal::Start::Genesis(ledger.genesis_block())
},
```

Before (simplex engine):
```rust
floor: simplex::Floor::Genesis(ledger.genesis_block().commitment()),
```

After (simplex engine):
```rust
floor: if recovered_head_height.is_some() {
    // Use the last finalization commitment as the floor
    simplex::Floor::Finalization(last_finalization_commitment)
} else {
    simplex::Floor::Genesis(ledger.genesis_block().commitment())
},
```

The exact API for `Floor::Finalization` depends on the commonware simplex version but typically takes the finalization certificate or its commitment.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- marshal initialization (line 1375) and simplex engine config (line 1437)

## Related Issues

- `104-recovery-loads-entire-block-archive-into-memory.md` -- related startup recovery inefficiency
- `110-seed-block-archive-coverage-not-validated-on-recovery.md` -- archive consistency on recovery
- `054-recovery-crash-during-replay-inconsistent-marker.md` -- crash during replay with inconsistent state
