# min_pending_price / min_queued_price Are O(N) Full Scans on Every Insertion

**Category**: performance -- txpool
**Severity**: high

## Summary

When the transaction pool is at capacity, every new insertion triggers an O(N) scan of all transactions to find the minimum gas price. The `min_pending_price()` and `min_queued_price()` functions iterate over every transaction in every sender queue to find the global minimum. With the default `max_pending_txs = 4096`, this scans up to 4,096 entries on every insertion attempt when the pool is full. At high transaction throughput (34 blocks/s), this becomes a significant CPU cost.

## Problem

The `reject_underpriced_when_full()` method is called on every `add()`. When the pool is at capacity, it calls `min_pending_price()` or `min_queued_price()` to determine if the new transaction's gas price is high enough to justify evicting an existing one. These functions perform a full scan of all transactions across all sender queues to find the minimum gas price.

This is called from the hot path: every transaction insertion via RPC or P2P gossip goes through `add()` -> `reject_underpriced_when_full()` -> `min_pending_price()`.

## Code Reference

File: `crates/node/txpool/src/pool.rs`, lines 347-361

```rust
fn min_pending_price(inner: &PoolInner) -> Option<u128> {
    inner
        .by_sender
        .values()
        .flat_map(|queue| queue.pending.iter().map(|tx| tx.effective_gas_price))
        .min()
}

fn min_queued_price(inner: &PoolInner) -> Option<u128> {
    inner
        .by_sender
        .values()
        .flat_map(|queue| queue.queued.iter().map(|tx| tx.effective_gas_price))
        .min()
}
```

The call site in `reject_underpriced_when_full()` (lines 312-345):

```rust
fn reject_underpriced_when_full(
    &self,
    inner: &PoolInner,
    tx: &OrderedTransaction,
    target: InsertionTarget,
) -> Result<(), TxPoolError> {
    match target {
        InsertionTarget::Pending => {
            if self.config.max_pending_txs == 0 {
                return Err(TxPoolError::PoolFull);
            }
            if inner.pending_count >= self.config.max_pending_txs
                && let Some(min_price) = Self::min_pending_price(inner) // <-- O(N) scan
                && tx.effective_gas_price <= min_price
            {
                return Err(TxPoolError::PoolFull);
            }
        }
        InsertionTarget::Queued => {
            if self.config.max_queued_txs == 0 {
                return Err(TxPoolError::PoolFull);
            }
            if inner.queued_count >= self.config.max_queued_txs
                && let Some(min_price) = Self::min_queued_price(inner) // <-- O(N) scan
                && tx.effective_gas_price <= min_price
            {
                return Err(TxPoolError::PoolFull);
            }
        }
        InsertionTarget::Replacement => {}
    }
    Ok(())
}
```

## Impact

1. **CPU waste**: O(N) scan on every insertion when pool is at capacity (N = 4,096 for pending, 1,024 for queued). With hundreds of insertions per second, this adds up.
2. **Latency spike**: Under high load with a full pool, every transaction submission incurs a full scan before it can be accepted or rejected, adding latency to the RPC `eth_sendRawTransaction` response.
3. **Scaling bottleneck**: If pool capacity is increased (e.g., to 8,192 or 16,384 for high-throughput networks), the cost of each insertion grows linearly. This makes the pool's performance inversely proportional to its configured capacity.
4. **Hold duration on write lock**: The scan happens while holding a write lock on `inner` (acquired in `add()` at line 198), blocking all concurrent pool reads and writes for the duration of the scan.

## Root Cause

The minimum price is recomputed from scratch on every insertion rather than being maintained incrementally as transactions are added and removed.

## Suggested Fix

Maintain a `BTreeMap<u128, usize>` (gas_price -> count) that is updated incrementally on insert/remove. This reduces the minimum price lookup from O(N) to O(1):

```rust
struct PoolInner {
    // ... existing fields ...
    pending_prices: BTreeMap<u128, usize>,  // gas_price -> count
    queued_prices: BTreeMap<u128, usize>,   // gas_price -> count
}

fn min_pending_price(inner: &PoolInner) -> Option<u128> {
    inner.pending_prices.keys().next().copied()  // O(1)
}

// On insert into pending:
*inner.pending_prices.entry(tx.effective_gas_price).or_insert(0) += 1;

// On remove from pending:
if let Some(count) = inner.pending_prices.get_mut(&tx.effective_gas_price) {
    *count -= 1;
    if *count == 0 {
        inner.pending_prices.remove(&tx.effective_gas_price);
    }
}
```

Alternatively, maintain a simple `min_price: Option<u128>` cache that is invalidated on removal and recomputed lazily. This is simpler but still requires an O(N) scan on cache invalidation.

## Files to Modify

- `crates/node/txpool/src/pool.rs` -- Replace `min_pending_price()` and `min_queued_price()` O(N) scans with an incrementally maintained index. Update `PoolInner` to include the price index, and update all insert/remove paths to maintain it.

## Related Issues

- None

## Labels

performance, txpool
