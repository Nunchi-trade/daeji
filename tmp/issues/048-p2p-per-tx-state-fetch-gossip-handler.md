# 048: Per-Transaction State Fetch in Gossip Handler Creates Unnecessary Overhead

**Category:** performance
**Severity:** medium

**Labels:** `performance`, `p2p`, `txpool`

## Summary

For every inbound gossipped transaction, the handler fetches fresh state from the ledger and constructs a new `TransactionValidator` instance. This means each incoming gossip message triggers an async call to `latest_state()` (which acquires a mutex and clones state) and allocates a new validator object with a new `PoolConfig::default()`. Under sustained gossip load, this creates significant overhead from mutex contention, repeated allocations, and redundant state fetches.

## Problem

The gossip handler loop at `crates/node/runner/src/runner.rs:1108-1123` processes each inbound gossipped transaction independently, creating fresh state and validator objects for every single transaction:

```rust
// Fetch the latest state on each validation so nonce
// and balance checks reflect finalized blocks.  The
// previous code captured state once at startup, making
// gossip validation increasingly stale.
let current_state = gossip_ledger.latest_state().await;
let validator = TransactionValidator::new(
    gossip_chain_id,
    current_state,
    PoolConfig::default(),
)
.with_pool(gossip_pool.clone());
if let Err(e) = validator.validate(tx.clone()).await {
    trace!(?tx_id, ?peer, error = %e, "tx gossip: peer tx failed validation");
    in_metrics.gossip_tx_invalid.inc();
    continue;
}

if gossip_ledger.submit_tx(tx).await {
```

The same pattern exists in the RPC submission path at `crates/node/runner/src/runner.rs:1211-1218`:

```rust
let state = ledger.latest_state().await;
let validator =
    TransactionValidator::new(chain_id, state, PoolConfig::default())
        .with_pool(pool);
validator.validate(tx.clone()).await.map_err(|err| {
    warn!(?tx_id, error = %err, "rpc submit: validator rejected tx");
    kora_rpc::RpcError::InvalidTransaction(err.to_string())
})?;
```

Each call involves:

1. **`latest_state()` acquires and releases the `LedgerView` inner mutex** -- this is the same mutex used by the consensus pipeline. The function is defined at `crates/node/ledger/src/lib.rs:297` (for `LedgerView`) and line 624 (for `Ledger`). Each call clones the `OverlayState` which involves cloning `Arc` references.

2. **`TransactionValidator::new()` allocates a new struct** on each call.

3. **`PoolConfig::default()` recreates the same static configuration** every time -- the default config has the same values for every call.

The code comment acknowledges this is a deliberate change from the previous design that cached state at startup (which made gossip validation "increasingly stale"), but the per-transaction approach swings too far in the opposite direction. At 34 blocks/s with 100ms between blocks, state changes are infrequent relative to gossip message rate.

## Impact

Under gossip flood conditions (hundreds of transactions per second from multiple peers, which is normal for an active EVM network), the per-transaction state fetch creates measurable overhead:

- **Mutex contention**: The `LedgerView` mutex is shared between the gossip handler and the consensus pipeline (which also needs to read/write state during block building and verification). Frequent acquisitions increase contention.
- **CPU overhead**: Each call involves atomic reference counting operations (`Arc::clone`) and struct allocation. At 1000 txs/s, this adds up.
- **Redundant work**: State changes only when a new block is finalized (~every 30ms at 34 blocks/s). Fetching state 1000 times per second when it only changes 34 times per second means ~97% of fetches are redundant.

On the current 10-node devnet with minimal transaction load, this overhead is not visible. Under production load with active DeFi activity (hundreds to thousands of transactions per second), this becomes a bottleneck that could cause voting delays and nullification.

## Root Cause

The refactoring that moved from cached state to per-transaction state correctly identified the staleness problem but did not introduce any batching or amortization. Each transaction is treated as an independent event requiring fresh state, even though state changes are relatively infrequent compared to gossip message rate.

## Suggested Fix

**Option 1 (recommended):** Amortize the state fetch across a time window. Cache the state for a short duration (e.g., 100ms, which spans ~3 blocks at 34 blocks/s):

```rust
// Before the gossip loop:
let config = PoolConfig::default();  // Allocate once
let mut cached_state: Option<(tokio::time::Instant, OverlayState<QmdbState>)> = None;
let cache_duration = Duration::from_millis(100);

// Inside the loop, for each incoming tx:
let state = match &cached_state {
    Some((created, state)) if created.elapsed() < cache_duration => state.clone(),
    _ => {
        let fresh = gossip_ledger.latest_state().await;
        cached_state = Some((tokio::time::Instant::now(), fresh.clone()));
        fresh
    }
};
let validator = TransactionValidator::new(gossip_chain_id, state, config.clone())
    .with_pool(gossip_pool.clone());
```

This reuses the same state for all transactions within a 100ms window, reducing mutex acquisitions by ~100x under sustained load while ensuring state freshness within a reasonable bound.

**Option 2:** Subscribe to block finalization events and update state only when a new block is finalized. This is more event-driven but requires a notification channel from the ledger.

**Option 3:** At minimum, hoist `PoolConfig::default()` outside the loop and reuse the same config instance:

```rust
let pool_config = PoolConfig::default(); // Allocate once outside the loop
// ...
loop {
    // Inside loop:
    let validator = TransactionValidator::new(gossip_chain_id, current_state, pool_config.clone())
```

## Files to Modify

- `crates/node/runner/src/runner.rs` -- gossip handler at line 1108-1123 (add state caching and hoist config)
- `crates/node/runner/src/runner.rs` -- RPC handler at line 1211-1218 (same optimization, lower priority since RPC is less frequent)

## Related Issues

- `043-txpool-toctou-validation-insertion.md` -- TOCTOU race in the same validate-then-insert code path
- `045-p2p-uniform-rate-quota-all-channels.md` -- Uniform rate quota allows high gossip message rates that amplify this overhead
