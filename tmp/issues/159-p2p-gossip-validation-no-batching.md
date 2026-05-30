# P2P Gossip: Per-Message Validation Creates TransactionValidator and Fetches State Individually

**Category**: performance
**Severity**: medium

## Summary

Every inbound gossip transaction independently fetches the latest ledger state (an async lock/clone operation) and creates a new `TransactionValidator` with a fresh `PoolConfig::default()`. There is no batching or amortization of state lookups or validator construction across transactions that arrive in the same time window, leading to significant overhead under burst traffic.

## Problem

The inbound gossip handler in Kora processes transactions one at a time in a sequential loop. For each received transaction, the code:

1. Calls `gossip_ledger.latest_state().await` -- this acquires an async lock on the ledger and clones the current state snapshot. Under contention (e.g., when finalization is also updating state), this can block.
2. Creates a new `TransactionValidator::new(chain_id, state, PoolConfig::default())` -- this allocates a new `PoolConfig::default()` struct and constructs a fresh validator object.
3. Calls `validator.validate(tx).await` -- runs ECDSA recovery, nonce/balance checks against the fetched state, and pool duplicate detection.

When multiple peers forward transactions simultaneously (common during load tests or network re-gossip), N transactions arriving in the same millisecond each independently execute steps 1-3, even though they could share a single state fetch and validator instance.

## Code Reference

**Inbound gossip handler** -- `crates/node/runner/src/runner.rs:1087-1131`:
```rust
context.child("tx_gossip_in").shared(true).spawn(move |_| async move {
    loop {
        let (peer, raw) = match receiver.recv().await {
            Ok(msg) => msg,
            Err(e) => {
                warn!(error = %e, "tx gossip: receive error, stopping inbound handler");
                break;
            }
        };

        in_metrics.gossip_tx_received.inc();
        let hash = keccak256(&raw);
        if !mark_seen(&seen, hash) {
            trace!(?hash, ?peer, "tx gossip: skipping already-seen transaction");
            continue;
        }

        let data = alloy_primitives::Bytes::copy_from_slice(raw.as_ref());
        let tx = Tx::new(data);
        let tx_id = tx.id();

        // Fetch the latest state on each validation so nonce
        // and balance checks reflect finalized blocks.  The
        // previous code captured state once at startup, making
        // gossip validation increasingly stale.
        let current_state = gossip_ledger.latest_state().await;  // <-- PER-TX state fetch
        let validator = TransactionValidator::new(
            gossip_chain_id,
            current_state,
            PoolConfig::default(),   // <-- PER-TX allocation
        )
        .with_pool(gossip_pool.clone());
        if let Err(e) = validator.validate(tx.clone()).await {
            trace!(?tx_id, ?peer, error = %e, "tx gossip: peer tx failed validation");
            in_metrics.gossip_tx_invalid.inc();
            continue;
        }

        if gossip_ledger.submit_tx(tx).await {
            debug!(?tx_id, ?peer, "tx gossip: accepted transaction from peer");
        } else {
            trace!(?tx_id, ?peer, "tx gossip: ledger rejected transaction (duplicate)");
        }
    }
});
```

Note the sequential `loop` with `receiver.recv().await` -- each iteration handles exactly one transaction before moving to the next. There is no draining of multiple pending messages from the channel.

## Impact

Under burst gossip traffic (e.g., a load test pushing 500 tx/s, with 9 peers each forwarding):

- **Mutex contention**: Repeated `latest_state()` calls compete with finalization for the ledger lock, potentially stalling both gossip validation and block finalization.
- **Redundant allocation**: Thousands of identical `PoolConfig::default()` and `TransactionValidator` objects are created per second, all with the same configuration.
- **No amortization**: A burst of 50 transactions arriving in 10ms results in 50 independent state fetches, even though a single fetch would suffice for the entire batch.
- **Head-of-line blocking**: While one transaction is being validated (especially if it touches cold storage), all subsequent pending messages queue up behind it.

## Root Cause

The gossip handler was designed as a simple per-message sequential loop. There is no batching layer to drain multiple messages from the channel, fetch state once, and validate the batch with a shared `TransactionValidator`.

## Suggested Fix

Implement a batch-oriented validation pipeline that amortizes state fetches:

```rust
let pool_config = PoolConfig::default();  // Allocate once at startup

context.child("tx_gossip_in").shared(true).spawn(move |_| async move {
    loop {
        // Wait for at least one message
        let first = match receiver.recv().await {
            Ok(msg) => msg,
            Err(e) => {
                warn!(error = %e, "tx gossip: receive error");
                break;
            }
        };

        // Drain up to N more messages that are already buffered
        let mut batch = vec![first];
        while batch.len() < 64 {
            match receiver.try_recv() {
                Ok(msg) => batch.push(msg),
                Err(_) => break,
            }
        }

        // Single state fetch for entire batch
        let current_state = gossip_ledger.latest_state().await;
        let validator = TransactionValidator::new(
            gossip_chain_id,
            current_state,
            pool_config.clone(),
        ).with_pool(gossip_pool.clone());

        for (peer, raw) in batch {
            let hash = keccak256(&raw);
            if !mark_seen(&seen, hash) { continue; }
            let data = alloy_primitives::Bytes::from(raw);
            let tx = Tx::new(data);
            if let Err(e) = validator.validate(tx.clone()).await {
                in_metrics.gossip_tx_invalid.inc();
                continue;
            }
            gossip_ledger.submit_tx(tx).await;
        }
    }
});
```

This reduces per-transaction overhead from O(1 state fetch + 1 validator creation) to O(1/batch_size) amortized.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- refactor the inbound gossip handler (lines 1087-1131) to use batch processing

## Related Issues

- `048-p2p-per-tx-state-fetch-gossip-handler.md` -- focuses on the per-tx state fetch cost specifically
- `156-p2p-gossip-tx-double-copy.md` -- another per-message overhead in the same handler (copy_from_slice)
- `161-p2p-gossip-no-backpressure.md` -- no backpressure from validation to receive loop (complementary concern)

## Labels

`performance`, `p2p`
