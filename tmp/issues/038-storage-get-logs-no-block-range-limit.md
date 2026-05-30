# Indexer get_logs Has No Block Range Limit -- OOM Risk From Unbounded Queries

**Category**: Bug -- Storage
**Severity**: Medium
**Labels**: `bug`, `storage`, `rpc`, `security`, `reliability`

## Summary

The `BlockIndex::get_logs()` method in the indexer does not enforce any maximum block range or result count limit. While the RPC layer caps the range at 10,000 blocks, internal callers of `get_logs()` have no protection. The `IndexerError::InvalidBlockRange` error type exists in the codebase but is never used -- it was clearly intended for this purpose but was never wired in.

## Problem

Kora is an EVM execution client with an in-memory block index (`BlockIndex` in `crates/storage/indexer/src/store.rs`) that serves log queries for the `eth_getLogs` RPC method. The `get_logs()` method accepts a `LogFilter` with optional `from_block` and `to_block` fields, but performs no validation on the range.

The RPC layer (`IndexedStateProvider::get_logs` in `crates/node/rpc/src/indexed_provider.rs:198-268`) does enforce a maximum block range via the `MAX_LOG_BLOCK_RANGE` constant (10,000 blocks, defined at line 30). However, the indexer's `get_logs()` method is a public API that can be called by any internal code path without this protection.

More importantly, even with the 10,000-block cap from the RPC layer, there is no limit on the number of *results* returned. A filter matching a heavily-used contract (e.g., ERC-20 Transfer events) over 10,000 blocks could return millions of log entries, each containing heap-allocated topic vectors and data bytes, leading to out-of-memory conditions.

The `IndexerError::InvalidBlockRange` error type exists at `crates/storage/indexer/src/error.rs:25-32` but is never used anywhere in the codebase:

```rust
/// Invalid block range for log filter.
#[error("invalid block range: from {from} > to {to}")]
InvalidBlockRange {
    /// Start of the range.
    from: u64,
    /// End of the range.
    to: u64,
},
```

## Code Reference

`crates/storage/indexer/src/store.rs:217-246`:
```rust
/// Gets logs matching the given filter.
pub fn get_logs(&self, filter: &LogFilter) -> Vec<IndexedLog> {
    let head = self.head_block_number();
    let from_block = filter.from_block.unwrap_or(0);
    let to_block = filter.to_block.unwrap_or(head).min(head);
    // No range validation here!

    let mut result = Vec::new();

    let blocks_by_number = self.blocks_by_number.read();
    let logs_by_block = self.logs_by_block.read();

    for block_num in from_block..=to_block {
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
            result.push(log.clone());  // Unbounded collection into Vec
        }
    }

    result
}
```

The RPC layer enforces a range cap but no result count limit (`crates/node/rpc/src/indexed_provider.rs:231-235`):
```rust
if to.saturating_sub(from) > MAX_LOG_BLOCK_RANGE {
    return Err(RpcError::InvalidParams(format!(
        "block range exceeds maximum of {MAX_LOG_BLOCK_RANGE}"
    )));
}
```

The unused error variant in `crates/storage/indexer/src/error.rs:25-32`:
```rust
/// Invalid block range for log filter.
#[error("invalid block range: from {from} > to {to}")]
InvalidBlockRange {
    /// Start of the range.
    from: u64,
    /// End of the range.
    to: u64,
},
```

## Impact

A single malicious or misconfigured RPC client can trigger an unbounded memory allocation by requesting logs that match many events. Even with the 10,000-block range cap, a filter matching a heavily-used contract could return millions of log entries. Each `IndexedLog` contains heap-allocated `Vec<B256>` topics and `Bytes` data, so a million logs could easily consume hundreds of megabytes.

Concrete scenario: An ERC-20 token contract emits a `Transfer` event on every token transfer. If the contract is heavily used (100 transfers per block), querying its logs over 10,000 blocks returns 1,000,000 log entries. At roughly 300 bytes per log (3 topics, ~32 bytes data, metadata), this consumes approximately 300MB in a single allocation -- enough to cause OOM on memory-constrained deployments.

Internal callers of `BlockIndex::get_logs()` that bypass the RPC layer have no range protection at all, allowing the full retained range (10,000 blocks) to be queried.

## Root Cause

The range validation was planned (the `IndexerError::InvalidBlockRange` error type was created) but never implemented. The RPC layer applies a block range cap, but the indexer itself accepts any range. Neither layer limits the number of results.

## Suggested Fix

1. Add block range validation at the indexer level, changing the return type to use the existing error type:

```rust
pub fn get_logs(&self, filter: &LogFilter) -> Result<Vec<IndexedLog>, IndexerError> {
    let head = self.head_block_number();
    let from_block = filter.from_block.unwrap_or(0);
    let to_block = filter.to_block.unwrap_or(head).min(head);

    if from_block > to_block {
        return Err(IndexerError::InvalidBlockRange {
            from: from_block,
            to: to_block,
        });
    }

    // ... existing iteration ...
    Ok(result)
}
```

2. Add a result count limit (e.g., 10,000 logs) to prevent OOM from a single high-event block or range:

```rust
const MAX_LOG_RESULTS: usize = 10_000;

for log in logs {
    if !Self::matches_filter(log, filter) {
        continue;
    }
    result.push(log.clone());
    if result.len() >= MAX_LOG_RESULTS {
        // Return what we have -- caller can narrow the range
        return Ok(result);
    }
}
```

3. Update callers (`IndexedStateProvider::get_logs`) to handle the new `Result` return type.

## Files to Modify

- `crates/storage/indexer/src/store.rs` -- `get_logs()` method (lines 217-246): add range validation and result count limit, change return type to `Result`
- `crates/node/rpc/src/indexed_provider.rs` -- `get_logs()` method (line 253): update to handle `Result` from indexer

## Related Issues

- [031-rpc-get-logs-dual-read-locks.md](031-rpc-get-logs-dual-read-locks.md) -- The same `get_logs()` method holds two read locks for the entire iteration, compounding the impact of unbounded queries.
