# Executor Fatal Abort Bug

## Overview

This document describes a critical (P0) bug in the Kora blockchain's block executor that can cause permanent chain death. A single malformed or stale transaction in the mempool causes the entire block proposal to abort, and because the offending transaction is never removed from the mempool, the failure repeats indefinitely across all validators.

---

## Background: What is Kora?

Kora is an EVM-compatible blockchain that uses **Simplex BFT consensus** (from the Commonware framework) with a fixed validator set of **4 nodes**. The architecture consists of:

- **Simplex consensus engine**: A BFT protocol where validators take turns proposing blocks in sequential "views." Each view has a designated leader. If the leader fails to propose a valid block, the view is "nullified" and consensus advances to the next view with a different leader.
- **REVM executor**: Executes EVM transactions against a state database (QMDB) to produce state transitions.
- **In-memory mempool**: Stores pending transactions submitted via RPC. Transactions are only pruned from the mempool upon finalization of the block that contains them.
- **Snapshot store**: Maintains speculative state snapshots for blocks that have been proposed but not yet finalized.

The consensus round-robin cycles through all 4 validators. If a validator's proposal fails, the view is nullified. Finalization requires 2f+1 votes (3 out of 4 validators).

---

## What the Executor Does

The `RevmExecutor` (in `crates/node/executor/src/revm.rs`) implements the `BlockExecutor` trait:

```rust
// crates/node/executor/src/traits.rs
pub trait BlockExecutor<S: StateDb>: Clone + Send + Sync + 'static {
    type Tx: Clone + Send + Sync + 'static;

    fn execute(
        &self,
        state: &S,
        context: &BlockContext,
        txs: &[Self::Tx],
    ) -> Result<ExecutionOutcome, ExecutionError>;

    fn validate_header(&self, header: &Header) -> Result<(), ExecutionError>;
}
```

The executor's job is to take a batch of raw transaction bytes, decode each one, execute it against the parent state using REVM (a Rust EVM implementation), accumulate the resulting state changes and receipts, and return an `ExecutionOutcome`. This outcome is then used to compute the new state root and construct the block.

The critical return type is `Result<ExecutionOutcome, ExecutionError>`. If this returns `Err(...)`, the entire block proposal is abandoned.

---

## The Bug

### Location

File: `crates/node/executor/src/revm.rs`, lines 388-408, inside the `execute()` method of `impl<S: StateDb> BlockExecutor<S> for RevmExecutor`.

### The Problematic Code

```rust
// crates/node/executor/src/revm.rs:388-411
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;  // LINE 391 - FATAL
    evm.set_tx(tx_env);

    let result_and_state =
        evm.replay().map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;  // LINE 395 - FATAL

    let gas_used = result_and_state.result.tx_gas_used();
    cumulative_gas = cumulative_gas.saturating_add(gas_used);

    let receipt =
        build_receipt(&result_and_state.result, tx_hash, gas_used, cumulative_gas);
    outcome.receipts.push(receipt);

    let state = result_and_state.state;
    let changes = extract_changes(state.clone());
    evm.ctx.modify_db(|db| db.commit(state));
    outcome.changes.merge(changes);
}
```

The two `?` operators on lines 391 and 395 propagate any error upward, causing the `execute()` function to return `Err(ExecutionError)` immediately. This aborts processing of ALL transactions in the block -- not just the one that failed.

### What Triggers the Failure

**Line 391 - `decode_tx_env(...)?`** fails when:
- The transaction bytes contain malformed RLP encoding
- The EIP-2718 envelope structure is invalid
- ECDSA signature recovery fails (corrupted signature)
- The chain ID is invalid
- The `TxEnv` builder validation fails

**Line 395 - `evm.replay().map_err(...)?`** fails when:
- The transaction nonce is too low (already used -- stale transaction)
- The transaction nonce is too high (gap in sequence)
- The sender has insufficient balance to cover gas + value
- A database/state read error occurs
- REVM encounters an internal error

### What Does NOT Trigger This Bug

EVM-level execution failures (contract reverts, assert failures, out-of-gas) produce `ExecutionResult::Revert` or `ExecutionResult::Halt`. These are handled correctly -- the transaction is included in the block with a failure receipt but does NOT abort the block. The `build_receipt()` function on line 401 handles these cases gracefully.

The bug specifically concerns **framework-level errors** (the transaction cannot be processed at all) versus **execution-level results** (the transaction was processed and produced a result, even if that result is a revert).

---

## The Failure Cascade

### Step-by-Step Chain of Events

```
1. Transaction with stale nonce enters mempool via RPC
   (No nonce validation against current state at ingress)
         |
         v
2. Validator becomes leader, calls propose()
         |
         v
3. propose() calls build_block()
         |
         v
4. build_block() calls mempool.build(max_txs, &excluded)
   -> Returns batch of transactions INCLUDING the stale one
         |
         v
5. build_block() calls executor.execute(&state, &context, &txs_bytes)
         |
         v
6. Executor loops through transactions, hits the stale tx
   -> decode_tx_env()? or evm.replay()? returns Err
   -> execute() returns Err(ExecutionError)
         |
         v
7. build_block() matches the Err case:
   "build_block: execution failed" warning logged
   -> returns None
         |
         v
8. propose() returns None to Simplex consensus
         |
         v
9. Simplex nullifies the view (no block produced)
         |
         v
10. BAD TRANSACTION REMAINS IN MEMPOOL
    (pruning only happens on finalization, line 226 of reporters/src/lib.rs)
         |
         v
11. Next view: different leader, same mempool, same bad tx, same failure
         |
         v
12. REPEAT FOREVER -- chain is permanently stalled
```

### The `build_block()` Code Path

From `crates/node/runner/src/app.rs`, lines 124-137:

```rust
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        warn!(
            parent = ?parent_digest,
            height,
            txs = txs.len(),
            error = ?err,
            "build_block: execution failed"
        );
        return None;  // ENTIRE BLOCK PROPOSAL ABANDONED
    }
};
```

When `execute()` returns an error, `build_block()` returns `None`, which propagates up to `propose()`, which returns `None` to the Simplex consensus engine. Simplex interprets this as a failed proposal and nullifies the view.

### Why the Bad Transaction Persists

Mempool pruning ONLY occurs upon finalization. From `crates/node/reporters/src/lib.rs`, line 226:

```rust
state.prune_mempool(&block.txs).await;
```

This is called inside the `FinalizedReporter` -- the component that processes blocks after they have been finalized (certified by 2f+1 validators). If no block ever gets finalized (because every proposal fails), no pruning ever happens, and the bad transaction remains in the mempool forever.

The `excluded` set in `build_block()` only excludes transactions that are in pending (unfinalized) ancestor snapshots -- it does NOT exclude transactions that previously caused execution failures.

---

## How This Manifests in Production

### Primary Symptom: Stale Nonces After Finalization

The most common trigger in a live devnet:

1. A user submits a transaction (nonce=5).
2. The transaction is included in block N and finalized.
3. The mempool prunes the transaction.
4. Meanwhile, due to a race condition or duplicate submission, the same transaction (or one with the same nonce from the same sender) is re-submitted to the mempool.
5. When the executor tries to execute it, the on-chain nonce is already 6, so nonce=5 fails with `NonceTooLow`.
6. Every subsequent block proposal that includes this transaction fails.

### Observable Indicators

- **Views advancing without finalization**: The consensus view counter increases but the finalized height remains static.
- **Nullification rate spike**: The percentage of nullified views jumps to 100% (normally it should be near 0% under stable operation).
- **Repeated `"build_block: execution failed"` warnings in logs**: With the error message showing `TxDecode` or `TxExecution` errors.
- **Mempool growing but blocks always empty or absent**: Transactions accumulate but are never included.
- **All 4 validators exhibit the same failure**: Because they all share the same mempool contents (transactions are gossiped), the same bad transaction causes failures for every validator when it becomes leader.

---

## Impact Assessment

### Severity: P0 (Critical -- Can Kill the Chain Permanently)

| Factor | Assessment |
|--------|-----------|
| **Blast radius** | Entire chain (all 4 validators affected simultaneously) |
| **Recovery** | Manual intervention required (restart nodes with cleared mempool, or deploy code fix) |
| **Trigger difficulty** | Easy -- any stale/duplicate transaction submission can trigger it |
| **Detection time** | May not be immediately obvious; looks like "consensus is slow" before it becomes clear the chain is dead |
| **Data loss** | No state corruption, but all pending transactions after the stall are indefinitely delayed |

### Why This is Worse Than a Single-Node Bug

In a typical blockchain, if one validator fails to propose, others succeed and the chain continues. This bug is catastrophic because:

1. The mempool is shared (transactions gossip to all validators).
2. ALL validators include the same bad transaction in their proposals.
3. ALL proposals fail.
4. The chain makes zero progress.

With 4 validators needing 3/4 for finalization, even if ONE validator somehow avoids the bad transaction, the chain cannot finalize because the other 3 will still fail when verifying the proposed block (the `verify_block()` path in `app.rs` has the same vulnerability -- it calls `BlockExecution::execute()` which uses the same executor).

---

## Proposed Fix

### Primary Fix: Skip Bad Transactions Instead of Aborting

Replace the `?` operators with `match` + `continue`:

```rust
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
        Ok(env) => env,
        Err(err) => {
            warn!(?tx_hash, error = ?err, "skipping undeccodable transaction");
            skipped_txs.push(tx_hash);
            continue;
        }
    };
    evm.set_tx(tx_env);

    let result_and_state = match evm.replay() {
        Ok(result) => result,
        Err(err) => {
            warn!(?tx_hash, error = ?err, "skipping unexecutable transaction");
            skipped_txs.push(tx_hash);
            continue;
        }
    };

    // ... process successful transaction normally (gas, receipt, state commit)
}
```

The `skipped_txs` vector should be returned as part of the `ExecutionOutcome` so that the caller can evict those transactions from the mempool immediately, preventing re-proposal.

### Error Classification

Not all errors should be treated identically. The distinction is between "this transaction is bad" versus "the system is broken":

| Error Variant | Current Behavior | Correct Behavior |
|---------------|-----------------|------------------|
| `TxDecode` (malformed RLP, bad signature) | Abort entire block | **Skip transaction, evict from mempool** |
| `TxExecution` (NonceTooLow) | Abort entire block | **Skip transaction, evict from mempool** |
| `TxExecution` (NonceTooHigh) | Abort entire block | **Skip transaction** (may become valid later) |
| `TxExecution` (InsufficientBalance) | Abort entire block | **Skip transaction** (may become valid later) |
| `State` (database I/O error) | Abort entire block | **Abort entire block** (system failure -- correct) |
| `Revert` (contract-level revert) | Included in block with failure receipt | Included in block (already correct) |
| `Halt` (out of gas, stack overflow) | Included in block with failure receipt | Included in block (already correct) |

Only true infrastructure/system failures (e.g., the state database is unreachable) should abort the entire block. All transaction-level failures should skip the individual transaction and continue processing the rest of the batch.

### Secondary Fix: Mempool Eviction on Skip

After `execute()` returns, any skipped transactions should be removed from the mempool:

```rust
// In build_block(), after successful execution:
for skipped_hash in &outcome.skipped_txs {
    mempool.remove(skipped_hash);
}
```

This prevents the same bad transaction from being proposed again in the next view.

### Tertiary Fix: Nonce Validation at Mempool Ingress

Add a pre-check when transactions are submitted via RPC:

```rust
// Before inserting into mempool:
let current_nonce = state.nonce(&sender).await?;
if tx.nonce < current_nonce {
    return Err("nonce too low -- transaction already executed");
}
```

This reduces the probability of stale transactions entering the mempool in the first place, but does NOT eliminate the bug (nonces can become stale between ingress and execution due to finalization of other blocks).

---

## How to Detect This Bug is Occurring

### Metrics to Monitor

1. **Nullification rate**: `nullified_views / total_views`. Normal: <5%. Bug active: 100%.
2. **Views advancing without finalization**: If `consensus_view - finalized_height` grows continuously, proposals are failing.
3. **Mempool size vs. blocks produced**: If mempool has transactions but blocks are empty/absent, something is preventing inclusion.

### Log Signatures

Search for these log patterns:

```
WARN build_block: execution failed
```

With error contents matching:
```
TxDecode("...")
TxExecution("NonceTooLow...")
TxExecution("InsufficientBalance...")
```

### Simplex Consensus Indicators

The Simplex configuration (from `crates/node/simplex/src/config.rs`) defines:
- `leader_timeout`: 2 seconds (time for a leader to propose)
- `certification_timeout`: 4 seconds (time to gather votes)
- `timeout_retry` (nullify retry): 1 second
- `activity_timeout`: 256 views (validator excluded after this many missed views)
- `skip_timeout`: 32 views

If all validators are nullifying, the chain will cycle through views at approximately the leader_timeout rate (2 seconds per view) without ever finalizing. With 4 validators, the chain will cycle through all leaders every 4 views (8 seconds) -- and if all fail, the pattern repeats indefinitely.

---

## Related Issues

1. **Mempool Pruning Bug** (`tmp/mempool-pruning-bug.md`): Pruning only happens on finalization; if the chain stalls, no pruning occurs, compounding this bug.
2. **Nonce Validation Gap** (`tmp/nonce-validation-gap.md`): Lack of nonce validation at RPC ingress makes it easy for stale transactions to enter the mempool.
3. **Duplicate Transaction Storm** (`tmp/duplicate-tx-storm.md`): Load generators with nonce race conditions can flood the mempool with stale-nonce transactions.

---

## File References

| File | Purpose |
|------|---------|
| `crates/node/executor/src/revm.rs` (lines 388-411) | The buggy execution loop with fatal `?` operators |
| `crates/node/executor/src/traits.rs` | `BlockExecutor` trait definition |
| `crates/node/executor/src/error.rs` | `ExecutionError` enum (all variants) |
| `crates/node/executor/src/outcome.rs` | `ExecutionOutcome` struct (needs `skipped_txs` field) |
| `crates/node/runner/src/app.rs` (lines 124-137) | `build_block()` that returns `None` on executor error |
| `crates/node/consensus/src/proposal.rs` (lines 100-103) | `ProposalBuilder` that propagates executor errors |
| `crates/node/consensus/src/traits.rs` | `Mempool` trait with `prune()` method |
| `crates/node/reporters/src/lib.rs` (line 226) | Where mempool pruning actually occurs (only on finalization) |
| `crates/node/simplex/src/config.rs` | Consensus timeout configuration |
