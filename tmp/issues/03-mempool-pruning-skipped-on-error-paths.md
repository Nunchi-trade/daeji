# Mempool: Finalization error paths skip pruning, causing stale transactions to persist

**Severity:** High
**Component:** `crates/node/reporters/src/lib.rs` -- `FinalizedReporter` / `handle_finalized_update`
**Affects:** All validator nodes
**Related issues:** #04 (Resolver permanent peer blocking after restart -- compounds with this bug to make recovery impossible)

---

## Summary

The `FinalizedReporter`'s `handle_finalized_update` function has 6 early-return paths that skip the `prune_mempool()` call, even though the block being processed IS consensus-finalized (irrevocable). When any of these error paths are taken, the finalized transactions remain in the mempool indefinitely. Those stale transactions will then be re-proposed in future blocks, where they fail execution (nonces already consumed), causing the executor to abort and every subsequent proposal to fail. This leads to a permanent chain stall.

This bug interacts with the executor's fatal abort behavior (see "Why This Is Dangerous" below) and the resolver catch-up bug (#04) to create a situation where the chain cannot self-recover and individual node restarts cannot help.

---

## Background

### What is Kora?

Kora is an EVM-compatible blockchain built on top of the [Commonware](https://github.com/commonwarexyz/monorepo) consensus framework. It uses the Simplex BFT consensus protocol with BLS12-381 threshold signatures for finality. The node implementation lives in the `crates/node/` directory, with the consensus runner at `crates/node/runner/`, the EVM executor at `crates/node/executor/`, and the ledger/state management at `crates/node/ledger/`.

### Kora's finalization pipeline

When the Simplex consensus engine finalizes a block (2/3+ validator signatures), the block is delivered to the `FinalizedReporter` via the Commonware `Reporter` trait. The reporter is responsible for:

1. **Re-executing** the block if no cached snapshot exists (e.g., the node was not the proposer, or after a restart)
2. **Persisting** the resulting state to QMDB (Kora's Merkle database layer)
3. **Indexing** the block for RPC queries (if RPC is enabled)
4. **Pruning** finalized transactions from the mempool
5. **Acknowledging** the block to the Marshal, which advances the delivery floor

The `FinalizedReporter` is wired into the consensus engine at `crates/node/runner/src/runner.rs` lines 484-488:

```rust
let mut finalized_reporter =
    FinalizedReporter::new(ledger.clone(), context.clone(), executor, context_provider);
if let Some(block_index) = block_index {
    finalized_reporter = finalized_reporter.with_block_index(block_index);
}
```

### How mempool pruning works

The mempool (`InMemoryMempool` in `crates/node/consensus/src/components/mempool.rs`) is a simple `BTreeMap<TxId, Tx>` protected by an `RwLock`. It has no TTL, no size limit, and no periodic cleanup. Transactions are only removed via `prune()`, which is called exclusively from `handle_finalized_update` at line 226 of `crates/node/reporters/src/lib.rs`:

```rust
state.prune_mempool(&block.txs).await;
```

The `prune_mempool` method (defined in `crates/node/ledger/src/lib.rs` line 477) delegates to `LedgerView::prune_mempool` (line 336), which computes `TxId`s from the block's transactions and calls `inner.mempool.prune(&tx_ids)` to remove them from the `BTreeMap`.

---

## The Bug

In `crates/node/reporters/src/lib.rs`, the `handle_finalized_update` function (lines 105-232) places the `prune_mempool()` call at line 226, AFTER all persistence and indexing work. The pruning is only reached if every preceding step succeeds. Here is the full flow:

### Step-by-step flow of `handle_finalized_update`

```
Line 120: Check if snapshot exists for this block's digest
Line 124: If snapshot missing OR RPC indexing needed:
  Line 131:   Get parent snapshot
  Line 133-146:   Execute block against parent state
  Line 149-158:   Compute QMDB state root
  Line 160-169:   Verify state root matches block header
  Line 171-186:   Insert snapshot into cache (if didn't already exist)
Line 204-207: Spawn persistence task (write snapshot to QMDB on disk)
Line 208-214: Await persistence task handle (JoinError check)
Line 216-220: Check persistence result (data write check)
Line 221-225: Index block for RPC (if enabled)
Line 226: *** PRUNE MEMPOOL ***  <--- only reached if ALL above succeeds
Line 229: Acknowledge to consensus
```

### All 6 early-return paths that skip pruning

Each of these paths calls `ack.acknowledge()` and returns, but NEVER calls `prune_mempool()`:

| # | Lines | Trigger | Error message |
|---|-------|---------|---------------|
| 1 | 142-146 | `BlockExecution::execute()` returns `Err` | `"failed to execute finalized block"` |
| 2 | 154-158 | `compute_root_from_store()` returns `Err` | `"failed to compute qmdb root"` |
| 3 | 160-169 | Computed state root != block's state root | `"state root mismatch for finalized block"` |
| 4 | 197-199 | No parent snapshot and no existing snapshot | `"missing parent snapshot for finalized block"` |
| 5 | 210-214 | Persistence task `JoinError` (panic/cancel) | `"persist task failed"` |
| 6 | 216-219 | Persistence data write failure | `"failed to persist finalized block"` |

The relevant code at the end of the function:

```rust
// Line 204-229 (crates/node/reporters/src/lib.rs)
let persist_state = state.clone();
let persist_handle = context
    .shared(true)
    .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
let persist_result = match persist_handle.await {
    Ok(result) => result,
    Err(err) => {
        error!(?digest, error = ?err, "persist task failed");
        ack.acknowledge();
        return;                        // <-- early return #5, pruning skipped
    }
};
if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    ack.acknowledge();
    return;                            // <-- early return #6, pruning skipped
}
if let (Some(index), Some(outcome), Some(block_context)) =
    (block_index.as_ref(), execution_outcome.as_ref(), execution_context.as_ref())
{
    index_finalized_block(index, &block, block_context, outcome);
}
state.prune_mempool(&block.txs).await;  // <-- only reached on full success
// Marshal waits for the application to acknowledge processing before advancing the
// delivery floor. Without this, the node can stall on finalized block delivery.
ack.acknowledge();
```

---

## Why This Is Dangerous

### The block IS finalized regardless of local errors

A finalized block in Simplex BFT has received 2/3+ threshold signatures. It is **irrevocable**. The block's transactions WILL be included in the chain state on every correct node. The nonces in those transactions are consumed network-wide. The transactions cannot be executed again -- they will always fail with a nonce error.

But when a local error prevents pruning, those same transactions remain in this node's mempool. The node does not know they have already been included.

### The deadly interaction: stale tx + executor abort = permanent stall

Here is how unpruned transactions cause a permanent chain stall:

1. Block N is finalized containing transaction T (nonce=5 from sender S)
2. Local persistence fails (e.g., disk full, QMDB write error)
3. `prune_mempool()` is skipped -- T remains in the mempool
4. Node becomes leader for block N+1
5. `build_block()` in `crates/node/runner/src/app.rs` (line 84) pulls T from the mempool
6. T is included in the new block proposal
7. Executor runs T against state where sender S already has nonce=6
8. T fails with a nonce error -- the executor returns `Err`
9. `build_block()` returns `None` (line 135)
10. Proposal fails, view is nullified
11. Other leaders hit the same issue if they have the same stale transaction
12. **Every proposal fails. Chain is permanently stalled.**

The `InMemoryMempool` has no expiry mechanism (`crates/node/consensus/src/components/mempool.rs` lines 13-71). It is a bare `BTreeMap<TxId, Tx>`. Without `prune()` being called, transactions live forever. And without block finalization, `prune()` is never called. This creates a circular dependency: no pruning -> stale txs -> no finalization -> no pruning.

---

## Proposed Fix

### Primary fix: Move pruning before persistence

The block is consensus-final regardless of whether this node successfully persists it to disk. Pruning should happen unconditionally for any finalized block, not conditionally upon successful persistence.

Restructure `handle_finalized_update` so that `prune_mempool` is called on every code path that processes a finalized block:

```rust
async fn handle_finalized_update<E, P>(
    state: LedgerService,
    context: tokio::Context,
    executor: E,
    provider: P,
    block_index: Option<Arc<BlockIndex>>,
    update: Update<Block>,
) where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    match update {
        Update::Tip(..) => {}
        Update::Block(block, ack) => {
            let digest = block.commitment();

            // ALWAYS prune finalized transactions from the mempool,
            // regardless of whether local persistence succeeds.
            // The block is consensus-final (irrevocable).
            state.prune_mempool(&block.txs).await;

            let snapshot_exists = state.query_state_root(digest).await.is_some();
            // ... rest of the function unchanged ...
        }
    }
}
```

Alternatively, if there is a concern about pruning before the snapshot is available (since subsequent proposals may need the parent snapshot), use a `defer`-style pattern to ensure pruning happens on every exit:

```rust
Update::Block(block, ack) => {
    // Use a scope guard or explicit calls on every path
    let result = process_finalized_block(&state, &context, &executor, &provider, &block_index, &block).await;

    // ALWAYS prune, regardless of process_finalized_block result
    state.prune_mempool(&block.txs).await;

    if let Err(err) = result {
        error!(?digest, error = ?err, "finalized block processing failed (pruning still applied)");
    }

    // Index and acknowledge...
    ack.acknowledge();
}
```

### Secondary fix: Nonce-based pruning

Add a fallback pruning mechanism that queries the finalized state for each sender's current nonce and removes all mempool transactions with nonces at or below the finalized nonce. This provides defense-in-depth even if the primary pruning is missed:

```rust
/// Remove all mempool transactions whose nonces have been consumed
/// according to the finalized QMDB state.
pub async fn prune_stale_by_nonce(&self) {
    let inner = self.inner.lock().await;
    let mempool_txs = inner.mempool.all_transactions(); // new method needed
    let mut stale_ids = Vec::new();

    for (id, tx) in &mempool_txs {
        if let Some((sender, nonce)) = decode_sender_nonce(&tx.bytes) {
            let finalized_nonce = inner.qmdb.state().nonce(&sender).await.unwrap_or(0);
            if nonce <= finalized_nonce {
                stale_ids.push(*id);
            }
        }
    }

    inner.mempool.prune(&stale_ids);
}
```

This could be called periodically (e.g., every 10 finalized blocks) or specifically after error paths.

---

## Testing Plan

### Unit tests

1. **Test early return paths still prune**: Create a mock `LedgerService` where `persist_snapshot` returns an error. Verify that after `handle_finalized_update` completes, the finalized transactions are no longer in the mempool.

2. **Test for each of the 6 error paths**: Inject failures at each of the 6 early-return points and verify pruning still occurs.

3. **Test nonce-based pruning**: Insert transactions with nonces 1-5 into the mempool. Set finalized state nonce to 3. Call `prune_stale_by_nonce()`. Verify transactions with nonces 1-3 are removed and 4-5 remain.

### Integration tests (e2e)

4. **Simulate persistence failure under load**: In the e2e test harness (`crates/e2e/src/harness.rs`), inject a QMDB write failure for a single finalized block. Submit new transactions. Verify the chain continues to finalize blocks (no stall from stale mempool entries).

5. **Stall recovery test**: Pre-populate the mempool with transactions that have already-consumed nonces. Start consensus. Verify that the node does NOT stall on proposals and the stale transactions are eventually pruned.

### Observability

6. Add a metric `kora_mempool_stale_pruned_total` that tracks how many transactions were pruned by the nonce-based fallback. A non-zero value indicates the primary pruning path was missed.

7. Add a log at `warn` level when the nonce-based pruning removes transactions, as it indicates the primary path failed:
   ```
   warn!(count = stale_ids.len(), "nonce-based fallback pruned stale mempool transactions");
   ```

---

## Reproduction Steps

### Using the Docker devnet

1. Start a clean 4-validator devnet:
   ```bash
   cd docker
   just reset
   just trusted-devnet
   ```

2. Wait for the chain to be healthy (check `just stats` shows Blocks/s > 50).

3. Run the high-concurrency load test to trigger mempool poisoning:
   ```bash
   cargo run --release -p loadgen -- \
     --total-txs 50000 \
     --accounts 50 \
     --concurrency 200 \
     --rpc-url http://127.0.0.1:8545 \
     --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
   ```

4. Observe the stall. After the loadgen completes, query node status:
   ```bash
   curl -s http://localhost:8545 -X POST \
     -H "Content-Type: application/json" \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}' | jq '.result | {currentView, finalizedCount, nullifiedCount}'
   ```
   Expected: `finalizedCount` is frozen while `currentView` and `nullifiedCount` keep rising.

5. Wait 60 seconds and query again. If `finalizedCount` has not changed, the stall is confirmed permanent.

6. Check validator logs for the repeating error pattern:
   ```
   [app] WARN build_block: execution failed parent=0x... height=... txs=... error=TxExecution("...")
   ```

### Why this triggers the bug

The loadgen broadcasts every transaction to all 4 validators (`--broadcast-rpc-urls`), so all mempools contain the same transactions. Under high concurrency, some transactions become stale (their nonces are consumed by a finalized block before they are included). Since `InMemoryMempool` performs no nonce validation, these stale transactions persist. When any validator becomes leader, its block proposal includes the stale transaction, the executor aborts the entire block (the `?` operator on line 395 of `crates/node/executor/src/revm.rs` propagates `NonceTooLow` as a fatal error), and the view is nullified. Since pruning only runs after successful finalization, and no finalization can occur, the stale transactions are never removed. This creates a permanent circular dependency.

### Recovery after reproduction

The only currently working recovery is a full cluster reset:
```bash
cd docker
just reset
just trusted-devnet
```

---

## Additional Context: Executor Fatal Abort Interaction

The pruning bug alone would not necessarily cause a permanent stall if the executor handled stale transactions gracefully. The full chain-killing interaction involves two bugs:

1. **This bug (pruning skip)**: Stale transactions persist in the mempool after finalization error paths.
2. **Executor fatal abort**: In `crates/node/executor/src/revm.rs` (line 395), the `?` operator propagates any single transaction failure (including `NonceTooLow`) as a fatal error for the entire block execution. This means a single stale transaction kills the entire block proposal.

The executor code:
```rust
// crates/node/executor/src/revm.rs:395
let result_and_state = evm.replay()
    .map_err(|e| ExecutionError::TxExecution(format!("{:?}", e)))?;
```

Fixing either bug independently provides partial relief:
- Fix pruning only: stale txs are removed promptly; the executor abort is never triggered by stale nonces.
- Fix executor only: stale txs persist but are skipped during execution; blocks still succeed.

Fixing both provides complete protection.

## Additional Context: `TransactionPool` vs `InMemoryMempool`

The codebase contains a more sophisticated `TransactionPool` implementation at `crates/node/txpool/src/pool.rs` that has nonce-aware pruning. Its `prune()` method (lines 358-401) finds the max confirmed nonce per sender from the finalized block's transactions and removes all mempool entries with `nonce <= confirmed_nonce` for that sender. However, it still relies on the finalized block's transaction hashes being present in the local pool's `by_hash` index to identify the sender. If the transaction was submitted to a different validator, the sender is never identified and no pruning occurs for that sender.

The `InMemoryMempool` (currently wired into the ledger) uses pure hash-based removal, which is even more limited. Wiring `TransactionPool` into the ledger would improve the situation but would not fully fix the problem -- the fundamental issue is that pruning only runs on the success path of `handle_finalized_update`.

---

## Related Files

| File | Role |
|------|------|
| `crates/node/reporters/src/lib.rs` | `handle_finalized_update` -- the buggy function (lines 105-232) |
| `crates/node/ledger/src/lib.rs` | `LedgerView::prune_mempool` (line 336) and `LedgerService::prune_mempool` (line 477) |
| `crates/node/consensus/src/components/mempool.rs` | `InMemoryMempool` -- the mempool implementation with no TTL/expiry (lines 1-71) |
| `crates/node/runner/src/app.rs` | `build_block` (line 84) -- pulls from mempool for proposals; returns `None` on execution failure at line 135 |
| `crates/node/runner/src/runner.rs` | `FinalizedReporter` wiring (lines 484-488) |
| `crates/node/consensus/src/traits.rs` | `Mempool` trait definition including `prune()` (line 62) |
| `crates/node/executor/src/revm.rs` | Executor fatal abort -- `?` operator at line 395 that kills entire block on any single tx failure |
| `crates/node/txpool/src/pool.rs` | `TransactionPool` -- more sophisticated pool with nonce-aware pruning (exists but not wired into ledger) |
| `docker/config/alerts.yml` | `MempoolPoisoning` alert (line 215) -- existing alert for this failure mode |
| `crates/e2e/src/harness.rs` | E2E test harness for integration tests |

---

## Verification Steps

After implementing the fix, verify correctness with these checks:

1. **Unit test**: Mock `persist_snapshot` to return `Err`. Call `handle_finalized_update` with a block containing known transactions. Assert those transactions are no longer in the mempool after the function returns.

2. **All 6 error paths**: For each of the 6 early-return paths, inject a failure at that point and verify `prune_mempool` was still called.

3. **Devnet stress test**: Run the reproduction steps above (50k tx loadgen with broadcast). After the loadgen completes, the chain should continue finalizing blocks (the `finalizedCount` should still be advancing). If it stalls, the fix is incomplete.

4. **Metric check**: After the stress test, query `kora_nodeStatus` on all 4 validators. The `nullifiedCount` should be low relative to `currentView` (baseline is ~26% idle nullification, not 100%).

5. **Code review**: Verify that every code path in `handle_finalized_update` that calls `ack.acknowledge()` also calls `state.prune_mempool(&block.txs).await` (or that pruning happens unconditionally before any error path).
