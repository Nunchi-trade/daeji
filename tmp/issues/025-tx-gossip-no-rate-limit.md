# 025: Transaction Gossip Lacks Per-Peer Rate Limiting -- DoS Vector

**Category**: bug, security
**Severity**: high
**Labels**: bug, security, p2p, txpool, performance

---

## Summary

The transaction gossip handler processes every incoming transaction from every peer without any per-peer rate limiting. A malicious peer can flood a validator with transaction messages at an unlimited rate, forcing the validator to perform expensive operations (ECDSA signature recovery, state lookups, RLP decoding) on each message. This can degrade consensus throughput by starving the event loop of CPU time and creating contention on state access.

---

## Problem

The transaction gossip inbound handler in `runner.rs` (lines 1087-1131) receives messages from a commonware P2P channel and processes each one sequentially in a single async task. For each incoming transaction, the handler performs:

1. **Hash computation and deduplication** via `mark_seen()` (line 1099) -- cheap but bounded by `TX_GOSSIP_SEEN_SET_CAPACITY` (65,536 entries at line 84).
2. **State fetching** via `gossip_ledger.latest_state()` (line 1112) -- requires acquiring a lock on the ledger's inner state.
3. **Full transaction validation** via `TransactionValidator::validate()` (line 1119) -- includes RLP decoding, ECDSA signature recovery (CPU-intensive elliptic curve operation), chain ID check, nonce check, and balance check against the state database.
4. **Pool insertion** via `gossip_ledger.submit_tx()` (line 1125).

There is no mechanism to limit the rate at which any single peer can submit transactions through this channel. The commonware P2P transport provides message delivery but no application-level per-peer throttling.

**File**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`

---

## Code Reference

The transaction gossip inbound handler (lines 1087-1131 of `runner.rs`):

```rust
// crates/node/runner/src/runner.rs:1087-1131
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
        // and balance checks reflect finalized blocks.
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
            debug!(?tx_id, ?peer, "tx gossip: accepted transaction from peer");
        } else {
            trace!(?tx_id, ?peer, "tx gossip: ledger rejected transaction (duplicate)");
        }
    }
});
```

The seen-set capacity constant and overflow behavior (lines 84, 720-727 of `runner.rs`):

```rust
// crates/node/runner/src/runner.rs:84
const TX_GOSSIP_SEEN_SET_CAPACITY: usize = 65_536;

// crates/node/runner/src/runner.rs:720-727
fn mark_seen(seen: &SeenSet, hash: B256) -> bool {
    let mut set = seen.lock();
    if set.len() >= TX_GOSSIP_SEEN_SET_CAPACITY {
        debug!(capacity = TX_GOSSIP_SEEN_SET_CAPACITY, "tx gossip seen-set full, clearing");
        set.clear();
    }
    set.insert(hash)
}
```

---

## Impact

A single malicious peer can:

1. **Exhaust CPU with signature recoveries**: Each unique transaction requires ECDSA signature recovery (`recover_sender_and_hash()` in `validator.rs:164`), which involves elliptic curve multiplication. At thousands of messages per second, this dominates the CPU budget for the gossip task.

2. **Create state access contention**: Each validation calls `gossip_ledger.latest_state().await` (line 1112), which acquires a lock on the ledger's inner state. Under flood conditions, this creates lock contention that can delay consensus-critical state access.

3. **Force seen-set clears**: By sending 65,536+ unique transaction hashes, the attacker can fill the seen-set (capped at `TX_GOSSIP_SEEN_SET_CAPACITY`), causing it to be cleared. This re-enables duplicate processing of transactions that were previously filtered, amplifying the attack.

4. **Amplify with "almost valid" transactions**: The most effective attack uses transactions that pass RLP decoding and signature recovery (consuming maximum CPU) but fail at the cheaper state checks (e.g., nonce too high). These transactions are expensive to reject.

5. **Block consensus throughput**: The gossip handler runs on the shared Tokio runtime. Sustained CPU consumption by the gossip task reduces the time available for consensus message processing, block verification, and block building.

---

## Root Cause

The commonware P2P transport delivers messages from all peers through a single channel without per-peer rate limiting at the application level. The gossip handler processes all messages in a single loop without tracking per-peer message rates or applying backpressure.

---

## Suggested Fix

1. **Per-peer rate limiting**: Track the message rate per peer and drop messages from peers that exceed a threshold (e.g., 100 tx/s per peer):

```rust
use std::collections::HashMap;
use std::time::Instant;

struct PeerRateLimiter {
    counts: HashMap<ed25519::PublicKey, (u64, Instant)>,
    max_per_second: u64,
}

impl PeerRateLimiter {
    fn check(&mut self, peer: &ed25519::PublicKey) -> bool {
        let entry = self.counts.entry(peer.clone()).or_insert((0, Instant::now()));
        if entry.1.elapsed().as_secs() >= 1 {
            *entry = (1, Instant::now());
            true
        } else if entry.0 >= self.max_per_second {
            false  // Rate exceeded
        } else {
            entry.0 += 1;
            true
        }
    }
}
```

2. **Bounded validation queue**: Accept raw transaction bytes into a bounded channel and perform validation asynchronously, preventing the P2P message handler from blocking:

```rust
let (validation_tx, mut validation_rx) = tokio::sync::mpsc::channel(1024);
// In the gossip handler:
if validation_tx.try_send((peer, raw)).is_err() {
    trace!("validation queue full, dropping gossip tx");
}
```

3. **Transaction pool backpressure**: When the pool is at capacity, stop accepting new gossip transactions:

```rust
if gossip_pool.is_full() {
    continue; // Drop without validation
}
```

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- Add per-peer rate limiting to the `tx_gossip_in` handler (around line 1087)

---

## Related Issues

- `028-txpool-balance-check-ignores-pending.md` (transaction pool validation gaps)
