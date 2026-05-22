# Mempool pruning relies on TxId matching -- transactions not in the local pool are silently skipped

## Summary

When blocks are finalized, the `FinalizedReporter` calls `state.prune_mempool(&block.txs)` to remove included transactions from the mempool. The pruning mechanism matches transactions by `TxId` (a content hash of the raw transaction bytes). While the pruning itself is correctly implemented, there are two gaps: (1) transactions submitted to other validators but not gossiped to this node will never be pruned because they were never in the local mempool, creating phantom entries if they are later re-submitted; and (2) the pruning only advances sender nonces for transactions it finds in the pool, so the sender's `next_nonce` may be stale after finalization if the finalized transaction was not in the local pool. This can cause the pool to accept transactions with nonces that have already been consumed on-chain.

## Priority

**P1 -- Operational Reliability**

In the current architecture where each validator has its own local mempool with no transaction gossip (see Issue 06), this is a latent correctness gap. It becomes actively exploitable once transaction gossip is implemented, and it already affects the accuracy of `eth_getTransactionCount("pending")` responses.

## Problem Description

### The finalization pruning path

**File**: `crates/node/reporters/src/lib.rs`, lines 112-161

```rust
async fn handle_finalized_update<E, P>(
    state: LedgerService,
    context: tokio::Context,
    executor: E,
    provider: P,
    block_index: Option<Arc<BlockIndex>>,
    mempool_broadcast: Option<MempoolEventSender>,
    gc_log: Option<Arc<SelfdestructGcLog>>,
    update: Update<Block>,
) where
    E: BlockExecutor<OverlayState<QmdbState>, Tx = Bytes>,
    P: BlockContextProvider,
{
    match update {
        Update::Tip(..) => {}
        Update::Block(block, ack) => {
            let result = finalize_block(/* ... */).await;
            // ... indexing, GC logging ...

            // Always prune the mempool regardless of whether finalization succeeded.
            state.prune_mempool(&block.txs).await;  // <-- LINE 154
            publish_mempool_inclusions(mempool_broadcast.as_ref(), &block);
            ack.acknowledge();
        }
    }
}
```

This correctly prunes on every finalized block, even when execution fails. The comment "Always prune the mempool regardless of whether finalization succeeded" shows this was a deliberate design choice.

### How prune_mempool delegates to the pool

**File**: `crates/node/ledger/src/lib.rs`, lines 452-457

```rust
pub async fn prune_mempool(&self, txs: &[Tx]) {
    let inner = self.inner.lock().await;
    let tx_ids: Vec<TxId> = txs.iter().map(Tx::id).collect();
    inner.mempool.prune(&tx_ids);
}
```

This converts the block's raw transactions into `TxId`s and calls `Mempool::prune()`.

### The TransactionPool::prune implementation

**File**: `crates/node/txpool/src/pool.rs`, lines 628-674

```rust
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();

    let mut confirmed_by_sender: HashMap<Address, u64> = HashMap::new();
    for id in tx_ids {
        let Some(hash) = inner.by_id.get(id) else {
            continue;   // <-- Gap 1: silently skips txs not in the local pool
        };
        if let Some(tx) = inner.by_hash.get(hash) {
            confirmed_by_sender
                .entry(tx.sender)
                .and_modify(|nonce| *nonce = (*nonce).max(tx.nonce))
                .or_insert(tx.nonce);
        }
    }

    // ... removes txs with nonce <= confirmed_nonce per sender ...
    // ... cleans up empty sender queues ...
    inner.update_counts();
}
```

### Gap 1: Transactions not in the local pool are silently skipped

When `inner.by_id.get(id)` returns `None`, the transaction is skipped with `continue`. This happens when:

- The transaction was submitted to a different validator (no gossip protocol exists today)
- The transaction was already pruned by a previous finalization
- The transaction was evicted from the pool due to capacity limits

In the first case, the sender's `next_nonce` is never advanced. If that sender later submits a transaction to this node with the already-consumed nonce, the pool may accept it (it would be rejected during execution, but it wastes mempool space and proposal bandwidth).

### Gap 2: Stale sender nonce after finalization

The `confirmed_by_sender` map is only populated from transactions found in the local pool. If a sender's transaction was finalized but was not in the local pool (submitted to another validator), the sender's `next_nonce` in `SenderQueue` is not updated. This means:

- `pool.has_nonce(&sender, consumed_nonce)` returns `false`
- A new transaction with the same (consumed) nonce would pass the duplicate check
- The transaction would be accepted into the pool and potentially proposed
- It would fail during EVM execution (nonce too low), wasting a block slot

### Current mitigation

The EVM executor already rejects transactions with consumed nonces during `execute()`. PR #122 (merged) ensures invalid transactions are skipped rather than aborting the entire block. So this gap does not cause consensus failures -- it causes wasted block space and unnecessary execution overhead.

Additionally, PR #134 (merged) added `has_nonce` checks at transaction ingress to reject same-nonce duplicates that are already in the pool. But this check only works if the original transaction is still in the pool; after pruning, the nonce slot is "forgotten."

## Fix

### Step 1: Decode sender and nonce from finalized transactions during pruning

When a `TxId` is not found in the local pool, decode the raw transaction to extract the sender address and nonce, then advance the sender's `next_nonce` accordingly:

```rust
fn prune(&self, tx_ids: &[TxId], raw_txs: &[Tx]) {
    let mut inner = self.inner.write();

    let mut confirmed_by_sender: HashMap<Address, u64> = HashMap::new();

    // First pass: match txs in the local pool
    for id in tx_ids {
        if let Some(hash) = inner.by_id.get(id) {
            if let Some(tx) = inner.by_hash.get(hash) {
                confirmed_by_sender
                    .entry(tx.sender)
                    .and_modify(|nonce| *nonce = (*nonce).max(tx.nonce))
                    .or_insert(tx.nonce);
            }
        }
    }

    // Second pass: decode txs not in the local pool to advance nonces
    for raw_tx in raw_txs {
        let id = raw_tx.id();
        if inner.by_id.contains_key(&id) {
            continue; // Already handled above
        }
        if let Some(ordered) = tx_to_ordered(raw_tx) {
            confirmed_by_sender
                .entry(ordered.sender)
                .and_modify(|nonce| *nonce = (*nonce).max(ordered.nonce))
                .or_insert(ordered.nonce);
        }
    }

    // ... rest of pruning logic unchanged ...
}
```

This requires changing the `Mempool::prune` trait signature to accept raw transactions in addition to (or instead of) TxIds. Alternatively, pass both the ids and the raw `Tx` objects.

### Step 2: Update the Mempool trait

**File**: `crates/node/txpool/src/traits.rs`

Either extend `prune()` to accept raw transactions:

```rust
fn prune(&self, txs: &[Tx]);  // Accept full Tx objects, derive TxId internally
```

Or add a separate method:

```rust
fn prune_with_nonce_advance(&self, tx_ids: &[TxId], raw_txs: &[Tx]);
```

### Step 3: Update LedgerView::prune_mempool

**File**: `crates/node/ledger/src/lib.rs`, lines 452-457

Pass the raw transactions through to the pool:

```rust
pub async fn prune_mempool(&self, txs: &[Tx]) {
    let inner = self.inner.lock().await;
    inner.mempool.prune(txs);  // Pass full Tx objects
}
```

## Effort Estimate

**2-4 hours**:
- 30 minutes to update the `Mempool::prune` trait signature
- 1 hour to implement nonce-advancing logic for unknown transactions in `TransactionPool::prune`
- 30 minutes to update `LedgerView::prune_mempool` and callers
- 1-2 hours for unit tests and e2e validation

## Affected Files

| File | Lines | Change |
|------|-------|--------|
| `crates/node/txpool/src/traits.rs` | 23 | Update `prune()` signature to accept `&[Tx]` |
| `crates/node/txpool/src/pool.rs` | 628-674 | Decode unknown transactions for nonce advancement |
| `crates/node/consensus/src/components/mempool.rs` | 61-66 | Update `InMemoryMempool::prune` signature |
| `crates/node/ledger/src/lib.rs` | 452-457 | Pass full `Tx` objects to `prune()` |
| `crates/node/reporters/src/lib.rs` | 154 | Already passes `&block.txs` -- no change needed |

## Testing Checklist

- [ ] Unit test: prune with a TxId not in the pool but with raw Tx provided -- verify sender nonce advances
- [ ] Unit test: after pruning an unknown tx, submitting a same-nonce tx should be rejected as `NonceTooLow`
- [ ] Unit test: prune with a mix of known and unknown transactions -- verify all sender nonces advance
- [ ] E2e test: submit tx to validator A, finalize, then submit same-nonce tx to validator B -- verify rejection
- [ ] Verify `InMemoryMempool` (the simpler consensus-crate mempool) also handles the updated signature
