# EVM: No aggregate block gas limit enforcement during execution

**Severity:** Medium

## Summary

The block executor tracks cumulative gas usage across transactions but never checks whether it exceeds the block gas limit. The `execute()` method iterates through every transaction in the block, accumulates gas via `saturating_add`, and records the total -- but at no point does it compare `cumulative_gas` against `context.header.gas_limit` or stop including transactions when the limit is reached. A block could theoretically contain more gas than the block gas limit allows, violating a fundamental EVM invariant.

## Background

### How blocks are built in Kora

Kora is an EVM-compatible blockchain using REVM (Cancun spec) for execution. Block production follows this path:

1. The leader node calls `build_block()` in `crates/node/runner/src/app.rs` (line 84)
2. Transactions are pulled from the mempool via `mempool.build(self.max_txs, &excluded)` (line 96)
3. The `RevmExecutor::execute()` method processes each transaction sequentially (in `crates/node/executor/src/revm.rs`, line 357)
4. Other validators verify blocks via `verify_block()` (line 166), which calls `BlockExecution::execute()` with the same executor

The `BlockContext` (defined in `crates/node/executor/src/context.rs`) carries the block header, which includes the gas limit. The default gas limit is 250,000,000 (configured in `crates/node/config/src/execution.rs` as `DEFAULT_GAS_LIMIT`). The maximum number of transactions per block is capped at `BLOCK_CODEC_MAX_TXS = 10,000` (in `crates/node/runner/src/runner.rs`, line 44).

### The executor's role

The `RevmExecutor` implements the `BlockExecutor` trait (defined in `crates/node/executor/src/traits.rs`, lines 11-27):

```rust
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

The `execute()` method receives a `BlockContext` (defined in `crates/node/executor/src/context.rs`) which carries the block header including `gas_limit`. It is responsible for processing a batch of transactions against a state database, producing an `ExecutionOutcome` containing receipts, state changes, and total gas used. In Ethereum, the executor is also responsible for enforcing the block gas limit -- stopping transaction inclusion when the cumulative gas would exceed the limit.

## The Code

**File:** `crates/node/executor/src/revm.rs`, lines 385-411

```rust
let mut outcome = ExecutionOutcome::new();
let mut cumulative_gas = 0u64;

for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;
    evm.set_tx(tx_env);

    let result_and_state =
        evm.replay().map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;

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

outcome.gas_used = cumulative_gas;
Ok(outcome)
```

The gas accounting issue:
- `cumulative_gas` is initialized to 0 on line 386
- Each transaction's gas is added via `saturating_add` on line 398
- The cumulative total is stored in each receipt (line 401) and in the outcome (line 410)
- **But there is no comparison against `context.header.gas_limit` anywhere in the loop**
- **There is no `break` statement to stop processing when the limit is exceeded**

The block gas limit is available through `context.header.gas_limit` (the `context` parameter is passed into the function on line 360), but it is only used to configure the REVM block environment on line 378:

```rust
.modify_block_chained(|blk: &mut BlockEnv| {
    // ...
    blk.gas_limit = context.header.gas_limit;
    // ...
});
```

This sets the per-block gas limit inside REVM's block environment, but REVM uses this for individual transaction validation (e.g., rejecting a single transaction whose gas limit exceeds the block gas limit), **not** for aggregate enforcement across all transactions in a block.

### What is missing

After line 398, before processing receipts, the executor should check:

```rust
if cumulative_gas > context.header.gas_limit {
    break; // Stop including transactions
}
```

## Current Mitigations

Several factors reduce the practical impact today:

1. **Gas limit headroom:** The default block gas limit is 250,000,000. A basic ETH transfer costs 21,000 gas. You would need approximately 11,905 basic transfers in a single block to exceed the limit.

2. **Transaction count cap:** `mempool.build(max_txs, &excluded)` limits the number of transactions per block. `BLOCK_CODEC_MAX_TXS` is set to 10,000 (in `crates/node/runner/src/runner.rs`, line 44). With 10,000 basic transfers at 21,000 gas each, the total would be 210,000,000 -- below the 250M limit.

3. **Mempool size limits:** The `PoolConfig` (defined in `crates/node/txpool/src/config.rs`) enforces `max_txs_per_sender: 256` (line 25), `max_pending_txs: 4096` (line 23), and `max_queued_txs: 1024` (line 24), which constrain how many transactions can accumulate.

These mitigations mean that under normal operation with basic transfers, the gas limit is unlikely to be exceeded.

## Why It Still Matters

1. **Contract calls consume variable gas.** A single contract call can consume millions of gas. With complex contract interactions (e.g., a DeFi aggregator routing through multiple pools), a block of 100 transactions could easily exceed 250M gas if each consumes 3-5M gas. The mempool `build()` function selects transactions without regard to their gas consumption relative to the block gas limit.

2. **Consensus validity.** In Ethereum, a block whose cumulative gas exceeds the block gas limit is invalid per consensus rules. Validators should reject such blocks. Kora's `verify_block()` method (in `crates/node/runner/src/app.rs`, line 166) re-executes the block and checks the state root, but does not verify that `gas_used <= gas_limit`. A malicious or buggy leader could propose a block exceeding the gas limit, and validators would accept it as long as the state root matches.

3. **Tooling compatibility.** Ethereum tooling (block explorers, indexers, analytics) assumes `block.gasUsed <= block.gasLimit`. Blocks violating this invariant could cause downstream tools to report errors or behave unexpectedly.

4. **Future risk.** As the chain handles more complex contract workloads, the gap between the transaction count limit and the gas limit becomes easier to cross. This is a latent bug that will surface under heavier contract usage.

## Proposed Fix

### Files to modify

| File | Change |
|------|--------|
| `crates/node/executor/src/revm.rs` | Add cumulative gas check in `execute()` loop (after line 398) |
| `crates/node/runner/src/app.rs` | Add gas limit validation in `verify_block()` (after line 193) |

### 1. Add cumulative gas check in `execute()`

In `crates/node/executor/src/revm.rs`, inside the transaction processing loop (lines 388-408), add a gas limit check. There are two approaches:

**Approach A: Pre-execution check (recommended, matches Ethereum behavior)**

Before executing each transaction, check if its gas limit would fit in the remaining block capacity. This avoids wasting execution work on transactions that would exceed the limit. Insert before line 391:

```rust
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);
    let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;

    // Skip transaction if its gas limit would exceed remaining block capacity
    let remaining_gas = context.header.gas_limit.saturating_sub(cumulative_gas);
    if tx_env.gas_limit > remaining_gas {
        break;
    }

    evm.set_tx(tx_env);
    // ... rest of loop unchanged ...
}
```

This is the approach used by Ethereum block builders (e.g., geth's `miner` package): a transaction is only included if its gas limit fits within the block's remaining gas budget.

**Approach B: Post-execution check**

After executing each transaction, check if cumulative gas has exceeded the limit. This is simpler but wastes execution work on the transaction that crosses the limit. Insert after line 398:

```rust
    let gas_used = result_and_state.result.tx_gas_used();
    cumulative_gas = cumulative_gas.saturating_add(gas_used);

    // Stop including transactions if block gas limit is exceeded
    if cumulative_gas > context.header.gas_limit {
        break;
    }

    let receipt =
        build_receipt(&result_and_state.result, tx_hash, gas_used, cumulative_gas);
    // ... rest of loop ...
```

**Note:** The `context` variable is already in scope -- it is the `context: &BlockContext` parameter passed to the `execute()` method on line 360. The gas limit is accessible via `context.header.gas_limit` (the same field used to set REVM's block environment on line 378).

### 2. Add gas limit validation to `verify_block()`

In `crates/node/runner/src/app.rs`, inside `verify_block()` (line 166), after the block execution completes (line 193) and before the state root comparison (line 210), add a gas limit check:

```rust
// After line 193 (successful execution), before line 210 (state root check):
if execution.outcome.gas_used > context.header.gas_limit {
    warn!(
        ?digest,
        gas_used = execution.outcome.gas_used,
        gas_limit = context.header.gas_limit,
        "block exceeds gas limit"
    );
    return false;
}
```

The `context` variable is constructed at line 182 (`let context = self.block_context(block.height, block.prevrandao);`), and `context.header.gas_limit` is set to `self.gas_limit` via the `block_context()` method at line 72.

This ensures validators reject blocks from malicious or buggy leaders that exceed the gas limit, regardless of whether the state root matches.

### 3. Improve block building (optional, defense-in-depth)

In `build_block()` (line 84 of the same file), the executor already processes all transactions from `mempool.build()` (line 96). Once the executor correctly enforces the gas limit via fix #1, blocks produced by the leader will naturally stop at the limit. However, for efficiency, it would be beneficial to have the mempool's `build()` method estimate gas consumption and avoid selecting more transactions than the block can fit. This would reduce wasted execution of transactions that will be dropped at the gas limit boundary.

The mempool's `build` method is called at line 96:
```rust
let txs = mempool.build(self.max_txs, &excluded);
```

A gas-aware variant could accept an additional `gas_budget: u64` parameter and stop collecting transactions when the sum of their `gas_limit` fields approaches the budget.

## Verification Checklist

1. Create a set of transactions where each consumes a known amount of gas (e.g., deploy a contract with a gas-heavy constructor consuming 5M gas each)
2. Submit enough transactions to exceed the 250M gas limit if all were included (e.g., 60 transactions at 5M gas each = 300M gas)
3. Verify the executor stops including transactions at the gas limit boundary
4. Verify the total `gas_used` in the `ExecutionOutcome` does not exceed `gas_limit`
5. Craft a block payload that exceeds the gas limit and submit it via `verify_block()` -- verify it returns `false`
6. Confirm that existing basic transfer workloads (e.g., loadgen with 10,000 simple transfers) are unaffected by the new check
7. Run `cargo test` across the workspace to confirm no regressions
8. Verify that the `ExecutionOutcome::gas_used` field (set at line 410 of revm.rs) correctly reflects only the included transactions (not the dropped ones)
