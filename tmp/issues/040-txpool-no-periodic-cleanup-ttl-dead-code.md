# No Periodic Cleanup Task -- TTL Expiration Was Dead Code (RESOLVED)

**Category**: Bug -- Transaction Pool
**Severity**: Medium
**Labels**: `bug`, `txpool`, `reliability`

## Status: RESOLVED

This issue has been fixed. A periodic cleanup task (`spawn_txpool_cleanup`) was added to `crates/node/runner/src/runner.rs:693-703` that calls `pool.cleanup()` on a timer. The TTL configuration is now functional.

## Summary

The transaction pool had a `cleanup()` method that removes transactions older than configurable TTL values (`pending_ttl_secs: 1800` for pending, `queued_ttl_secs: 3600` for queued), but no code called it on a periodic schedule. This meant expired transactions would accumulate in the pool indefinitely, consuming memory until the pool hit its capacity limits and started evicting based on size rather than age. The fix adds a periodic task that invokes `cleanup()` on a timer.

## Problem

Kora is an EVM execution client with a transaction pool (`TransactionPool` in `crates/node/txpool/src/pool.rs`) that queues incoming transactions for inclusion in future blocks. The pool supports TTL-based expiration via its `cleanup()` method and configurable TTL values in `PoolConfig` (`crates/node/txpool/src/config.rs`).

Before the fix, the `cleanup()` method existed but was never called by any periodic task. The TTL fields (`pending_ttl_secs` and `queued_ttl_secs`) in the pool configuration were effectively dead code -- they could be set but had no effect at runtime.

## Code Reference

The fix -- the periodic cleanup task in `crates/node/runner/src/runner.rs:693-703`:
```rust
fn spawn_txpool_cleanup(pool: TransactionPool, context: cw_tokio::Context) {
    context.child("txpool_cleanup").shared(false).spawn(move |ctx| async move {
        loop {
            ctx.sleep(TXPOOL_CLEANUP_INTERVAL).await;
            let removed = pool.cleanup();
            if removed > 0 {
                debug!(removed, "expired transactions cleaned from txpool");
            }
        }
    });
}
```

The `cleanup()` method in `crates/node/txpool/src/pool.rs:527-558`:
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

The TTL configuration in `crates/node/txpool/src/config.rs:18-21`:
```rust
/// Time-to-live for pending transactions, in seconds.
pub pending_ttl_secs: u64,
/// Time-to-live for queued transactions, in seconds.
pub queued_ttl_secs: u64,
```

With defaults at `crates/node/txpool/src/config.rs:33-34`:
```rust
pending_ttl_secs: 30 * 60,   // 30 minutes
queued_ttl_secs: 60 * 60,    // 60 minutes
```

## Resolution

A periodic cleanup task was added as `spawn_txpool_cleanup()` in `crates/node/runner/src/runner.rs:693-703`. It runs as a commonware child context task (using the `cw_tokio` runtime), sleeping for `TXPOOL_CLEANUP_INTERVAL` between each invocation of `pool.cleanup()`. When transactions are removed, it logs the count at debug level.

The TTL configuration in `crates/node/txpool/src/config.rs` is now functional:
- `pending_ttl_secs`: 1800 (30 minutes) -- pending (executable) transactions older than this are removed
- `queued_ttl_secs`: 3600 (60 minutes) -- queued (future nonce) transactions older than this are removed

## Files

- `crates/node/runner/src/runner.rs:693-703` -- `spawn_txpool_cleanup()` periodic task (the fix)
- `crates/node/txpool/src/pool.rs:527-558` -- `cleanup()` method (now called periodically)
- `crates/node/txpool/src/config.rs:18-21,33-34` -- TTL configuration (now functional)

## Related Issues

None.
