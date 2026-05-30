# EVM: No post-execution hooks for block rewards, system transactions, or epoch transitions

**Severity:** Medium
**Component:** `kora-executor`
**Affects:** Protocol economics, fee model, future protocol upgrades

---

## Summary

The `BlockExecutor` trait and its `RevmExecutor` implementation have no mechanism to execute protocol-level state modifications after user transaction processing. The `execute()` method processes user transactions in a loop and immediately returns the resulting `ExecutionOutcome`. There is no `post_execute()`, `apply_system_transactions()`, or `finalize_block()` hook. This prevents implementing block rewards, EIP-1559 fee burning, system transactions (e.g., validator set updates, bridge messages), and epoch transitions. Currently the chain operates with zero block rewards and zero base fee, which is acceptable for testnet/devnet but will need to change before mainnet.

---

## Background

Kora is an EVM-compatible blockchain built on the Commonware consensus framework (Simplex BFT). It uses REVM for EVM transaction execution.

### How the Executor Fits into the Block Pipeline

The block production pipeline is:

1. A leader is elected by consensus for a given view.
2. The consensus layer calls `RevmApplication::build_block()` (`crates/node/runner/src/app.rs`, line 84).
3. `build_block()` drains transactions from the mempool, constructs a `BlockContext`, and calls `self.executor.execute(&parent_snapshot.state, &context, &txs_bytes)` (line 125).
4. The executor returns an `ExecutionOutcome` containing state changes, receipts, and total gas used.
5. The application computes a state root from the changes and packages the result into a `Block`.
6. Validators verify the block by re-executing (same `execute()` path) and comparing state roots.
7. After finalization, the `FinalizedReporter` persists the snapshot and prunes the mempool.

### The `BlockExecutor` Trait

Defined in `crates/node/executor/src/traits.rs` (lines 11-27):

```rust
// crates/node/executor/src/traits.rs, lines 11-27
pub trait BlockExecutor<S: StateDb>: Clone + Send + Sync + 'static {
    /// Transaction type accepted for execution.
    type Tx: Clone + Send + Sync + 'static;

    /// Execute a batch of transactions against the given state.
    ///
    /// Returns the execution outcome containing state changes and receipts.
    fn execute(
        &self,
        state: &S,
        context: &BlockContext,
        txs: &[Self::Tx],
    ) -> Result<ExecutionOutcome, ExecutionError>;

    /// Validate a block header.
    fn validate_header(&self, header: &Header) -> Result<(), ExecutionError>;
}
```

The trait has exactly two methods: `execute()` and `validate_header()`. There is no lifecycle hook for pre-block system calls, post-transaction protocol logic, or block finalization.

### The `ExecutionOutcome` Type

Defined in `crates/node/executor/src/outcome.rs` (lines 8-16):

```rust
// crates/node/executor/src/outcome.rs, lines 8-16
pub struct ExecutionOutcome {
    /// State changes from execution.
    pub changes: ChangeSet,
    /// Transaction receipts.
    pub receipts: Vec<ExecutionReceipt>,
    /// Total gas used by all transactions.
    pub gas_used: u64,
}
```

The outcome has no field for system-level state changes, block rewards, or burned fees. All state changes are lumped into a single `ChangeSet`.

---

## What's Missing

### The Transaction Loop Has No Post-Processing

In `crates/node/executor/src/revm.rs` (lines 357-411), the `execute()` implementation processes all user transactions and then immediately returns:

```rust
// crates/node/executor/src/revm.rs, lines 385-411
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

    let receipt = build_receipt(&result_and_state.result, tx_hash, gas_used, cumulative_gas);
    outcome.receipts.push(receipt);

    let state = result_and_state.state;
    let changes = extract_changes(state.clone());
    evm.ctx.modify_db(|db| db.commit(state));
    outcome.changes.merge(changes);
}

outcome.gas_used = cumulative_gas;
Ok(outcome)
// ^^^ Returns immediately. No post-execution hook.
```

Between line 407 (`outcome.changes.merge(changes)`) and line 411 (`Ok(outcome)`), there is no opportunity to:
- Credit the block proposer with rewards
- Burn base fees (EIP-1559)
- Execute system transactions
- Apply epoch-boundary state transitions

---

## Current State Confirming No Rewards or Fee Burning

There are **two independent locations** that construct `BlockContext` with `Address::ZERO` and zero base fee. Both must be updated when adding rewards or fee mechanisms.

### Location 1: `RevmApplication::block_context()` (block proposal and verification)

`crates/node/runner/src/app.rs` (lines 68-78):

```rust
// crates/node/runner/src/app.rs, lines 68-78
fn block_context(&self, height: u64, prevrandao: B256) -> BlockContext {
    let header = Header {
        number: height,
        timestamp: height,      // Note: timestamp == height, not wall-clock time
        gas_limit: self.gas_limit,
        beneficiary: Address::ZERO,   // <-- No block producer reward recipient
        base_fee_per_gas: Some(0),    // <-- Zero base fee, no EIP-1559 pricing
        ..Default::default()
    };
    BlockContext::new(header, B256::ZERO, prevrandao)
    //                       ^^^^^^^^^^^ parent_hash always zero
}
```

### Location 2: `RevmContextProvider::context()` (finalized block re-execution)

`crates/node/runner/src/runner.rs` (lines 116-128):

```rust
// crates/node/runner/src/runner.rs, lines 116-128
impl BlockContextProvider for RevmContextProvider {
    fn context(&self, block: &Block) -> BlockContext {
        let header = Header {
            number: block.height,
            timestamp: block.height,
            gas_limit: self.gas_limit,
            beneficiary: Address::ZERO,   // <-- Same zero beneficiary
            base_fee_per_gas: Some(0),    // <-- Same zero base fee
            ..Default::default()
        };
        BlockContext::new(header, B256::ZERO, block.prevrandao)
        //                       ^^^^^^^^^^^ parent_hash always zero
    }
}
```

This second location is used by `FinalizedReporter` when re-executing finalized blocks for catch-up or RPC indexing (see `crates/node/reporters/src/lib.rs` line 132). If hooks are added, this provider must produce the same context as `app.rs` to ensure deterministic re-execution.

### Key observations:
- **`beneficiary: Address::ZERO`** -- The coinbase address is set to the zero address in BOTH locations, meaning even if there were a reward mechanism, it would credit the zero address (effectively burning).
- **`base_fee_per_gas: Some(0)`** -- Base fee is hardcoded to zero. The executor has full EIP-1559 base fee calculation logic (`calculate_base_fee()` in `revm.rs` lines 326-352), but it is never used because the application always passes zero.
- **`parent_hash: B256::ZERO`** -- The parent hash in the `BlockContext` is always zero. This means `BLOCKHASH` opcode lookups via the block env will not work (separate issue, see issue #9).
- **No coinbase payment in executor** -- REVM itself handles the coinbase transfer as part of transaction execution (gas fees go to `beneficiary`), but with a zero base fee and `Address::ZERO` as beneficiary, this is economically meaningless.

---

## What This Prevents

### 1. Block Rewards
There is no way to credit the block proposer with newly minted tokens or a share of transaction fees. Any block reward mechanism would need to modify state after all transactions are processed (to know the total fees collected), but the executor returns immediately after the transaction loop.

### 2. EIP-1559 Fee Burning
EIP-1559 requires that `base_fee * gas_used` be permanently removed from circulation (burned) after each block. Currently base fee is zero, and even if it were nonzero, there is no mechanism to track or enforce the burn. The `ExecutionOutcome` reports `gas_used` but the caller has no hook to apply economic consequences.

### 3. System Transactions
Many L2 chains and protocol upgrades require "system transactions" -- state modifications injected by the protocol rather than by user signatures. Examples:
- Depositing bridged assets from L1
- Updating the validator set at epoch boundaries
- Writing L1 block hashes for cross-chain verification
These require the ability to execute additional state changes that are not part of the user transaction set.

### 4. Epoch Transitions
Kora uses a fixed epoch length (`EPOCH_LENGTH: u64 = u64::MAX` in `crates/node/runner/src/runner.rs`, line 55), effectively running as a single infinite epoch. When dynamic validator sets are needed, epoch boundaries will require state transitions (e.g., updating the validator registry, distributing staking rewards).

### 5. EIP-4788 Beacon Root Contract
Post-Cancun Ethereum calls a system contract at the start of each block to store the parent beacon block root. The executor spec defaults to Cancun (`spec_id: SpecId::CANCUN` in `config.rs` line 66) but makes no such system call. The `build_mainnet()` call on line 383 of `revm.rs` uses REVM's standard mainnet handler set with no modifications.

---

## Additional Observations

### PREVRANDAO / DIFFICULTY May Be Zero
The `prevrandao` value in `BlockContext` is set from `ledger.seed_for_parent()` in `crates/node/runner/src/app.rs` (line 81):

```rust
// crates/node/runner/src/app.rs, lines 80-82
async fn get_prevrandao(&self, parent_digest: ConsensusDigest) -> B256 {
    self.ledger.seed_for_parent(parent_digest).await.unwrap_or(B256::ZERO)
}
```

If the seed has not yet been stored by the `SeedReporter` (e.g., for the first block after genesis, or if the reporter hasn't processed the notarization yet), `seed_for_parent` returns `None` and `prevrandao` falls back to `B256::ZERO`. This means `block.difficulty` (mapped to `prevrandao` post-merge) is zero, which could affect smart contracts that use `PREVRANDAO` for randomness.

### Blob Gas Tracking Incomplete
EIP-4844 blob transactions are decoded in the executor (`revm.rs` lines 499-517), with `max_fee_per_blob_gas` and `blob_versioned_hashes` set on the transaction environment. However:
- The `BlockContext` has a `blob_base_fee` field (`context.rs` line 18) that is never set by the application (always `None`)
- There is no blob gas accounting in `ExecutionOutcome`
- The REVM `BlockEnv` blob gas fields (`blob_excess_gas_and_price`, `blob_gasprice`) are not populated in the `execute()` method's block env setup (`revm.rs` lines 374-381). The `modify_block_chained` closure only sets `number`, `timestamp`, `beneficiary`, `gas_limit`, `basefee`, and `prevrandao`.
- A pre-execution hook could set up blob gas pricing in the `BlockEnv` before transactions are processed

### No Custom Precompiles
The executor uses `ctx.build_mainnet()` (`revm.rs` lines 234 and 383) which provides only Ethereum mainnet precompiles. There is no mechanism to register custom precompiles for chain-specific functionality (e.g., L2 bridge contracts, BLS verification).

---

## Proposed Design

### Option A: Add Post-Execution Hook to `BlockExecutor` Trait

Extend the trait with an optional `finalize_block` method:

```rust
pub trait BlockExecutor<S: StateDb>: Clone + Send + Sync + 'static {
    type Tx: Clone + Send + Sync + 'static;

    fn execute(
        &self,
        state: &S,
        context: &BlockContext,
        txs: &[Self::Tx],
    ) -> Result<ExecutionOutcome, ExecutionError>;

    /// Apply protocol-level state modifications after user transactions.
    ///
    /// Called after execute() with the same state and context, plus the
    /// execution outcome from user transactions. Returns additional state
    /// changes to merge into the outcome.
    fn finalize_block(
        &self,
        _state: &S,
        _context: &BlockContext,
        _outcome: &ExecutionOutcome,
    ) -> Result<ChangeSet, ExecutionError> {
        Ok(ChangeSet::new())  // Default no-op for backward compatibility
    }

    fn validate_header(&self, header: &Header) -> Result<(), ExecutionError>;
}
```

The caller (`RevmApplication::build_block()` and the verification path) would call `finalize_block()` after `execute()` and merge the resulting changes into the outcome before computing the state root.

### Option B: Separate `SystemTransactionExecutor`

Create a composable executor that chains user execution with system execution:

```rust
pub trait SystemExecutor<S: StateDb>: Clone + Send + Sync + 'static {
    /// Generate and execute system transactions for the block.
    fn system_transactions(
        &self,
        state: &S,
        context: &BlockContext,
        user_outcome: &ExecutionOutcome,
    ) -> Result<ExecutionOutcome, ExecutionError>;
}
```

This keeps the `BlockExecutor` trait focused on user transactions and allows system logic to be configured independently.

### Option C: Pre- and Post-Execution Hooks

For maximum flexibility (covers both EIP-4788 beacon root storage at block start and rewards at block end):

```rust
pub trait BlockExecutor<S: StateDb>: Clone + Send + Sync + 'static {
    type Tx: Clone + Send + Sync + 'static;

    /// Called before user transactions. Used for system calls like EIP-4788.
    fn pre_execute(
        &self,
        _state: &S,
        _context: &BlockContext,
    ) -> Result<ChangeSet, ExecutionError> {
        Ok(ChangeSet::new())
    }

    fn execute(
        &self,
        state: &S,
        context: &BlockContext,
        txs: &[Self::Tx],
    ) -> Result<ExecutionOutcome, ExecutionError>;

    /// Called after user transactions. Used for block rewards, fee burning.
    fn post_execute(
        &self,
        _state: &S,
        _context: &BlockContext,
        _outcome: &ExecutionOutcome,
    ) -> Result<ChangeSet, ExecutionError> {
        Ok(ChangeSet::new())
    }

    fn validate_header(&self, header: &Header) -> Result<(), ExecutionError>;
}
```

---

## Implementation Notes

1. The `ChangeSet` type (`kora_qmdb::ChangeSet`) already supports `merge()`, so combining system changes with user changes is straightforward.

2. **Three call sites** must call the same hooks in the same order to ensure deterministic state roots:
   - `RevmApplication::build_block()` in `crates/node/runner/src/app.rs` (line 125) -- block proposal
   - `RevmApplication::verify_block()` in `crates/node/runner/src/app.rs` (line 184) -- block verification
   - `handle_finalized_update()` in `crates/node/reporters/src/lib.rs` (lines 133-147) -- finalized block re-execution for catch-up

3. The `BlockExecution::execute()` method in `kora-consensus` (`crates/node/consensus/src/execution.rs`) wraps the `BlockExecutor` call and would need to be updated to call the hooks as well.

4. The `beneficiary` address in `BlockContext` needs to be set to the actual proposer's address (currently `Address::ZERO`). The proposer identity is available in the consensus context but not passed to the application's `block_context()` method. This requires changes in **both**:
   - `RevmApplication::block_context()` in `app.rs` (line 68)
   - `RevmContextProvider::context()` in `runner.rs` (line 117)

5. The `RevmContextProvider` in `runner.rs` implements `BlockContextProvider` (the trait defined in `crates/node/reporters/src/lib.rs` lines 34-37). When the `block_context()` method in `app.rs` changes, the `RevmContextProvider` must be updated to match. Consider extracting the context construction into a shared function to prevent divergence.

6. The `timestamp` field is currently set to `height` (block number), not wall-clock time. This is a separate concern but will interact with any epoch-based logic that uses timestamps.

---

## Related Issues

- **Issue #1 (executor-fatal-abort):** The `execute()` method uses `?` on decode/replay errors, aborting the entire block. Hooks must handle the case where `execute()` returns `Err` -- should hooks still run? Probably not (block is invalid).
- **Issue #9 (BLOCKHASH returns zero):** The `parent_hash` in `BlockContext` is always `B256::ZERO`, and the `StateDbAdapter` returns zero for all block hash lookups. A pre-execution hook could populate block hash history.
- **Issue #10 (no block gas limit enforcement):** The executor does not check `cumulative_gas > gas_limit`. A post-execution hook could enforce this, or it could be added to the transaction loop.
- **Issue #19 (test coverage gaps):** The executor has zero tests for real signed transaction execution. Adding hooks will require additional test infrastructure.

---

## Testing Plan

```
1. Unit tests for finalize_block / post_execute:
   - Default implementation returns empty ChangeSet
   - Custom implementation that credits beneficiary with a fixed reward
   - Verify merged outcome includes both user and system changes
   - Verify state root is deterministic with hooks

2. Integration tests:
   - Execute block with reward hook, verify beneficiary balance increased
   - Execute block with fee-burning hook, verify total supply decreased
   - Verify build_block and verify_block produce identical state roots with hooks
   - Verify RevmContextProvider and RevmApplication produce identical outcomes with hooks

3. E2e test:
   - Run multi-validator network with reward hook
   - Verify all validators agree on state root (hooks are deterministic)
   - Verify beneficiary balance increases over multiple blocks
```

---

## Verification Steps

After implementing hooks, verify correctness with:

```bash
# 1. Confirm the trait compiles with the new methods
cargo build -p kora-executor

# 2. Run existing executor tests (should pass unchanged -- default hooks are no-ops)
cargo nextest run -p kora-executor

# 3. Run new hook-specific tests
cargo nextest run -p kora-executor -- test_post_execute
cargo nextest run -p kora-executor -- test_finalize_block

# 4. Verify all three call sites produce identical state roots
cargo nextest run -p kora-runner -- test_hook_determinism

# 5. Full regression suite
just test

# 6. E2e with hooks enabled
cargo nextest run -p kora-e2e --run-ignored all -- --test-threads=1
```

### Manual verification checklist:
- [ ] `BlockExecutor` trait in `crates/node/executor/src/traits.rs` has new hook methods with default no-op implementations
- [ ] `RevmExecutor::execute()` in `revm.rs` unchanged (hooks are called by the caller, not inside execute)
- [ ] `RevmApplication::build_block()` in `app.rs` calls hooks after `execute()` and merges changes before `compute_root`
- [ ] `RevmApplication::verify_block()` in `app.rs` calls the same hooks in the same order
- [ ] `handle_finalized_update()` in `reporters/src/lib.rs` calls the same hooks when re-executing
- [ ] `RevmContextProvider` in `runner.rs` and `block_context()` in `app.rs` produce identical contexts
- [ ] All existing tests pass without modification (backward-compatible default implementations)
