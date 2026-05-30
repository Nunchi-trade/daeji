# 080: Replacement Transactions Blocked by Validator Despite Pool Support

**Category:** txpool
**Severity:** medium

## Summary

The transaction pool has complete infrastructure for replacement transactions (submitting a new tx with the same nonce but higher gas price to "speed up" or "cancel" a pending tx), including a `ReplacementUnderpriced` error variant, a `replacement_bump_percent` configuration field (defaulting to 10%), and correct replacement logic in `SenderQueue::insert()`. However, the validator unconditionally rejects any transaction whose `(sender, nonce)` already exists in the pool with `NonceAlreadyInPool` *before* the pool's replacement logic is ever reached, making the replacement infrastructure dead code.

## Problem

Kora is a minimal Ethereum-compatible execution client with a transaction pool that queues pending transactions for inclusion in blocks. Transaction replacement is a standard Ethereum feature: users submit a new transaction with the same nonce as a pending one but with a higher gas price, causing the old transaction to be replaced. This is used by wallets like MetaMask for "speed up" and "cancel" operations.

### The validator blocks all same-nonce submissions

**File:** `crates/node/txpool/src/validator.rs`, lines 113-120

```rust
// Reject if the pool already contains a transaction from this sender
// with the same nonce.  This prevents same-nonce conflicts from
// passing validation when only finalized state is checked.
if let Some(pool) = &self.pool
    && pool.has_nonce(&sender, nonce)
{
    return Err(TxPoolError::NonceAlreadyInPool { sender, nonce });
}
```

The `has_nonce()` method performs a linear scan over the sender's pending and queued transactions:

**File:** `crates/node/txpool/src/pool.rs`, lines 508-513

```rust
pub fn has_nonce(&self, sender: &Address, nonce: u64) -> bool {
    let inner = self.inner.read();
    let Some(queue) = inner.by_sender.get(sender) else {
        return false;
    };
    queue.pending.iter().chain(queue.queued.iter()).any(|tx| tx.nonce == nonce)
}
```

### The pool's replacement logic works correctly but is unreachable

**File:** `crates/node/txpool/src/pool.rs`, lines 225-232

```rust
if let Some(replaced) = queue.insert(tx.clone()) {
    if replaced.hash == tx.hash {
        return Err(TxPoolError::ReplacementUnderpriced);
    }
    replaced_hash = Some(replaced.hash);
    inner.remove_by_hash(&replaced.hash);
    debug!(hash = ?replaced.hash, "replaced transaction");
}
```

When `SenderQueue::insert()` finds an existing transaction with the same nonce, it compares effective gas prices and either replaces it (returning the old tx) or returns the new tx unchanged (indicating underpricing). This logic works correctly but is never reached because the validator rejects the transaction first.

### The replacement bump percentage is dead code

**File:** `crates/node/txpool/src/config.rs`, lines 16-17

```rust
/// Percentage bump required for replacement transactions.
pub replacement_bump_percent: u8,
```

This field defaults to 10 and has a builder method `with_replacement_bump_percent()` (line 91), but the value is never read by the validator or pool during transaction processing. It is purely dead configuration.

## Code Reference

**File:** `crates/node/txpool/src/validator.rs:113-120`
```rust
// Reject if the pool already contains a transaction from this sender
// with the same nonce.  This prevents same-nonce conflicts from
// passing validation when only finalized state is checked.
if let Some(pool) = &self.pool
    && pool.has_nonce(&sender, nonce)
{
    return Err(TxPoolError::NonceAlreadyInPool { sender, nonce });
}
```

**File:** `crates/node/txpool/src/pool.rs:225-232`
```rust
if let Some(replaced) = queue.insert(tx.clone()) {
    if replaced.hash == tx.hash {
        return Err(TxPoolError::ReplacementUnderpriced);
    }
    replaced_hash = Some(replaced.hash);
    inner.remove_by_hash(&replaced.hash);
    debug!(hash = ?replaced.hash, "replaced transaction");
}
```

**File:** `crates/node/txpool/src/config.rs:16-17`
```rust
pub replacement_bump_percent: u8,  // defaults to 10, never read
```

## Impact

- **Users cannot replace stuck transactions:** A fundamental Ethereum UX expectation. If a transaction is stuck (e.g., gas price too low for inclusion), the user's only option is to wait for it to expire from the pool.
- **MetaMask "speed up" / "cancel" fails:** These wallet features submit replacement transactions with higher gas prices. Both operations fail with `NonceAlreadyInPool`.
- **Dead configuration misleads operators:** The `replacement_bump_percent` configuration field and its builder method suggest replacement is supported, but it is not.
- **Nonce locking:** If a user submits a transaction that lands in the pool but cannot be included (e.g., due to a temporarily high base fee), the nonce is locked until the transaction expires. No new transactions with that nonce can be submitted.

## Root Cause

The validator performs an unconditional rejection of same-nonce transactions without checking whether the new transaction qualifies as a valid replacement (higher gas price by at least the configured bump percentage). This appears to have been a simplification during initial implementation -- the comment says "This prevents same-nonce conflicts from passing validation when only finalized state is checked," suggesting the intent was to avoid validation complexity rather than to explicitly disable replacement.

## Suggested Fix

Modify the validator to allow same-nonce submissions when the new transaction's effective gas price exceeds the existing one by at least `replacement_bump_percent`:

```rust
// Before:
if let Some(pool) = &self.pool
    && pool.has_nonce(&sender, nonce)
{
    return Err(TxPoolError::NonceAlreadyInPool { sender, nonce });
}

// After:
if let Some(pool) = &self.pool
    && pool.has_nonce(&sender, nonce)
{
    // Check if this qualifies as a valid replacement
    let bump_percent = u128::from(self.config.replacement_bump_percent);
    if let Some(existing_price) = pool.gas_price_for_nonce(&sender, nonce) {
        let min_replacement_price = existing_price + (existing_price * bump_percent / 100);
        let new_price = effective_gas_price(&envelope);
        if new_price < min_replacement_price {
            return Err(TxPoolError::ReplacementUnderpriced);
        }
        // Allow replacement -- pool's insert() will handle the swap
    } else {
        return Err(TxPoolError::NonceAlreadyInPool { sender, nonce });
    }
}
```

This requires adding a `gas_price_for_nonce()` method to `TransactionPool` that returns the effective gas price of the existing transaction with the given nonce, and threading `replacement_bump_percent` from `PoolConfig` into the validator (it is already available in `self.config` since validator uses `PoolConfig`).

## Files to Modify

- `crates/node/txpool/src/validator.rs` (lines 113-120) -- conditional replacement check instead of unconditional rejection
- `crates/node/txpool/src/pool.rs` -- add `gas_price_for_nonce()` accessor method; existing `SenderQueue::insert()` replacement logic (lines 225-232) is already correct
- `crates/node/txpool/src/config.rs` (line 17) -- `replacement_bump_percent` field is already defined, just needs to be used

## Related Issues

None.

## Labels

bug, txpool, correctness
