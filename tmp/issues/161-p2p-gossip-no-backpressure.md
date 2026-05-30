# P2P Gossip: No Backpressure from Validation to Receive Loop -- Head-of-Line Blocking

**Category**: bug
**Severity**: medium

## Summary

The gossip inbound handler runs as a single sequential loop that receives a transaction from the P2P layer and then performs full inline validation (state fetch, ECDSA signature recovery, nonce/balance checks, pool insertion) before processing the next message. A single slow validation blocks all subsequent gossip processing, and there is no mechanism for load shedding when the node is CPU-saturated. The P2P channel's backlog (1,024 messages) is the only buffer, and when it fills, the P2P layer starts dropping messages indiscriminately.

## Problem

The inbound gossip handler processes transactions sequentially in a tight `loop`:

1. `receiver.recv().await` -- waits for the next gossip message from the P2P layer
2. Inline dedup check via `mark_seen()` -- fast (parking_lot mutex, O(1) hash lookup)
3. `gossip_ledger.latest_state().await` -- potentially slow (async lock contention with finalization)
4. `TransactionValidator::new(...).validate(tx).await` -- expensive (ECDSA recovery, state lookups for balance/nonce, pool duplicate check)
5. `gossip_ledger.submit_tx(tx).await` -- pool insertion

Steps 3-5 can take milliseconds to complete, especially if a transaction touches cold storage or if the ledger lock is contended. During this time, NO other gossip messages are processed -- they queue up in the P2P channel's internal buffer.

The P2P channel is bounded at 1,024 messages. If validation consistently takes longer than the inter-arrival rate of gossip messages, the buffer fills and the P2P layer begins dropping messages. These dropped messages may include legitimate transactions from honest peers mixed in with flood traffic.

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

        // ALL of the following runs INLINE -- blocks the receive loop:
        let current_state = gossip_ledger.latest_state().await;       // async lock
        let validator = TransactionValidator::new(
            gossip_chain_id,
            current_state,
            PoolConfig::default(),
        )
        .with_pool(gossip_pool.clone());
        if let Err(e) = validator.validate(tx.clone()).await {        // ECDSA + state checks
            trace!(?tx_id, ?peer, error = %e, "tx gossip: peer tx failed validation");
            in_metrics.gossip_tx_invalid.inc();
            continue;
        }

        if gossip_ledger.submit_tx(tx).await {                       // pool insertion
            debug!(?tx_id, ?peer, "tx gossip: accepted transaction from peer");
        } else {
            trace!(?tx_id, ?peer, "tx gossip: ledger rejected transaction (duplicate)");
        }
    }
});
```

## Impact

1. **Head-of-line blocking**: A single transaction that is slow to validate (e.g., touching cold storage slots, triggering EVM precompile lookups, or hitting lock contention on `latest_state()`) blocks processing of ALL subsequent gossip messages. At 30+ blocks/s with a 10-validator network, even a 10ms delay per transaction can cause message queue buildup.

2. **Denial-of-service vector**: An attacker can craft transactions that are expensive to validate but ultimately invalid (e.g., transactions with valid signatures that touch many storage slots but have insufficient balance). Each such transaction blocks the gossip pipeline for milliseconds, reducing throughput for legitimate transactions.

3. **Indiscriminate message dropping**: When the 1,024-message P2P buffer fills, the transport layer drops messages without priority. High-fee legitimate transactions are equally likely to be dropped as low-fee or spam transactions.

4. **No load shedding**: The handler validates every received transaction regardless of CPU pressure. There is no mechanism to skip validation when the node is already saturated (e.g., during block execution).

## Root Cause

The gossip handler was designed as a simple sequential receive-validate loop without any concurrency, backpressure, or load-shedding mechanisms. The assumption was that validation would always be fast relative to the gossip message arrival rate.

## Suggested Fix

Decouple receiving from validation using a producer-consumer pattern:

```rust
// Receive task: fast, non-blocking -- only does dedup
let (validation_tx, mut validation_rx) = tokio::sync::mpsc::channel(1024);
context.child("tx_gossip_recv").shared(true).spawn(move |_| async move {
    loop {
        let (peer, raw) = match receiver.recv().await {
            Ok(msg) => msg,
            Err(e) => { warn!(error = %e, "gossip receive error"); break; }
        };
        in_metrics.gossip_tx_received.inc();
        let hash = keccak256(&raw);
        if !mark_seen(&seen, hash) { continue; }
        // Non-blocking send to validation queue
        if validation_tx.try_send((peer, raw)).is_err() {
            warn!("gossip validation queue full, dropping transaction");
            in_metrics.gossip_tx_dropped.inc();
        }
    }
});

// Validation task: runs with bounded concurrency
let semaphore = Arc::new(tokio::sync::Semaphore::new(8));
context.child("tx_gossip_validate").shared(true).spawn(move |_| async move {
    while let Some((peer, raw)) = validation_rx.recv().await {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let ledger = gossip_ledger.clone();
        let pool = gossip_pool.clone();
        tokio::spawn(async move {
            let state = ledger.latest_state().await;
            let validator = TransactionValidator::new(chain_id, state, PoolConfig::default())
                .with_pool(pool);
            let tx = Tx::new(alloy_primitives::Bytes::from(raw));
            if validator.validate(tx.clone()).await.is_ok() {
                ledger.submit_tx(tx).await;
            }
            drop(permit);
        });
    }
});
```

This architecture:
- Keeps the receive loop fast (only dedup check, O(1))
- Provides explicit backpressure via the bounded validation queue
- Allows concurrent validation with a configurable concurrency limit (semaphore)
- Enables load shedding with `try_send` when the validation pipeline is saturated
- Adds a metric (`gossip_tx_dropped`) so operators can monitor load shedding

## Files to Modify

- `crates/node/runner/src/runner.rs` -- refactor inbound gossip handler (lines 1087-1131) into receive + validate tasks

## Related Issues

- `048-p2p-per-tx-state-fetch-gossip-handler.md` -- per-tx state fetch is one source of the per-message latency
- `159-p2p-gossip-validation-no-batching.md` -- complementary fix: batch multiple transactions into a single validation pass
- `158-p2p-gossip-seen-set-wholesale-clear.md` -- seen-set clear events compound the problem by causing re-validation storms
- `025-tx-gossip-no-rate-limit.md` -- no rate limiting on inbound gossip

## Labels

`bug`, `p2p`, `reliability`, `performance`
