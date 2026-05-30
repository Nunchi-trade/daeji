# eth_getFilterChanges for Log Filters Bypasses MAX_LOG_BLOCK_RANGE Limit

## Category
bug/security -- rpc

## Severity
medium

## Summary
When `eth_getFilterChanges` is called on a log filter, it constructs a block range from `last_poll_block + 1` to the current head without capping the range. If a client polls infrequently (or an attacker deliberately delays polling), the range can span hundreds of thousands of blocks. This either produces a confusing "block range exceeds maximum" error from the downstream `get_logs()` implementation, or -- with a permissive provider -- triggers an unbounded log scan that can cause OOM or denial of service.

## Problem
The `getFilterChanges` handler for log filters (in `EthApiImpl`) computes the block range to scan as `from = last_poll_block + 1` and `to = head`. This range is passed directly to `provider.get_logs()` without being capped at `MAX_LOG_BLOCK_RANGE` (10,000 blocks).

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 802-851:
```rust
FilterSnapshot::Log { criteria, last_poll_block } => {
    let head = self.current_block_number().await;
    if let Some(lpb) = last_poll_block
        && head <= lpb
    {
        entry.touch();
        return Ok(FilterChanges::Logs(Vec::new()));
    }

    let changes_filter = if criteria.block_hash.is_some() {
        // block_hash filters: already handled
        // ...
        criteria.clone()
    } else {
        let from = last_poll_block.map(|lpb| lpb.saturating_add(1)).unwrap_or(0);
        let to = match &criteria.to_block {
            Some(BlockNumberOrTag::Number(n)) => n.to::<u64>().min(head),
            _ => head,
        };
        RpcLogFilter {
            from_block: Some(BlockNumberOrTag::Number(U64::from(from))),
            to_block: Some(BlockNumberOrTag::Number(U64::from(to))),
            address: criteria.address.clone(),
            topics: criteria.topics.clone(),
            block_hash: None,
        }
    };

    let provider = self.state_provider.read().await;
    let logs = provider.get_logs(changes_filter).await?;

    // Update cursor to head
    let mut filter = entry.lock().await;
    if let Filter::Log { last_poll_block: lpb, .. } = &mut *filter {
        *lpb = Some(head);
    }
    entry.touch();
    Ok(FilterChanges::Logs(logs))
}
```

The downstream `IndexedStateProvider::get_logs()` enforces `MAX_LOG_BLOCK_RANGE`:

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs`, lines 231-235:
```rust
if to.saturating_sub(from) > MAX_LOG_BLOCK_RANGE {
    return Err(RpcError::InvalidParams(format!(
        "block range exceeds maximum of {MAX_LOG_BLOCK_RANGE}"
    )));
}
```

This means a filter-changes call with a range larger than 10,000 blocks will fail with an opaque error. Worse, the cursor is updated to `head` regardless (line 848), so the blocks between `last_poll_block` and `head` are silently skipped on failure -- the client will never see those logs.

At the current block rate (33 blocks/s), a client that polls every 6 minutes would accumulate a range of ~12,000 blocks, exceeding the limit.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 825-840 (range construction without cap):
```rust
let from = last_poll_block.map(|lpb| lpb.saturating_add(1)).unwrap_or(0);
let to = match &criteria.to_block {
    Some(BlockNumberOrTag::Number(n)) => n.to::<u64>().min(head),
    _ => head,
};
RpcLogFilter {
    from_block: Some(BlockNumberOrTag::Number(U64::from(from))),
    to_block: Some(BlockNumberOrTag::Number(U64::from(to))),
    address: criteria.address.clone(),
    topics: criteria.topics.clone(),
    block_hash: None,
}
```

`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs`, line 30 (limit constant):
```rust
const MAX_LOG_BLOCK_RANGE: u64 = 10_000;
```

## Impact
- **Silent log loss:** When the range exceeds 10,000 blocks, the error causes the filter-changes call to fail, but the cursor may still advance to `head` on the next successful poll. The client permanently misses logs from the gap.
- **Poor error UX:** The error message "block range exceeds maximum of 10000" gives no indication that the client should poll more frequently. It looks like a server misconfiguration, not a client timing issue.
- **DoS with permissive providers:** If a future provider implementation does not enforce `MAX_LOG_BLOCK_RANGE`, the filter-changes path becomes an unbounded log scan. The `NoopStateProvider` currently returns `NotImplemented` for `get_logs()`, but any provider that does implement it without the range check is vulnerable.

## Root Cause
The `getFilterChanges` log-filter branch does not pre-cap the block range before delegating to `get_logs()`. It trusts the downstream provider to enforce limits, which produces a poor UX and is not enforced by all provider implementations. The cursor update logic at line 848 also does not account for the possibility of a failed `get_logs()` call.

## Suggested Fix
Cap the range to `MAX_LOG_BLOCK_RANGE` in the `getFilterChanges` handler itself, and advance the cursor only by the capped amount. The client catches up over multiple polls:

```rust
let from = last_poll_block.map(|lpb| lpb.saturating_add(1)).unwrap_or(0);
let uncapped_to = match &criteria.to_block {
    Some(BlockNumberOrTag::Number(n)) => n.to::<u64>().min(head),
    _ => head,
};
// Cap the range to prevent hitting downstream limits.
let to = uncapped_to.min(from.saturating_add(MAX_LOG_BLOCK_RANGE));

// ... build filter with capped range ...

// Update cursor to `to`, not `head`, so remaining blocks are caught next poll.
if let Filter::Log { last_poll_block: lpb, .. } = &mut *filter {
    *lpb = Some(to);
}
```

This requires importing `MAX_LOG_BLOCK_RANGE` from `indexed_provider.rs` or defining a shared constant.

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs` -- cap the block range in `getFilterChanges` log-filter branch and advance cursor by capped amount only
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs` -- export `MAX_LOG_BLOCK_RANGE` as a public constant (or move to a shared location)

## Related Issues
- `023-getlogs-no-result-limit-dos.md` -- `eth_getLogs` block range and result limit; this issue is specific to the filter-changes bypass path
- `038-storage-get-logs-no-block-range-limit.md` -- the underlying indexer `get_logs()` has no range limit either

## Labels
bug, security, rpc, correctness
