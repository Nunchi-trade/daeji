# Indexer insert_block Acquires 5 Write Locks Sequentially (Lock Convoy, Non-Atomic Visibility)

## Category
performance -- storage / indexer

## Severity
medium

## Summary
The `BlockIndex::insert_block()` method acquires and releases 5 separate `RwLock` write guards in sequence to insert a block and its associated transactions, receipts, and logs. This creates two problems: (1) a lock convoy where each write-lock acquisition contends with concurrent RPC readers, and (2) non-atomic visibility where a concurrent reader can observe a partially-indexed block (e.g., visible by hash but not by number, or with transactions but without receipts).

## Problem
The `BlockIndex` type stores indexed block data in 5 separate `RwLock<HashMap<...>>` maps:

**File:** `/Users/will/dev/nunchi/daeji/crates/storage/indexer/src/store.rs`, lines 19-26:
```rust
pub struct BlockIndex {
    blocks_by_hash: RwLock<HashMap<B256, IndexedBlock>>,
    blocks_by_number: RwLock<HashMap<u64, B256>>,
    transactions: RwLock<HashMap<B256, IndexedTransaction>>,
    receipts: RwLock<HashMap<B256, IndexedReceipt>>,
    logs_by_block: RwLock<HashMap<B256, Vec<IndexedLog>>>,
    head_block: AtomicU64,
}
```

The `insert_block()` method acquires each lock independently:

**File:** `/Users/will/dev/nunchi/daeji/crates/storage/indexer/src/store.rs`, lines 56-113:
```rust
pub fn insert_block(
    &self,
    block: IndexedBlock,
    txs: Vec<IndexedTransaction>,
    receipts: Vec<IndexedReceipt>,
) {
    let block_hash = block.hash;
    let block_number = block.number;

    debug!(number = block_number, hash = %block_hash, txs = txs.len(), "indexing block");

    let mut all_logs = Vec::new();
    for receipt in &receipts {
        all_logs.extend(receipt.logs.clone());
    }

    {
        let mut blocks_by_hash = self.blocks_by_hash.write();    // Lock 1
        blocks_by_hash.insert(block_hash, block);
    }

    {
        let mut blocks_by_number = self.blocks_by_number.write(); // Lock 2
        blocks_by_number.insert(block_number, block_hash);
    }

    {
        let mut transactions = self.transactions.write();          // Lock 3
        for tx in txs {
            transactions.insert(tx.hash, tx);
        }
    }

    {
        let mut receipts_map = self.receipts.write();              // Lock 4
        for receipt in receipts {
            receipts_map.insert(receipt.transaction_hash, receipt);
        }
    }

    {
        let mut logs_by_block = self.logs_by_block.write();        // Lock 5
        logs_by_block.insert(block_hash, all_logs);
    }

    let mut current = self.head_block.load(Ordering::Acquire);
    while block_number > current {
        match self.head_block.compare_exchange_weak(
            current,
            block_number,
            Ordering::Release,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(c) => current = c,
        }
    }
}
```

Additionally, at line 67-70, the logs are cloned from receipts before any locks are taken:
```rust
let mut all_logs = Vec::new();
for receipt in &receipts {
    all_logs.extend(receipt.logs.clone());
}
```

This clones all log data unnecessarily, since the receipts are moved into the receipt map later.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/storage/indexer/src/store.rs`, lines 56-113 (full method shown above).

For comparison, the `prune_before()` method at lines 121-174 takes a smarter approach, collecting data under short read locks first, then doing batch removals under write locks with fewer lock boundaries.

## Impact
- **Non-atomic visibility:** A concurrent RPC reader calling `get_block_by_hash()` can see a block but then fail when calling `get_block_by_number()` with the same block number (because lock 2 hasn't been acquired yet). Similarly, `get_transaction()` may return a transaction for which `get_receipt()` returns `None` (because lock 4 hasn't been acquired yet). This causes inconsistent RPC responses.
- **Lock contention at high block rates:** At 33 blocks/s, the indexer writes to all 5 maps roughly 33 times per second. Each write lock contends with concurrent RPC readers holding read locks on the same maps. Five separate lock acquisitions per block means 165 lock operations per second on the hot path.
- **Unnecessary allocation:** The `all_logs` clone at lines 67-70 copies all log data from receipts before the receipts are consumed. Since the receipts are moved into the receipt map, the logs could be extracted from the receipts directly without cloning.

## Root Cause
The indexer uses five separate `RwLock`-protected HashMaps instead of a single lock around all indexed state. Each map is locked and unlocked independently during insertion, creating a window where the block is partially visible. This design was likely chosen for read concurrency (allowing reads on different maps to proceed in parallel), but the write path pays a steep cost.

## Suggested Fix
**Option 1 (preferred): Coalesce into a single lock.** Group all indexed data into a single `RwLock<IndexState>` struct, ensuring atomic visibility and a single lock acquisition per insert:

```rust
struct IndexState {
    blocks_by_hash: HashMap<B256, IndexedBlock>,
    blocks_by_number: HashMap<u64, B256>,
    transactions: HashMap<B256, IndexedTransaction>,
    receipts: HashMap<B256, IndexedReceipt>,
    logs_by_block: HashMap<B256, Vec<IndexedLog>>,
}

pub struct BlockIndex {
    state: RwLock<IndexState>,
    head_block: AtomicU64,
}
```

This trades some read concurrency (readers on different maps now contend) for write atomicity and simplicity. Given that the maps are all accessed together for most RPC methods anyway, the read contention impact is minimal.

**Option 2: Stage-and-swap.** Build the complete indexed data outside any lock, then acquire a single write lock and insert everything atomically:

```rust
pub fn insert_block(&self, block: IndexedBlock, txs: Vec<IndexedTransaction>, receipts: Vec<IndexedReceipt>) {
    // Prepare all data outside locks
    let block_hash = block.hash;
    let block_number = block.number;
    let all_logs: Vec<IndexedLog> = receipts.iter().flat_map(|r| r.logs.clone()).collect();

    // Single atomic insert
    let mut state = self.state.write();
    state.blocks_by_hash.insert(block_hash, block);
    state.blocks_by_number.insert(block_number, block_hash);
    for tx in txs { state.transactions.insert(tx.hash, tx); }
    for receipt in receipts { state.receipts.insert(receipt.transaction_hash, receipt); }
    state.logs_by_block.insert(block_hash, all_logs);
    drop(state);

    // Update head atomically
    // ...
}
```

**Option 3: Extract logs without cloning.** Regardless of the lock strategy, avoid the unnecessary log clone by extracting logs from receipts after they are consumed:

```rust
let (receipt_map_entries, all_logs): (Vec<_>, Vec<_>) = receipts.into_iter()
    .map(|r| {
        let logs = r.logs.clone();
        ((r.transaction_hash, r), logs)
    })
    .unzip();
let all_logs: Vec<IndexedLog> = all_logs.into_iter().flatten().collect();
```

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/storage/indexer/src/store.rs` -- restructure `BlockIndex` to use a single lock, or at minimum ensure atomic insertion across all maps

## Related Issues
- `031-rpc-get-logs-dual-read-locks.md` -- `get_logs()` acquires two read locks (`blocks_by_number` and `logs_by_block`) independently, which can observe inconsistent state due to this non-atomic insert
- `033-rpc-get-tx-by-hash-unnecessary-write-lock.md` -- transaction lookup lock contention related to the same indexer maps
- `140-indexer-hashmap-never-shrinks-after-pruning.md` -- indexer HashMaps never shrink after pruning; related to indexer memory management

## Labels
performance, correctness, storage, rpc
