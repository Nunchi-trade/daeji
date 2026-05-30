# Mempool: Replace InMemoryMempool with nonce-aware TransactionPool

**Priority:** High
**Component:** `kora-consensus`, `kora-txpool`, `kora-ledger`
**Type:** Architectural improvement

---

## Summary

The production mempool used by Kora validators is `InMemoryMempool`, a flat `BTreeMap<TxId, Tx>` that indexes transactions by hash and has zero awareness of sender addresses, nonces, or gas prices. A fully-featured replacement, `TransactionPool`, already exists in the `kora-txpool` crate with per-sender nonce ordering, replacement-by-gas-price, and nonce-based bulk eviction -- but it is not wired in. Swapping the active mempool implementation eliminates several classes of bugs, including stale-nonce persistence, duplicate transaction storms, and same-nonce conflicts.

---

## Background

Kora is an EVM-compatible blockchain built on the Commonware consensus framework (Simplex BFT). It uses REVM for transaction execution.

Kora uses a fixed validator set of 4 nodes. Leader election is performed via a threshold VRF (Verifiable Random Function) using BLS12-381 signatures, where a random seed derived from threshold-signed notarizations determines which validator proposes each block. Finalization requires 2f+1 votes (3 out of 4 validators).

**Important:** Kora does NOT implement P2P transaction gossip between validators. Each validator maintains its own independent mempool. Transactions only reach a validator if submitted directly to that validator's RPC endpoint, or via external tooling (e.g., the loadgen broadcasting to all validators). This architectural fact is critical because it means different validators can have different mempool contents for the same sender.

The transaction lifecycle in Kora is:

1. **RPC ingestion**: A user submits a signed EVM transaction via the JSON-RPC endpoint (`eth_sendRawTransaction`).
2. **Validation**: The `TransactionValidator` (`crates/node/txpool/src/validator.rs`) checks RLP decode, chain ID, signature, gas price, intrinsic gas, nonce bounds, and balance before accepting the transaction.
3. **Mempool insertion**: The validator inserts the transaction into its local mempool via `ledger.submit_tx(tx)`.
4. **Block proposal**: When a validator is elected leader, it calls `build_block()`, which drains transactions from the mempool and passes them to the REVM executor.
5. **Execution**: The executor runs each transaction through REVM, producing state changes and receipts.
6. **Consensus**: The proposed block is voted on by all validators (Simplex BFT requires 2/3+ votes).
7. **Finalization**: Once finalized, the finalization reporter persists the block and prunes the executed transactions from the mempool.

The mempool is the critical bridge between transaction ingestion and block building. Its properties directly determine what transactions enter the executor, in what order, and how stale transactions are cleaned up.

---

## The Problem: InMemoryMempool

The current production mempool is `InMemoryMempool`, defined in `crates/node/consensus/src/components/mempool.rs`:

```rust
// crates/node/consensus/src/components/mempool.rs

/// Simple in-memory mempool backed by a BTreeMap.
#[derive(Debug, Clone)]
pub struct InMemoryMempool {
    inner: Arc<RwLock<BTreeMap<TxId, Tx>>>,
}

impl Mempool for InMemoryMempool {
    fn insert(&self, tx: Tx) -> bool {
        let id = tx.id();
        let mut inner = self.inner.write();
        inner.insert(id, tx).is_none()
    }

    fn build(&self, max_txs: usize, excluded: &std::collections::BTreeSet<TxId>) -> Vec<Tx> {
        let inner = self.inner.read();
        let mut candidates: Vec<_> = inner
            .iter()
            .filter(|(id, _)| !excluded.contains(id))
            .map(|(id, tx)| (tx_order_key(tx), *id, tx.clone()))
            .collect();
        candidates.sort_by_key(|(order, id, _)| (*order, *id));
        candidates.into_iter().take(max_txs).map(|(_, _, tx)| tx).collect()
    }

    fn prune(&self, tx_ids: &[TxId]) {
        let mut inner = self.inner.write();
        for id in tx_ids {
            inner.remove(id);
        }
    }

    fn len(&self) -> usize {
        self.inner.read().len()
    }
}
```

This implementation has the following problems:

### 1. No sender awareness

The `BTreeMap<TxId, Tx>` is keyed by transaction hash (`keccak256(raw_bytes)`). There is no per-sender data structure. The mempool cannot answer questions like "what is the next expected nonce for sender X?" or "how many pending transactions does sender Y have?"

While there is a `tx_order_key()` helper (lines 33-41) that decodes the sender and nonce for sorting purposes during `build()`, this is a best-effort sort computed on every call -- it does not maintain any persistent sender state. It returns `(0, sender, nonce)` on successful decode, or `(1, Address::ZERO, u64::MAX)` on failure (sorting bad transactions to the end).

### 2. No nonce validation on insert

`insert()` accepts any `Tx` as long as the hash is not already present. A transaction with nonce 0 for a sender whose on-chain nonce is already 100 will be accepted and will sit in the mempool indefinitely, poisoning every block proposal (see issue #01).

### 3. No replacement-by-gas-price

If a user submits two transactions with the same nonce (a common pattern for "speeding up" a stuck transaction), both are accepted because they have different hashes. The executor will fail on the second one (nonce already used), and the second transaction will persist in the mempool as a poison pill.

### 4. No size limits per sender

A single sender can flood the mempool with an unlimited number of transactions, potentially crowding out transactions from other senders.

### 5. Hash-based pruning only

The `prune()` method removes transactions by hash. After a block is finalized, only the exact transactions included in that block are removed. There is no mechanism to remove other transactions from the same sender that are now stale (e.g., transactions with nonces below the sender's new on-chain nonce).

---

## The Solution Already Exists: TransactionPool

The `kora-txpool` crate (`crates/node/txpool/`) contains a fully-featured `TransactionPool` that already implements the `Mempool` trait. It addresses every deficiency listed above.

### TransactionPool structure (`crates/node/txpool/src/pool.rs`):

```rust
// crates/node/txpool/src/pool.rs

#[derive(Debug)]
struct PoolInner {
    by_hash: HashMap<B256, OrderedTransaction>,
    by_sender: HashMap<Address, SenderQueue>,
    pending_count: usize,
    queued_count: usize,
}

/// A thread-safe transaction pool with nonce ordering and fee prioritization.
#[derive(Debug)]
pub struct TransactionPool {
    inner: RwLock<PoolInner>,
    config: PoolConfig,
}
```

Key features:

- **Dual indexing**: `by_hash` for O(1) lookup by transaction hash, `by_sender` for per-sender nonce management.
- **Configurable limits**: `PoolConfig` (`crates/node/txpool/src/config.rs`) provides `max_pending_txs` (default 4096), `max_queued_txs` (default 1024), `max_txs_per_sender` (default 256), `max_tx_size` (default 128 KB), `min_gas_price`, and `replacement_bump_percent` (default 10%).
- **Nonce-based bulk eviction**: `remove_confirmed(sender, confirmed_nonce)` removes all transactions with nonces up to and including the confirmed nonce, then promotes any queued transactions that are now executable.

### SenderQueue: per-sender nonce ordering (`crates/node/txpool/src/ordering.rs`):

```rust
// crates/node/txpool/src/ordering.rs

/// Per-sender queue managing nonce-ordered transactions.
#[derive(Debug, Clone)]
pub struct SenderQueue {
    /// Sender address.
    pub sender: Address,
    /// Next expected nonce for pending transactions.
    pub next_nonce: u64,
    /// Executable transactions with consecutive nonces.
    pub pending: Vec<OrderedTransaction>,
    /// Future transactions waiting for nonce gaps to fill.
    pub queued: Vec<OrderedTransaction>,
}
```

Each sender gets a `SenderQueue` that maintains:

- `next_nonce`: The lowest nonce that has not yet been confirmed on-chain. Transactions with nonces below this are rejected on insert.
- `pending`: Transactions with consecutive nonces starting from `next_nonce`. These are immediately executable.
- `queued`: Transactions with nonces beyond the pending range (nonce gaps). These are promoted to `pending` automatically when the gap is filled.

The `insert()` method handles replacement-by-gas-price:

```rust
// crates/node/txpool/src/ordering.rs, lines 91-116
pub fn insert(&mut self, tx: OrderedTransaction) -> Option<OrderedTransaction> {
    if tx.nonce < self.next_nonce {
        return Some(tx);  // Reject stale nonce
    }

    if tx.nonce == self.next_nonce + self.pending.len() as u64 {
        self.pending.push(tx);
        self.promote_queued();
        None
    } else if tx.nonce > self.next_nonce + self.pending.len() as u64 {
        // Future nonce -- queue it
        let pos = self.queued.binary_search_by(|q| q.nonce.cmp(&tx.nonce)).unwrap_or_else(|p| p);
        self.queued.insert(pos, tx);
        None
    } else {
        // Same nonce as existing pending tx -- replace if higher gas price
        let idx = (tx.nonce - self.next_nonce) as usize;
        if idx < self.pending.len() {
            let existing = &self.pending[idx];
            if tx.effective_gas_price > existing.effective_gas_price {
                let old = std::mem::replace(&mut self.pending[idx], tx);
                return Some(old);
            }
        }
        Some(tx)  // Reject: existing tx has equal or higher gas price
    }
}
```

The `remove_confirmed()` method handles post-finalization cleanup:

```rust
// crates/node/txpool/src/ordering.rs, lines 130-137
pub fn remove_confirmed(&mut self, confirmed_nonce: u64) {
    self.pending.retain(|tx| tx.nonce > confirmed_nonce);
    self.queued.retain(|tx| tx.nonce > confirmed_nonce);
    if confirmed_nonce >= self.next_nonce {
        self.next_nonce = confirmed_nonce + 1;
    }
    self.promote_queued();
}
```

### TransactionPool already implements the Mempool trait (`crates/node/txpool/src/pool.rs`, lines 300-406):

```rust
impl Mempool for TransactionPool {
    fn insert(&self, tx: Tx) -> bool {
        let Some(ordered) = tx_to_ordered(&tx) else {
            trace!("failed to decode transaction for mempool insert");
            return false;  // Rejects malformed transactions at insertion time
        };
        match self.add(ordered) {
            Ok(()) => true,
            Err(e) => {
                trace!(?e, "failed to insert transaction");
                false
            }
        }
    }

    fn build(&self, max_txs: usize, excluded: &BTreeSet<TxId>) -> Vec<Tx> {
        // Nonce-aware building: iterates per-sender queues, selecting the
        // highest-gas-price executable transaction at each step, respecting
        // nonce ordering within each sender.
        // ...
    }

    fn prune(&self, tx_ids: &[TxId]) {
        // Nonce-aware pruning: determines the highest confirmed nonce per sender
        // from the pruned tx set, then calls remove_confirmed() which also
        // evicts all stale transactions below that nonce and promotes queued txs.
        // ...
    }

    fn len(&self) -> usize {
        self.inner.read().by_hash.len()
    }
}
```

The `prune()` implementation is particularly important. Unlike `InMemoryMempool::prune()` which only removes the exact transaction hashes provided, `TransactionPool::prune()` (lines 358-401) computes the highest confirmed nonce per sender and then calls `remove_confirmed()`, which removes *all* transactions at or below that nonce. This means that after a block containing nonce 5 for sender X is finalized, any lingering transactions with nonces 0-5 for that sender are also evicted -- even if they were different transactions (e.g., a replacement attempt that lost).

The actual `prune()` code shows the nonce-based approach:

```rust
// crates/node/txpool/src/pool.rs, lines 358-401
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();

    // Step 1: Find the max confirmed nonce per sender from the given tx_ids
    let mut confirmed_by_sender: HashMap<Address, u64> = HashMap::new();
    for id in tx_ids {
        if let Some(tx) = inner.by_hash.get(&id.0) {
            confirmed_by_sender
                .entry(tx.sender)
                .and_modify(|nonce| *nonce = (*nonce).max(tx.nonce))
                .or_insert(tx.nonce);
        }
    }

    // Step 2: Remove all txs with nonce <= confirmed_nonce for each sender
    for (sender, confirmed_nonce) in confirmed_by_sender {
        if let Some(queue) = inner.by_sender.get_mut(&sender) {
            // ... collect hashes to remove from by_hash ...
            queue.remove_confirmed(confirmed_nonce);
        }
    }
    // ... cleanup empty senders, update counts ...
}
```

**Critical limitation:** The `prune()` method can only identify senders for transactions that exist in its local `by_hash` map. If a finalized block contains a transaction that was submitted to a DIFFERENT validator (and thus only exists in that validator's pool), the sender will not be found in the local pool's `by_hash`, and no nonce-based pruning will occur for that sender. This is a cross-validator pruning gap. To fully solve this, after finalization, the system should query the finalized state for each sender's on-chain nonce and call `remove_confirmed()` directly.

---

## The Wiring Change

The change needed is in `crates/node/ledger/src/lib.rs`. The `LedgerState` struct currently uses `InMemoryMempool`:

```rust
// crates/node/ledger/src/lib.rs, lines 68-77
struct LedgerState {
    /// Pending transactions that are not yet included in finalized blocks.
    mempool: InMemoryMempool,
    /// Execution snapshots indexed by digest so we can replay ancestors.
    snapshots: InMemorySnapshotStore<OverlayState<QmdbState>>,
    /// Cached seeds for each digest used to compute prevrandao.
    seeds: InMemorySeedTracker,
    /// Underlying QMDB ledger service for persistence.
    qmdb: QmdbLedger,
}
```

And it is constructed in `LedgerView::init_with_config_and_genesis()` at line 148:

```rust
// crates/node/ledger/src/lib.rs, line 148
mempool: InMemoryMempool::new(),
```

The change is to replace these with `TransactionPool`:

```rust
use kora_txpool::{TransactionPool, PoolConfig};

struct LedgerState {
    mempool: TransactionPool,
    // ... rest unchanged
}

// In init_with_config_and_genesis():
mempool: TransactionPool::new(PoolConfig::default()),
```

The `proposal_components()` method (line 255) currently returns `InMemoryMempool` in its signature. This needs to be updated to return `TransactionPool`, or generalized to return `impl Mempool`. The same applies to all call sites in `crates/node/runner/src/app.rs` that use the mempool.

Both `InMemoryMempool` and `TransactionPool` implement the same `Mempool` trait (though from different crate copies -- `kora_consensus::traits::Mempool` and `kora_txpool::traits::Mempool`). These trait definitions are identical and should be unified into a single source.

---

## What This Fixes

### 1. Stale nonce persistence (root cause of chain stall)

With `InMemoryMempool`, a transaction with a stale nonce sits in the mempool forever because there is no nonce check on insert and hash-based pruning does not help (the stale transaction has a different hash than whatever was finalized at that nonce).

With `TransactionPool`, stale nonces are rejected at insert time (`SenderQueue::insert()` returns `Some(tx)` when `tx.nonce < self.next_nonce`). Even if a stale transaction somehow enters the pool, `prune()` after finalization will call `remove_confirmed()`, which evicts everything at or below the confirmed nonce.

### 2. Duplicate transaction storms

With `InMemoryMempool`, if a transaction is gossiped multiple times with slightly different encoding (but same logical content), each variant gets a unique hash and is inserted separately. The executor then fails on the second copy (nonce already used).

With `TransactionPool`, replacement-by-gas-price handles same-nonce conflicts. If a second transaction arrives for the same sender and nonce, it replaces the existing one only if it has a higher gas price; otherwise it is rejected.

### 3. Same-nonce conflicts (speed-up transactions)

Users commonly resubmit a transaction with the same nonce but higher gas price to "speed it up." With `InMemoryMempool`, both the original and the replacement are in the pool. The executor will include the first one it encounters and fail on the second.

With `TransactionPool`, the `SenderQueue` keeps at most one transaction per nonce. Higher-gas-price replacements evict the original.

### 4. Missing post-finalization nonce sweep

With `InMemoryMempool`, `prune()` only removes the exact transactions that were in the finalized block. If sender X had transactions at nonces 0, 1, 2, 3 in the mempool, and the finalized block included nonces 0 and 1, only those two are removed. Nonces 2 and 3 remain. If nonces 2 and 3 were submitted by a different wallet session and are actually stale (the sender's on-chain nonce jumped to 5 because of transactions from another source), they persist forever.

With `TransactionPool`, `prune()` calls `remove_confirmed()` with the highest confirmed nonce, which removes everything at or below that nonce and advances `next_nonce`. This is the correct EVM mempool behavior.

### 5. Per-sender flood protection

With `InMemoryMempool`, a single sender can insert an unlimited number of transactions. With `TransactionPool`, the `max_txs_per_sender` limit (default 256) prevents any single sender from monopolizing the mempool.

---

## What Additional Changes Are Needed

### 1. Unify the Mempool trait

The `Mempool` trait is currently defined in two places with identical signatures:

- `crates/node/consensus/src/traits.rs` (line 49)
- `crates/node/txpool/src/traits.rs` (line 10)

These should be unified into a single trait definition, likely in a shared `kora-domain` or `kora-consensus` crate, with both `InMemoryMempool` and `TransactionPool` implementing the canonical version.

### 2. Update `proposal_components()` return type

Both `LedgerView::proposal_components()` (line 255) and `LedgerService::proposal_components()` (line 441) in `crates/node/ledger/src/lib.rs` currently return `InMemoryMempool` by concrete type:

```rust
// LedgerView (line 255):
pub async fn proposal_components(
    &self,
) -> (OverlayState<QmdbState>, InMemoryMempool, InMemorySnapshotStore<OverlayState<QmdbState>>)

// LedgerService (line 441):
pub async fn proposal_components(
    &self,
) -> (OverlayState<QmdbState>, InMemoryMempool, InMemorySnapshotStore<OverlayState<QmdbState>>)
```

Both need to return `TransactionPool` instead (or be generalized with a type parameter / trait object).

### 3. Ensure `prune_mempool` in the finalization reporter invokes the nonce-aware prune

The call at `crates/node/reporters/src/lib.rs`, line 226:

```rust
state.prune_mempool(&block.txs).await;
```

This calls through to `LedgerView::prune_mempool()` which calls `mempool.prune(&tx_ids)`. When the mempool is `TransactionPool`, this will automatically invoke the nonce-aware `prune()` implementation that does `remove_confirmed()` per sender. No code change is needed here -- it is handled by the trait dispatch -- but it should be verified in testing.

### 4. Consider initializing `next_nonce` from on-chain state

When a `SenderQueue` is created for a new sender, it is initialized with `next_nonce` set to the nonce of the first transaction inserted (see `pool.rs` line 108: `SenderQueue::new(sender, tx.nonce)`). Ideally, it should be initialized from the sender's on-chain nonce so that stale transactions are rejected immediately. This may require passing a state reader to the pool's insert path, or performing a pre-validation step in the RPC layer.

Note: The RPC ingestion path already runs `TransactionValidator` (in `crates/node/runner/src/runner.rs`) which checks `nonce >= state_nonce` against QMDB. However, QMDB only updates after finalization, creating a window where stale nonces can pass validation. The `TransactionPool`'s per-sender `next_nonce` tracking provides an additional layer of defense beyond what `TransactionValidator` provides.

### 5. Ensure early return paths in finalization still prune the mempool

The `handle_finalized_update` function in `crates/node/reporters/src/lib.rs` (lines 105-232) has six early-return paths (lines 142-146, 154-158, 160-169, 197-199, 210-214, 216-220) that call `ack.acknowledge()` and return WITHOUT calling `prune_mempool()`. A finalized block is irrevocable -- its transactions have consumed nonces regardless of local persistence success. When `TransactionPool` is wired in, these paths become even more important to fix, because the nonce-aware pruning is the primary mechanism for cleaning up stale transactions. Consider moving `prune_mempool` before the persist step.

### 6. Update imports throughout the codebase

The `crates/node/ledger/src/lib.rs` file currently imports `InMemoryMempool` at line 17:

```rust
use kora_consensus::{
    ConsensusError, Mempool as _, SeedTracker as _, Snapshot, SnapshotStore as _,
    components::{InMemoryMempool, InMemorySeedTracker, InMemorySnapshotStore},
};
```

This import needs to change to bring in `TransactionPool` from `kora_txpool` instead. The `Mempool as _` import should come from whichever crate owns the unified trait.

---

## Testing

### 1. Nonce-aware insertion

- Insert transactions for sender A with nonces 0, 1, 2. Verify all are pending.
- Insert a transaction with nonce 5 (gap). Verify it goes to queued.
- Insert nonces 3 and 4. Verify nonce 5 is promoted to pending.
- Insert a transaction with nonce 0 again (stale). Verify it is rejected.

### 2. Replacement by gas price

- Insert a transaction for sender A at nonce 0 with gas price 100.
- Insert another transaction for sender A at nonce 0 with gas price 150.
- Verify the first is replaced and the pool contains only the gas-price-150 transaction.
- Insert a transaction at nonce 0 with gas price 90. Verify it is rejected (lower than current).

### 3. `remove_confirmed` after finalization

- Insert transactions for sender A with nonces 0, 1, 2, 3.
- Call `prune()` with the TxIds of nonces 0 and 1.
- Verify that `next_nonce` is now 2.
- Verify that only nonces 2 and 3 remain in the pool.
- Verify that inserting a transaction with nonce 0 or 1 is now rejected.

### 4. `build()` respects nonce ordering

- Insert transactions for sender A (nonces 0, 1, 2) and sender B (nonces 0, 1).
- Call `build(10, &empty_set)`.
- Verify that transactions are returned in a valid execution order: for each sender, nonces must be in ascending order. Between senders, higher gas price transactions should come first.

### 5. Per-sender limits

- Configure `max_txs_per_sender = 3`.
- Insert 3 transactions for sender A. Verify success.
- Insert a 4th transaction for sender A. Verify it fails with `TxPoolError::SenderFull`.

### 6. Integration test: end-to-end block production

- Wire `TransactionPool` into the ledger.
- Submit transactions via RPC.
- Verify blocks are produced and finalized.
- Verify that after finalization, stale transactions are cleaned up.
- Verify that a stale-nonce transaction does not stall the chain.

---

## Migration Considerations

- **No persistent state**: Both `InMemoryMempool` and `TransactionPool` are in-memory only. There is no on-disk mempool state to migrate. After a restart, the mempool is empty regardless of implementation.
- **Configuration**: `TransactionPool` requires a `PoolConfig`. The defaults (`max_pending_txs: 4096`, `max_queued_txs: 1024`, `max_txs_per_sender: 256`) are reasonable for a devnet/testnet. Production deployments may want to tune these via CLI flags or config file.
- **Trait unification**: The two `Mempool` trait definitions need to be consolidated before the swap. The simplest approach is to have `kora-txpool` re-export `kora-consensus`'s `Mempool` trait, or move the trait to a shared crate.
- **API surface**: `proposal_components()` changes its return type. All call sites in `crates/node/runner/src/app.rs` need to be updated. Since both types implement the same `Mempool` trait, the changes should be mechanical.
- **Backward compatibility**: This is a drop-in replacement at the trait level. The `Mempool` trait interface (`insert`, `build`, `prune`, `len`) is identical. The only externally visible behavior change is that invalid transactions are now rejected at insertion time and stale transactions are cleaned up more aggressively -- both of which are strictly improvements.
- **Block size limit**: The block builder uses `BLOCK_CODEC_MAX_TXS = 10,000` (defined in `crates/node/runner/src/runner.rs`, line 44) as the `max_txs` parameter passed to `mempool.build()`. This limit applies regardless of which mempool implementation is used.

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/node/ledger/src/lib.rs` | Replace `InMemoryMempool` with `TransactionPool` in `LedgerState` struct (line 70). Update import (line 17). Update constructor at line 148. |
| `crates/node/ledger/src/lib.rs` | Update `proposal_components()` return type on both `LedgerView` (line 255) and `LedgerService` (line 441) to return `TransactionPool` instead of `InMemoryMempool`. |
| `crates/node/runner/src/app.rs` | Update `build_block()` (line 92) and any other code that destructures `proposal_components()` to use the new return type. The `use kora_consensus::Mempool as _` import on line 85 may need to change depending on trait unification. |
| `crates/node/consensus/src/traits.rs` | (Optional) Unify the `Mempool` trait. Either keep this as the canonical definition and have `kora-txpool` depend on it, or move the trait to a shared crate. |
| `crates/node/txpool/src/traits.rs` | (Optional) Remove the duplicate `Mempool` trait definition if it is unified into `kora-consensus`. |
| `crates/node/ledger/Cargo.toml` | Add `kora-txpool` as a dependency. |

---

## Verification

### 1. Compile check

```bash
cargo build
```

The project must compile cleanly after the type swap. All call sites that use `proposal_components()` or the mempool directly must be updated.

### 2. Existing tests pass

```bash
cargo test -p kora-ledger
cargo test -p kora-txpool
cargo test -p kora-runner
```

All existing unit tests must continue to pass. The `kora-txpool` crate already has comprehensive tests for `TransactionPool` behavior (pool_add_and_pending, pool_duplicate_rejected, pool_sender_limit, pool_prune_advances_sender_nonce, pool_build_treats_excluded_ancestors_as_nonce_progress, pool_prune_promotes_queued_transactions_after_gap_fills).

### 3. Integration test: stale nonce rejected at pool level

After wiring in `TransactionPool`, submit a transaction with a stale nonce (nonce < sender's current `next_nonce` in the pool). Verify that `ledger.submit_tx()` returns `false` (rejected by `TransactionPool::insert()` via `SenderQueue::insert()`).

### 4. Integration test: nonce-aware pruning works

1. Submit transactions with nonces 0, 1, 2, 3 for sender A.
2. Build and finalize a block containing nonces 0 and 1.
3. Verify `prune_mempool()` removes nonces 0 and 1, and advances `next_nonce` to 2.
4. Verify nonces 2 and 3 remain executable.
5. Verify submitting a new transaction with nonce 0 or 1 is rejected.

### 5. Devnet E2E test

```bash
just devnet-up
# Submit transactions via RPC
# Verify blocks are produced and finalized
# Submit a stale-nonce transaction and verify it is rejected or cleaned up
# Verify chain does not stall
```

---

## Related Issues

- **Issue #01 (Executor fatal abort)**: The executor `?` operator bug is the immediate cause of chain stalls, but the root cause is bad transactions entering the mempool. Replacing `InMemoryMempool` with `TransactionPool` prevents most bad transactions from entering the pipeline in the first place.
- **Duplicate transaction storms**: When the loadgen broadcasts the same transaction to multiple validators, the `InMemoryMempool` accepts all copies. `TransactionPool`'s per-sender nonce tracking and replacement-by-gas-price eliminate this class of issue. See `tmp/duplicate-tx-storm.md` for details on the 70% failure rate observed during load testing.
- **Nonce validation gap**: The `TransactionValidator` checks nonces against QMDB state, which lags behind finalization. `TransactionPool`'s per-sender `next_nonce` tracking provides a second layer of defense. See `tmp/nonce-validation-gap.md` for the full analysis.
- **Issue #26 (Transaction TTL)**: `TransactionPool` records timestamps on `OrderedTransaction` (ordering.rs line 20) but never uses them for expiration. After wiring the pool, add TTL-based eviction using these existing timestamps.
- **Issue #27 (Pool size limits are soft)**: The current `TransactionPool` implementation only logs warnings when `max_pending_txs` (4,096) or `max_queued_txs` (1,024) are exceeded -- it still accepts the transaction. After wiring the pool, enforce these as hard limits with rejection and/or LRU eviction of lowest-gas-price entries.

---

## Additional Consideration: Soft Pool Limits

When wiring `TransactionPool`, be aware that its pool size limits are advisory only (`crates/node/txpool/src/pool.rs`, lines 125-139). The pool logs warnings but does NOT reject transactions when limits are exceeded:

```rust
if inner.pending_count > self.config.max_pending_txs {
    warn!(count = inner.pending_count, max = self.config.max_pending_txs, "pool exceeds pending limit");
}
// Transaction is STILL inserted after the warning
```

This should be converted to a hard limit as part of the wiring work. When the pool is full, either:
1. Reject the new transaction with `TxPoolError::PoolFull`, or
2. Evict the lowest-priority (lowest gas price) transaction to make room

See Issue #27 for standalone tracking of this concern.
