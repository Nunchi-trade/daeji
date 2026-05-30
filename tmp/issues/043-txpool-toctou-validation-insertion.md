# 043: TOCTOU Race Between Transaction Validation and Pool Insertion

**Category:** bug
**Severity:** low

**Labels:** `bug`, `txpool`, `correctness`

## Summary

Transaction submission follows a two-step process: first validate (nonce, balance, signature checks), then insert into the pool. Between these two steps, another concurrent task could insert a conflicting transaction with the same sender and nonce, causing the second insertion to fail with a confusing error. This is a classic time-of-check-time-of-use (TOCTOU) race condition.

## Problem

Both the RPC submission path and the gossip handler path follow the same validate-then-insert pattern. The validator checks pool state (including whether a nonce already exists in the pool) during validation, but the pool state can change before insertion occurs because multiple tasks process transactions concurrently.

In the **gossip path** at `crates/node/runner/src/runner.rs:1112-1125`:

```rust
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
    // accepted
```

In the **RPC path** at `crates/node/runner/src/runner.rs:1211-1219`:

```rust
let state = ledger.latest_state().await;
let validator =
    TransactionValidator::new(chain_id, state, PoolConfig::default())
        .with_pool(pool);
validator.validate(tx.clone()).await.map_err(|err| {
    warn!(?tx_id, error = %err, "rpc submit: validator rejected tx");
    kora_rpc::RpcError::InvalidTransaction(err.to_string())
})?;
if ledger.submit_tx(tx).await {
    // accepted
```

The nonce-in-pool check happens during validation at `crates/node/txpool/src/validator.rs:116-120`:

```rust
// Reject if the pool already contains a transaction from this sender
// with the same nonce.
if let Some(pool) = &self.pool
    && pool.has_nonce(&sender, nonce)
{
    return Err(TxPoolError::NonceAlreadyInPool { sender, nonce });
}
```

The `has_nonce()` check reads the pool state under a read lock at `crates/node/txpool/src/pool.rs:508`. Between this read and the subsequent `submit_tx()` call, another task could insert a transaction with the same sender+nonce.

## Impact

This is a **minor correctness issue** -- no invalid transactions enter the pool. The race only affects error reporting: a client may receive a `NonceAlreadyInPool` or "transaction rejected by mempool" error even though their submission was the first they sent for that nonce. This is confusing for wallet software that may retry on certain error types.

Under normal load, the race window is very small. Under high gossip traffic with multiple validators forwarding the same transaction, collisions become more frequent but remain harmless -- the transaction is only inserted once.

## Root Cause

The validate-then-insert pattern is inherently racy when multiple tasks can submit transactions concurrently. The validator reads pool state under one lock acquisition, but the pool state can change before the insertion acquires its own write lock. This is a classic TOCTOU (time-of-check-time-of-use) pattern.

## Suggested Fix

**Option 1 (recommended):** Accept the race and improve error handling. Have the `submit_tx` path return a specific error variant (e.g., `ConcurrentInsert`) that the RPC layer translates to a user-friendly message like "transaction may already be pending." This is the lightest-touch fix and recognizes that the race is benign.

**Option 2:** Make validate-and-insert atomic by combining them into a single locked operation:

```rust
impl TransactionPool {
    pub fn validate_and_add<S: StateDbRead>(
        &self,
        tx: Tx,
        chain_id: u64,
        state: S,
        config: &PoolConfig,
    ) -> Result<(), TxPoolError> {
        let mut inner = self.inner.write();
        // Validate under the write lock
        // Insert immediately after validation
    }
}
```

This eliminates the race but increases lock contention since validation (including signature recovery) would hold the pool's write lock.

**Option 3:** Use an optimistic approach -- attempt insertion first and only validate if the nonce is new. This requires restructuring the pool's `add()` method to perform validation internally.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- gossip handler (line 1112) and RPC handler (line 1211)
- `crates/node/txpool/src/validator.rs` -- nonce check at line 116
- `crates/node/txpool/src/pool.rs` -- `has_nonce()` at line 508, potentially add `validate_and_add()`

## Related Issues

- `048-p2p-per-tx-state-fetch-gossip-handler.md` -- Per-transaction state fetch in gossip handler (same code path)
