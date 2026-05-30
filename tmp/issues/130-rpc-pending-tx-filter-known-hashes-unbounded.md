# Pending Transaction Filter known_hashes HashSet Grows Unboundedly Per Filter Instance

## Category
bug/security -- rpc

## Severity
medium

## Summary
Each pending transaction filter (`Filter::PendingTransaction`) maintains a `known_hashes: HashSet<B256>` that is replaced wholesale on every `eth_getFilterChanges` call with a clone of the entire `pending_txs` key set. With up to 1,024 concurrent filters and up to 10,000 pending transactions, an attacker can force approximately 312 MB of hash-set memory allocation by simply creating filters and polling them. This constitutes a memory exhaustion vector on memory-constrained nodes.

## Problem
The `Filter::PendingTransaction` variant stores a `known_hashes: HashSet<B256>` field (defined in the `filters.rs` module) that tracks which pending transaction hashes have already been reported to the client. On each `eth_getFilterChanges` call, the implementation in `eth.rs` replaces this set with a complete clone of the current `pending_txs` HashMap's key set.

**Filter definition -- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/filters.rs`, lines 55-63:**
```rust
/// Pending transaction filter cursor.
PendingTransaction {
    /// Pending transaction hashes already reported to this filter.
    known_hashes: HashSet<B256>,
    /// Snapshot index into the shared insertion-order vec at the time
    /// of last poll (or filter creation). New hashes are those at
    /// indices >= this value that are not in `known_hashes`.
    last_seen_index: usize,
},
```

**Update on poll -- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 907-916:**
```rust
let current_hashes: HashSet<B256> =
    self.pending_txs.read().await.keys().copied().collect();

let mut filter = entry.lock().await;
if let Filter::PendingTransaction { known_hashes: kh, last_seen_index: idx } =
    &mut *filter
{
    *kh = current_hashes;
    *idx = new_index;
}
```

The `FilterStore` (in `filters.rs`, line 23) allows up to `DEFAULT_MAX_FILTERS = 1024` active filters. The `pending_txs` map can hold up to `MAX_PENDING_TXS = 10_000` entries (defined at `eth.rs`, line 45). Each `B256` is 32 bytes, plus `HashSet` overhead (~48 bytes per entry).

**Worst-case memory:** 1,024 filters x 10,000 hashes x ~80 bytes = ~780 MB of hash-set data alone. Even conservatively (32 bytes per hash + overhead), this exceeds 312 MB.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/filters.rs`, lines 20-23 (constants):
```rust
pub(crate) const DEFAULT_FILTER_TTL: Duration = Duration::from_secs(5 * 60);
pub(crate) const DEFAULT_MAX_FILTERS: usize = 1024;
```

`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, line 45 (max pending txs):
```rust
const MAX_PENDING_TXS: usize = 10_000;
```

`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 907-916 (hash set clone on poll):
```rust
let current_hashes: HashSet<B256> =
    self.pending_txs.read().await.keys().copied().collect();

let mut filter = entry.lock().await;
if let Filter::PendingTransaction { known_hashes: kh, last_seen_index: idx } =
    &mut *filter
{
    *kh = current_hashes;
    *idx = new_index;
}
```

## Impact
- **Memory exhaustion:** An attacker can create 1,024 pending-tx filters via `eth_newPendingTransactionFilter` (no authentication required) and periodically poll them. Under sustained transaction load (10,000 pending txs), this forces 312-780 MB of hash-set allocations. On the current devnet (4 GB per node), this consumes 8-20% of available memory per node.
- **GC pressure:** Even without reaching OOM, repeated allocation and deallocation of large hash sets creates significant allocator pressure, causing latency spikes for all RPC methods.
- **Low attack cost:** The attacker only needs to make 1,024 `eth_newPendingTransactionFilter` calls (one-time) and then periodically poll each filter (1,024 `eth_getFilterChanges` calls per interval). No transaction submission or ETH balance is required.

## Root Cause
The `known_hashes` set is cloned from the full `pending_txs` key set on every poll, with no per-filter cap on the set size. The `last_seen_index` cursor already provides the same deduplication guarantee (preventing previously-reported hashes from being re-reported), making the `known_hashes` set redundant. The set exists as a defensive measure from an earlier implementation but is no longer necessary given the deque-based ordering.

## Suggested Fix
**Option 1 (preferred): Remove `known_hashes` entirely.** The `last_seen_index` cursor is sufficient for deduplication since the insertion-order deque ensures monotonic indexing:

```rust
PendingTransaction {
    last_seen_index: usize,
},
```

Update the `getFilterChanges` handler to skip the hash-set clone.

**Option 2: Cap per-filter hash set size.** If `known_hashes` is kept for extra safety, cap it:

```rust
const MAX_KNOWN_HASHES_PER_FILTER: usize = 1_000;

// When updating:
if current_hashes.len() > MAX_KNOWN_HASHES_PER_FILTER {
    known_hashes.clear(); // fall back to cursor-only tracking
} else {
    *known_hashes = current_hashes;
}
```

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/filters.rs` -- remove or cap `known_hashes` field in `Filter::PendingTransaction`
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs` -- update `getFilterChanges` handler to stop cloning the full pending_txs key set

## Related Issues
- `041-txpool-in-memory-mempool-no-limits.md` -- related unbounded memory growth in the transaction pool itself

## Labels
bug, security, rpc, performance
