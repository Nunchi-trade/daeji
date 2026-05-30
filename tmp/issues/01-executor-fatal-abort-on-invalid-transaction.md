# Executor: Single invalid transaction aborts entire block proposal, causing permanent chain stall

**Severity:** P0 / Critical
**Component:** `kora-executor`, `kora-runner`
**Affects:** All validators in the network

---

## Summary

A single invalid transaction in the mempool causes the REVM executor to return a fatal error during block building, which the consensus application interprets as a total block failure and returns `None`. Because mempool pruning only happens on finalization, and finalization cannot occur if no blocks are produced, the offending transaction is never removed. Every subsequent leader hits the same failure, creating a permanent chain stall that requires manual operator intervention.

---

## Background

Kora is an EVM-compatible blockchain built on top of the Commonware consensus framework (Simplex BFT). It uses REVM (Rust EVM) for transaction execution.

The block production pipeline works as follows:

1. Transactions arrive via RPC and are inserted into the mempool.
2. When a validator is elected leader for a view, the consensus layer calls `build_block()` on the application.
3. `build_block()` drains transactions from the mempool and passes them to the executor (`RevmExecutor::execute()`).
4. The executor iterates over each transaction, decodes it, runs it through REVM, and accumulates state changes and receipts into an `ExecutionOutcome`.
5. The resulting block is proposed to the other validators.
6. Once a block is finalized (2/3+ votes), the finalization reporter persists the block and prunes the executed transactions from the mempool.

---

## The Bug

The core issue is in `crates/node/executor/src/revm.rs`, lines 388-411, inside the `execute()` method of `RevmExecutor`:

```rust
// crates/node/executor/src/revm.rs, lines 388-411
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = decode_tx_env(tx_bytes, self.config.chain_id)?;  // <-- line 391: ? aborts on decode failure
    evm.set_tx(tx_env);

    let result_and_state =
        evm.replay().map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;  // <-- line 395: ? aborts on execution failure

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

There are two `?` operators in this loop:

1. **Line 391** (`decode_tx_env(...)?`): If a transaction cannot be RLP-decoded or its signer cannot be recovered, the entire `execute()` call returns `Err(ExecutionError::TxDecode(...))`.

2. **Line 395** (`evm.replay().map_err(...)?`): If REVM rejects a transaction at the framework level (nonce mismatch, insufficient balance for gas, etc.), the entire `execute()` call returns `Err(ExecutionError::TxExecution(...))`.

Both of these abort the entire block -- not just the offending transaction. All previously-processed valid transactions in the same batch are discarded.

---

## The Failure Cascade

Here is the step-by-step failure sequence:

1. **A bad transaction enters the mempool.** For example, a user submits a transaction with a stale nonce, or malformed RLP bytes are injected. The `InMemoryMempool` performs no validation beyond deduplication, so the transaction is accepted.

2. **A leader is elected and calls `build_block()`.** The method is in `crates/node/runner/src/app.rs`, line 84. It drains transactions from the mempool (line 96: `mempool.build(self.max_txs, &excluded)`) and passes them to the executor.

3. **The executor hits the `?` operator.** When it reaches the bad transaction, `decode_tx_env()` or `evm.replay()` returns an error. The `?` propagates it, and `execute()` returns `Err(...)`.

4. **`build_block()` returns `None`.** The error is caught in `crates/node/runner/src/app.rs`, lines 124-137:

   ```rust
   // crates/node/runner/src/app.rs, lines 124-137
   let exec_start = Instant::now();
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
           return None;  // <-- entire block proposal abandoned
       }
   };
   ```

   Returning `None` from `build_block()` tells the consensus layer that this leader has no block to propose. The view is nullified.

5. **The transaction stays in the mempool.** Mempool pruning happens exclusively in the finalization reporter, at `crates/node/reporters/src/lib.rs`, line 226:

   ```rust
   // crates/node/reporters/src/lib.rs, line 226
   state.prune_mempool(&block.txs).await;
   ```

   This is called only after a block is successfully finalized. Since no block was produced, no finalization occurs, and no pruning happens. The bad transaction remains in the mempool.

6. **The next leader hits the same failure.** The consensus layer rotates to a new leader for the next view. That leader calls `build_block()`, drains the same mempool (which still contains the bad transaction), and the executor fails again. The cycle repeats.

7. **Permanent stall.** Views continue to advance (the consensus timeout mechanism ensures liveness of view changes), but no blocks are ever finalized. The chain is permanently stalled. All 4 validators in a typical devnet are affected because they all share the same mempool contents via gossip.

**Note on `verify_block()`:** The verification path (`crates/node/runner/src/app.rs`, line 166, `verify_block()`) calls the same executor via `BlockExecution::execute()`. This means even if only ONE validator manages to avoid the bad transaction during proposal, the other 3 validators will fail when verifying the proposed block, because re-execution of the block's transactions can also hit the `?` abort. With 4 validators needing 3/4 for finalization (2f+1), a single failure in verification can prevent the quorum needed for finalization.

---

## What DOES NOT Trigger This Bug

EVM-level reverts and halts are handled correctly. After `evm.replay()` succeeds, the `ExecutionResult` is inspected in `build_receipt()` (`crates/node/executor/src/revm.rs`, lines 602-623):

```rust
fn build_receipt(
    result: &ExecutionResult,
    tx_hash: B256,
    gas_used: u64,
    cumulative_gas_used: u64,
) -> ExecutionReceipt {
    let (success, logs, contract_address) = match result {
        ExecutionResult::Success { logs, output, .. } => {
            let contract_addr = match output {
                Output::Create(_, addr) => *addr,
                Output::Call(_) => None,
            };
            (true, logs.clone(), contract_addr)
        }
        ExecutionResult::Revert { .. } => (false, Vec::new(), None),
        ExecutionResult::Halt { .. } => (false, Vec::new(), None),
    };
    ExecutionReceipt::new(tx_hash, success, gas_used, cumulative_gas_used, logs, contract_address)
}
```

All three arms (`Success`, `Revert`, `Halt`) produce a valid receipt. A contract that reverts, runs out of gas, or triggers a SELFDESTRUCT will not stall the chain. The transaction is included in the block with `success: false`, and the chain continues.

---

## What DOES Trigger This Bug

The failures that trigger the fatal abort happen *before* REVM can produce an `ExecutionResult`. These are framework-level rejections:

### At `decode_tx_env` (line 391):

- **Malformed RLP**: Corrupted or truncated transaction bytes that fail `TxEnvelope::decode_2718()`.
- **Invalid signature**: A transaction where `recover_signer()` fails (bad v/r/s values).
- **Unsupported transaction type**: A future EIP envelope type that the current decoder does not handle.

### At `evm.replay()` (line 395):

REVM's `replay()` can return an `EVMError` before execution begins. Common causes include:

- **NonceTooLow**: The transaction nonce is less than the sender's current nonce in state. This is the most common trigger -- a user resubmits a transaction or a previously-confirmed transaction lingers in the mempool.
- **NonceTooHigh**: The transaction nonce is too far ahead (when nonce gaps are not allowed by the spec).
- **InsufficientBalance**: The sender's balance cannot cover `gas_limit * gas_price + value` at the framework validation level (before any EVM execution).
- **CreateInitCodeSizeLimit**: Contract creation initcode exceeds the EIP-3860 limit.
- **CallerAccountNotFound**: The sender address has no account in state.

---

## Error Classification Table

| Error Source | Error Type | Current Behavior | Correct Behavior |
|---|---|---|---|
| `decode_tx_env` | `TxDecode` (malformed RLP) | Abort block | Skip transaction |
| `decode_tx_env` | `TxDecode` (bad signature) | Abort block | Skip transaction |
| `evm.replay()` | NonceTooLow | Abort block | Skip transaction |
| `evm.replay()` | NonceTooHigh | Abort block | Skip transaction |
| `evm.replay()` | InsufficientBalance (pre-check) | Abort block | Skip transaction |
| `evm.replay()` | CreateInitCodeSizeLimit | Abort block | Skip transaction |
| `evm.replay()` | CallerAccountNotFound | Abort block | Skip transaction |
| `evm.replay()` | Database/state error | Abort block | Abort block (correct) |
| `ExecutionResult` | Revert | Receipt with `success: false` | Already correct |
| `ExecutionResult` | Halt (OutOfGas, etc.) | Receipt with `success: false` | Already correct |

The only error that should legitimately abort the entire block is a database/state error, which indicates infrastructure failure. All transaction-level errors should skip the individual transaction and continue processing the remaining batch.

---

## Impact

- **Scope:** Entire chain. All validators are affected because they all receive the same transaction via gossip and all hit the same executor failure.
- **Severity:** P0 -- complete chain halt. No transactions can be processed. No blocks are finalized.
- **Recovery:** Manual restart of all validators with mempool cleared, or manual removal of the offending transaction (no tooling exists for this currently).
- **Ease of trigger:** Trivially easy. Any user can submit a transaction with a stale nonce (e.g., nonce 0 when their account is already at nonce 5). This is a normal occurrence in any EVM network -- wallets retry, users double-click, nonces get stale.
- **Blast radius:** A single malicious or careless user can halt the entire network for all participants.

---

## Compounding Factor: Early Return Paths Skip Pruning

Even if the chain does manage to finalize a block, the `handle_finalized_update` function in `crates/node/reporters/src/lib.rs` (lines 105-232) has **six** early-return paths that call `ack.acknowledge()` and return WITHOUT calling `prune_mempool()`:

| Lines | Condition |
|-------|-----------|
| 142-146 | Block execution failure during re-execution |
| 154-158 | Root computation failure |
| 160-169 | State root mismatch for finalized block |
| 197-199 | Missing parent snapshot (no cached snapshot) |
| 210-214 | Persist task panicked/cancelled (JoinError) |
| 216-220 | Persist data write failure |

Since a finalized block is irrevocable (it has threshold BLS signatures from 2/3+ validators), its transactions have consumed nonces in the canonical state. If any of these error paths trigger, finalized transactions remain in the mempool and will be re-proposed in future blocks, causing the exact same executor failure described here.

---

## Error Types Reference

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

Currently ALL variants abort the block via `?`. The correct behavior should be:
- `State` errors: Abort block (system failure -- correct)
- `TxDecode` / `TxExecution` / `InvalidTx` / `CodeNotFound`: Skip transaction, continue block
- `Revert` / `Halt`: Already handled correctly (included as failed receipt)
- `BlockValidation`: Abort block (invalid block -- correct)

---

## Observable Symptoms

When this bug is active, operators will observe:

1. **100% nullification rate**: Every view is nullified because every leader returns `None` from `build_block()`. The Grafana dashboard will show views advancing but zero finalized blocks.
2. **Repeated log warnings**: Every leader rotation produces:
   ```
   WARN build_block: execution failed parent=0x... height=N txs=M error=TxDecode("...") | TxExecution("...")
   ```
3. **Mempool size never decreases**: The mempool length stays constant or grows (as new transactions arrive) because pruning never fires.
4. **Views advancing without finalization**: The consensus view counter increments but the finalized height stays flat. The gap between view number and finalized height grows unboundedly.
5. **Consensus timing pattern**: The Simplex consensus configuration (from `crates/node/runner/src/runner.rs`) uses a `leader_timeout` of 2 seconds and `certification_timeout` of 4 seconds. If all validators are nullifying, the chain cycles through views at approximately 2 seconds per view. With 4 validators, all leaders cycle every 4 views (~8 seconds) and the pattern repeats indefinitely.

---

## Proposed Fix

Replace the `?` operators in the executor loop with `match` + `continue`, so that individual transaction failures skip the bad transaction instead of aborting the entire block.

Additionally, track skipped transactions in the `ExecutionOutcome` so that the caller can evict them from the mempool immediately (rather than waiting for finalization, which will never happen for a skipped transaction).

### Fixed code for `crates/node/executor/src/revm.rs`:

```rust
for tx_bytes in txs {
    let tx_hash = keccak256(tx_bytes);

    let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
        Ok(env) => env,
        Err(err) => {
            warn!(?tx_hash, ?err, "skipping undeccodable transaction");
            outcome.skipped_txs.push(tx_hash);
            continue;
        }
    };
    evm.set_tx(tx_env);

    let result_and_state = match evm.replay() {
        Ok(result) => result,
        Err(err) => {
            warn!(?tx_hash, ?err, "skipping invalid transaction");
            outcome.skipped_txs.push(tx_hash);
            continue;
        }
    };

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

### Add `skipped_txs` to `ExecutionOutcome` (`crates/node/executor/src/outcome.rs`):

The current struct (lines 9-16) needs a new field:

```rust
pub struct ExecutionOutcome {
    pub changes: ChangeSet,
    pub receipts: Vec<ExecutionReceipt>,
    pub gas_used: u64,
    /// Transaction hashes that were skipped due to decode or validation errors.
    /// These should be evicted from the mempool since they will never execute successfully.
    pub skipped_txs: Vec<B256>,
}
```

Also update the `new()` method (lines 20-23) to initialize the new field:

```rust
pub fn new() -> Self {
    Self { changes: ChangeSet::new(), receipts: Vec::new(), gas_used: 0, skipped_txs: Vec::new() }
}
```

And update the `Default` derive -- since the struct derives `Default`, `Vec<B256>` already defaults to empty, so the derive continues to work. But the manual `new()` constructor must be updated as shown above.

### Evict skipped transactions in `build_block()` (`crates/node/runner/src/app.rs`):

After execution succeeds, evict skipped transactions from the mempool so they do not poison future proposals:

```rust
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        // This should now only happen for true infrastructure errors (database failures).
        warn!(parent = ?parent_digest, height, txs = txs.len(), error = ?err, "build_block: execution failed");
        return None;
    }
};

// Evict transactions that the executor skipped (decode errors, nonce issues, etc.)
if !outcome.skipped_txs.is_empty() {
    warn!(count = outcome.skipped_txs.len(), "evicting skipped transactions from mempool");
    let skipped_ids: Vec<TxId> = outcome.skipped_txs.iter().map(|h| TxId(*h)).collect();
    self.ledger.prune_mempool_by_ids(&skipped_ids).await;
}
```

Note: `prune_mempool_by_ids` does not exist yet on `LedgerService`. You will need to add it.
The existing `prune_mempool(&self, txs: &[Tx])` (at `crates/node/ledger/src/lib.rs`, line 477) takes full `Tx` objects and internally converts them to `TxId`. You need a variant that accepts `TxId` directly:

```rust
// Add to LedgerView (after line 340 in crates/node/ledger/src/lib.rs):
pub async fn prune_mempool_by_ids(&self, tx_ids: &[TxId]) {
    let inner = self.inner.lock().await;
    inner.mempool.prune(tx_ids);
}

// Add to LedgerService (after line 479 in crates/node/ledger/src/lib.rs):
pub async fn prune_mempool_by_ids(&self, tx_ids: &[TxId]) {
    self.view.prune_mempool_by_ids(tx_ids).await;
}
```

---

## Testing

### Unit test: batch with one bad transaction

Submit a batch of 3 transactions to `RevmExecutor::execute()` where the second transaction has a stale nonce (nonce 0 when the sender is already at nonce 1). Verify:

- `execute()` returns `Ok(outcome)`, not `Err(...)`.
- `outcome.receipts` contains 2 receipts (for the valid transactions).
- `outcome.skipped_txs` contains 1 entry (the hash of the bad transaction).
- State changes from the valid transactions are correctly applied.

### Unit test: malformed RLP

Submit a batch where one transaction is random bytes (not valid RLP). Verify:

- `execute()` returns `Ok(outcome)`.
- The malformed transaction is in `skipped_txs`.
- Other transactions succeed normally.

### E2E test: stale nonce does not stall chain

In a 4-validator devnet:

1. Fund an account and submit a transfer (nonce 0) -- wait for finalization.
2. Submit another transaction with nonce 0 (stale).
3. Submit a valid transaction with nonce 1.
4. Verify that the chain continues to finalize blocks.
5. Verify that nonce-1 transaction is included and finalized.
6. Verify that the stale nonce-0 transaction is evicted from the mempool.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/executor/src/revm.rs` | Replace `?` operators on lines 391 and 395 with `match` + `continue`. Add skipped tx tracking. |
| `crates/node/executor/src/outcome.rs` | Add `skipped_txs: Vec<B256>` field to `ExecutionOutcome`. Update `new()` constructor. |
| `crates/node/runner/src/app.rs` | After execution in `build_block()` (line 137), add eviction of skipped transactions from mempool. |
| `crates/node/ledger/src/lib.rs` | Add `prune_mempool_by_ids(&self, tx_ids: &[TxId])` to both `LedgerView` and `LedgerService`. |

---

## Verification

After implementing the fix, verify it works with these checks:

### 1. Unit test: run `cargo test` in the executor crate

```bash
cd crates/node/executor && cargo test
```

All existing tests must pass. The `ExecutionOutcome::new()` change must be compatible with existing test usage.

### 2. Manual verification with devnet

```bash
# Start a 4-validator devnet
just devnet-up

# Wait for chain to produce a few blocks, then submit a stale-nonce transaction
cast send --rpc-url http://localhost:8545 --private-key <key> --nonce 0 <to-address> --value 1wei

# (where the sender's on-chain nonce is already > 0)
# Verify blocks continue to be produced:
cast block-number --rpc-url http://localhost:8545
# Run again after a few seconds -- number should increase

# Check logs for the new skip message:
docker logs kora-0 2>&1 | grep "skipping"
```

### 3. Verify the executor no longer returns Err for bad transactions

After the fix, `execute()` should only return `Err` for true infrastructure failures (database errors). For transaction-level failures, it should return `Ok(outcome)` with the bad transactions listed in `outcome.skipped_txs`.

### 4. Verify mempool eviction

After a block is built that skips transactions, check that the mempool length decreases. The skipped transactions should not appear in subsequent `build_block()` calls.

---

## Load Test Evidence (2026-05-22)

A load test of 1,000 transactions (10 accounts, 50 concurrency) against a fresh 4-validator devnet confirmed this bug is actively causing chain degradation:

- **Results:** 865/1000 success (86.5%), 135 failures, 8.09 TPS over 107 seconds
- **Block builder stuck in failure loop:** The block builder repeatedly attempts to build blocks with ~195-202 pooled transactions, but aborts every attempt due to `NonceTooHigh`:
  ```
  WARN build_block: execution failed parent=0x... height=288 txs=195 error=TxExecution("Transaction(NonceTooHigh { tx: 24, state: 0 })")
  ```
- **Failure rate:** 1,373 `build_block` failures in a single minute on one validator
- **Root cause confirmed:** The block builder includes all pool txs in nonce-unaware order. When a tx with nonce 24 is encountered before nonces 0-23, `evm.replay()` returns `NonceTooHigh` and the `?` operator aborts the entire block. No transactions are finalized in that round.
- **Compounding with restart:** During the load test, the resolver panicked (`"resolver should not finish"`), causing node0 to restart. After restart, the pool retained ~195 stale txs with nonces ahead of the reset state (nonce 0), making the block builder permanently stuck in the NonceTooHigh loop.

This is the **single highest-impact bug** preventing load test throughput. The proposed fix (skip individual bad txs instead of aborting the block) would have allowed the remaining valid txs to be finalized.

---

## Related Issues

- **Mempool has no input validation**: The `InMemoryMempool` accepts any bytes without checking RLP validity, signature recovery, or nonce staleness. This is the root cause of bad transactions entering the pipeline. See issue #02 for replacing `InMemoryMempool` with the nonce-aware `TransactionPool`.
- **Mempool pruning gap**: Even after this fix, transactions that are skipped (not included in a block) have no pruning path unless the executor explicitly reports them. The `prune_mempool()` call in the finalization reporter (line 226 of `crates/node/reporters/src/lib.rs`) only removes transactions that were *included* in the finalized block, not transactions that were *rejected* during building.
- **Early return paths in finalization skip pruning**: Six error paths in `handle_finalized_update` (`crates/node/reporters/src/lib.rs`, lines 143-220) call `ack.acknowledge()` and return without calling `prune_mempool()`. This compounds the issue because even successfully finalized transactions may not be pruned if a local persistence error occurs.
- **Duplicate transaction storms**: When the loadgen broadcasts the same transaction to multiple validators, each independently accepts it. After one validator finalizes it, the others still hold stale copies. See the source material in `tmp/duplicate-tx-storm.md` for details on the 70% failure rate observed during load testing.
