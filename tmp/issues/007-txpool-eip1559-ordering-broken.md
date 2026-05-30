# 007: Transaction Pool Sorts by max_fee_per_gas Instead of Effective Tip (EIP-1559 Broken)

**Category:** bug / txpool
**Severity:** critical
**Labels:** bug, correctness, txpool

---

## Summary

The transaction pool's `effective_gas_price()` function returns `max_fee_per_gas` for EIP-1559 transactions instead of computing the actual effective gas price (`min(max_fee_per_gas, base_fee + max_priority_fee_per_gas)`). This value is used for both pool ordering (which transactions get included first in blocks) and fee-based eviction (which transactions get dropped when the pool is full). An `effective_tip()` method exists that correctly computes the priority fee, but it is never called in any ordering or eviction logic.

## Problem

The `effective_gas_price()` function at `crates/node/txpool/src/validator.rs:194-202` is defined as `const fn`, which structurally prevents passing a dynamic `base_fee` parameter. It returns `max_fee_per_gas` for EIP-1559, EIP-4844, and EIP-7702 transactions:

```rust
const fn effective_gas_price(envelope: &TxEnvelope) -> u128 {
    match envelope {
        TxEnvelope::Legacy(tx) => tx.tx().gas_price,
        TxEnvelope::Eip2930(tx) => tx.tx().gas_price,
        TxEnvelope::Eip1559(tx) => tx.tx().max_fee_per_gas, // WRONG: should be effective_gas_price
        TxEnvelope::Eip4844(tx) => tx.tx().tx().max_fee_per_gas,
        TxEnvelope::Eip7702(tx) => tx.tx().max_fee_per_gas,
    }
}
```

This same incorrect logic is duplicated in `tx_to_ordered()` at `crates/node/txpool/src/pool.rs:636-657`, which is used by the `Mempool::insert()` path (consensus-delivered transactions that bypass RPC validation):

```rust
fn tx_to_ordered(tx: &Tx) -> Option<OrderedTransaction> {
    // ...
    let effective_gas_price = match &envelope {
        TxEnvelope::Legacy(tx) => tx.tx().gas_price,
        TxEnvelope::Eip2930(tx) => tx.tx().gas_price,
        TxEnvelope::Eip1559(tx) => tx.tx().max_fee_per_gas,   // Same bug
        TxEnvelope::Eip4844(tx) => tx.tx().tx().max_fee_per_gas,
        TxEnvelope::Eip7702(tx) => tx.tx().max_fee_per_gas,
    };
    // ...
}
```

The `Ord` implementation for `OrderedTransaction` at `crates/node/txpool/src/ordering.rs:59-67` sorts by `effective_gas_price` (which contains the wrong value):

```rust
impl Ord for OrderedTransaction {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .effective_gas_price
            .cmp(&self.effective_gas_price)      // Sorts by max_fee, not tip
            .then_with(|| self.timestamp.cmp(&other.timestamp))
            .then_with(|| self.hash.cmp(&other.hash))
    }
}
```

An `effective_tip()` method exists at `crates/node/txpool/src/ordering.rs:39-42` that correctly computes the priority fee, but it is **never called** anywhere in the codebase:

```rust
pub fn effective_tip(&self, base_fee: Option<u128>) -> u128 {
    base_fee
        .map_or(self.effective_gas_price, |base| self.effective_gas_price.saturating_sub(base))
}
```

Note that even `effective_tip()` would give wrong results because it subtracts `base_fee` from `self.effective_gas_price`, but `self.effective_gas_price` is already `max_fee_per_gas` (not the true effective gas price). With the correct `effective_gas_price`, the tip calculation would work.

## Code Reference

See the code blocks above. The key locations are:

- `crates/node/txpool/src/validator.rs:194-202` -- `effective_gas_price()` returns `max_fee_per_gas` for EIP-1559
- `crates/node/txpool/src/pool.rs:636-657` -- `tx_to_ordered()` duplicates the same incorrect logic
- `crates/node/txpool/src/ordering.rs:59-67` -- `Ord::cmp()` sorts by `effective_gas_price` instead of tip
- `crates/node/txpool/src/ordering.rs:39-42` -- `effective_tip()` exists but is never called

## Impact

1. **Validators lose revenue**: Transactions with high tips are deprioritized in favor of transactions with high `max_fee_per_gas` but low `max_priority_fee_per_gas`, reducing fee income.

2. **User experience degraded**: Users paying high priority fees to expedite transactions see no benefit. A user who sets `max_fee=100 gwei, priority_fee=1 gwei` is ordered ahead of a user who sets `max_fee=50 gwei, priority_fee=50 gwei`, even though the latter is willing to pay a 50x higher tip.

3. **Incorrect eviction**: When the pool is full, high-tip transactions may be evicted in favor of low-tip transactions that merely have a higher `max_fee_per_gas`. The `SenderQueue::insert()` method at `crates/node/txpool/src/ordering.rs:91-127` uses `effective_gas_price` for replacement decisions.

4. **DEX/MEV implications**: In fee-competitive scenarios (DEX arbitrage, liquidations), incorrect ordering means the wrong transactions win.

Per the Ethereum specification (EIP-1559), the correct effective gas price is:
```
effective_gas_price = min(max_fee_per_gas, base_fee + max_priority_fee_per_gas)
```
And ordering should be by priority fee (tip):
```
priority_fee = effective_gas_price - base_fee
```

## Root Cause

The `effective_gas_price()` function was implemented as a simplified `const fn` that returns `max_fee_per_gas` rather than performing the EIP-1559 calculation. The `const fn` constraint prevents passing a dynamic `base_fee` parameter, and the function was never updated when EIP-1559 support was added.

## Suggested Fix

1. Remove the `const` qualifier from `effective_gas_price()` and pass the current base fee:

```rust
fn effective_gas_price(envelope: &TxEnvelope, base_fee: u128) -> u128 {
    match envelope {
        TxEnvelope::Legacy(tx) => tx.tx().gas_price,
        TxEnvelope::Eip2930(tx) => tx.tx().gas_price,
        TxEnvelope::Eip1559(tx) => {
            let tx = tx.tx();
            std::cmp::min(
                tx.max_fee_per_gas,
                base_fee + tx.max_priority_fee_per_gas,
            )
        }
        TxEnvelope::Eip4844(tx) => {
            let tx = tx.tx().tx();
            std::cmp::min(
                tx.max_fee_per_gas,
                base_fee + tx.max_priority_fee_per_gas,
            )
        }
        TxEnvelope::Eip7702(tx) => {
            let tx = tx.tx();
            std::cmp::min(
                tx.max_fee_per_gas,
                base_fee + tx.max_priority_fee_per_gas,
            )
        }
    }
}
```

2. Update `OrderedTransaction::cmp()` to sort by `effective_tip(Some(base_fee))` instead of `effective_gas_price`.

3. Thread the current base fee through the pool's ordering and eviction logic.

4. Fix the duplicate in `tx_to_ordered()` at `pool.rs:636-657` to use the same corrected function.

## Files to Modify

- `crates/node/txpool/src/validator.rs:194-202` -- Fix `effective_gas_price()` to compute EIP-1559 effective price
- `crates/node/txpool/src/pool.rs:636-657` -- Fix `tx_to_ordered()` duplicate logic
- `crates/node/txpool/src/ordering.rs:59-67` -- Update `Ord::cmp()` to sort by tip, not max_fee
- `crates/node/txpool/src/ordering.rs:10-23` -- Add base_fee field to `OrderedTransaction`

## Related Issues

None directly, but this interacts with the EIP-1559 base fee computation in `crates/node/runner/src/app.rs:215-226` (`compute_base_fee()`).
