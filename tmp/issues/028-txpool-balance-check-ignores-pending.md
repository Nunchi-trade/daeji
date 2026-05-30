# 028: Transaction Pool Balance Check Ignores Cumulative Cost of Pending Transactions

**Category**: bug
**Severity**: high
**Labels**: bug, txpool, correctness

---

## Summary

The transaction pool's balance validation checks each incoming transaction independently against the sender's current on-chain balance, without considering the cumulative cost of transactions already pending in the pool from the same sender. This allows a sender to submit multiple transactions whose total cost exceeds their balance, polluting the pool and wasting block space when only the first transaction can succeed.

---

## Problem

In `TransactionValidator::validate()` (lines 122-130 of `validator.rs`), the balance check queries the sender's balance from the QMDB state database and compares it to the maximum cost of the single incoming transaction. It does not query the transaction pool for other pending transactions from the same sender.

The `max_tx_cost()` function (lines 241-247 of `validator.rs`) computes `gas_limit * max_fee_per_gas + value` for a single transaction. The balance check passes if `balance >= max_cost` for that single transaction. Since each transaction is checked independently, a sender with 10 ETH can submit ten transactions each costing 9 ETH, and all ten will pass validation even though only the first can actually execute.

The pool does have nonce-based deduplication (lines 116-120 of `validator.rs` check `pool.has_nonce()`), which prevents two transactions with the same nonce from the same sender. But it does not prevent multiple transactions with sequential nonces that collectively exceed the sender's balance.

**File**: `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/validator.rs`

---

## Code Reference

The balance check in `validate()` (lines 122-130 of `validator.rs`):

```rust
// crates/node/txpool/src/validator.rs:122-130
let max_cost = max_tx_cost(&envelope);
let balance = self
    .state
    .balance(&sender)
    .await
    .map_err(|e| TxPoolError::StateError(e.to_string()))?;
if balance < max_cost {
    return Err(TxPoolError::InsufficientBalance { need: max_cost, have: balance });
}
// Does NOT consider other pending txs from the same sender!
```

The `max_tx_cost()` function (lines 241-247 of `validator.rs`):

```rust
// crates/node/txpool/src/validator.rs:241-247
fn max_tx_cost(envelope: &TxEnvelope) -> U256 {
    let gas_limit = U256::from(envelope.gas_limit());
    let max_fee = U256::from(effective_gas_price(envelope));
    let value = envelope.value();

    gas_limit * max_fee + value
}
```

The nonce deduplication check (lines 116-120 of `validator.rs`) -- this prevents same-nonce conflicts but not cumulative cost overflows:

```rust
// crates/node/txpool/src/validator.rs:116-120
if let Some(pool) = &self.pool
    && pool.has_nonce(&sender, nonce)
{
    return Err(TxPoolError::NonceAlreadyInPool { sender, nonce });
}
```

The pool's `build()` method (lines 677-715 of `pool.rs`) which selects transactions for block building -- it includes all valid-nonce transactions without a cumulative balance check:

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
    // ... selects transactions by gas price priority without cumulative balance check ...
}
```

---

## Impact

1. **Pool pollution**: Invalid transactions (those that will fail due to insufficient balance after earlier transactions execute) occupy pool space that could be used by legitimate transactions from other senders. With the default pool size, a single attacker can fill a significant fraction of the pool.

2. **Block space waste**: The block proposer includes all pending transactions. When executed in order, the first transaction succeeds and consumes the sender's balance. All subsequent transactions from that sender fail with "insufficient balance" during EVM execution. Each failing transaction still consumes the intrinsic gas (21,000 gas minimum), wasting block gas limit capacity.

3. **Denial of service via pool displacement**: An attacker can submit many high-gas-price transactions with sequential nonces, each valued at just under their balance. These transactions will have high effective gas prices, causing the pool's priority ordering to place them ahead of legitimate transactions. Only one will succeed; the rest waste block space and displace legitimate transactions.

Example attack:
- Sender has 10 ETH balance
- Sends tx0 (nonce=0): value=9 ETH, gas cost=1 ETH. Balance check: 10 >= 10. Passes.
- Sends tx1 (nonce=1): value=9 ETH, gas cost=1 ETH. Balance check: 10 >= 10. Passes.
- Sends tx2 (nonce=2): value=9 ETH, gas cost=1 ETH. Balance check: 10 >= 10. Passes.
- At execution: tx0 succeeds (balance becomes 0 ETH). tx1 and tx2 fail with insufficient balance.

---

## Root Cause

The balance check in the validator queries the base state (QMDB) for the sender's balance but does not track cumulative pending cost per sender. The `TransactionValidator` has an optional reference to the pool (for nonce dedup) but does not query the pool for total pending cost. This would require the pool to maintain per-sender cumulative cost tracking, which currently does not exist.

---

## Suggested Fix

Add cumulative pending cost tracking to the transaction pool and check it during validation:

1. **Track per-sender pending cost in the pool**:

```rust
// In pool.rs, add to the SenderQueue struct:
pub struct SenderQueue {
    pub pending: Vec<OrderedTransaction>,
    pub next_nonce: u64,
    pub pending_cost: U256,  // sum of max_tx_cost for all pending txs
}
```

2. **Add a method to query cumulative pending cost**:

```rust
impl TransactionPool {
    pub fn pending_cost(&self, sender: &Address) -> U256 {
        self.inner.read()
            .by_sender
            .get(sender)
            .map(|q| q.pending_cost)
            .unwrap_or(U256::ZERO)
    }
}
```

3. **Check cumulative cost in the validator**:

```rust
// In validator.rs, after the individual balance check:
if let Some(pool) = &self.pool {
    let pending_cost = pool.pending_cost(&sender);
    let total_cost = pending_cost + max_cost;
    if balance < total_cost {
        return Err(TxPoolError::InsufficientBalance { need: total_cost, have: balance });
    }
}
```

This mirrors Geth's approach, where the transaction pool maintains an "executable balance" per sender that decreases as pending transactions accumulate.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/pool.rs` -- Add `pending_cost` field to `SenderQueue`, maintain it during `add()` and `prune()`, expose via `pending_cost()` method
- `/Users/will/dev/nunchi/daeji/crates/node/txpool/src/validator.rs` -- Add cumulative cost check after the individual balance check (around line 130)

---

## Related Issues

- `025-tx-gossip-no-rate-limit.md` (transaction processing without rate limits)
