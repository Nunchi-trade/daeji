# 073: Missing `newHeads` and `logs` WebSocket Subscriptions

**Category:** rpc
**Severity:** medium

## Summary

Kora's `eth_subscribe` WebSocket handler only supports the `newPendingTransactions` subscription kind. The two other standard Ethereum WebSocket subscription kinds -- `newHeads` (stream block headers on each new block) and `logs` (stream contract event logs matching a filter) -- are explicitly rejected with error code `-32004`. These subscriptions are required by virtually all Ethereum dApp frontends, indexing infrastructure, and developer tooling.

## Problem

Kora is a minimal Ethereum-compatible execution client. Its RPC layer includes WebSocket support with an `eth_subscribe` endpoint, but only the `newPendingTransactions` subscription kind is implemented. When a client attempts to subscribe to `newHeads` or `logs`, the handler rejects the request immediately.

**File:** `crates/node/rpc/src/subscription.rs`, lines 77-80

```rust
if kind != "newPendingTransactions" {
    pending.reject(unsupported_subscription("eth", &kind)).await;
    return;
}
```

The rejection is handled by a utility function at line 224:

```rust
fn unsupported_subscription(namespace: &str, kind: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        codes::METHOD_NOT_SUPPORTED,
        format!("{namespace}_subscribe does not support {kind:?}"),
        None::<()>,
    )
}
```

The existing infrastructure for `newPendingTransactions` is well-structured. It uses a `broadcast::Sender` channel pattern with backpressure handling via a helper function:

**File:** `crates/node/rpc/src/subscription.rs`, lines 209-222

```rust
async fn recv_broadcast<T>(receiver: &mut broadcast::Receiver<T>, subscription: &str) -> Option<T>
where
    T: Clone,
{
    loop {
        match receiver.recv().await {
            Ok(event) => return Some(event),
            Err(RecvError::Lagged(skipped)) => {
                warn!(subscription, skipped, "subscription receiver lagged; skipping events");
            }
            Err(RecvError::Closed) => return None,
        }
    }
}
```

This pattern can be directly reused for `newHeads` and `logs` subscriptions.

## Code Reference

**File:** `crates/node/rpc/src/subscription.rs:77-80`
```rust
if kind != "newPendingTransactions" {
    pending.reject(unsupported_subscription("eth", &kind)).await;
    return;
}
```

## Impact

- **ethers.js** `provider.on("block", ...)` uses `newHeads` under the hood -- fails on Kora
- **viem** `watchContractEvent` and `watchBlocks` both require WebSocket subscriptions -- fail on Kora
- **The Graph** and other indexers subscribe to `logs` for real-time event processing -- must fall back to less efficient polling
- **MetaMask** falls back to polling `eth_blockNumber` when `newHeads` is unavailable, resulting in higher latency for chain-tip tracking
- Any dApp using `contract.on("Event", ...)` over WebSocket will fail with error `-32004`
- Block explorers and indexers that subscribe to logs must poll `eth_getFilterChanges` instead (less efficient, higher latency)

## Root Cause

Only the `newPendingTransactions` subscription kind was implemented in the initial RPC build-out. The infrastructure (broadcast channels, backpressure handling, serialization) is already complete and well-structured, so adding the other two kinds is primarily a wiring exercise.

## Suggested Fix

### `newHeads`
1. Create a `NewHeadEventSender = broadcast::Sender<RpcBlock>` channel (analogous to the existing `PendingTxEventSender`)
2. In the runner (`crates/node/runner/src/runner.rs`), broadcast a stripped-down `RpcBlock` (header fields only) each time a block is finalized
3. In `subscription.rs`, add a `"newHeads"` match arm that subscribes to this channel and streams block headers to the WebSocket client

### `logs`
1. Create a `LogEventSender = broadcast::Sender<RpcLog>` channel
2. In the runner, broadcast each log from each finalized block's receipts
3. In `subscription.rs`, add a `"logs"` match arm that accepts an optional filter object (address list, topic filters) and streams only matching logs

### Example code change in subscription.rs:

```rust
// Before:
if kind != "newPendingTransactions" {
    pending.reject(unsupported_subscription("eth", &kind)).await;
    return;
}

// After:
match kind.as_str() {
    "newPendingTransactions" => { /* existing implementation */ }
    "newHeads" => { /* subscribe to new_head_sender, stream RpcBlock headers */ }
    "logs" => { /* subscribe to log_sender, apply filter, stream matching RpcLog */ }
    _ => {
        pending.reject(unsupported_subscription("eth", &kind)).await;
        return;
    }
}
```

## Files to Modify

- `crates/node/rpc/src/subscription.rs` (lines 77-80) -- add `newHeads` and `logs` match arms
- `crates/node/rpc/src/subscription.rs` (lines 209-222) -- existing broadcast pattern to replicate
- `crates/node/runner/src/runner.rs` -- create and wire broadcast channels for new heads and logs

## Related Issues

- `078-rpc-missing-standard-methods.md` -- related RPC completeness gaps

## Labels

enhancement, rpc
