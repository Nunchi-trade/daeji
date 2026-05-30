# Transaction Pool build() Has O(senders x max_txs) Quadratic Scaling Per Block

**Category**: Transaction Pool / Performance
**Severity**: Medium

## Summary

The `build()` method in `TransactionPool` uses a naive linear scan across all sender queues for each transaction it selects, resulting in O(S x T) time complexity where S is the number of distinct senders and T is the target transaction count. At the current pool limits (up to 1000 senders, 1000 target transactions, 34 blocks/second), this produces up to 34 million comparisons per second on the consensus-critical hot path. As the network grows and distinct sender count increases, block building latency will increase quadratically, potentially delaying proposals and increasing nullification risk.

## Problem

In `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/pool.rs`, the `build()` method at lines 677-715 iterates over all sender queues on each iteration of the output loop to find the best candidate transaction. For each of the `max_txs` output positions (up to 1000), it performs a full linear scan of all sender queues via `senders.iter_mut()` + `min_by()`:

```rust
// crates/node/txpool/src/pool.rs:677-715
fn build(&self, max_txs: usize, excluded: &BTreeSet<TxId>) -> Vec<Tx> {
    let inner = self.inner.read();
    let mut senders: HashMap<Address, BuildSenderState> = inner
        .by_sender
        .iter()
        .filter(|(_, queue)| !queue.pending.is_empty())
        .map(|(sender, queue)| {
            (
                *sender,
                BuildSenderState {
                    txs: queue.pending.clone(),
                    index: 0,
                    expected_nonce: queue.next_nonce,
                },
            )
        })
        .collect();
    let pending_count = senders.values().map(|state| state.txs.len()).sum();
    let mut result = Vec::with_capacity(max_txs.min(pending_count));

    while result.len() < max_txs {
        let Some((sender, tx)) = senders
            .iter_mut()
            .filter_map(|(sender, state)| {
                state.next_candidate(excluded).map(|tx| (*sender, tx))
            })
            .min_by(|(_, left), (_, right)| left.cmp(right))  // Linear scan for min
        else {
            break;
        };

        if let Some(state) = senders.get_mut(&sender) {
            state.consume();
            result.push(ordered_to_tx(&tx));
        }
    }

    result
}
```

Additionally, `next_candidate()` (lines 34-55) calls `ordered_tx_id()` for each candidate to check against the `excluded` set, and `ordered_tx_id()` performs a full allocation + encode + hash per call (see issue 083):

```rust
// crates/node/txpool/src/pool.rs:34-55
fn next_candidate(&mut self, excluded: &BTreeSet<TxId>) -> Option<OrderedTransaction> {
    while let Some(tx) = self.txs.get(self.index) {
        if tx.nonce < self.expected_nonce {
            self.index += 1;
            continue;
        }
        if tx.nonce > self.expected_nonce {
            return None;
        }
        if excluded.contains(&ordered_tx_id(tx)) {  // allocation + encode + hash
            self.expected_nonce = tx.nonce.saturating_add(1);
            self.index += 1;
            continue;
        }
        return Some(tx.clone());
    }
    None
}
```

## Impact

At the current pool limits:
- `max_txs = 1000` (block capacity)
- Up to 4096 pending transactions from potentially ~1000 distinct senders
- 34 blocks/second

This produces up to 1,000 x 1,000 = 1,000,000 comparisons per block build, or 34 million comparisons/second in the consensus hot path. Each comparison involves calling `next_candidate()` which may call `ordered_tx_id()` (allocation + hash).

As the network grows and the number of distinct senders increases:
- **Block building latency increases quadratically**: With 5,000 senders and 1,000 target txs, the cost rises to 5 million comparisons per block build.
- **Delayed proposals risk nullification**: If block building takes too long, the block proposal may not reach quorum before the consensus timeout, causing the block to be nullified (wasted consensus round).
- **CPU pressure on the consensus thread**: Block building runs inside `tokio::task::spawn_blocking` (see `crates/node/runner/src/app.rs:355` and `crates/node/consensus/src/proposal.rs:183`), but the blocking thread still competes for CPU time with consensus and networking tasks.

## Root Cause

The algorithm uses a naive linear scan to find the minimum-gas-price candidate across all senders on each iteration. This is the simplest correct approach for "merging K sorted sequences," but has poor asymptotic complexity. The standard efficient approach is to use a min-heap (priority queue) which reduces the per-selection cost from O(S) to O(log S).

## Suggested Fix

Replace the linear scan with a `BinaryHeap` (min-heap) of `(OrderedTransaction, Address)` tuples:

```rust
use std::collections::BinaryHeap;
use std::cmp::Reverse;

fn build(&self, max_txs: usize, excluded: &BTreeSet<TxId>) -> Vec<Tx> {
    let inner = self.inner.read();
    let mut senders: HashMap<Address, BuildSenderState> = /* ... same init ... */;

    // Initialize heap with each sender's best candidate
    let mut heap: BinaryHeap<Reverse<(OrderedTransaction, Address)>> = senders
        .iter_mut()
        .filter_map(|(addr, state)| {
            state.next_candidate(excluded).map(|tx| Reverse((tx, *addr)))
        })
        .collect();

    let mut result = Vec::with_capacity(max_txs.min(pending_count));

    while result.len() < max_txs {
        let Some(Reverse((tx, sender))) = heap.pop() else { break; };

        result.push(ordered_to_tx(&tx));

        if let Some(state) = senders.get_mut(&sender) {
            state.consume();
            // Push this sender's next candidate back onto the heap
            if let Some(next_tx) = state.next_candidate(excluded) {
                heap.push(Reverse((next_tx, sender)));
            }
        }
    }

    result
}
```

This requires implementing `Ord` for the `(OrderedTransaction, Address)` tuple -- since `OrderedTransaction` already implements `Ord` (via `crates/node/txpool/src/ordering.rs:59-67`), the tuple just needs an `Address` tiebreaker:

```rust
// crates/node/txpool/src/ordering.rs:59-67
impl Ord for OrderedTransaction {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .effective_gas_price
            .cmp(&self.effective_gas_price)      // higher gas price = lower (better) order
            .then_with(|| self.timestamp.cmp(&other.timestamp))  // earlier = better
            .then_with(|| self.hash.cmp(&other.hash))            // deterministic tiebreak
    }
}
```

**Complexity improvement**:
- **Before**: O(S x T) = O(1,000 x 1,000) = 1,000,000 comparisons per block build
- **After**: O(T x log S) = O(1,000 x 10) = 10,000 heap operations per block build
- **100x reduction** in comparisons for the current pool configuration

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/pool.rs` (lines 677-715) -- replace linear scan in `build()` with `BinaryHeap`-based selection

## Related Issues

- `083-txpool-ordered-tx-id-reencodes.md` -- `ordered_tx_id()` re-encodes on every call; fixing that reduces the constant factor within each `next_candidate()` call, but the quadratic algorithm remains the dominant cost

## Labels

performance, txpool
