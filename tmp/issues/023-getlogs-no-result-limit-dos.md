# 023: eth_getLogs Has No Result Count Limit -- Memory Exhaustion / DoS Vector

**Category**: bug, security
**Severity**: high
**Status**: PARTIALLY FIXED (block range limit exists; result count limit missing)
**Labels**: bug, security, rpc, performance

---

## Summary

The `eth_getLogs` RPC method enforces a block range limit of 10,000 blocks but has no cap on the number of log entries returned. Within the allowed 10,000-block range, a query targeting a common event topic (e.g., ERC-20 `Transfer`) can return millions of log entries, exhausting memory and blocking the RPC thread. Standard Ethereum clients like Geth and Erigon cap results at 10,000 logs in addition to limiting the block range.

---

## Problem

The RPC layer correctly rejects queries spanning more than `MAX_LOG_BLOCK_RANGE` (10,000) blocks at lines 231-235 of `indexed_provider.rs`. However, the underlying `BlockIndex::get_logs()` method in `store.rs` returns an unbounded `Vec<IndexedLog>`. If the queried range contains a high-volume event topic, the result vector can grow without limit.

Additionally, `get_logs()` acquires two `RwLock` read guards simultaneously (`blocks_by_number` and `logs_by_block` at lines 225-226 of `store.rs`), blocking all writers (i.e., block indexing) for the entire duration of the scan.

**File**: `/Users/will/dev/nunchi/daeji/crates/storage/indexer/src/store.rs`

---

## Code Reference

The block range check in `indexed_provider.rs` (lines 231-235):

```rust
// crates/node/rpc/src/indexed_provider.rs:30-31
const MAX_LOG_BLOCK_RANGE: u64 = 10_000;

// crates/node/rpc/src/indexed_provider.rs:231-235
if to.saturating_sub(from) > MAX_LOG_BLOCK_RANGE {
    return Err(RpcError::InvalidParams(format!(
        "block range exceeds maximum of {MAX_LOG_BLOCK_RANGE}"
    )));
}
```

The unbounded `get_logs()` method in `store.rs` (lines 218-246):

```rust
// crates/storage/indexer/src/store.rs:218-246
pub fn get_logs(&self, filter: &LogFilter) -> Vec<IndexedLog> {
    let head = self.head_block_number();
    let from_block = filter.from_block.unwrap_or(0);
    let to_block = filter.to_block.unwrap_or(head).min(head);

    let mut result = Vec::new();

    let blocks_by_number = self.blocks_by_number.read();  // <-- read guard #1
    let logs_by_block = self.logs_by_block.read();         // <-- read guard #2

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
            result.push(log.clone());  // <-- no limit on result.len()
        }
    }

    result
}
```

---

## Impact

1. **Memory exhaustion**: A query for a common event topic (e.g., ERC-20 `Transfer`, Uniswap `Swap`) across 10,000 blocks could return millions of log entries. Each `IndexedLog` contains multiple `B256` fields, `Address`, `Bytes` data, and several `u64` fields, so millions of entries can easily consume gigabytes of memory, triggering an OOM kill.

2. **Writer starvation**: While processing the query, the two `RwLock` read guards on `blocks_by_number` and `logs_by_block` prevent any concurrent `insert_block()` calls from acquiring write locks. On a chain producing ~33 blocks/second, a multi-second log scan would cause block indexing to stall, leading to a growing backlog of unindexed blocks.

3. **Timeout cascading**: Large result sets take long enough to serialize and transmit that HTTP clients time out and retry, multiplying the load.

4. **Deviation from standard**: Geth and Erigon both enforce a result count cap (typically 10,000 logs). Applications relying on standard behavior may send queries that are safe on those clients but dangerous on Kora.

---

## Root Cause

The block range limit was added (`MAX_LOG_BLOCK_RANGE = 10_000` at `indexed_provider.rs:30`) but no result count limit was implemented. The `get_logs()` method in `store.rs` accumulates all matching logs into an unbounded `Vec`.

---

## Suggested Fix

Add a result count limit to `get_logs()`:

**Before** (in `store.rs` line 241):
```rust
result.push(log.clone());
```

**After**:
```rust
const MAX_LOG_RESULTS: usize = 10_000;

result.push(log.clone());
if result.len() >= MAX_LOG_RESULTS {
    break;
}
```

Alternatively, return an error when the limit is exceeded (which is what Geth does):

```rust
if result.len() >= MAX_LOG_RESULTS {
    // Return a partial result or error, matching Geth behavior
    break;
}
```

Additionally, consider adding a per-query timeout so that even if the result count is within limits, a slow query does not hold the read locks indefinitely.

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/storage/indexer/src/store.rs` -- Add `MAX_LOG_RESULTS` constant and limit check inside `get_logs()` (around line 241)
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs` -- Optionally, add a result count limit at the RPC layer as a second defense (around line 251)

---

## Related Issues

- `031-rpc-get-logs-dual-read-locks.md` (dual lock contention in the same code path)
- `038-storage-get-logs-no-block-range-limit.md` (related log query issue)
