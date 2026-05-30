# Receipt-to-Transaction Index Alignment Breaks When Gas Limit Causes Early Break

**Category**: Executor / Correctness
**Severity**: High

## Summary

The block execution loop uses two inconsistent strategies for non-executable transactions: decode and execution failures emit placeholder receipts (preserving the 1:1 receipt-to-transaction alignment), but the gas limit check performs an early `break` without emitting any receipts for the remaining transactions. This means `receipts.len()` can be less than `txs.len()`, breaking the alignment invariant that existing tests and downstream code (block indexer, RPC handlers) depend on. Any code that pairs `receipts[i]` with `txs[i]` by index will silently return wrong data when the gas limit triggers a break.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs`, the block execution loop (starting at line 410) handles non-executable transactions with two different strategies:

**Strategy 1: Placeholder receipts for decode/execution failures (preserves alignment)**

When a transaction fails to decode (lines 413-420) or fails to execute (lines 432-438), a placeholder receipt is pushed and the loop continues with the next transaction:

```rust
// crates/node/executor/src/revm.rs:413-420
let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
    Ok(env) => env,
    Err(e) => {
        warn!(hash = ?tx_hash, error = %e, "skipping undecodable transaction");
        outcome.receipts.push(build_skipped_receipt(tx_hash, cumulative_gas));
        continue;  // placeholder receipt emitted, alignment preserved
    }
};
```

```rust
// crates/node/executor/src/revm.rs:432-438
let result_and_state = match evm.replay() {
    Ok(result) => result,
    Err(e) => {
        debug!(hash = ?tx_hash, error = ?e, "skipping unexecutable transaction");
        outcome.receipts.push(build_skipped_receipt(tx_hash, cumulative_gas));
        continue;  // placeholder receipt emitted, alignment preserved
    }
};
```

**Strategy 2: Early break for gas limit (breaks alignment)**

When the next transaction's gas limit would exceed the block gas limit, the loop breaks immediately without emitting any receipts for the current or remaining transactions:

```rust
// crates/node/executor/src/revm.rs:422-429
// Enforce block gas limit: we `break` (not `continue`) because Ethereum
// semantics stop inclusion at the gas limit -- remaining txs are simply not
// included. Unlike decode failures above, gas-limited txs get no placeholder
// receipts, so `receipts.len()` may be less than `txs.len()`.
let tx_gas_limit = tx_env.gas_limit;
if cumulative_gas.saturating_add(tx_gas_limit) > context.header.gas_limit {
    break;  // NO receipt emitted, alignment broken
}
```

The code comment at lines 422-425 explicitly acknowledges this inconsistency but does not resolve it.

**Tests assert strict alignment (contradicting the break behavior)**

Three test cases assert `receipts.len() == txs.len()`, reinforcing the alignment invariant:

```rust
// crates/node/executor/src/revm.rs:1133
assert_eq!(outcome.receipts.len(), txs.len(), "receipt count must match tx count");

// crates/node/executor/src/revm.rs:1154
assert_eq!(outcome.receipts.len(), txs.len(), "receipt count must match tx count");

// crates/node/executor/src/revm.rs:1177
assert_eq!(outcome.receipts.len(), txs.len(), "receipt count must match tx count");
```

However, none of these tests cover the gas limit scenario (where `cumulative_gas + tx_gas_limit > block_gas_limit`), so the test suite does not expose the contradiction.

## Impact

1. **Silent data corruption in the block indexer**: The `index_finalized_block()` function in `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:1047-1107` pairs receipts with transactions by iterating over `outcome.receipts` and zipping with `tx_metadata`. When `receipts.len() < txs.len()`, the zip stops at the shorter length, so the remaining transactions are silently dropped from the index -- they become invisible to the RPC layer.

    ```rust
    // crates/node/reporters/src/lib.rs:1072-1090
    let receipt_envelopes: Vec<ReceiptEnvelope> = outcome
        .receipts
        .iter()
        .zip(tx_metadata.iter())   // <-- truncated if receipts shorter
        .filter_map(|(receipt, metadata)| {
            // ...
        })
        .collect();
    ```

2. **Incorrect RPC responses**: Any RPC handler (e.g., `eth_getTransactionReceipt`) that looks up receipts by block index would return the wrong receipt for transactions after the gas-limit break point, because the receipt array is shorter than the transaction array and there is no offset tracking.

3. **Incorrect receipts root**: The receipts trie root (computed in `index_finalized_block`) would be calculated from a truncated receipt list, producing a root that does not match the actual execution outcome. This could cause issues for light clients or cross-chain verification systems that rely on the receipts root.

4. **Test coverage gap**: The existing tests enforce strict alignment but do not test the gas limit path, creating a false sense of correctness.

## Root Cause

Two different error-handling strategies were chosen for conceptually different failure modes:
- Decode/execution failures: the transaction was "attempted" but failed, so it gets a placeholder receipt to maintain the alignment.
- Gas limit exceedance: the transaction was "not included" per Ethereum semantics, so no receipt is emitted.

The inconsistency is that downstream code assumes all transactions in the `txs` array have a corresponding receipt, but the gas limit break violates this assumption.

## Suggested Fix

**Option A (recommended)** -- Track the number of included transactions explicitly in the `ExecutionOutcome`:

```rust
// BEFORE:
pub struct ExecutionOutcome {
    pub receipts: Vec<ExecutionReceipt>,
    // ... other fields
}

// AFTER:
pub struct ExecutionOutcome {
    pub receipts: Vec<ExecutionReceipt>,
    pub included_tx_count: usize,  // number of txs that were actually processed
    // ... other fields
}
```

Set `included_tx_count` after the execution loop completes:

```rust
// At the end of the for loop in execute():
outcome.included_tx_count = outcome.receipts.len();
// Or equivalently, track a counter during iteration
```

Update all downstream consumers to use `included_tx_count` (or `receipts.len()`) rather than `txs.len()` when pairing receipts with transactions. In `index_finalized_block`, only index the first `included_tx_count` transactions.

**Option B** -- Emit placeholder receipts for gas-limited transactions too:

```rust
// BEFORE (crates/node/executor/src/revm.rs:422-429):
if cumulative_gas.saturating_add(tx_gas_limit) > context.header.gas_limit {
    break;
}

// AFTER:
if cumulative_gas.saturating_add(tx_gas_limit) > context.header.gas_limit {
    // Emit placeholder receipts for all remaining transactions
    outcome.receipts.push(build_skipped_receipt(tx_hash, cumulative_gas));
    for remaining_tx in &txs[/* remaining index */..] {
        let remaining_hash = keccak256(remaining_tx);
        outcome.receipts.push(build_skipped_receipt(remaining_hash, cumulative_gas));
    }
    break;
}
```

This makes `receipts.len() == txs.len()` a true invariant. However, it introduces non-standard receipt semantics (receipts for transactions that were never attempted).

**Option C** -- Return only the included transactions from `execute()`:

```rust
pub fn execute(
    &self,
    state: &S,
    context: &BlockContext,
    txs: &[Self::Tx],
) -> Result<(ExecutionOutcome, Vec<Self::Tx>), ExecutionError> {  // also return included txs
```

This is the cleanest API but requires updating all callers.

**Recommendation**: Option A is the most pragmatic. It requires minimal API changes and makes the invariant explicit without introducing non-standard receipt types. Add a test for the gas limit break path to prevent regression.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` (lines 422-429) -- gas limit break with no placeholder receipt
- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` (lines 413-420, 432-438) -- decode/execution failure placeholder receipts (for reference)
- `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` (lines 1133, 1154, 1177) -- test assertions for `receipts.len() == txs.len()`
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` (lines 1047-1107) -- `index_finalized_block()` which zips receipts with transactions
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (line 355 area) -- block building code that consumes `ExecutionOutcome`

## Related Issues

- `081-executor-block-hash-truncation-ordering.md` -- another executor correctness issue with consensus-divergence risk

## Labels

bug, correctness, executor
