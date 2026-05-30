# TransactionPool: Global size limits are advisory-only -- no rejection or backpressure

**Severity:** Medium
**Component:** `kora-txpool`
**Affects:** Node stability, memory safety, denial-of-service resistance

---

## Summary

The `TransactionPool` defines configurable limits for pending transactions (`max_pending_txs = 4,096`) and queued transactions (`max_queued_txs = 1,024`), but these limits are never enforced. When either limit is exceeded, the pool logs a warning and continues to accept the transaction. There is no rejection, no backpressure, and no eviction. Under sustained load or deliberate spam, the pool can grow without bound until the node runs out of memory.

This stands in contrast to the per-sender limit (`max_txs_per_sender = 256`), which IS enforced as a hard limit -- the pool returns `TxPoolError::SenderFull` and refuses the transaction. The inconsistency suggests that the global limits were intended to be enforced but were left as monitoring-only, possibly as a temporary measure during development.

---

## Background

Kora is an EVM-compatible blockchain built on the Commonware consensus framework (Simplex BFT). It uses REVM for EVM transaction execution.

### How transactions enter the pool

1. A user submits a signed transaction via the JSON-RPC endpoint (`eth_sendRawTransaction`) in `crates/node/rpc/src/eth.rs` (line 299).
2. The RPC handler invokes a `tx_submit` callback, which routes through `TransactionValidator` and into the mempool via `ledger.submit_tx(tx)`.
3. Inside `TransactionPool`, the `add()` method (in `crates/node/txpool/src/pool.rs`, line 99) performs several checks:
   - Duplicate detection (line 102): rejects if the hash already exists.
   - Per-sender limit (line 110): rejects if the sender has `>= max_txs_per_sender` transactions.
   - Nonce-based insertion via `SenderQueue::insert()` (line 114): handles replacement-by-gas-price and queued vs. pending classification.
4. After successful insertion, `update_counts()` is called to recalculate the aggregate pending and queued counts across all senders.
5. The counts are compared against the configured limits -- but only for logging purposes.

### The pool configuration

The `PoolConfig` struct is defined in `crates/node/txpool/src/config.rs`:

```rust
// crates/node/txpool/src/config.rs, lines 20-31
impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_pending_txs: 4096,
            max_queued_txs: 1024,
            max_txs_per_sender: 256,
            max_tx_size: 128 * 1024, // 128 KB
            min_gas_price: 0,
            replacement_bump_percent: 10,
        }
    }
}
```

The config provides builder methods for each field (`with_max_pending_txs()`, `with_max_queued_txs()`, etc.), suggesting that operators are expected to tune these values. But tuning them has no effect beyond changing the threshold at which a warning is logged.

---

## The Code

**File:** `crates/node/txpool/src/pool.rs`, lines 99-141

```rust
/// Adds a validated transaction to the pool.
pub fn add(&self, tx: OrderedTransaction) -> Result<(), TxPoolError> {
    let mut inner = self.inner.write();

    if inner.by_hash.contains_key(&tx.hash) {
        return Err(TxPoolError::AlreadyExists);
    }

    let sender = tx.sender;
    let queue =
        inner.by_sender.entry(sender).or_insert_with(|| SenderQueue::new(sender, tx.nonce));

    if queue.total_count() >= self.config.max_txs_per_sender {
        return Err(TxPoolError::SenderFull(sender));   // <-- HARD limit: rejects
    }

    if let Some(replaced) = queue.insert(tx.clone()) {
        if replaced.hash == tx.hash {
            return Err(TxPoolError::AlreadyExists);
        }
        inner.by_hash.remove(&replaced.hash);
        debug!(hash = ?replaced.hash, "replaced transaction");
    }

    inner.by_hash.insert(tx.hash, tx);
    inner.update_counts();

    if inner.pending_count > self.config.max_pending_txs {
        warn!(                                          // <-- SOFT limit: warns only
            count = inner.pending_count,
            max = self.config.max_pending_txs,
            "pool exceeds pending limit"
        );
    }

    if inner.queued_count > self.config.max_queued_txs {
        warn!(                                          // <-- SOFT limit: warns only
            count = inner.queued_count,
            max = self.config.max_queued_txs,
            "pool exceeds queued limit"
        );
    }

    Ok(())                                              // <-- Always returns Ok
}
```

The critical observation is the control flow:

1. Lines 102-104: Duplicate check -- returns `Err` (hard limit).
2. Lines 110-112: Per-sender limit -- returns `Err` (hard limit).
3. Lines 114-120: Nonce-based insertion into `SenderQueue`.
4. Line 122: Transaction is inserted into `by_hash`.
5. Line 123: Counts are updated.
6. Lines 125-139: Counts are checked against limits -- but only warnings are emitted.
7. Line 141: `Ok(())` is returned unconditionally.

The transaction has already been inserted into both `by_hash` and the sender's queue by the time the limit check runs (lines 125-139). Even if the code were modified to return an error at this point, the transaction would need to be rolled back from the data structures it was already added to. The limit check is purely informational.

### Why the per-sender limit works but the global limit does not

The per-sender limit (line 110) is checked BEFORE insertion. If the sender already has `max_txs_per_sender` transactions, the new one is rejected and the data structures remain unchanged. This is the correct pattern.

The global limits (lines 125-139) are checked AFTER insertion. The transaction is already in the pool. The warning has no effect on the return value or the pool state.

---

## Impact

### 1. Unbounded memory growth

Each `OrderedTransaction` in the pool contains a full `TxEnvelope` (the decoded transaction with signature data), plus metadata fields (`hash`, `sender`, `nonce`, `effective_gas_price`, `timestamp`). For a typical EIP-1559 transaction, this is roughly 200-400 bytes of structured data, plus the raw transaction bytes stored in the envelope. The `by_hash` index adds another entry per transaction.

With `max_pending_txs = 4,096` and `max_queued_txs = 1,024`, the intended maximum pool size is 5,120 transactions. At ~400 bytes each, this is roughly 2 MB -- a reasonable bound. But since the limits are not enforced, an attacker (or a misconfigured load generator) can push the pool to millions of transactions, consuming hundreds of megabytes or more.

### 2. No defense against transaction spam

Without a hard global limit, the only constraint on pool growth is `max_txs_per_sender = 256` per sender. An attacker with access to many sender addresses (cheap to generate) can submit 256 transactions per address. With 1,000 addresses, that is 256,000 transactions in the pool -- 50x the intended `max_pending_txs` limit.

The `min_gas_price` config defaults to `0`, so there is no economic cost to submitting spam transactions in the current configuration.

### 3. Degraded block building performance

The `build()` method (line 316 in `pool.rs`) iterates over all senders' pending queues to select the highest-priority transactions for a block. With 10,000+ senders in the pool (due to no global limit), this iteration becomes expensive. Block building latency increases, which can cause the node to miss its proposal window in the consensus protocol.

### 4. Interaction with missing transaction TTL

This issue compounds with the absence of transaction TTL (time-to-live). Without TTL, transactions remain in the pool indefinitely once inserted. Without a hard size limit, the pool can only shrink via explicit pruning after block finalization. If finalization stalls (e.g., due to network partition or consensus failure), the pool grows monotonically.

### 5. The `Mempool::insert` trait interface masks the error

The `Mempool` trait (defined in `crates/node/txpool/src/traits.rs`) defines `insert()` as returning `bool`:

```rust
// crates/node/txpool/src/traits.rs, lines 10-14
pub trait Mempool: Clone + Send + Sync + 'static {
    /// Insert a transaction into the mempool.
    ///
    /// Returns `true` if the transaction was newly inserted.
    fn insert(&self, tx: Tx) -> bool;
    // ...
}
```

The `TransactionPool` implementation of this trait (lines 301-314 in `pool.rs`) converts the `Result<(), TxPoolError>` from `add()` into a `bool`:

```rust
fn insert(&self, tx: Tx) -> bool {
    let Some(ordered) = tx_to_ordered(&tx) else {
        trace!("failed to decode transaction for mempool insert");
        return false;
    };

    match self.add(ordered) {
        Ok(()) => true,
        Err(e) => {
            trace!(?e, "failed to insert transaction");
            false
        }
    }
}
```

Even if `add()` were changed to return `Err(TxPoolError::PoolFull)` when the global limit is exceeded, this trait method would simply return `false` -- the caller (the ledger layer) would not know WHY the insertion failed. The RPC layer would not be able to distinguish "pool is full, retry later" from "transaction is invalid."

---

## Contrast with the per-sender limit

The per-sender limit demonstrates the correct enforcement pattern:

```rust
// pool.rs, line 110-112
if queue.total_count() >= self.config.max_txs_per_sender {
    return Err(TxPoolError::SenderFull(sender));
}
```

This check:
- Runs BEFORE the transaction is inserted into any data structure.
- Returns a typed error (`TxPoolError::SenderFull`) that propagates to the caller.
- The `TxPoolError::SenderFull` variant already exists in `crates/node/txpool/src/error.rs` (line 14).

The global limit check should follow this same pattern.

---

## Proposed Fix

### Step 1: Check global limits BEFORE insertion

Move the limit checks to before the `queue.insert()` call, and return `Err(TxPoolError::PoolFull)` when exceeded. The `PoolFull` variant already exists in `crates/node/txpool/src/error.rs` (line 11) but is never used anywhere in the codebase.

```rust
pub fn add(&self, tx: OrderedTransaction) -> Result<(), TxPoolError> {
    let mut inner = self.inner.write();

    if inner.by_hash.contains_key(&tx.hash) {
        return Err(TxPoolError::AlreadyExists);
    }

    // Check global limits BEFORE insertion
    if inner.pending_count >= self.config.max_pending_txs {
        return Err(TxPoolError::PoolFull);
    }
    if inner.queued_count >= self.config.max_queued_txs {
        return Err(TxPoolError::PoolFull);
    }

    let sender = tx.sender;
    let queue =
        inner.by_sender.entry(sender).or_insert_with(|| SenderQueue::new(sender, tx.nonce));

    if queue.total_count() >= self.config.max_txs_per_sender {
        return Err(TxPoolError::SenderFull(sender));
    }

    // ... rest of insertion logic ...
}
```

There is a subtlety here: whether a new transaction will end up in the pending or queued category depends on its nonce relative to the sender's `next_nonce`. Before insertion, we do not know which category it will land in. A conservative approach is to check whether the TOTAL pool size (`pending_count + queued_count`) exceeds a combined limit. Alternatively, check each category after determining where the transaction would be placed but before actually inserting it.

### Step 2: Add differentiated PoolFull variants (optional)

The existing `TxPoolError::PoolFull` variant (line 11 in `error.rs`) carries no detail. Consider adding specificity:

```rust
#[error("pool pending queue is full ({count}/{max})")]
PendingPoolFull { count: usize, max: usize },

#[error("pool queued queue is full ({count}/{max})")]
QueuedPoolFull { count: usize, max: usize },
```

This helps operators and RPC callers understand which limit was hit.

### Step 3: Implement priority-based eviction (optional, higher effort)

Instead of a hard reject when the pool is full, evict the lowest-priority transaction to make room for a higher-priority one. This is how mature EVM clients (geth, reth) handle pool saturation:

1. When the pool is full and a new transaction arrives, compare its `effective_gas_price` against the lowest `effective_gas_price` currently in the pool.
2. If the new transaction has a higher gas price, evict the lowest-priority transaction and insert the new one.
3. If the new transaction has a lower gas price, reject it with `PoolFull`.

This requires maintaining a min-heap or sorted index by gas price across the entire pool, which is a non-trivial data structure addition. The `OrderedTransaction` already implements `Ord` by `effective_gas_price` (descending), `timestamp` (ascending), and `hash` (ascending), as defined in `crates/node/txpool/src/ordering.rs` (lines 59-67), so the ordering logic already exists.

### Step 4: Propagate PoolFull through the RPC layer

Currently, the `Mempool::insert()` trait returns `bool`, which loses the error type information. To propagate `PoolFull` to the RPC caller:

1. Change the `Mempool` trait's `insert()` to return `Result<bool, TxPoolError>` or a similar error type. This is a breaking trait change that affects `InMemoryMempool` as well.
2. In the `tx_submit` callback wired in `crates/node/runner/src/runner.rs`, map `TxPoolError::PoolFull` to an appropriate JSON-RPC error.
3. In `crates/node/rpc/src/eth.rs`, the `send_raw_transaction` method (line 299) should return an RPC error with a descriptive message and appropriate error code when the pool is full.

The standard Ethereum JSON-RPC convention for "transaction pool is full" is error code `-32000` (server error) with a message like `"transaction pool is full"`. This is what geth returns in this situation.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/txpool/src/pool.rs` | Move limit checks before insertion. Return `Err(TxPoolError::PoolFull)` when exceeded. Remove the post-insertion warning-only checks. |
| `crates/node/txpool/src/error.rs` | The `PoolFull` variant already exists (line 11). Optionally add `PendingPoolFull` and `QueuedPoolFull` variants with count/max fields for better diagnostics. |
| `crates/node/txpool/src/traits.rs` | Consider changing `Mempool::insert()` return type from `bool` to `Result<bool, TxPoolError>` to propagate specific error types to callers. |
| `crates/node/rpc/src/eth.rs` | Map `TxPoolError::PoolFull` to JSON-RPC error code `-32000` with message `"transaction pool is full"` in the `send_raw_transaction` handler. |
| `crates/node/runner/src/runner.rs` | Update the `tx_submit` callback to propagate pool-full errors from the mempool insert path to the RPC layer. |

---

## Testing

### 1. Unit test: hard rejection at pending limit

Configure a pool with `max_pending_txs = 3`. Insert 3 transactions from 3 different senders (all at nonce 0, so they are all pending). Insert a 4th transaction from a new sender. Verify that it is rejected with `TxPoolError::PoolFull` and that `pool.pending_count()` remains 3.

### 2. Unit test: hard rejection at queued limit

Configure a pool with `max_queued_txs = 2`. Insert a transaction at nonce 0 for sender A (goes to pending). Insert 2 transactions at nonce 5 and nonce 6 for senders B and C respectively (nonce gap, so they go to queued). Insert a 3rd queued transaction. Verify rejection with `TxPoolError::PoolFull` and that `pool.queued_count()` remains 2.

### 3. Unit test: pool accepts after space is freed

Configure a pool with `max_pending_txs = 2`. Fill it. Remove one transaction via `pool.remove()`. Insert a new transaction. Verify it succeeds.

### 4. Unit test: eviction of lowest-priority transaction (if eviction is implemented)

Configure a pool with `max_pending_txs = 2`. Insert 2 transactions with gas prices 100 and 200. Insert a 3rd transaction with gas price 300. Verify that the gas-price-100 transaction is evicted and the pool contains the gas-price-200 and gas-price-300 transactions. Insert a 4th transaction with gas price 50. Verify it is rejected (lower than any existing transaction).

### 5. Integration test: RPC returns meaningful error

Submit transactions via `eth_sendRawTransaction` until the pool is full. Verify that subsequent submissions receive a JSON-RPC error response with code `-32000` and a message indicating the pool is full, rather than a silent acceptance.

### 6. Load test: pool size stays bounded

Run the load generator against a single node. Configure `max_pending_txs = 1000`. Verify via metrics or the `pool.len()` method that the pool size never exceeds the configured limit, even under sustained submission rates exceeding the chain's throughput.

---

## Cross-References

- **Issue 02 (Replace InMemoryMempool with TransactionPool):** The `TransactionPool` is the intended production mempool. Its limit enforcement should be correct before it is wired into the production path. The `InMemoryMempool` has no size limits at all (it is a flat `BTreeMap` with no bounds), so this issue is specific to `TransactionPool`.
- **Issue 26 (Transaction TTL):** Without TTL, transactions persist in the pool indefinitely. Combined with no hard size limit, this means the pool can grow monotonically during periods when finalization is not pruning transactions. Fixing both issues together provides two layers of defense: TTL removes stale transactions by age, and hard size limits cap total pool occupancy regardless of age.
