# get_logs Holds Two RwLock Read Guards Across Full Block Range Iteration

**Category**: Performance -- RPC
**Severity**: Medium
**Labels**: `performance`, `rpc`

## Summary

The `BlockIndex::get_logs()` method in the indexer acquires read locks on two separate data structures (`blocks_by_number` and `logs_by_block`) and holds both for the entire duration of a potentially large block range iteration. This blocks all write operations (block insertions from the finalization pipeline) for the duration of the query, causing write starvation under concurrent RPC load.

## Problem

Kora is an EVM execution client built on commonware simplex BFT consensus. Its RPC layer serves Ethereum-compatible JSON-RPC requests, including `eth_getLogs`. When a client calls `eth_getLogs`, the request flows through the RPC handler (`EthApiImpl::get_logs` in `crates/node/rpc/src/eth.rs`) to the `IndexedStateProvider::get_logs` method (`crates/node/rpc/src/indexed_provider.rs:198`), which delegates to `BlockIndex::get_logs()` in the in-memory indexer (`crates/storage/indexer/src/store.rs:218-246`).

The `BlockIndex::get_logs()` method acquires `parking_lot::RwLock` read guards on two internal data structures at the top of the function and holds them for the entire loop, which can iterate over up to 10,000 blocks (the RPC layer caps the range via `MAX_LOG_BLOCK_RANGE` in `crates/node/rpc/src/indexed_provider.rs:30`).

During the loop, any call to `BlockIndex::insert_block()` (`crates/storage/indexer/src/store.rs:56-113`) is blocked because it needs write locks on the same `blocks_by_number` and `logs_by_block` maps. Since `parking_lot::RwLock` is a synchronous lock, the blocking happens directly on the async worker thread.

## Code Reference

`crates/storage/indexer/src/store.rs:218-246`:
```rust
pub fn get_logs(&self, filter: &LogFilter) -> Vec<IndexedLog> {
    let head = self.head_block_number();
    let from_block = filter.from_block.unwrap_or(0);
    let to_block = filter.to_block.unwrap_or(head).min(head);

    let mut result = Vec::new();

    let blocks_by_number = self.blocks_by_number.read();  // Lock 1 acquired
    let logs_by_block = self.logs_by_block.read();          // Lock 2 acquired

    for block_num in from_block..=to_block {  // Up to 10,000 iterations
        let Some(block_hash) = blocks_by_number.get(&block_num) else {
            continue;
        };

        let Some(logs) = logs_by_block.get(block_hash) else {
            continue;
        };

        for log in logs {
            if !Self::matches_filter(log, filter) {
                continue;
            }
            result.push(log.clone());
        }
    }

    result
    // Both locks released here when function returns
}
```

The `insert_block` method that contends with this takes write locks on the same maps:

`crates/storage/indexer/src/store.rs:72-99`:
```rust
{
    let mut blocks_by_hash = self.blocks_by_hash.write();
    blocks_by_hash.insert(block_hash, block);
}

{
    let mut blocks_by_number = self.blocks_by_number.write();   // Blocked by get_logs reader
    blocks_by_number.insert(block_number, block_hash);
}
// ...
{
    let mut logs_by_block = self.logs_by_block.write();         // Blocked by get_logs reader
    logs_by_block.insert(block_hash, all_logs);
}
```

The RPC handler acquires the state_provider read lock before calling into the indexer:

`crates/node/rpc/src/eth.rs:714-717`:
```rust
async fn get_logs(&self, filter: RpcLogFilter) -> RpcResult<Vec<RpcLog>> {
    let provider = self.state_provider.read().await;
    provider.get_logs(filter).await.map_err(Into::into)
}
```

## Impact

On a chain producing 34 blocks per second, a single `eth_getLogs` query spanning the full 10,000-block range holds both read locks for a measurable duration (proportional to the number of matching logs and blocks). During that window, all write operations on the indexer -- specifically block insertions from the finalization pipeline -- are stalled. Multiple concurrent `get_logs` queries from block explorers or indexing services compound the problem since `parking_lot::RwLock` read guards are shared but still prevent writes.

Concrete scenario: A block explorer polls `eth_getLogs` with a wide range filter to catch up on events. While this query runs, the node's finalization pipeline cannot insert new blocks into the indexer. If the query takes longer than one block time (~30ms at 34 blocks/s), block insertions queue up. Under sustained RPC load, this can cause the indexer to fall behind the consensus layer.

## Root Cause

The method acquires both read locks at the top of the function scope and holds them until the function returns, rather than acquiring and releasing them per-block or per-batch. This is a design choice that prioritizes snapshot consistency (the caller sees a consistent view across all blocks in the range) over write throughput.

## Suggested Fix

Process the block range in batches of 100-200 blocks, releasing and re-acquiring both locks between batches. This trades off strict snapshot consistency for improved write throughput:

```rust
pub fn get_logs(&self, filter: &LogFilter) -> Vec<IndexedLog> {
    let head = self.head_block_number();
    let from_block = filter.from_block.unwrap_or(0);
    let to_block = filter.to_block.unwrap_or(head).min(head);

    let mut result = Vec::new();
    let batch_size = 200;
    let mut current = from_block;

    while current <= to_block {
        let batch_end = (current + batch_size - 1).min(to_block);
        let blocks_by_number = self.blocks_by_number.read();
        let logs_by_block = self.logs_by_block.read();

        for block_num in current..=batch_end {
            let Some(block_hash) = blocks_by_number.get(&block_num) else {
                continue;
            };
            let Some(logs) = logs_by_block.get(block_hash) else {
                continue;
            };
            for log in logs {
                if Self::matches_filter(log, filter) {
                    result.push(log.clone());
                }
            }
        }
        // Locks released here at end of scope
        current = batch_end + 1;
    }
    result
}
```

Alternatively, limit the maximum block range at the indexer level (e.g., 1024 blocks) and return an error for larger ranges, which would make this problem irrelevant.

## Files to Modify

- `crates/storage/indexer/src/store.rs` -- `get_logs()` method (lines 218-246)

## Related Issues

- [038-storage-get-logs-no-block-range-limit.md](038-storage-get-logs-no-block-range-limit.md) -- The indexer has no block range limit of its own, relying on the RPC layer to cap the range. Combined with this issue, that means internal callers can trigger extremely long lock holds.
