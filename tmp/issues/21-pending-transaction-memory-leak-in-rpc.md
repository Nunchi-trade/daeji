# RPC: Pending transaction map in EthApiImpl grows without bound, leaking memory on long-running nodes

**Severity:** Medium (P2)
**Component:** `crates/node/rpc/src/eth.rs`, `crates/node/reporters/src/lib.rs`
**Affects:** All long-running Kora nodes with RPC enabled

---

## Summary

Kora is an EVM-compatible blockchain built on the [Commonware](https://github.com/commonwarexyz/monorepo) consensus framework (Simplex BFT). It exposes a JSON-RPC server implementing the standard Ethereum `eth_*` namespace so that wallets, dApps, and tooling (Foundry, ethers.js, etc.) can interact with the chain.

The `EthApiImpl` struct in `crates/node/rpc/src/eth.rs` (line 197) maintains a `pending_txs` field of type `Arc<RwLock<HashMap<B256, RpcTransaction>>>`. When a client calls `eth_sendRawTransaction`, the decoded transaction is unconditionally inserted into this map. Entries are only removed when a subsequent `eth_getTransactionByHash` call discovers that the transaction has been indexed into a finalized block. There is no TTL, no size cap, no background eviction, and no finalization-triggered cleanup. If a transaction is submitted but never included in a block, or is included but never queried by hash, the entry remains in memory forever.

---

## Background: What `pending_txs` Does

The `pending_txs` map serves a single purpose: it allows `eth_getTransactionByHash` to return transaction data for transactions that have been submitted but not yet finalized. Without it, there would be a gap between when a client submits a transaction and when it appears in the indexer -- during this window, querying the transaction by hash would incorrectly return `null`, breaking standard Ethereum client expectations.

The lifecycle is intended to be:

1. Client calls `eth_sendRawTransaction` with raw bytes.
2. The RPC handler decodes the bytes into an `RpcTransaction`, inserts it into `pending_txs`, and returns the transaction hash.
3. Client (or anyone) calls `eth_getTransactionByHash` with that hash.
4. If the transaction has been indexed (finalized and persisted), the indexer returns it and the entry is removed from `pending_txs`.
5. If not yet indexed, the entry from `pending_txs` is returned as a pending transaction.

The problem is in step 4: removal only happens as a side effect of a query. If no query ever arrives, the entry is never removed.

---

## The Insertion Path (No Guards)

In `crates/node/rpc/src/eth.rs`, the `send_raw_transaction` method (lines 299-309):

```rust
async fn send_raw_transaction(&self, data: Bytes) -> RpcResult<B256> {
    let tx_hash = alloy_primitives::keccak256(&data);
    let pending_tx = raw_tx_to_pending_rpc(&data)?;

    if let Some(ref submit) = self.tx_submit {
        submit(data).await?;
    }

    self.pending_txs.write().await.insert(tx_hash, pending_tx);
    Ok(tx_hash)
}
```

Every successful call inserts into `pending_txs`. There is no check for:
- Whether the map already exceeds a size threshold
- Whether an entry with the same hash already exists (HashMap silently replaces, but the old value was already occupying memory until this point)
- Whether the transaction was actually accepted by the mempool (the insert happens unconditionally after `submit()`)

The `raw_tx_to_pending_rpc` function (lines 490-519) decodes the raw bytes into a full `RpcTransaction` struct, which includes the `input` field -- a clone of the transaction's calldata. For contract interactions, this can be several kilobytes.

---

## The Only Removal Path (Query-Triggered)

The sole removal path is in `get_transaction_by_hash` (lines 348-356):

```rust
async fn get_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<RpcTransaction>> {
    let provider = self.state_provider.read().await;
    let indexed = provider.transaction_by_hash(hash).await?;
    if indexed.is_some() {
        self.pending_txs.write().await.remove(&hash);
        return Ok(indexed);
    }
    Ok(self.pending_txs.read().await.get(&hash).cloned())
}
```

Removal requires ALL of the following conditions:
1. Someone must call `eth_getTransactionByHash` with the exact hash.
2. The transaction must have been finalized and indexed by the state provider.
3. The state provider must successfully return the indexed transaction.

If ANY of these conditions is not met, the entry remains. Common scenarios where removal never occurs:

### Scenario A: Transaction never included in a block
A transaction with a nonce that is too high, gas price that is too low (once Kora implements gas markets), or that is simply never selected by a leader will sit in the mempool indefinitely. Even if it is eventually evicted from the mempool (via the fixes in Issues #01 and #02), the `pending_txs` entry in the RPC layer is completely independent of the mempool and has no eviction path.

### Scenario B: Transaction included but never queried
Many transactions are "fire and forget" -- the client submits and tracks confirmation via `eth_getTransactionReceipt` or by watching block events, never calling `eth_getTransactionByHash`. The `get_transaction_receipt` method (lines 358-364) does NOT remove the `pending_txs` entry:

```rust
async fn get_transaction_receipt(&self, hash: B256) -> RpcResult<Option<RpcTransactionReceipt>> {
    let provider = self.state_provider.read().await;
    provider.receipt_by_hash(hash).await.map_err(Into::into)
}
```

### Scenario C: Indexer not configured
If the node is running without a block indexer (`block_index: None` in `handle_finalized_update` at `crates/node/reporters/src/lib.rs`, line 221), no transactions are indexed at all. The `provider.transaction_by_hash(hash)` call will always return `None`, so the `pending_txs` entry is never removed even if queried.

### Scenario D: Load generator / automated tooling
The Kora load generator (`bin/loadgen/src/main.rs`) submits thousands of transactions via `eth_sendRawTransaction` and tracks them by receipt, never calling `eth_getTransactionByHash`. Every transaction it submits leaks a `pending_txs` entry.

---

## Memory Per Entry

The `RpcTransaction` struct (defined in `crates/node/rpc/src/types.rs`, lines 115-160) contains:

| Field | Type | Size (bytes) |
|-------|------|-------------|
| `hash` | `B256` | 32 |
| `nonce` | `U64` | 8 |
| `block_hash` | `Option<B256>` | 33 (None for pending) |
| `block_number` | `Option<U64>` | 9 (None for pending) |
| `transaction_index` | `Option<U64>` | 9 (None for pending) |
| `from` | `Address` | 20 |
| `to` | `Option<Address>` | 21 |
| `value` | `U256` | 32 |
| `gas` | `U64` | 8 |
| `gas_price` | `U256` | 32 |
| `input` | `Bytes` | 24 (header) + calldata length |
| `tx_type` | `U64` | 8 |
| `chain_id` | `Option<U64>` | 9 |
| `max_fee_per_gas` | `Option<U256>` | 33 |
| `max_priority_fee_per_gas` | `Option<U256>` | 33 |
| `v` | `U64` | 8 |
| `r` | `U256` | 32 |
| `s` | `U256` | 32 |
| **Subtotal (struct)** | | **~393 bytes** |
| HashMap entry overhead (key + bucket) | | ~80 bytes |
| **Total per entry (simple transfer)** | | **~473 bytes** |
| **Total per entry (contract call, 1KB calldata)** | | **~1,500 bytes** |

### Growth Rates

For a node receiving sustained RPC traffic:

| Traffic pattern | Txs/sec | Per-entry size | MB/hour | GB/day |
|---|---|---|---|---|
| Light RPC (wallets) | 10 | ~500 bytes | ~18 MB | ~0.4 GB |
| Moderate RPC (dApp) | 100 | ~500 bytes | ~180 MB | ~4.3 GB |
| Load generator | 3,000 | ~500 bytes | ~5,400 MB | ~130 GB |
| Contract-heavy traffic | 100 | ~1,500 bytes | ~540 MB | ~13 GB |

Under the load generator at ~3,000 TPS (the observed throughput from Issue #17's load testing), the RPC layer leaks approximately 1.5 MB/second. A 30-minute load test leaks ~2.7 GB. Combined with the snapshot store leak (Issue #07), this accelerates time-to-OOM significantly.

---

## The Data Structure Problem

The `pending_txs` field uses `HashMap<B256, RpcTransaction>`:

```rust
// crates/node/rpc/src/eth.rs, line 197
pending_txs: Arc<RwLock<HashMap<B256, RpcTransaction>>>,
```

`HashMap` provides O(1) insert and lookup but has no concept of:
- **Insertion order** -- cannot evict "oldest" entries without a separate ordering structure
- **Time-to-live** -- no timestamp is stored with entries
- **Capacity limit** -- grows unboundedly until system OOM
- **LRU eviction** -- no access-time tracking

This makes it impossible to implement any eviction policy without replacing the data structure.

---

## Impact

- **Memory leak on all RPC-enabled nodes.** Any node accepting `eth_sendRawTransaction` calls leaks memory proportional to the number of transactions submitted. The leak rate depends on traffic volume, not chain activity -- even if the chain is finalizing blocks normally, entries that are never queried by hash accumulate.

- **Load testing causes significant leaks.** The load generator submits thousands of transactions and never queries them by hash. A standard load test run leaks hundreds of megabytes.

- **Compounds with Issue #07.** The snapshot store leak (Issue #07) grows with chain height (one entry per block). This RPC leak grows with transaction volume. Together, they create two independent linear memory growth paths that compound under load.

- **No monitoring exists.** There is no Prometheus metric tracking the size of `pending_txs`. The only observable symptom is growing RSS (`runtime_process_rss`), which is ambiguous -- it could be caused by this leak, the snapshot store leak, or legitimate memory use.

- **No workaround.** Restarting the node clears the `pending_txs` map, but also clears all in-memory state (snapshots, seeds, consensus journals on tmpfs deployments). There is no way to selectively drain the pending transaction map.

---

## Proposed Fixes

### Option 1: Periodic Cleanup Task (Minimum Viable)

Spawn a background task that periodically sweeps entries older than a configurable TTL (e.g., 5 minutes). This requires storing an insertion timestamp alongside each entry.

```rust
// Replace HashMap<B256, RpcTransaction> with:
struct PendingEntry {
    tx: RpcTransaction,
    inserted_at: Instant,
}

pending_txs: Arc<RwLock<HashMap<B256, PendingEntry>>>,
```

Spawn a cleanup task in the constructor:

```rust
let pending = pending_txs.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        let mut map = pending.write().await;
        let cutoff = Instant::now() - Duration::from_secs(300); // 5 min TTL
        map.retain(|_, entry| entry.inserted_at > cutoff);
    }
});
```

**Pros:** Simple, handles all leak scenarios.
**Cons:** Periodic scan of entire map; does not bound peak memory between sweeps; transactions that are still genuinely pending (waiting for inclusion) are evicted, which changes `eth_getTransactionByHash` behavior.

### Option 2: Bounded HashMap with LRU Eviction

Replace the `HashMap` with an LRU cache (e.g., the `lru` crate or `quick_cache`). Cap at a fixed number of entries (e.g., 10,000). When the cap is reached, the least-recently-inserted entry is evicted.

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

pending_txs: Arc<RwLock<LruCache<B256, RpcTransaction>>>,
```

Initialize with a capacity:

```rust
pending_txs: Arc::new(RwLock::new(
    LruCache::new(NonZeroUsize::new(10_000).unwrap())
)),
```

**Pros:** Bounded memory (10,000 * ~500 bytes = ~5 MB cap); O(1) insert/lookup/eviction; no background task needed.
**Cons:** Under high throughput, recently-submitted transactions may be evicted before they can be queried. A cap of 10,000 at 3,000 TPS means entries live ~3.3 seconds -- potentially too short for polling clients.

### Option 3: Finalization-Triggered Cleanup

Add a method to `EthApiImpl` that accepts a list of transaction hashes from a finalized block and removes them from `pending_txs`. Call this from the `FinalizedReporter` after a block is persisted.

```rust
// In EthApiImpl:
pub async fn remove_finalized_txs(&self, tx_hashes: &[B256]) {
    let mut pending = self.pending_txs.write().await;
    for hash in tx_hashes {
        pending.remove(hash);
    }
}
```

Wire this into `handle_finalized_update` in `crates/node/reporters/src/lib.rs`, after line 226 (`state.prune_mempool(&block.txs).await`):

```rust
// After mempool pruning:
let tx_hashes: Vec<B256> = block.txs.iter()
    .map(|tx| alloy_primitives::keccak256(tx))
    .collect();
eth_api.remove_finalized_txs(&tx_hashes).await;
```

**Pros:** Removes entries exactly when they should be removed (after finalization); no artificial TTL or cap.
**Cons:** Only handles Scenario B (included but never queried). Does NOT handle Scenario A (never included in a block). Transactions with bad nonces, insufficient gas, or that are otherwise unexecutable will still leak forever. Requires plumbing an `EthApiImpl` handle (or a `pending_txs` clone) into the reporter, which crosses crate boundaries.

### Option 4: LRU Cache with TTL and Finalization Cleanup (Recommended)

Combine Options 2 and 3 for defense in depth:

1. **Replace `HashMap` with a bounded LRU cache** (cap at 10,000-50,000 entries depending on expected traffic). This provides a hard memory ceiling.

2. **Add finalization-triggered cleanup** so that included transactions are removed promptly, freeing capacity for new pending transactions.

3. **Optionally add a TTL layer** (store `Instant` with each entry, sweep periodically) to proactively evict stale entries that will never be included.

This combination handles all four leak scenarios:
- Scenario A (never included): TTL eviction or LRU eviction at capacity
- Scenario B (included, never queried): finalization cleanup
- Scenario C (no indexer): TTL eviction or LRU eviction at capacity
- Scenario D (load generator): LRU eviction at capacity

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/rpc/src/eth.rs` | Replace `HashMap<B256, RpcTransaction>` with a bounded LRU cache (line 197). Update `new()` and `with_tx_submit()` constructors (lines 212-231) to initialize with capacity. Update `send_raw_transaction` (line 307) -- insertion is unchanged but now bounded. Update `get_transaction_by_hash` (lines 348-356) -- lookup and removal API may differ for LRU. Add `remove_finalized_txs()` method for finalization cleanup. Add a `pending_txs_handle()` accessor that returns a clone of the `Arc<RwLock<...>>` for the reporter to use. |
| `crates/node/reporters/src/lib.rs` | In `handle_finalized_update` (after line 226), call into the RPC pending tx map to remove finalized transaction hashes. This requires receiving a handle to the pending tx map (or an `EthApiImpl` reference) as a parameter to `FinalizedReporter`. |
| `crates/node/rpc/src/server.rs` | Expose the `EthApiImpl`'s pending tx handle from the `RpcServer` builder so the runner can pass it to the reporter. |
| `crates/node/runner/src/runner.rs` | Wire the pending tx handle from the RPC server into the `FinalizedReporter` construction. |
| `Cargo.toml` (rpc crate) | Add `lru` (or `quick_cache`) dependency if using an external LRU crate. |

---

## Prometheus Monitoring

After implementing the fix, add a gauge metric to track the pending transaction map size:

```rust
// In EthApiImpl, on each insert/remove:
metrics::gauge!("kora_rpc_pending_txs_count").set(self.pending_txs.read().await.len() as f64);
```

Add to the Grafana dashboard (`docker/grafana/dashboards/kora-overview.json`):

```promql
# Pending tx cache utilization
kora_rpc_pending_txs_count / 10000  # ratio of capacity used

# Rate of pending tx insertions (proxy for RPC traffic)
rate(kora_rpc_pending_txs_inserts_total[5m])
```

---

## Testing Plan

### Unit test: LRU eviction at capacity

Insert N+1 transactions into a cache with capacity N. Verify that the oldest entry (by insertion order) is evicted and that the newest N entries are retained. Verify that looking up the evicted hash returns `None`.

```rust
#[tokio::test]
async fn test_pending_txs_lru_eviction() {
    let eth = EthApiImpl::new_with_capacity(1337, NoopStateProvider, 3);
    // Insert 4 transactions (capacity is 3)
    for i in 0..4u64 {
        let data = create_signed_tx(i); // helper to create valid signed tx with nonce i
        eth.send_raw_transaction(data).await.unwrap();
    }
    // First tx should be evicted
    let hash_0 = /* hash of tx with nonce 0 */;
    let result = eth.get_transaction_by_hash(hash_0).await.unwrap();
    assert!(result.is_none(), "oldest tx should have been evicted");
    // Last 3 should still be present
    for i in 1..4u64 {
        let hash_i = /* hash of tx with nonce i */;
        let result = eth.get_transaction_by_hash(hash_i).await.unwrap();
        assert!(result.is_some(), "tx {i} should still be in cache");
    }
}
```

### Unit test: TTL eviction (if TTL is implemented)

Insert a transaction, advance time past the TTL, trigger a sweep, and verify the entry is evicted.

```rust
#[tokio::test]
async fn test_pending_txs_ttl_eviction() {
    let eth = EthApiImpl::new_with_capacity_and_ttl(1337, NoopStateProvider, 100, Duration::from_secs(1));
    let data = create_signed_tx(0);
    let hash = eth.send_raw_transaction(data).await.unwrap();
    // Immediately available
    assert!(eth.get_transaction_by_hash(hash).await.unwrap().is_some());
    // Wait for TTL + sweep interval
    tokio::time::sleep(Duration::from_secs(2)).await;
    // Should be evicted
    assert!(eth.get_transaction_by_hash(hash).await.unwrap().is_none());
}
```

### Unit test: finalization cleanup removes entry

Insert a transaction into `pending_txs`, then call `remove_finalized_txs` with its hash. Verify the entry is removed.

```rust
#[tokio::test]
async fn test_finalization_removes_pending_tx() {
    let eth = EthApiImpl::new(1337, NoopStateProvider);
    let data = create_signed_tx(0);
    let hash = eth.send_raw_transaction(data).await.unwrap();
    assert!(eth.get_transaction_by_hash(hash).await.unwrap().is_some());
    // Simulate finalization cleanup
    eth.remove_finalized_txs(&[hash]).await;
    assert!(eth.get_transaction_by_hash(hash).await.unwrap().is_none());
}
```

### Integration test: submit tx, finalize block, verify removal from pending

In a single-node or 4-validator devnet:

1. Submit a transaction via `eth_sendRawTransaction`.
2. Verify `eth_getTransactionByHash` returns the transaction with `blockHash: null` (pending).
3. Wait for the block containing the transaction to be finalized.
4. Verify `eth_getTransactionByHash` returns the transaction with `blockHash` populated (from the indexer, not from `pending_txs`).
5. Verify that the `pending_txs` map size has decreased (check via the Prometheus metric or a debug endpoint).

### Integration test: submit tx that is never included, verify TTL eviction

1. Submit a transaction with a nonce far in the future (e.g., nonce 999999) so it is never executable.
2. Verify it appears in `eth_getTransactionByHash` immediately.
3. Wait for the TTL to expire (or for the LRU cache to fill with subsequent transactions).
4. Verify `eth_getTransactionByHash` returns `None`.

### Load test: verify memory stability under sustained RPC traffic

1. Start a devnet node with the fix applied.
2. Run the load generator for 50,000 transactions.
3. Monitor `runtime_process_rss` and `kora_rpc_pending_txs_count` via Prometheus.
4. Verify that `pending_txs_count` stays below the configured capacity (e.g., 10,000) and does not grow without bound.
5. Verify that RSS does not exhibit the linear growth pattern characteristic of the leak.

---

## Verification Steps

### Step 1: Confirm the bug exists (pre-fix baseline)

Search for any removal path besides the query-triggered one:

```bash
grep -n "pending_txs.*remove\|pending_txs.*retain\|pending_txs.*clear\|pending_txs.*drain" crates/node/rpc/src/eth.rs
```

Expected output: only line 352 (`self.pending_txs.write().await.remove(&hash)`) inside `get_transaction_by_hash`. No background cleanup, no finalization hook, no capacity check.

### Step 2: Confirm no size bound exists

```bash
grep -n "capacity\|max_pending\|pending.*limit\|pending.*cap" crates/node/rpc/src/eth.rs
```

Expected output: no matches.

### Step 3: After the fix, run unit tests

```bash
cargo test -p kora-rpc
```

### Step 4: After the fix, run integration tests

```bash
just trusted-devnet
sleep 30
cargo run --release --bin loadgen -- --total-txs 10000 --accounts 10 --rpc-url http://127.0.0.1:8545
# Check pending tx count metric:
curl -s http://127.0.0.1:9090/api/v1/query?query=kora_rpc_pending_txs_count | jq '.data.result[].value[1]'
# Should be well below the LRU capacity, not 10000
```

### Step 5: Verify RSS stability

```bash
# Sample RSS at start and after load test
curl -s http://127.0.0.1:3000/metrics | grep runtime_process_rss
# Run load test
cargo run --release --bin loadgen -- --total-txs 50000 --accounts 20 --rpc-url http://127.0.0.1:8545
# Sample RSS again
curl -s http://127.0.0.1:3000/metrics | grep runtime_process_rss
# Difference should be small (< 50 MB), not proportional to 50000 * 500 bytes = 25 MB
```

---

## Cross-References

- **Issue #02 (Replace InMemoryMempool with TransactionPool):** The `TransactionPool` provides nonce-aware insertion and replacement, which reduces the number of unexecutable transactions entering the system. However, even with a perfect mempool, the RPC `pending_txs` leak remains because it is independent of the mempool data structure.
- **Issue #07 (Snapshot store unbounded memory growth):** Another unbounded in-memory data structure that leaks memory on long-running nodes. Fixing both is necessary to achieve stable RSS on production deployments.
- **Issue #17 (Nonce validation gap at ingress):** Without nonce validation at ingress, transactions with stale or duplicate nonces enter both the mempool AND the `pending_txs` map. Fixing Issue #17 reduces the rate of unexecutable transactions but does not eliminate the `pending_txs` leak for transactions that pass validation but are never included in a block.
