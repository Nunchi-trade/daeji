# Triple Block Execution -- One Third of EVM Compute Is Wasted

**Category**: Performance
**Severity**: High
**Labels**: `performance`, `executor`, `consensus`

## Summary

Every block in the Kora consensus client is executed via REVM up to three separate times: once during proposal, once during verification, and once during finalization. The third execution (finalization) exists solely to regenerate `ExecutionOutcome` data (receipts, gas_used, logs) for RPC indexing -- data that was already computed during verification but discarded. Eliminating this redundant execution would reduce total EVM compute by approximately 33%.

## Problem

The Kora node processes each block through three execution phases, each running the full EVM over all transactions:

**1. Proposal** (`build_block` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:257-432`): The leader validator executes all transactions to produce a changeset and state root. The `ExecutionOutcome` (receipts, logs, gas_used) is computed but only `gas_used` is retained (for block fee caching at line 410). Receipts and logs are discarded.

**2. Verification** (`verify_block` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:469-725`): Every other validator re-executes all transactions via `BlockExecution::execute()` at line 594 to verify that the state root matches the proposed block. The `ExecutionOutcome` is computed but only `changes` (the state diff) is retained for snapshot insertion at line 687. Receipts and logs are again discarded.

**3. Finalization** (`finalize_block` at `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:476-615`): The `FinalizedReporter` re-executes the block a third time to produce `ExecutionOutcome` for RPC indexing (receipts, transaction records, event logs). This is the only execution whose receipt data is actually used.

The finalization re-execution is explicitly triggered by the condition at `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:494`:

```rust
if !snapshot_exists || block_index.is_some() {
```

When a block index is configured (which it always is in production), every finalized block is re-executed, even if the snapshot already exists from verification.

## Code Reference

**Proposal execution** (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:348-384`):

```rust
let outcome = {
    let executor = self.executor.clone();
    let state = parent_snapshot.state.clone();
    match tokio::task::spawn_blocking(move || {
        executor.execute(&state, &context, &txs_bytes)
    })
    .await
    {
        Ok(Ok(outcome)) => outcome,
        // ...
    }
};
// outcome.changes is used, outcome.receipts/gas_used are discarded
```

**Verification execution** (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:593-617`):

```rust
let execution =
    match BlockExecution::execute(&parent_snapshot, &self.executor, &context, &block.txs)
        .await
    {
        Ok(result) => result,
        // ...
    };
// execution.outcome.changes used for snapshot, receipts/logs discarded
```

**Finalization re-execution** (`/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:528-563`):

```rust
if let Some(parent_snapshot) = parent_snapshot {
    let block_context = provider.context(block);
    let execution =
        BlockExecution::execute(&parent_snapshot, executor, &block_context, &block.txs)
            .await
            .map_err(|err| FinalizationError::ExecutionFailed(Box::new(err)))?;
    // ... state root verification ...
    execution_outcome = Some(execution.outcome);  // receipts/logs used for RPC indexing
}
```

## Impact

At the current devnet throughput of 33 blocks/second with 10 validators:
- Each non-leader validator executes every block twice (verification + finalization)
- The leader executes every block three times (proposal + verification of own block + finalization)
- For blocks containing transactions (especially contract interactions), EVM execution dominates the per-block CPU cost

Eliminating the finalization re-execution would:
- Reduce total EVM compute by ~33% per validator
- Free CPU budget for higher block throughput or more transactions per block
- Reduce the finalization pipeline latency, shrinking the gap between consensus tip and finalized height
- Reduce the likelihood of the snapshot eviction race (issue #011)

The estimated throughput improvement is 15-20% in blocks/second, as EVM execution is on the critical finalization path.

## Root Cause

The `ExecutionOutcome` (containing receipts, logs, gas_used, and selfdestructed addresses) is computed during both proposal and verification but is not stored in the `Snapshot` struct. The `Snapshot<S>` struct at `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs` stores `state: S` (overlay), `changes: ChangeSet`, `state_root: StateRoot`, and `tx_ids: BTreeSet<TxId>`, but has no field for `ExecutionOutcome`. When finalization needs the receipts/logs for RPC indexing, it must re-execute the block to regenerate them.

## Suggested Fix

Cache the `ExecutionOutcome` alongside the snapshot during verification:

1. Add an optional `execution_outcome: Option<ExecutionOutcome>` field to `Snapshot<S>`:

```rust
pub struct Snapshot<S> {
    pub parent: Option<Digest>,
    pub state: S,
    pub state_root: StateRoot,
    pub changes: ChangeSet,
    pub tx_ids: BTreeSet<TxId>,
    pub execution_outcome: Option<ExecutionOutcome>,  // NEW: cached for finalization
}
```

2. During `verify_block`, store the outcome in the snapshot after successful verification (app.rs around line 681):

```rust
let snapshot = Snapshot::new(Some(parent_digest), next_state, state_root,
                             execution.outcome.changes.clone(), tx_ids)
    .with_execution_outcome(Some(execution.outcome));
```

3. During `finalize_block`, retrieve the cached outcome instead of re-executing (reporters/src/lib.rs around line 494):

```rust
if let Some(outcome) = snapshot.execution_outcome.take() {
    // Use cached result -- skip re-execution
    execution_outcome = Some(outcome);
} else if block_index.is_some() {
    // Fallback: re-execute (only for catch-up or restored snapshots)
    // ... existing re-execution code ...
}
```

Memory overhead is modest: `ExecutionOutcome` contains receipts (one per transaction) and the gas_used counter. For a block with 100 transactions, this adds roughly 10-20 KB per snapshot -- negligible compared to the `ChangeSet` and `OverlayState` already stored.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/components/snapshot.rs` -- `Snapshot<S>` struct needs `execution_outcome` field
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (lines 678-690) -- `verify_block` snapshot insertion should cache the outcome
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` (lines 490-530) -- `finalize_block` should check for cached outcome before re-executing
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (lines 346-360) -- `insert_snapshot` and `cache_snapshot` may need to accept optional outcome

## Related Issues

- `012-blockhash-opcode-broken-proposal.md` -- inconsistent BlockContext between proposal/verification and finalization paths
- `019-dual-base-fee-paths-diverge.md` -- another proposal vs. finalization inconsistency
- `020-qmdb-persistence-blocks-finalization.md` -- finalization pipeline performance bottleneck
