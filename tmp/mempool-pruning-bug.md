# Mempool Pruning Bug: Stale Transactions Persist After Finalization

## Context: What is Kora?

Kora is an EVM-compatible blockchain using Simplex BFT consensus (from the Commonware framework) with 4 validators. Leaders are elected via threshold VRF (BLS12-381). When a block is finalized, it has received 2/3+ threshold signatures and is irrevocable -- it IS the canonical chain, regardless of whether any individual node successfully persists it locally.

---

## The Bug

**Transactions with `nonce < on_chain_nonce` persist in the mempool forever after finalization, because pruning only removes transactions by exact hash match with the finalized block's contents.**

### Root Cause Location

**File**: `crates/node/reporters/src/lib.rs`, line 226

```rust
state.prune_mempool(&block.txs).await;
```

**File**: `crates/node/ledger/src/lib.rs`, lines 336-340

```rust
pub async fn prune_mempool(&self, txs: &[Tx]) {
    let inner = self.inner.lock().await;
    let tx_ids: Vec<TxId> = txs.iter().map(Tx::id).collect();
    inner.mempool.prune(&tx_ids);
}
```

The pruning mechanism works by computing the `TxId` (keccak256 hash of the raw transaction bytes) for each transaction in the finalized block, then removing those exact hashes from the mempool. This means:

- Only the specific transaction instances that were included in the block get removed.
- Other transactions for the same sender that have now-stale nonces are **never removed**.
- Transactions submitted to a different validator than the one that included them are never removed (different hash for same nonce is not possible, but the same sender's lower-nonce txs on other validators remain).

---

## Where Pruning Happens (The Finalization Callback)

**File**: `crates/node/reporters/src/lib.rs`, function `handle_finalized_update` (lines 105-232)

The full finalization pipeline:

```
FinalizedReporter::report(Update::Block(block, ack))
    |
    |-- 1. Check if snapshot exists for this digest        (line 120)
    |
    |-- 2. If missing: re-execute block                    (lines 124-189)
    |       |-- Get parent snapshot                        (line 131)
    |       |-- Execute transactions                       (lines 133-147)
    |       |-- Compute state root                         (lines 149-159)
    |       |-- Verify state root matches block            (lines 160-169)
    |       |-- Insert snapshot if not cached              (lines 171-186)
    |
    |-- 3. Persist snapshot to QMDB                        (lines 204-220)
    |       |-- Spawn persist task                         (lines 205-207)
    |       |-- Await result                              (lines 208-220)
    |       |-- ERROR PATHS: early return WITHOUT pruning  (lines 211, 216)
    |
    |-- 4. Index block for RPC (optional)                  (lines 221-225)
    |
    |-- 5. PRUNE MEMPOOL                                   (line 226)
    |       state.prune_mempool(&block.txs).await;
    |
    |-- 6. Acknowledge to consensus                        (line 229)
            ack.acknowledge();
```

### Critical Observation: What `prune` Actually Does

For `TransactionPool` (production implementation), `prune` is defined at `crates/node/txpool/src/pool.rs`, lines 358-401:

```rust
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();

    // Step 1: Find the max confirmed nonce per sender from the given tx_ids
    let mut confirmed_by_sender: HashMap<Address, u64> = HashMap::new();
    for id in tx_ids {
        if let Some(tx) = inner.by_hash.get(&id.0) {
            confirmed_by_sender
                .entry(tx.sender)
                .and_modify(|nonce| *nonce = (*nonce).max(tx.nonce))
                .or_insert(tx.nonce);
        }
    }

    // Step 2: Remove all txs with nonce <= confirmed_nonce for each sender
    for (sender, confirmed_nonce) in confirmed_by_sender {
        if let Some(queue) = inner.by_sender.get_mut(&sender) {
            // Removes from pending AND queued where nonce <= confirmed_nonce
            queue.remove_confirmed(confirmed_nonce);
            ...
        }
    }
    ...
}
```

This is actually more sophisticated than a pure hash-match -- it removes all txs with nonce <= the highest confirmed nonce for that sender. **However**, it only processes senders whose transactions are found in `by_hash` by the given `tx_ids`. If the finalized block contains a transaction that is NOT in this validator's pool (because it was submitted to a different validator), the sender is never identified and no pruning occurs for that sender.

For `InMemoryMempool` (simple implementation), `prune` is purely hash-based removal (lines 61-66):

```rust
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();
    for id in tx_ids {
        inner.remove(id);
    }
}
```

This is the fundamental problem: **pruning requires the exact transaction hash to be present in the local pool**.

---

## The Failure Scenario

### Preconditions

- 4 validators: A, B, C, D
- Sender S has on-chain nonce = 100
- User submits tx(nonce=100) to validator A
- User also submits tx(nonce=100) to validator B (same or different tx hash -- does not matter)
- User submits tx(nonce=101) to validator B

### Timeline

```
T=0: State: sender S nonce = 100
     Validator A mempool: [tx_A(nonce=100)]
     Validator B mempool: [tx_B(nonce=100), tx_B(nonce=101)]

T=1: Validator A is leader, proposes block including tx_A(nonce=100)
     Block finalized by consensus (2/3+ threshold signatures)

T=2: Finalization callback fires on all validators

     On Validator A:
       - prune_mempool([tx_A(nonce=100)])
       - tx_A found in pool -> sender S confirmed_nonce = 100
       - All S's txs with nonce <= 100 removed
       - Pool: [] (clean)

     On Validator B:
       - prune_mempool([tx_A(nonce=100)])
       - tx_A.id() looked up in by_hash -> NOT FOUND (B has tx_B, not tx_A)
       - confirmed_by_sender is EMPTY
       - NO pruning occurs for sender S
       - Pool: [tx_B(nonce=100), tx_B(nonce=101)]  (STALE!)

T=3: On-chain nonce for S is now 101 (tx_A consumed nonce 100)
     Validator B's pool still has tx_B(nonce=100) which is PERMANENTLY STALE

T=4: Validator B becomes leader
     - mempool.build() returns txs sorted by (gas_price, nonce)
     - For sender S: expected_nonce starts at next_nonce (still 100 in B's queue)
     - tx_B(nonce=100) is selected first (it's the next expected nonce)
     - Block proposal includes tx_B(nonce=100)

T=5: executor.execute() processes tx_B(nonce=100)
     - EVM checks: account nonce is 101, tx nonce is 100
     - NonceTooLow error
     - Executor returns Err(...)
     - build_block() returns None  [crates/node/runner/src/app.rs:127-136]

T=6: No block proposed this view -> VIEW NULLIFIED
     Stale tx remains in pool (pruning only happens on finalization)

T=7: Next time B is leader -> SAME FAILURE (T=4 repeats)
```

---

## Why This Causes Chain Death

The failure cascade leads to permanent stall under specific conditions:

### Single-Validator Stall

If only validator B has stale transactions, the chain does NOT die -- validators A, C, D can still propose blocks when they are leaders. B's rounds are simply nullified. This degrades throughput by ~25% (1 of 4 validators is non-functional).

### Multi-Validator Stall (Chain Death)

If 2+ validators accumulate stale transactions for the same sender:

```
All validators have stale tx(nonce=N) where on-chain nonce > N
    |
    v
Every leader's block proposal fails at execution
    |
    v
Every view is nullified
    |
    v
No new blocks finalized
    |
    v
No finalization -> No pruning -> Stale txs never removed
    |
    v
PERMANENT CHAIN HALT
```

This is a **liveness failure**. Consensus can still elect leaders and attempt views, but no block is ever successfully proposed because every proposal includes a stale transaction that causes executor failure.

### Interaction with the Executor Abort Bug

The executor in Kora uses a fail-fast approach. In `crates/node/runner/src/app.rs`, line 125-136:

```rust
let outcome = match self.executor.execute(&parent_snapshot.state, &context, &txs_bytes) {
    Ok(outcome) => outcome,
    Err(err) => {
        warn!(parent = ?parent_digest, height, txs = txs.len(), error = ?err,
              "build_block: execution failed");
        return None;  // ENTIRE BLOCK PROPOSAL ABANDONED
    }
};
```

A single invalid transaction (stale nonce) causes the **entire** block execution to fail, not just that one transaction. This is because:

1. The executor processes all transactions as a batch.
2. If any transaction encounters a fatal error (as opposed to a revert), the executor returns `Err`.
3. `build_block` interprets any `Err` as total failure and returns `None`.
4. Returning `None` means no block is proposed for this view.

If the executor instead skipped invalid transactions (treating `NonceTooLow` as a no-op rather than a fatal error), the stale transaction would waste a slot but not kill the block. However, the current design does not distinguish between "transaction that is individually invalid" and "execution environment failure."

---

## Early Return Paths That Skip Pruning

The `handle_finalized_update` function has **six** early-return paths that skip `prune_mempool`:

| Line | Condition | Consequence |
|------|-----------|-------------|
| 143-146 | Block execution failure | Finalized txs stay in pool |
| 153-158 | Root computation failure | Finalized txs stay in pool |
| 166-169 | State root mismatch | Finalized txs stay in pool |
| 197-199 | Missing parent snapshot (no cached snapshot) | Finalized txs stay in pool |
| 210-214 | Persist task panicked/cancelled (JoinError) | Finalized txs stay in pool |
| 216-220 | Persist data write failure | Finalized txs stay in pool |

In all cases, `ack.acknowledge()` is called (allowing consensus to advance), but `prune_mempool` is skipped. The block IS finalized (irrevocable), so its transactions have consumed nonces in the canonical state. Yet they remain in the local mempool.

**Line 216-220 is the most dangerous** because disk I/O errors are not uncommon (full disk, hardware failure, filesystem corruption). The persist failure is transient/local, but the mempool poisoning is permanent.

---

## Metrics That Indicate This Bug is Active

Observable symptoms when this bug is occurring:

1. **Increasing nullification rate**: The `NodeStateReporter` (file: `crates/node/reporters/src/lib.rs`, lines 453-475) tracks nullifications via `self.state.inc_nullified()`. A sustained increase in nullifications with no corresponding increase in finalizations indicates block proposal failures.

2. **`build_block: execution failed` log messages**: Repeated warnings at `crates/node/runner/src/app.rs:128-134` with `NonceTooLow` errors point directly to stale transactions.

3. **Mempool size not decreasing after finalization**: If `mempool.len()` remains constant or grows while blocks are being finalized, transactions are not being pruned.

4. **`build_block: mempool has unincluded txs but produced empty block`**: The diagnostic log at `crates/node/runner/src/app.rs:102-108` fires when the mempool has transactions but none can be built into a valid block.

5. **Specific sender's transactions persisting**: If monitoring shows the same sender's transactions appearing in failed block proposals across multiple rounds, those are the stale transactions.

---

## Proposed Fix

### Fix 1: Nonce-Based Pruning After Finalization (Recommended)

After each finalization, query the finalized state for the nonce of every sender that has transactions in the mempool, and remove all transactions with `nonce < finalized_nonce`.

**Implementation sketch** (in `crates/node/reporters/src/lib.rs`):

```rust
// After line 226, or replacing it:
async fn prune_stale_transactions(state: &LedgerService, block: &Block) {
    // 1. Prune transactions that were in the finalized block (existing behavior)
    state.prune_mempool(&block.txs).await;

    // 2. For each sender in the mempool, check their finalized nonce
    //    and remove txs below it
    let senders = state.mempool_senders().await;
    for sender in senders {
        let finalized_nonce = state.finalized_nonce(&sender).await;
        state.prune_sender_below_nonce(&sender, finalized_nonce).await;
    }
}
```

The `TransactionPool` already has `remove_confirmed(sender, confirmed_nonce)` (line 188) which does exactly this -- it removes all txs with `nonce <= confirmed_nonce` and promotes queued txs. The missing piece is calling it with the **finalized state nonce** rather than relying on hash-matched block contents.

### Fix 2: Always Prune on All Error Paths (Defense in Depth)

Move pruning before persistence, or ensure it happens on every path:

```rust
// In handle_finalized_update, restructure to:
Update::Block(block, ack) => {
    // ALWAYS prune first -- block is consensus-final regardless of local state
    state.prune_mempool(&block.txs).await;

    // Then attempt persistence (may fail, but mempool is already clean)
    let snapshot_exists = state.query_state_root(digest).await.is_some();
    // ... rest of logic ...

    ack.acknowledge();
}
```

This ensures that even if persistence fails, the finalized transactions are removed from the mempool. The block is irrevocable at this point.

### Fix 3: Executor Should Skip Invalid Transactions (Separate Bug)

The executor should not abort the entire block when a single transaction has a stale nonce. Instead:

```rust
// In the executor, when processing a batch:
for tx in transactions {
    match execute_single(tx, state) {
        Ok(receipt) => receipts.push(receipt),
        Err(ExecutionError::NonceTooLow { .. }) => {
            // Skip this tx, it's stale -- do not abort the block
            warn!("skipping stale transaction");
            continue;
        }
        Err(fatal) => return Err(fatal),  // Only abort on truly fatal errors
    }
}
```

This is a defense-in-depth measure. Even if stale transactions enter the mempool, they should not kill block proposals.

---

## Interaction Between the Two Bugs

The pruning bug and the executor abort bug form a **deadly combination**:

```
Pruning Bug: stale txs persist in mempool
     +
Executor Bug: single stale tx kills entire block
     =
Any stale tx -> permanent block proposal failure for that validator
```

Fixing either one independently provides partial relief:
- Fix pruning only: stale txs are removed promptly, executor abort never triggered by stale nonces.
- Fix executor only: stale txs persist but are skipped during execution, blocks still succeed.

Fixing both provides complete protection.

---

## Code References

| Location | Line(s) | Relevance |
|----------|---------|-----------|
| `crates/node/reporters/src/lib.rs` | 226 | The single `prune_mempool` call |
| `crates/node/reporters/src/lib.rs` | 143, 155, 166, 197, 211, 216 | Early returns that skip pruning |
| `crates/node/ledger/src/lib.rs` | 336-340 | `prune_mempool` implementation (converts to TxId list) |
| `crates/node/ledger/src/lib.rs` | 477-479 | `LedgerService::prune_mempool` delegation |
| `crates/node/txpool/src/pool.rs` | 358-401 | `TransactionPool::prune` (nonce-based per sender, but only for matched hashes) |
| `crates/node/txpool/src/pool.rs` | 188-216 | `remove_confirmed` (the method that SHOULD be called with finalized nonces) |
| `crates/node/txpool/src/ordering.rs` | 130-137 | `SenderQueue::remove_confirmed` (removes nonces <= threshold, promotes queued) |
| `crates/node/consensus/src/components/mempool.rs` | 61-66 | `InMemoryMempool::prune` (pure hash removal, no nonce awareness) |
| `crates/node/runner/src/app.rs` | 125-136 | Block execution failure handling (returns None on any Err) |
| `crates/node/consensus/src/execution.rs` | 33-35 | `BlockExecution::execute` (maps executor Err to ConsensusError) |
