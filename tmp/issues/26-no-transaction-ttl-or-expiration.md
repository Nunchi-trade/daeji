# No Transaction TTL or Expiration in Mempool

**Severity**: Medium
**Component**: `kora-txpool`, `kora-consensus`
**Labels**: `bug`, `mempool`, `resource-leak`

## Summary

Transactions accepted into the mempool remain there indefinitely. There is no time-to-live (TTL) mechanism, no periodic cleanup task, and no expiration check at any point in the transaction lifecycle. The only way a transaction is ever removed from the mempool is through finalization-triggered pruning (`prune()`), which itself depends on blocks being successfully produced and finalized. This creates a dangerous circular dependency when the chain stalls: stale transactions cannot be removed because pruning requires finalization, but stale transactions may themselves be the cause of failed block production.

## Current Behavior

### The `timestamp` field exists but is never used for expiration

The `OrderedTransaction` struct in `crates/node/txpool/src/ordering.rs` records a `timestamp` field (line 20):

```rust
pub struct OrderedTransaction {
    pub hash: B256,
    pub sender: Address,
    pub nonce: u64,
    pub effective_gas_price: u128,
    /// Timestamp when transaction was received.
    pub timestamp: u64,
    pub envelope: TxEnvelope,
}
```

This timestamp is populated with the current system time when a transaction enters the pool via the `tx_to_ordered()` function in `crates/node/txpool/src/pool.rs` (line 295):

```rust
fn tx_to_ordered(tx: &Tx) -> Option<OrderedTransaction> {
    // ...
    Some(OrderedTransaction::new(
        hash,
        sender,
        nonce,
        effective_gas_price,
        current_timestamp(),  // <-- recorded here
        envelope,
    ))
}
```

However, the timestamp is **only** used as a tie-breaker in the `Ord` implementation for gas-price ordering (line 64 of `ordering.rs`):

```rust
impl Ord for OrderedTransaction {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .effective_gas_price
            .cmp(&self.effective_gas_price)
            .then_with(|| self.timestamp.cmp(&other.timestamp))  // tie-break only
            .then_with(|| self.hash.cmp(&other.hash))
    }
}
```

No code path anywhere in the crate checks whether a transaction's timestamp has exceeded a TTL threshold. There is no `is_expired()` method, no `sweep_expired()` function, and no periodic cleanup task.

### `TransactionPool` (production pool) -- no expiration, no sweep

Looking at `crates/node/txpool/src/pool.rs`, the `TransactionPool` stores transactions in two data structures: `by_hash` (a `HashMap<B256, OrderedTransaction>`) and `by_sender` (a `HashMap<Address, SenderQueue>`). The `add()` method enforces per-sender limits and a soft global limit (via `warn!` logs at lines 126-139), but there is no hard cap and no time-based eviction:

```rust
if inner.pending_count > self.config.max_pending_txs {
    warn!(
        count = inner.pending_count,
        max = self.config.max_pending_txs,
        "pool exceeds pending limit"  // warning only -- no eviction
    );
}
```

The only removal paths are:
1. `remove()` -- removes a single transaction by hash (manual removal).
2. `remove_confirmed()` -- removes transactions below a confirmed nonce for a sender.
3. `prune()` -- removes transactions by ID after finalization.
4. `clear()` -- wipes the entire pool.

None of these are time-based. None run automatically.

### `InMemoryMempool` (simple pool) -- no timestamp at all

The simpler `InMemoryMempool` in `crates/node/consensus/src/components/mempool.rs` does not even track timestamps. It is a plain `BTreeMap<TxId, Tx>` behind an `Arc<RwLock<...>>`. Transactions inserted here live forever unless explicitly pruned by finalization:

```rust
pub struct InMemoryMempool {
    inner: Arc<RwLock<BTreeMap<TxId, Tx>>>,
}
```

### `PoolConfig` has no TTL field

The configuration struct in `crates/node/txpool/src/config.rs` contains limits for counts and sizes but no time-based configuration:

```rust
pub struct PoolConfig {
    pub max_pending_txs: usize,
    pub max_queued_txs: usize,
    pub max_txs_per_sender: usize,
    pub max_tx_size: usize,
    pub min_gas_price: u128,
    pub replacement_bump_percent: u8,
    // no tx_ttl field
}
```

### Pruning is only triggered by finalization

In `crates/node/ledger/src/lib.rs`, `prune_mempool()` is called with the transaction IDs of a finalized block (line 336):

```rust
pub async fn prune_mempool(&self, txs: &[Tx]) {
    let inner = self.inner.lock().await;
    let tx_ids: Vec<TxId> = txs.iter().map(Tx::id).collect();
    inner.mempool.prune(&tx_ids);
}
```

This means pruning requires successful block finalization. If the chain stalls (no new blocks finalize), no transactions are ever removed from the mempool, regardless of how old they are.

## Why This Matters

### 1. Stale transactions accumulate indefinitely

Transactions from previous test runs, from senders who have since submitted replacements off-chain, or from abandoned operations remain in the mempool forever. In a devnet environment where nodes are restarted frequently but mempool state may be resubmitted via bootstrap transactions, this creates a growing backlog of unexecutable garbage.

### 2. Circular dependency with chain stalls

The chain can stall for a variety of reasons (see interaction with other bugs below). When it does, the only mechanism to clean the mempool -- finalization-triggered pruning -- stops working. Stale transactions that might be causing the stall (e.g., transactions referencing outdated state) cannot be evicted. The operator's only recourse is a full node restart, which is operationally expensive and risks losing other in-flight state.

### 3. Memory usage grows linearly

Under sustained load, the mempool grows without bound. The `max_pending_txs` and `max_queued_txs` limits in `PoolConfig` are advisory only (they trigger `warn!` logs but do not reject or evict transactions). Over hours or days of operation, memory consumption from mempool data structures will grow continuously.

### 4. Block building wastes cycles on dead transactions

The `build()` method in `TransactionPool` iterates over all pending transactions from all senders to assemble a block. Without expiration, this includes transactions that may reference state from hours ago and will certainly fail execution. The block builder has no way to distinguish fresh transactions from ancient ones.

## Interaction with Other Issues

### Issue 01 -- Executor abort on transaction failure

When a stale transaction causes an execution error (e.g., insufficient balance that was true hours ago but not at the time of original submission), the executor abort bug can kill block production entirely. With TTL, these stale transactions would expire and be swept from the pool before they ever reach the executor. TTL acts as a natural safety valve against the executor abort bug.

### Issue 03 -- Pruning skipped on error paths

If pruning is skipped due to error-path bugs, transactions that should have been removed on finalization remain in the pool. Without TTL, they stay forever. With TTL, even if the pruning path fails, the transactions would eventually expire through the sweep mechanism.

### Issue 07 -- Snapshot memory growth

Both the snapshot cache and the mempool contribute to unbounded memory growth. Without TTL on mempool entries and without eviction on snapshot entries, two separate subsystems leak memory simultaneously, compounding the problem under load.

## Proposed Fix

### 1. Add `tx_ttl` to `PoolConfig`

In `crates/node/txpool/src/config.rs`, add a `tx_ttl` field representing the maximum age of a transaction in seconds before it is eligible for expiration. Provide a builder method `with_tx_ttl()` following the existing pattern.

Suggested defaults:
- **Production**: 6 hours (21600 seconds) -- long enough for transactions to survive temporary chain slowdowns, short enough to prevent indefinite accumulation.
- **Devnet**: 5 minutes (300 seconds) -- devnet test runs are short-lived; stale transactions from previous runs should be cleaned quickly.

```rust
pub struct PoolConfig {
    // ... existing fields ...
    /// Maximum time-to-live for a transaction in seconds.
    /// Transactions older than this are eligible for expiration.
    pub tx_ttl: u64,
}
```

### 2. Add `is_expired()` to `OrderedTransaction`

In `crates/node/txpool/src/ordering.rs`, add a method that checks whether a transaction has exceeded its TTL:

```rust
impl OrderedTransaction {
    pub fn is_expired(&self, ttl_secs: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_sub(self.timestamp) > ttl_secs
    }
}
```

### 3. Add `sweep_expired()` to `TransactionPool`

In `crates/node/txpool/src/pool.rs`, add a method that removes all expired transactions from both `by_hash` and `by_sender`:

```rust
impl TransactionPool {
    /// Remove all transactions whose age exceeds the configured TTL.
    /// Returns the number of transactions removed.
    pub fn sweep_expired(&self) -> usize {
        let ttl = self.config.tx_ttl;
        let mut inner = self.inner.write();
        let expired_hashes: Vec<B256> = inner.by_hash.values()
            .filter(|tx| tx.is_expired(ttl))
            .map(|tx| tx.hash)
            .collect();

        let count = expired_hashes.len();
        for hash in &expired_hashes {
            inner.by_hash.remove(hash);
        }

        for queue in inner.by_sender.values_mut() {
            queue.pending.retain(|tx| !tx.is_expired(ttl));
            queue.queued.retain(|tx| !tx.is_expired(ttl));
        }
        inner.by_sender.retain(|_, queue| !queue.is_empty());
        inner.update_counts();

        count
    }
}
```

### 4. Filter expired transactions during block building

In the `build()` method of both `TransactionPool` and `InMemoryMempool`, skip expired transactions so they are never included in a block proposal. This provides a second line of defense beyond the sweep task -- even if the sweep has not yet run, expired transactions are not selected for blocks.

### 5. Spawn a periodic sweep task

In `crates/node/runner/src/runner.rs`, spawn a background task that calls `sweep_expired()` on the transaction pool at a regular interval (e.g., every 30 seconds). This runs independently of block production and finalization, breaking the circular dependency:

```rust
// In the runner, after creating the mempool:
let pool_for_sweep = pool.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        let removed = pool_for_sweep.sweep_expired();
        if removed > 0 {
            info!(removed, "swept expired transactions from mempool");
        }
    }
});
```

### 6. Add timestamp tracking to `InMemoryMempool`

In `crates/node/consensus/src/components/mempool.rs`, either:
- Switch to storing `(Tx, u64)` tuples with insertion timestamps, or
- Wrap `Tx` in a struct that tracks insertion time.

This ensures the simpler mempool implementation also benefits from TTL-based expiration.

### 7. Emit metrics for expired transaction counts

Add a counter metric (e.g., `kora_mempool_expired_total`) that increments each time a transaction is swept due to expiration. This gives operators visibility into whether TTL is actively cleaning up stale transactions and whether the TTL value is appropriately tuned.

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/txpool/src/config.rs` | Add `tx_ttl` field to `PoolConfig`, add `with_tx_ttl()` builder, update `Default` and `new()` |
| `crates/node/txpool/src/ordering.rs` | Add `is_expired(ttl_secs: u64) -> bool` method to `OrderedTransaction` |
| `crates/node/txpool/src/pool.rs` | Add `sweep_expired() -> usize` method; filter expired txs in `build()` |
| `crates/node/runner/src/runner.rs` | Spawn periodic sweep task in the `run()` method of `ProductionRunner` |
| `crates/node/consensus/src/components/mempool.rs` | Add timestamp tracking to `InMemoryMempool`; implement expiration filtering |
| `crates/node/txpool/src/traits.rs` | (Optional) Add `sweep_expired()` to the `Mempool` trait if both implementations should share the interface |

## Testing Plan

### Unit Tests

1. **Expiration detection**: Create an `OrderedTransaction` with a timestamp 10 minutes in the past. Call `is_expired(300)` (5-minute TTL). Assert it returns `true`. Create another with a timestamp 1 minute ago. Assert `is_expired(300)` returns `false`.

2. **Sweep removes expired transactions**: Insert 5 transactions into a `TransactionPool` with timestamps spread across a range. Call `sweep_expired()` with a TTL that should expire 3 of them. Assert pool length is 2. Assert the remaining transactions are the non-expired ones.

3. **Sweep preserves non-expired transactions**: Insert transactions with recent timestamps. Call `sweep_expired()`. Assert no transactions were removed.

4. **Build skips expired transactions**: Insert a mix of expired and non-expired transactions. Call `build()`. Assert the result contains only non-expired transactions.

5. **Sender queue consistency after sweep**: Insert transactions for multiple senders, some expired. After sweep, verify `by_sender` queues are consistent with `by_hash` (no orphan entries in either direction). Verify empty sender queues are removed.

### Integration Tests

1. **TTL expiration under normal operation**: Start a node with a 5-second TTL. Submit a transaction. Wait 10 seconds without producing blocks. Query mempool length. Assert it is 0.

2. **Sustained load with TTL prevents unbounded growth**: Submit transactions at a steady rate with a short TTL. Verify that mempool size stabilizes at a bounded level rather than growing indefinitely.

3. **Chain stall recovery**: Simulate a chain stall (no finalization). Submit transactions. Verify they are swept after TTL expires. Resume block production. Verify the chain recovers without stale transactions interfering.
