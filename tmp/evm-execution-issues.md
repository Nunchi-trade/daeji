# EVM Execution Layer Issues

## What is Kora?

Kora is a blockchain node built in Rust that uses the Simplex BFT consensus protocol (from the Commonware framework) to produce and finalize blocks. It executes Ethereum-compatible (EVM) transactions using REVM, an implementation of the Ethereum Virtual Machine written in Rust. State is persisted in QMDB, a custom Merkle database that provides cryptographic state roots for consensus verification.

## How EVM Execution Fits into the Block Pipeline

The EVM executor sits at the heart of Kora's block production and verification:

```
Simplex Consensus (propose/verify)
    |
    v
RevmApplication (crates/node/runner/src/app.rs)
    |
    |--- propose() -> build_block()
    |        |
    |        |-- mempool.build(max_txs, &excluded)   -> Select transactions
    |        |-- executor.execute(&state, &ctx, &txs) -> Execute transactions via REVM
    |        |-- ledger.compute_root(changes)         -> Compute new state root
    |        |-- Return Block { state_root, txs, ... }
    |
    |--- verify() -> verify_block()
             |
             |-- executor.execute(&state, &ctx, &txs) -> Re-execute transactions
             |-- ledger.compute_root(changes)         -> Recompute state root
             |-- Assert computed root == proposed root
```

### Execution Flow Within the Executor

For each block, the `RevmExecutor::execute()` method:

```
Transaction Bytes (Vec<Bytes>)
    |
    v
decode_tx_env()         -- RLP decode + ECDSA signature recovery
    |
    v
evm.set_tx(tx_env)     -- Load into REVM context
    |
    v
evm.replay()           -- Execute EVM bytecode
    |
    v
extract_changes()      -- Convert EvmState -> ChangeSet (nonce, balance, storage, code)
    |
    v
build_receipt()        -- Create transaction receipt (status, gas, logs)
    |
    v
db.commit(state)       -- Update in-memory EVM state for subsequent transactions
```

---

## Issue 1: Fatal Abort on Invalid Transaction

**Severity: CRITICAL**

**Location:** `crates/node/executor/src/revm.rs`, lines 391 and 395

### Description

The executor's transaction processing loop uses Rust's `?` (early return) operator on two fallible operations. If any single transaction in the batch fails to decode or fails at the REVM framework level, the entire block execution is aborted and the caller receives an `Err(ExecutionError)`.

```rust
// crates/node/executor/src/revm.rs:388-408
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;  // LINE 391 - FATAL
    evm.set_tx(tx_env);

    let result_and_state =
        evm.replay().map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;  // LINE 395 - FATAL

    let gas_used = result_and_state.result.tx_gas_used();
    cumulative_gas = cumulative_gas.saturating_add(gas_used);

    let receipt = build_receipt(&result_and_state.result, tx_hash, gas_used, cumulative_gas);
    outcome.receipts.push(receipt);

    let state = result_and_state.state;
    let changes = extract_changes(state.clone());
    evm.ctx.modify_db(|db| db.commit(state));
    outcome.changes.merge(changes);
}
```

### What Can Trigger This

**`decode_tx_env` (line 391) fails on:**
- Malformed RLP encoding
- Invalid transaction envelope structure
- ECDSA signature recovery failure (corrupted signature bytes)
- Failed `TxEnv` builder validation

**`evm.replay()` (line 395) fails on:**
- `NonceTooLow` / `NonceTooHigh` (stale transaction from mempool)
- `InsufficientBalance` (balance changed since mempool admission)
- Database state read errors propagated through the adapter
- REVM internal errors

**Important distinction:** EVM-level reverts (`revert()`, `assert()` failure, out-of-gas) do NOT trigger this. They produce `ExecutionResult::Revert` or `ExecutionResult::Halt` which are correctly handled as failed-but-included transactions with receipts.

### Why This Causes Permanent Chain Stalls

In `crates/node/runner/src/app.rs` lines 125-136:

```rust
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        warn!(parent = ?parent_digest, height, txs = txs.len(), error = ?err, "build_block: execution failed");
        return None;  // ENTIRE BLOCK PROPOSAL ABORTED
    }
};
```

When `build_block()` returns `None`, the consensus view is nullified. Because the offending transaction is never pruned from the mempool (pruning only happens on finalization), every subsequent leader picks up the same bad transaction, hits the same error, and nullifies again -- forever.

### Impact

This is the primary root cause of permanent chain stalls observed in production. A single stale-nonce transaction in the mempool can halt the entire network indefinitely.

**Cross-reference:** See `executor-fatal-abort.md` for the complete call chain, fix proposal, and error classification table.

---

## Issue 2: BLOCKHASH Opcode Always Returns Zero

**Severity: MEDIUM**

**Location:** `crates/node/executor/src/adapter.rs`, line 76-79

### Description

The `BLOCKHASH` EVM opcode (opcode `0x40`) allows smart contracts to access the hash of one of the 256 most recent blocks. In Kora's implementation, this always returns `B256::ZERO`:

```rust
// crates/node/executor/src/adapter.rs:76-79
fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
    // Block hash lookups not supported yet
    Ok(B256::ZERO)
}
```

The `_number` parameter (prefixed with underscore) confirms it is intentionally unused. No block hash history is maintained or queryable through the `StateDbRead` trait (which has no `block_hash` method).

### Root Cause

The `StateDbRead` trait in `crates/storage/traits/src/state.rs` provides only account-level operations (nonce, balance, code_hash, code, storage). There is no interface for historical block hash lookups. The QMDB storage layer does not maintain a block hash ring buffer.

### Impact

Smart contracts that call `blockhash(blockNumber)` will receive `0x0000...0000` instead of the actual block hash. This breaks:

- **Randomness schemes** that use `blockhash(block.number - 1)` as an entropy source
- **Commit-reveal protocols** that verify commitments against block hashes
- **Some DeFi protocols** (e.g., certain oracle designs, MEV protection mechanisms)
- **On-chain governance** contracts that use block hashes for snapshot verification

Basic transfers, token operations, and DEX swaps generally do not depend on `BLOCKHASH`, so day-to-day usage is unaffected.

### Required Fix

1. Add a `block_hash(&self, number: u64) -> Result<B256, StateDbError>` method to `StateDbRead`
2. Maintain a ring buffer of the last 256 block hashes (either in QMDB or a separate data structure)
3. Wire the lookup through `StateDbAdapter::block_hash_ref()`

---

## Issue 3: Async-to-Sync Bridge (block_in_place)

**Severity: LOW (operational risk under extreme load)**

**Location:** `crates/node/executor/src/adapter.rs`, lines 15-23

### Description

REVM's `DatabaseRef` trait is synchronous -- it expects blocking function calls that return immediately. However, Kora's state layer (`QmdbState` via the `StateDbRead` trait) is fully async. The adapter bridges this gap:

```rust
// crates/node/executor/src/adapter.rs:15-23
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current()
        && handle.runtime_flavor() == RuntimeFlavor::MultiThread
    {
        return tokio::task::block_in_place(|| handle.block_on(f));
    }

    futures::executor::block_on(f)
}
```

This function is called on every state access: `basic_ref()`, `code_by_hash_ref()`, `storage_ref()`. During a single transaction execution, REVM may issue dozens of state reads (loading balances, storage slots, contract code).

### How It Works

- **Multi-threaded Tokio runtime:** Uses `tokio::task::block_in_place()` which moves the current task off the worker thread, then blocks the OS thread waiting for the async future to complete.
- **Other runtimes (or no runtime):** Falls back to `futures::executor::block_on()`.

### Risks

1. **Thread starvation under load:** Each concurrent block verification (during catch-up or parallel proposal verification) blocks an OS thread. If many verifications run concurrently, Tokio's worker thread pool can be exhausted.
2. **Latency spikes:** `block_in_place` involves moving the task to a blocking thread, which adds scheduling overhead per state access.
3. **No batching:** Each storage slot read is a separate async call wrapped in a separate `block_in_place`. There is no mechanism to batch reads.

### Current Status

This has not been observed as a problem in practice because:
- Block execution is typically fast (< 1ms for small blocks)
- The REVM state caching layer (`State<WrapDatabaseRef<...>>`) reduces redundant lookups within a block
- Production load has been low

### Impact

Theoretical risk. Could become problematic during:
- Network catch-up (many blocks verified in parallel)
- High-throughput scenarios (many storage-heavy transactions per block)
- DDoS conditions (many concurrent RPC `eth_call` requests hitting `simulate_call`)

---

## Issue 4: No Post-Execution Hooks

**Severity: MEDIUM (limits future protocol features)**

**Location:** `crates/node/executor/src/revm.rs`, lines 357-411 (the `execute` method) and `crates/node/executor/src/traits.rs`

### Description

The `BlockExecutor` trait and its REVM implementation provide no mechanism to execute protocol-level state modifications after transaction processing is complete. The interface is:

```rust
// crates/node/executor/src/traits.rs:11-27
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

After the transaction loop completes, the executor immediately returns the `ExecutionOutcome`. There is no `post_execute()`, `apply_system_transactions()`, or `finalize_block()` hook.

### What This Prevents

- **Block rewards:** No way to credit the block proposer (beneficiary is set to `Address::ZERO` in `app.rs:74`)
- **Protocol fee burning:** No mechanism for EIP-1559-style fee burn after all transactions execute
- **System transactions:** No way to inject validator set updates, bridge messages, or protocol upgrades
- **Epoch transitions:** No hook for end-of-epoch state changes (e.g., rotating validator sets)
- **Event emission:** No way to emit block-level events (block sealed, epoch advanced) after execution

### Current State

The `BlockContext` includes `beneficiary: Address::ZERO` (set in `crates/node/runner/src/app.rs:74`), confirming that no block rewards are distributed. The EIP-1559 base fee calculation exists (`calculate_base_fee` at line 326) but with `base_fee_per_gas: Some(0)` (app.rs:74), fees are effectively zero.

---

## Issue 5: No Block Gas Limit Enforcement During Execution

**Severity: MEDIUM**

**Location:** `crates/node/executor/src/revm.rs`, lines 386-410

### Description

The executor tracks cumulative gas usage but never checks whether it exceeds the block gas limit:

```rust
// crates/node/executor/src/revm.rs:386-410
let mut cumulative_gas = 0u64;

for tx_bytes in txs {
    // ... decode, execute ...
    let gas_used = result_and_state.result.tx_gas_used();
    cumulative_gas = cumulative_gas.saturating_add(gas_used);  // Just accumulates, never checked

    let receipt = build_receipt(&result_and_state.result, tx_hash, gas_used, cumulative_gas);
    // ... commit state ...
}

outcome.gas_used = cumulative_gas;
```

There is no:
```rust
if cumulative_gas > context.header.gas_limit {
    break; // Stop including transactions
}
```

### Impact

The block gas limit is set in the `BlockEnv` for individual transaction execution (REVM will enforce per-transaction gas limits), but there is no enforcement that the aggregate gas across all transactions in a block stays within the block's gas limit. This means:

1. A block proposer could include more transactions than the block gas limit allows
2. The resulting block would have `gas_used > gas_limit` in its header, which is invalid per Ethereum consensus rules
3. However, in practice this is mitigated because `mempool.build(max_txs, ...)` limits the number of transactions, and block builders control the gas limit

### Mitigation

Currently, the gas limit is set to 250,000,000 (configured in the runner) and `max_txs` limits the transaction count. With basic transfers costing 21,000 gas, you would need ~12,000 transactions in a single block to exceed the limit. The `max_txs` configuration prevents this in practice.

---

## Issue 6: State Access Patterns and the Overlay Architecture

**Severity: INFORMATIONAL**

**Location:** `crates/node/executor/src/adapter.rs` and `crates/storage/overlay/src/overlay.rs`

### Architecture

State reads during execution follow a layered path:

```
REVM EVM Execution
    |
    v
StateDbAdapter (sync wrapper)
    |
    v
block_on() (async-to-sync bridge)
    |
    v
OverlayState<QmdbState>
    |
    |-- Check ChangeSet overlay (pending uncommitted changes)
    |-- If not found: fall through to QmdbState (QMDB on disk)
    |
    v
QmdbState (QmdbHandle<AccountStore, StorageStore, CodeStore>)
    |
    v
QMDB Storage (Merkle-ized key-value store)
```

The `OverlayState` (defined in `crates/storage/overlay/src/overlay.rs`) wraps a base `QmdbState` with a `ChangeSet` of pending (not-yet-committed) modifications:

```rust
pub struct OverlayState<S> {
    base: S,
    changes: Arc<ChangeSet>,
}
```

### Behavior During Execution

Within a single block execution:
- REVM maintains its own in-memory `State` cache (via `State::builder().with_database_ref(adapter).build()`)
- After each transaction, `evm.ctx.modify_db(|db| db.commit(state))` merges the transaction's state changes into REVM's cache
- Subsequent transactions in the same block see previous transactions' effects through REVM's cache (without going back to QMDB)
- The `StateDbAdapter` is only called for cold (first-access-in-block) reads

### Observations

1. **No write-through during execution:** Changes are accumulated in memory and returned as `outcome.changes`. They are only committed to QMDB after verification succeeds and finalization occurs.
2. **Multiple reads per account access:** `basic_ref()` makes three sequential async calls (`nonce`, `balance`, `code_hash`) -- not batched.
3. **Account existence check:** If `nonce()` returns `AccountNotFound`, the adapter returns `None` (account does not exist). This is correct EVM semantics.
4. **Storage for non-existent accounts:** Returns `U256::ZERO`, which matches EVM semantics (non-existent storage reads as zero).

---

## Issue 7: Missing EVM Features and Unsupported Opcodes

**Severity: LOW-MEDIUM (depends on use case)**

### Spec Configuration

**Location:** `crates/node/executor/src/config.rs`, line 65

```rust
pub const fn new(chain_id: u64) -> Self {
    Self {
        chain_id,
        spec_id: SpecId::CANCUN,
        // ...
    }
}
```

The executor targets the **Cancun** hardfork spec (configurable to PRAGUE via `with_spec_id()`). This means:

### Supported Features (via REVM Cancun)

| Feature | EIP | Status |
|---------|-----|--------|
| EIP-1559 dynamic fees | EIP-1559 | Supported (but base_fee = 0) |
| Access lists | EIP-2930 | Supported |
| PUSH0 opcode | EIP-3855 | Supported |
| Transient storage (TSTORE/TLOAD) | EIP-1153 | Supported |
| Blob transactions | EIP-4844 | Decoded but incomplete |
| MCOPY opcode | EIP-5656 | Supported |
| SELFDESTRUCT restrictions | EIP-6780 | Supported |
| EIP-7702 delegation | EIP-7702 | Decoded and converted |

### Incomplete / Unsupported

| Feature | Issue |
|---------|-------|
| **BLOCKHASH** (opcode 0x40) | Returns zero (see Issue 2) |
| **Blob gas accounting** | EIP-4844 blob gas is not tracked. `BlockContext.blob_base_fee` exists but is never wired into REVM's `BlockEnv.blob_gasprice` |
| **Beacon root contract** (EIP-4788) | No system call to the beacon root contract at block start |
| **DIFFICULTY/PREVRANDAO** | `prevrandao` is set from `ledger.seed_for_parent()` which may return `B256::ZERO` |
| **Block reward / coinbase payment** | `beneficiary = Address::ZERO`, no reward distributed |
| **Custom precompiles** | Uses `build_mainnet()` -- standard Ethereum precompiles only (ecrecover, sha256, ripemd160, identity, modexp, ecAdd, ecMul, ecPairing, blake2f, point_evaluation). No Kora-specific precompiles. |

### Precompiles

The executor calls `ctx.build_mainnet()` (lines 234 and 383), which includes all standard Ethereum precompiled contracts for the Cancun spec. No custom precompiles are registered. If Kora needs protocol-level functionality accessible from smart contracts (e.g., staking, bridge operations), custom precompiles would need to be added.

### Transaction Types Supported

| Type | EIP | Decode | Execute | Notes |
|------|-----|--------|---------|-------|
| Legacy | Pre-2718 | Yes | Yes | Full support |
| Access List | EIP-2930 | Yes | Yes | Full support |
| Dynamic Fee (EIP-1559) | EIP-1559 | Yes | Yes | Full support |
| Blob (EIP-4844) | EIP-4844 | Yes | Partial | Decoded, validated (hash version, count), but blob gas not tracked in block |
| Delegation (EIP-7702) | EIP-7702 | Yes | Yes | Authorization list converted and passed to REVM |

---

## Issue 8: Access List Zero-Address Rejection

**Severity: LOW**

**Location:** `crates/node/executor/src/validation.rs`, lines 306-315

### Description

```rust
// crates/node/executor/src/validation.rs:306-315
fn validate_access_list(&self, access_list: &AccessList) -> Result<(), ExecutionError> {
    for item in access_list.iter() {
        if item.address.is_zero() {
            return Err(ExecutionError::InvalidTx(
                "access list contains zero address".to_string(),
            ));
        }
    }
    Ok(())
}
```

The EVM specification does not prohibit `address(0)` in access lists. While including it is unusual, some edge cases may legitimately touch the zero address (e.g., contracts that interact with pre-deployed system contracts at address 0, or contracts that check the zero address balance as a burn verification).

### Impact

Transactions with `address(0)` in their access list will be rejected during pre-validation, even though they would execute correctly in standard Ethereum. This is a minor deviation from spec compliance.

---

## Summary Table

| # | Issue | Severity | Impact | File | Line(s) |
|---|-------|----------|--------|------|----------|
| 1 | Fatal abort on invalid tx | **CRITICAL** | Permanent chain stalls | `crates/node/executor/src/revm.rs` | 391, 395 |
| 2 | BLOCKHASH returns zero | **MEDIUM** | Contracts get wrong data | `crates/node/executor/src/adapter.rs` | 76-79 |
| 3 | Async-sync bridge | **LOW** | Thread starvation risk | `crates/node/executor/src/adapter.rs` | 15-23 |
| 4 | No post-execution hooks | **MEDIUM** | Cannot add rewards/system txs | `crates/node/executor/src/traits.rs` | 11-27 |
| 5 | No block gas limit enforcement | **MEDIUM** | Over-gas blocks possible | `crates/node/executor/src/revm.rs` | 386-410 |
| 6 | State access patterns | **INFO** | 3 sequential reads per account | `crates/node/executor/src/adapter.rs` | 48-57 |
| 7 | Missing EVM features | **LOW-MED** | Blob gas, beacon root, precompiles | `crates/node/executor/src/revm.rs` | 383 |
| 8 | Zero-address access list rejection | **LOW** | Spec deviation | `crates/node/executor/src/validation.rs` | 306-315 |

---

## Error Type Reference

All execution errors are defined in `crates/node/executor/src/error.rs`:

```rust
pub enum ExecutionError {
    State(StateDbError),      // Database read failure (system-level)
    TxDecode(String),         // RLP decode / signature recovery failure
    TxExecution(String),      // REVM framework error (nonce mismatch, insufficient balance)
    Revert(Bytes),            // Smart contract revert with ABI-encoded data
    InvalidTx(String),        // Pre-validation failure (gas, chain_id, nonce, balance)
    BlockValidation(String),  // Block-level header validation failure
    CodeNotFound(B256),       // Missing contract bytecode for given hash
}
```

**Critical insight:** Currently ALL variants abort the block via `?`. The correct behavior should be:
- `State` errors: Abort block (system failure)
- `TxDecode` / `TxExecution` / `InvalidTx`: Skip transaction, continue block
- `Revert` / `Halt`: Already handled correctly (included as failed receipt)
- `BlockValidation`: Abort block (invalid block)

---

## Configuration Reference

| Parameter | Value | Location |
|-----------|-------|----------|
| Default Spec ID | `CANCUN` | `crates/node/executor/src/config.rs:65` |
| Default Chain ID | `1` (overridden per deployment) | `crates/node/executor/src/config.rs:63` |
| Gas Limit Min | `5,000` | `crates/node/executor/src/config.rs:19` |
| Gas Limit Max | `u64::MAX` | `crates/node/executor/src/config.rs:19` |
| Gas Limit Delta Divisor | `1024` (changes by at most ~0.1% per block) | `crates/node/executor/src/config.rs:19` |
| EIP-1559 Elasticity | `2` | `crates/node/executor/src/config.rs:39` |
| EIP-1559 Max Change Denominator | `8` (12.5% max change per block) | `crates/node/executor/src/config.rs:39` |
| Block Gas Limit (runtime) | `250,000,000` | `crates/node/runner/src/app.rs` (constructor) |
| Beneficiary | `Address::ZERO` | `crates/node/runner/src/app.rs:74` |
| Base Fee | `0` (hardcoded in app) | `crates/node/runner/src/app.rs:74` |

---

## File Reference

| File | Purpose |
|------|---------|
| `crates/node/executor/src/revm.rs` | Core REVM executor: block execution, gas estimation, base fee calculation, tx decoding |
| `crates/node/executor/src/adapter.rs` | Async-to-sync bridge: adapts `StateDbRead` to REVM's `DatabaseRef` |
| `crates/node/executor/src/traits.rs` | `BlockExecutor` trait definition |
| `crates/node/executor/src/config.rs` | `ExecutionConfig`, `GasLimitBounds`, `BaseFeeParams` |
| `crates/node/executor/src/error.rs` | `ExecutionError` enum |
| `crates/node/executor/src/context.rs` | `BlockContext` and `ParentBlock` structs |
| `crates/node/executor/src/outcome.rs` | `ExecutionOutcome` and `ExecutionReceipt` |
| `crates/node/executor/src/validation.rs` | `TxValidator` pre-execution checks |
| `crates/node/runner/src/app.rs` | `RevmApplication` -- consensus integration calling executor |
| `crates/storage/traits/src/state.rs` | `StateDbRead` / `StateDbWrite` / `StateDb` traits |
| `crates/storage/overlay/src/overlay.rs` | `OverlayState<S>` -- layered state for pending changes |
| `crates/storage/qmdb-ledger/src/ledger.rs` | `QmdbLedger` / `QmdbState` -- persistent QMDB storage |
