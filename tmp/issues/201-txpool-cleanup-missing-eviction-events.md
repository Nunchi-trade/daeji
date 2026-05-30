# cleanup() Does Not Emit TxEvicted Events for Expired Transactions

**Category**: bug -- txpool
**Severity**: high

## Summary

The `cleanup()` method removes expired transactions from the pool but does not emit `MempoolEvent::TxEvicted` events for them. This breaks the event lifecycle contract: every `TxAdded` event should eventually be matched by a `TxEvicted` event or a confirmed-inclusion event. Subscribers (such as WebSocket `newPendingTransactions` listeners and monitoring systems) will see transactions silently disappear without explanation.

## Problem

The `cleanup()` method iterates through all transactions in the pool, identifies those whose timestamp exceeds the configured TTL, and removes them via `inner.remove_by_hash()`. However, it never sends `MempoolEvent::TxEvicted` events to the event broadcast channel.

By contrast, the other removal paths in the same file correctly emit events:
- `remove_with_reason()` (line 425-428) emits `TxEvicted` with a reason string
- The eviction loop in `add()` (lines 297-300) emits `TxEvicted` for each evicted transaction
- The replacement path in `add()` (lines 290-292) emits `TxEvicted` with reason "replaced"

## Code Reference

File: `crates/node/txpool/src/pool.rs`, lines 527-558

```rust
/// Removes expired transactions and returns the number removed.
pub fn cleanup(&self) -> usize {
    let now = current_timestamp();
    let mut inner = self.inner.write();
    let expired: Vec<B256> = inner
        .by_sender
        .values()
        .flat_map(|queue| {
            let pending = queue.pending.iter().filter_map(|tx| {
                (now.saturating_sub(tx.timestamp) > self.config.pending_ttl_secs)
                    .then_some(tx.hash)
            });
            let queued = queue.queued.iter().filter_map(|tx| {
                (now.saturating_sub(tx.timestamp) > self.config.queued_ttl_secs)
                    .then_some(tx.hash)
            });
            pending.chain(queued)
        })
        .collect();

    let mut removed = 0;
    for hash in expired {
        if inner.remove_by_hash(&hash).is_some() {
            removed += 1;
            // Missing: emit TxEvicted event
        }
    }
    inner.update_counts();
    drop(inner);
    if removed > 0 {
        self.sync_metrics();
    }
    removed
}
```

For comparison, `remove_with_reason()` (lines 417-432) correctly emits events:

```rust
pub fn remove_with_reason(&self, hash: &B256, reason: &str) -> Option<OrderedTransaction> {
    let mut inner = self.inner.write();
    let tx = inner.remove_by_hash(hash)?;
    inner.update_counts();
    drop(inner);

    if let Some(events) = &self.events {
        let _ =
            events.send(MempoolEvent::TxEvicted { hash: *hash, reason: reason.to_string() });
    }

    self.sync_metrics();
    Some(tx)
}
```

## Impact

1. **Silent transaction disappearance**: Subscribers listening for `MempoolEvent` events (e.g., WebSocket `newPendingTransactions` subscribers, monitoring dashboards) will see transactions appear via `TxAdded` but never receive a corresponding `TxEvicted` event when those transactions expire. The transactions simply vanish from the pool.
2. **Broken event lifecycle contract**: The expected lifecycle is `TxAdded -> TxEvicted | confirmed`. Expired transactions break this contract, creating "orphaned" `TxAdded` events that are never matched.
3. **Debugging difficulty**: Without eviction events for expired transactions, operators cannot distinguish between transactions that were included in blocks, transactions that were replaced, and transactions that simply expired.

## Root Cause

The `cleanup()` method was implemented to remove expired transactions but was not wired into the event emission system that other removal paths use. It calls `inner.remove_by_hash()` directly without going through `remove_with_reason()`.

## Suggested Fix

Collect the hashes of expired transactions before dropping the write lock, then emit events after the lock is released (matching the pattern used in `add()`).

**After:**
```rust
pub fn cleanup(&self) -> usize {
    let now = current_timestamp();
    let mut inner = self.inner.write();
    let expired: Vec<B256> = inner
        .by_sender
        .values()
        .flat_map(|queue| {
            let pending = queue.pending.iter().filter_map(|tx| {
                (now.saturating_sub(tx.timestamp) > self.config.pending_ttl_secs)
                    .then_some(tx.hash)
            });
            let queued = queue.queued.iter().filter_map(|tx| {
                (now.saturating_sub(tx.timestamp) > self.config.queued_ttl_secs)
                    .then_some(tx.hash)
            });
            pending.chain(queued)
        })
        .collect();

    let mut removed_hashes = Vec::new();
    for hash in expired {
        if inner.remove_by_hash(&hash).is_some() {
            removed_hashes.push(hash);
        }
    }
    inner.update_counts();
    drop(inner);

    // Emit eviction events outside the lock
    if let Some(events) = &self.events {
        for hash in &removed_hashes {
            let _ = events.send(MempoolEvent::TxEvicted {
                hash: *hash,
                reason: "expired".to_string(),
            });
        }
    }

    if !removed_hashes.is_empty() {
        self.sync_metrics();
    }
    removed_hashes.len()
}
```

## Files to Modify

- `crates/node/txpool/src/pool.rs` -- Add event emission in `cleanup()` (lines 527-558)

## Related Issues

- None

## Labels

bug, correctness, txpool
