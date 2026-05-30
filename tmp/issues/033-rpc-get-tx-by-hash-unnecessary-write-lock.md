# get_transaction_by_hash Acquires Unnecessary Write Lock on Every Confirmed Lookup

**Category**: Performance -- RPC
**Severity**: Medium
**Labels**: `performance`, `rpc`

## Summary

The `get_transaction_by_hash` RPC handler acquires a write lock on the `pending_txs` map every time a transaction is found in the indexed (confirmed) store, even when the transaction was never in the pending pool. This write lock serializes all concurrent `get_transaction_by_hash` calls with `send_raw_transaction`, creating unnecessary contention under RPC load.

## Problem

Kora is an EVM execution client that serves Ethereum-compatible JSON-RPC. The `EthApiImpl` struct maintains an in-memory `pending_txs: Arc<RwLock<HashMap<B256, RpcTransaction>>>` map that tracks unconfirmed transactions submitted via `send_raw_transaction`. When `get_transaction_by_hash` finds a transaction in the confirmed (indexed) store, it optimistically removes the hash from the pending pool to keep it clean -- but it does this unconditionally, using a write lock, even if the hash was never in the pending pool.

The `pending_txs` map uses a `tokio::sync::RwLock`, meaning write lock acquisition is an async operation that serializes with all other readers and writers. Every confirmed-tx lookup blocks all concurrent `send_raw_transaction` calls (which also take a write lock to insert new entries) and vice versa.

Additionally, there is a separate `pending_tx_order: Arc<RwLock<VecDeque<B256>>>` deque that tracks insertion order for filter support. When `get_transaction_by_hash` removes a hash from `pending_txs`, it does not remove the corresponding entry from `pending_tx_order`, causing the deque to accumulate stale entries over time.

## Code Reference

`crates/node/rpc/src/eth.rs:599-607`:
```rust
async fn get_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<RpcTransaction>> {
    let provider = self.state_provider.read().await;
    let indexed = provider.transaction_by_hash(hash).await?;
    if indexed.is_some() {
        self.pending_txs.write().await.remove(&hash);  // Write lock on EVERY confirmed hit
        return Ok(indexed);
    }
    Ok(self.pending_txs.read().await.get(&hash).cloned())
}
```

The `pending_txs` field definition in `crates/node/rpc/src/eth.rs:287`:
```rust
pending_txs: Arc<RwLock<HashMap<B256, RpcTransaction>>>,
```

The `send_raw_transaction` handler also takes a write lock on `pending_txs` (`crates/node/rpc/src/eth.rs:516-517`):
```rust
let mut txs = self.pending_txs.write().await;
let mut order = self.pending_tx_order.write().await;
txs.insert(tx_hash, pending_tx.clone());
order.push_back(tx_hash);
```

## Impact

Under RPC load typical of block explorers, indexers, and wallet UIs that poll `eth_getTransactionByHash` for confirmation status, every successful lookup of a confirmed transaction triggers a write lock acquisition. This serializes with all `send_raw_transaction` calls. On the devnet producing 34 blocks/s, this creates a serialization bottleneck on the RPC hot path.

Concrete scenario: A block explorer polls 100 recently confirmed transactions per second via `eth_getTransactionByHash`. Each poll acquires a write lock on `pending_txs`, even though none of those transactions were ever in the pending pool (they were submitted by other nodes). Meanwhile, users submitting transactions via `send_raw_transaction` experience increased latency because each submission must wait for the write lock.

## Root Cause

The implementation optimistically removes every confirmed hash from the pending pool using a write lock, without first checking whether the hash is actually present. A read-first-then-write or skip-cleanup approach would avoid the lock contention in the common case where the hash was never pending.

## Suggested Fix

**Option 1** (recommended): Check with a read lock first, only acquire write lock when needed:

```rust
async fn get_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<RpcTransaction>> {
    let provider = self.state_provider.read().await;
    let indexed = provider.transaction_by_hash(hash).await?;
    if indexed.is_some() {
        if self.pending_txs.read().await.contains_key(&hash) {
            self.pending_txs.write().await.remove(&hash);
        }
        return Ok(indexed);
    }
    Ok(self.pending_txs.read().await.get(&hash).cloned())
}
```

**Option 2**: Skip the cleanup entirely. The pending pool already has a capped eviction mechanism (`MAX_PENDING_TXS = 10,000` at `crates/node/rpc/src/eth.rs:45`) with a deque-based FIFO eviction policy in `send_raw_transaction` (`crates/node/rpc/src/eth.rs:526-540`), so pending entries for confirmed transactions will eventually be evicted naturally.

## Files to Modify

- `crates/node/rpc/src/eth.rs` -- `get_transaction_by_hash()` method (lines 599-607)

## Related Issues

None.
