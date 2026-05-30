# `send_raw_transaction` Returns Success But Silently Drops Transaction When Submit Callback Is Missing

**Category**: bug
**Severity**: high
**Labels**: bug, reliability, rpc

## Summary

When the `tx_submit` callback is `None` (indicating the node is not configured to submit transactions to the mempool), the `send_raw_transaction` RPC method still decodes the transaction, inserts it into the in-memory `pending_txs` map, and returns the transaction hash to the caller as if the transaction was accepted. However, the transaction is never broadcast to the consensus pipeline or other nodes. It silently sits in the pending map until evicted. This is a data loss path that returns a success response to the caller.

## Problem

In `crates/node/rpc/src/eth.rs`, the `send_raw_transaction` method (lines 504-560) handles the case where `self.tx_submit` is `None` by setting `accepted = false` at line 512, which prevents the transaction from being broadcast to the pending-transaction subscription channel at line 556-558. More critically, the transaction is never submitted to the mempool or consensus pipeline at all.

Despite this, the method continues to:
1. Insert the transaction into the `pending_txs` map (line 518)
2. Add it to the `pending_tx_order` deque (line 519)
3. Return `Ok(tx_hash)` to the caller (line 559)

The caller receives a success response with a valid transaction hash, and has no way to know that the transaction will never be mined. The transaction will remain in the `pending_txs` map (consuming memory) until it is evicted by newer transactions when the map exceeds `max_pending_txs`.

This behavior is documented in the test suite: the test `send_raw_transaction_with_no_callback_silently_accepts_but_drops` at line 2427 explicitly tests and documents this silent-drop behavior, calling it a "failure mode."

## Code Reference

**File**: `crates/node/rpc/src/eth.rs`, lines 504-560

```rust
async fn send_raw_transaction(&self, data: Bytes) -> RpcResult<B256> {
    let tx_hash = alloy_primitives::keccak256(&data);
    let pending_tx = raw_tx_to_pending_rpc(&data)?;

    let accepted = if let Some(ref submit) = self.tx_submit {
        submit(data).await?;
        true
    } else {
        false                    // <-- tx_submit is None, transaction goes nowhere
    };

    {
        let mut txs = self.pending_txs.write().await;
        let mut order = self.pending_tx_order.write().await;
        txs.insert(tx_hash, pending_tx.clone());    // <-- inserted even when not accepted
        order.push_back(tx_hash);                     // <-- tracked even when not accepted

        // ... eviction logic for when pending_txs exceeds cap ...
    }

    if accepted {
        self.broadcast_pending_tx(tx_hash, pending_tx);  // <-- skipped when tx_submit is None
    }
    Ok(tx_hash)             // <-- returns success regardless of whether tx was actually submitted
}
```

## Impact

- **Silent transaction loss**: Users submitting transactions to a misconfigured node (e.g., a read-only secondary node, an RPC gateway, or a node where the tx_submit callback failed to initialize) will receive a success response. They will then wait indefinitely for the transaction to be mined, not realizing it was silently dropped.
- **Difficult to diagnose**: The transaction hash appears valid. The transaction will show up in `eth_getTransactionByHash` (from the pending_txs map) temporarily, further misleading the user into thinking it is being processed. Eventually it will be evicted and disappear entirely.
- **Memory waste**: Dropped transactions consume memory in the `pending_txs` map until evicted, even though they will never be processed.
- **Production risk**: This scenario is not hypothetical. In Kora's architecture, secondary (non-validator) nodes may not have a tx_submit callback configured. If such a node exposes the `eth_sendRawTransaction` RPC endpoint, users connecting to it will silently lose transactions.

## Root Cause

The `send_raw_transaction` method does not fail-fast when `tx_submit` is `None`. The control flow unconditionally inserts the transaction into `pending_txs` and returns the hash, regardless of whether the transaction can actually be submitted to the consensus pipeline. The method should either reject the transaction with an error or not insert it into `pending_txs` when submission is not possible.

## Suggested Fix

Return an error immediately when `tx_submit` is `None`, before inserting the transaction into any data structure:

**Before:**
```rust
async fn send_raw_transaction(&self, data: Bytes) -> RpcResult<B256> {
    let tx_hash = alloy_primitives::keccak256(&data);
    let pending_tx = raw_tx_to_pending_rpc(&data)?;

    let accepted = if let Some(ref submit) = self.tx_submit {
        submit(data).await?;
        true
    } else {
        false
    };
    // ... inserts into pending_txs regardless ...
    Ok(tx_hash)
}
```

**After:**
```rust
async fn send_raw_transaction(&self, data: Bytes) -> RpcResult<B256> {
    let submit = self.tx_submit.as_ref().ok_or_else(|| {
        RpcError::Internal("transaction submission not available on this node".to_string())
    })?;

    let tx_hash = alloy_primitives::keccak256(&data);
    let pending_tx = raw_tx_to_pending_rpc(&data)?;

    submit(data).await?;

    {
        let mut txs = self.pending_txs.write().await;
        let mut order = self.pending_tx_order.write().await;
        txs.insert(tx_hash, pending_tx.clone());
        order.push_back(tx_hash);
        // ... eviction logic ...
    }

    self.broadcast_pending_tx(tx_hash, pending_tx);
    Ok(tx_hash)
}
```

The existing test `send_raw_transaction_with_no_callback_silently_accepts_but_drops` (line 2427) should be updated to verify the error response instead of the silent-drop behavior.

## Files to Modify

- `crates/node/rpc/src/eth.rs` -- lines 504-560, add early error when `tx_submit` is `None`

## Related Issues

- `046-p2p-production-runner-uses-local-transport.md` -- related misconfiguration issue where the production runner might not have proper P2P setup
- `078-rpc-missing-standard-methods.md` -- broader RPC completeness issues
