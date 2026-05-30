# 042: Pending Pool Eviction Cascades to Demote Dependent High-Fee Transactions

**Category:** bug
**Severity:** medium

**Labels:** `bug`, `txpool`, `correctness`, `performance`

## Summary

When the pending pool overflows its capacity limit (default 4096 transactions), the pool evicts the single transaction with the lowest effective gas price. However, evicting a mid-sequence transaction creates a nonce gap in the sender's pending queue, causing all subsequent pending transactions from that sender to be demoted to the queued (non-executable) pool. This means evicting one cheap transaction can render hundreds of expensive transactions un-buildable.

## Problem

The `TransactionPool` eviction strategy evaluates each transaction individually rather than considering the cascading effect on the sender's nonce chain. When the pending pool reaches capacity and a new transaction must be inserted, `evict_lowest_pending()` finds the cheapest pending transaction across all senders and removes it. The `SenderQueue::remove_by_hash()` method then demotes all pending transactions with higher nonces from that sender to the queued pool, since they can no longer execute due to the nonce gap.

Consider a sender with transactions at nonces 0 through 255. If nonce 0 has a gas price of 1 gwei but nonces 1-255 have a gas price of 1000 gwei, evicting nonce 0 as the cheapest transaction demotes all 255 remaining high-fee transactions to the queued pool. These transactions become invisible to block building until the evicted nonce is re-submitted.

## Code Reference

`crates/node/txpool/src/pool.rs:363-373` -- `evict_lowest_pending` finds the cheapest pending transaction across all senders and removes it:

```rust
fn evict_lowest_pending(inner: &mut PoolInner) -> Option<OrderedTransaction> {
    let hash = inner
        .by_sender
        .values()
        .flat_map(|queue| queue.pending.iter())
        .min_by_key(|tx| (tx.effective_gas_price, std::cmp::Reverse(tx.timestamp), tx.hash))
        .map(|tx| tx.hash)?;
    let removed = inner.remove_by_hash(&hash);
    inner.update_counts();
    removed
}
```

`crates/node/txpool/src/ordering.rs:130-141` -- `SenderQueue::remove_by_hash` demotes all transactions after the removed one to the queued pool:

```rust
pub fn remove_by_hash(&mut self, hash: &B256) -> Option<OrderedTransaction> {
    if let Some(idx) = self.pending.iter().position(|tx| tx.hash == *hash) {
        let removed = self.pending.remove(idx);
        let mut moved = self.pending.split_off(idx);  // All txs after gap
        self.queued.append(&mut moved);                // Demoted to queued
        self.queued.sort_by_key(|tx| tx.nonce);
        return Some(removed);
    }

    let idx = self.queued.iter().position(|tx| tx.hash == *hash)?;
    Some(self.queued.remove(idx))
}
```

## Impact

A griefing strategy emerges: an attacker submits a very cheap "anchor" transaction at nonce 0, then submits many expensive dependent transactions at nonces 1-255. When the anchor is evicted under pool pressure, all 255 expensive transactions are demoted to the queued pool and become un-buildable. This reduces the effective value of the pool and degrades block quality (blocks include fewer high-fee transactions than they should). The attack is cheap -- the attacker only needs one low-fee transaction to render up to 255 high-fee transactions invisible.

Under normal operation (no attack), this behavior also causes problems during pool pressure: legitimate users with long transaction chains can lose all their pending transactions if their first one happens to be the cheapest in the pool.

## Root Cause

The pool correctly maintains the nonce ordering invariant (transactions cannot be executed out of nonce order), so demoting dependent transactions when a gap forms is logically correct. The problem is that the eviction strategy evaluates each transaction individually (`min_by_key(|tx| tx.effective_gas_price, ...)`) rather than considering the total value of the sender's entire pending chain.

## Suggested Fix

**Option 1 (recommended):** Use the sender's minimum pending gas price as the eviction key. This ensures senders with cheap "anchor" transactions are evicted as a complete unit:

```rust
fn evict_lowest_pending(inner: &mut PoolInner) -> Option<OrderedTransaction> {
    // Find the sender whose cheapest pending tx has the lowest price
    let (sender, _min_price) = inner
        .by_sender
        .iter()
        .filter(|(_, queue)| !queue.pending.is_empty())
        .map(|(sender, queue)| {
            let min = queue.pending.iter()
                .map(|tx| tx.effective_gas_price)
                .min()
                .unwrap();
            (*sender, min)
        })
        .min_by_key(|&(_, price)| price)?;

    // Evict the cheapest tx from that sender (which will cascade)
    let hash = inner.by_sender[&sender]
        .pending.iter()
        .min_by_key(|tx| tx.effective_gas_price)
        .map(|tx| tx.hash)?;
    let removed = inner.remove_by_hash(&hash);
    inner.update_counts();
    removed
}
```

**Option 2:** When evicting, calculate the total cumulative gas price of the sender's entire pending chain and compare across senders. Evict the sender whose entire chain has the lowest total value.

**Option 3:** Only evict transactions at the tail of a sender's pending queue (highest nonce), so no cascade occurs.

## Files to Modify

- `crates/node/txpool/src/pool.rs` -- `evict_lowest_pending()` at line 363
- `crates/node/txpool/src/ordering.rs` -- `SenderQueue::remove_by_hash()` at line 130 (no change needed, but relevant for understanding)

## Related Issues

- `041-txpool-in-memory-mempool-no-limits.md` -- InMemoryMempool has no eviction at all
- `044-txpool-min-gas-price-static-not-tracking-base-fee.md` -- Static minimum gas price allows cheap transactions to enter the pool
