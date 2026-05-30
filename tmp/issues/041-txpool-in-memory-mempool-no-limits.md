# 041: InMemoryMempool Has No Size Limits, Fee Ordering, or DoS Protection

**Category:** bug
**Severity:** medium

**Labels:** `bug`, `txpool`, `security`, `good first issue`

## Summary

The `InMemoryMempool` in the `kora_consensus` crate is a simple `BTreeMap<TxId, Tx>` with no size limits, no per-sender caps, and no fee-based eviction. It is a public API exported from the crate. While it is not used in the production validator path (which uses `LedgerMempool` backed by `TransactionPool`), its availability as public API means it could be accidentally wired into a production configuration.

## Problem

The `InMemoryMempool` struct implements the `Mempool` trait but lacks all DoS protections that the production `TransactionPool` provides. Specifically:

1. **No size limits**: The `insert()` method unconditionally inserts into the `BTreeMap` with no cap on total entries.
2. **No per-sender limits**: Any single sender can insert unlimited transactions.
3. **No fee-based ordering**: The `build()` method sorts by `(valid_decode_flag, sender, nonce)` -- meaning transactions are ordered by sender address and nonce, not by gas price or priority fee.
4. **No eviction**: When the pool grows large, no low-value transactions are removed.

The production path uses `LedgerMempool` (defined in `crates/node/ledger/src/lib.rs:37-39`), which wraps `TransactionPool` with full eviction, ordering, and capacity limits. However, `InMemoryMempool` is exported from the `kora_consensus` crate's public API and could be used by tests or alternative configurations.

## Code Reference

`crates/node/consensus/src/components/mempool.rs:15-71`:

```rust
pub struct InMemoryMempool {
    inner: Arc<RwLock<BTreeMap<TxId, Tx>>>,
}

impl Mempool for InMemoryMempool {
    fn insert(&self, tx: Tx) -> bool {
        let id = tx.id();
        let mut inner = self.inner.write();
        inner.insert(id, tx).is_none()  // No size check, no validation
    }

    fn build(&self, max_txs: usize, excluded: &std::collections::BTreeSet<TxId>) -> Vec<Tx> {
        let inner = self.inner.read();
        let mut candidates: Vec<_> = inner
            .iter()
            .filter(|(id, _)| !excluded.contains(id))
            .map(|(id, tx)| (tx_order_key(tx), *id, tx.clone()))
            .collect();
        candidates.sort_by_key(|(order, id, _)| (*order, *id));
        candidates.into_iter().take(max_txs).map(|(_, _, tx)| tx).collect()
    }

    fn prune(&self, tx_ids: &[TxId]) {
        let mut inner = self.inner.write();
        for id in tx_ids {
            inner.remove(id);
        }
    }

    fn len(&self) -> usize {
        self.inner.read().len()
    }
}
```

The ordering function at line 33-41 sorts by sender address and nonce, not by fee:

```rust
fn tx_order_key(tx: &Tx) -> (u8, Address, u64) {
    let Ok(envelope) = TxEnvelope::decode_2718(&mut tx.bytes.as_ref()) else {
        return (1, Address::ZERO, u64::MAX);
    };
    let Ok(sender) = envelope.recover_signer() else {
        return (1, Address::ZERO, u64::MAX);
    };
    (0, sender, envelope.nonce())
}
```

## Impact

If `InMemoryMempool` is accidentally used in a production configuration (e.g., during a refactoring that changes the wiring in `runner.rs`), the node would have zero DoS resistance in its transaction pool. Any peer could submit unlimited transactions, exhausting memory. Even in test configurations, using this mempool masks issues that would occur with the production pool's eviction and ordering behavior, reducing test fidelity.

## Root Cause

The `InMemoryMempool` was likely created as a minimal implementation for early development or testing, before the full `TransactionPool` was built. It was never removed or restricted to test-only scope.

## Suggested Fix

**Option 1 (preferred):** Gate `InMemoryMempool` behind `#[cfg(test)]` so it cannot be used in release builds:

```rust
#[cfg(test)]
pub struct InMemoryMempool { ... }
```

**Option 2:** Add a clear documentation warning and rename it to `TestOnlyMempool` or `UnboundedMempool` to make its limitations obvious:

```rust
/// WARNING: This mempool has NO size limits, NO fee ordering, and NO DoS
/// protection. It is intended for testing only. For production use,
/// see `LedgerMempool` backed by `TransactionPool`.
#[deprecated(note = "Use LedgerMempool for production")]
pub struct InMemoryMempool { ... }
```

**Option 3:** Remove it entirely if it is not used by any tests. The `TransactionPool` can be used in tests with relaxed configuration.

## Files to Modify

- `crates/node/consensus/src/components/mempool.rs` -- `InMemoryMempool` definition (lines 15-71)
- `crates/node/consensus/src/components/mod.rs` -- module re-exports (if `InMemoryMempool` is re-exported)

## Related Issues

- `042-txpool-eviction-cascades-to-high-fee-txs.md` -- Eviction strategy issues in the production pool
- `044-txpool-min-gas-price-static-not-tracking-base-fee.md` -- Static minimum gas price in pool config
