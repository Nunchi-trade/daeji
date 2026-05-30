# Executor and Block Building Analysis

## Summary

The block building pipeline has a **fatal architectural flaw**: a single failed transaction aborts the entire block. The `?` operator in `RevmExecutor::execute()` propagates any transaction error immediately, causing `build_block()` to return `None`, which nullifies the consensus round. Combined with a mempool that never prunes failed transactions, this creates a permanent stall.

---

## 1. RevmExecutor::execute() — The Fatal `?` Operator

**File:** `crates/node/executor/src/revm.rs` (Lines 354-412)

```rust
fn execute(
    &self,
    state: &S,
    context: &BlockContext,
    txs: &[Self::Tx],
) -> Result<ExecutionOutcome, ExecutionError> {
    // ... REVM setup (lines 361-383) ...

    let mut outcome = ExecutionOutcome::new();
    let mut cumulative_gas = 0u64;

    for tx_bytes in txs {
        let tx_hash = keccak256(tx_bytes);

        // FATAL LINE 391: `?` aborts entire block on decode error
        let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;
        evm.set_tx(tx_env);

        // FATAL LINE 395: `?` aborts entire block on execution error
        let result_and_state =
            evm.replay().map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;

        let gas_used = result_and_state.result.tx_gas_used();
        cumulative_gas = cumulative_gas.saturating_add(gas_used);

        let receipt = build_receipt(&result_and_state.result, tx_hash, gas_used, cumulative_gas);
        outcome.receipts.push(receipt);

        // Lines 404-407: Commit per-transaction state changes
        let state = result_and_state.state;
        let changes = extract_changes(state.clone());
        evm.ctx.modify_db(|db| db.commit(state));  // DatabaseCommit trait
        outcome.changes.merge(changes);
    }

    outcome.gas_used = cumulative_gas;
    Ok(outcome)
}
```

### Two Fatal Points

| Line | Code | Triggers On |
|------|------|-------------|
| 391 | `decode_tx_env(tx_bytes, chain_id)?` | Invalid RLP, wrong chain ID, nonce validation by REVM |
| 395 | `evm.replay().map_err(...)? ` | NonceTooHigh, NonceTooLow, insufficient balance, out of gas |

Both use the `?` operator which immediately returns `Err(ExecutionError)` from the entire function. **No remaining transactions are processed.**

### Partial State Commitment Issue

Lines 404-407 commit state changes per-transaction via `db.commit(state)`. If transaction N succeeds but transaction N+1 fails:
- Transactions 0..N have their state committed to REVM's in-memory database
- The entire block is discarded (error returned)
- Those committed changes are lost (REVM database is dropped)

This isn't a data corruption issue (the partial state is never persisted), but it means wasted computation.

---

## 2. decode_tx_env() — Transaction Decoding

**File:** `crates/node/executor/src/revm.rs` (Lines 433-543)

```rust
fn decode_tx_env(tx_bytes: &Bytes, _chain_id: u64) -> Result<revm::context::TxEnv, ExecutionError> {
    let envelope = TxEnvelope::decode_2718(&mut tx_bytes.as_ref())
        .map_err(|e| ExecutionError::TxDecode(format!("{}", e)))?;

    let mut builder = revm::context::TxEnv::builder();

    match &envelope {
        TxEnvelope::Legacy(signed) => {
            builder = builder.nonce(tx.nonce) /* ... */;
        }
        TxEnvelope::Eip1559(signed) => {
            builder = builder.nonce(tx.nonce) /* ... */;
        }
        // ... other variants
    }

    builder.build()
        .map_err(|e| ExecutionError::TxDecode(format!("failed to build tx env: {:?}", e)))
}
```

- Handles Legacy, EIP-2930, EIP-1559, EIP-4844, EIP-7702 transaction types
- Builder validation can reject transactions (nonce bounds, etc.)
- Any failure becomes `ExecutionError::TxDecode` → aborts block

---

## 3. StateDbAdapter — Bridging Async to Sync

**File:** `crates/node/executor/src/adapter.rs`

```rust
impl<S: StateDbRead> DatabaseRef for StateDbAdapter<S> {
    type Error = ExecutionError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match block_on(self.state.nonce(&address)) {
            Ok(nonce) => {
                let balance = block_on(self.state.balance(&address))?;
                let code_hash = block_on(self.state.code_hash(&address))?;
                Ok(Some(AccountInfo { nonce, balance, code_hash, code: None, account_id: None }))
            }
            Err(StateDbError::AccountNotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match block_on(self.state.storage(&address, &index)) {
            Ok(value) => Ok(value),
            Err(StateDbError::AccountNotFound(_)) => Ok(U256::ZERO),
            Err(e) => Err(e.into()),
        }
    }
}
```

- Uses `block_on()` to bridge async `StateDb` → REVM's sync `DatabaseRef`
- `AccountNotFound` → `Ok(None)` (non-existent accounts return no data)
- Other errors → `ExecutionError::State` → propagated via `?` → block aborted

---

## 4. RevmApplication::build_block()

**File:** `crates/node/runner/src/app.rs` (Lines 84-164)

```rust
async fn build_block(&self, parent: &Block) -> Option<Block> {
    let parent_snapshot = self.ledger.parent_snapshot(parent_digest).await?;  // FAIL POINT 1

    let (_, mempool, snapshots) = self.ledger.proposal_components().await;
    let excluded = self.collect_pending_tx_ids(&snapshots, parent_digest);
    let txs = mempool.build(self.max_txs, &excluded);

    // Diagnostic warning: mempool has txs but none were selected
    if txs.is_empty() && mempool_len > excluded_len {
        warn!("build_block: mempool has unincluded txs but produced empty block");
    }

    let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
        Ok(outcome) => outcome,
        Err(err) => {
            warn!(error = ?err, "build_block: execution failed");
            return None;  // FAIL POINT 2 — NULLIFICATION
        }
    };

    let state_root = self.ledger.compute_root_from_store(parent_digest, outcome.changes.clone())
        .await.ok()?;  // FAIL POINT 3

    Some(Block { parent: parent.id(), height, prevrandao, state_root, txs })
}
```

### Failure Points

| # | Line | Cause | Result |
|---|------|-------|--------|
| 1 | 89 | Parent snapshot not in memory | `None` → nullification |
| 2 | 135 | Any transaction fails in executor | `None` → nullification |
| 3 | 145 | State root computation fails | `None` → nullification |

**Consequence of returning `None`:** Simplex consensus sees an empty proposal. The round is nullified. The nullification counter increments. A new leader is elected for the next view. But the same poisoned transactions remain in the mempool.

---

## 5. RevmApplication::verify_block()

**File:** `crates/node/runner/src/app.rs` (Lines 166-246)

```rust
async fn verify_block(&self, block: &Block) -> bool {
    // Check cached
    if self.ledger.query_state_root(digest).await.is_some() { return true; }

    // Get parent
    let Some(parent_snapshot) = self.ledger.parent_snapshot(parent_digest).await else {
        return false;  // FAIL POINT 1
    };

    // Re-execute
    let execution = match BlockExecution::execute(&parent_snapshot, &self.executor, &context, &block.txs).await {
        Ok(result) => result,
        Err(err) => { return false; }  // FAIL POINT 2
    };

    // Compute and compare state root
    let state_root = match self.ledger.compute_root_from_store(...).await {
        Ok(root) => root,
        Err(err) => { return false; }  // FAIL POINT 3
    };

    if state_root != block.state_root {
        warn!("state root mismatch");
        return false;  // FAIL POINT 4 — MOST CRITICAL (consensus divergence)
    }

    // Cache verified state
    self.ledger.insert_snapshot(digest, parent_digest, next_state, state_root, ...).await;
    true
}
```

### Failure Points

| # | Line | Cause | Impact |
|---|------|-------|--------|
| 1 | 178 | Missing parent snapshot | Block rejected |
| 2 | 191 | Execution failure | Block rejected |
| 3 | 205 | Root computation failure | Block rejected |
| 4 | 217 | **State root mismatch** | Consensus violation — validator isolated |

**State root mismatch** is the most dangerous: it means two validators executed the same transactions and got different results. This can happen if:
- Different overlay state base (one validator persisted, another didn't)
- Non-deterministic execution (unlikely with REVM)
- Different transaction ordering (possible if mempool ordering differs)

---

## 6. Error Type Hierarchy

**File:** `crates/node/executor/src/error.rs`

```rust
pub enum ExecutionError {
    State(StateDbError),        // Database read failure
    TxDecode(String),           // RLP/envelope decode failure
    TxExecution(String),        // REVM execution failure (nonce, gas, etc.)
    Revert(Bytes),              // EVM call reverted with data
    InvalidTx(String),          // Transaction validation failure
    BlockValidation(String),    // Block-level validation failure
    CodeNotFound(B256),         // Contract code missing
}
```

**Problem:** All errors are treated equally fatal. There's no distinction between:
- "This transaction is bad, skip it" (e.g., NonceTooLow)
- "The system is broken" (e.g., State database failure)

---

## 7. Complete Block Building Pipeline

```
InMemoryMempool.build(max_txs, excluded)
    │
    ├── O(n) ECDSA recovery for ordering
    ├── Sort by (sender, nonce)
    ├── Take top max_txs
    │
    ▼
build_block() [app.rs:84-164]
    │
    ├── Get parent snapshot from ledger
    ├── Get txs from mempool
    │
    ▼
executor.execute(state, context, txs) [revm.rs:354-412]
    │
    ├── For EACH transaction:
    │   ├── decode_tx_env() ──── ? ──── Err → ABORT ENTIRE BLOCK
    │   ├── evm.replay()   ──── ? ──── Err → ABORT ENTIRE BLOCK
    │   ├── extract_changes()
    │   └── db.commit(state)
    │
    ├── If any tx fails: Err(ExecutionError) returned
    │   └── build_block returns None → NULLIFICATION
    │
    └── If all succeed: Ok(ExecutionOutcome)
        │
        ▼
    compute_root_from_store()
        │
        └── Some(Block) → PROPOSAL SUCCESS
```

---

## 8. Required Fix

Replace the abort-on-failure pattern with skip-and-continue:

```rust
// CURRENT (broken):
let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;

// FIXED:
let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
    Ok(env) => env,
    Err(err) => {
        warn!(tx_hash = ?keccak256(tx_bytes), error = ?err, "skipping failed tx");
        continue;  // Skip this tx, try next one
    }
};
```

Same pattern for `evm.replay()`. Additionally, track skipped transaction IDs so they can be removed from the mempool.
