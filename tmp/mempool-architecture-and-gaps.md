# Mempool Architecture and Gaps

## What is Kora?

Kora is an EVM-compatible blockchain built on the Commonware framework. It uses **Simplex BFT** consensus with a network of 4 validators. Leader election is performed via a **threshold VRF** (Verifiable Random Function) using BLS12-381 signatures, where a random seed derived from threshold-signed notarizations determines which validator proposes each block. The system processes Ethereum-format transactions (EIP-1559, EIP-2930, Legacy) and executes them using REVM (Rust EVM).

---

## Mempool Architecture Overview

Kora has **two distinct mempool implementations** that serve different roles in the transaction lifecycle:

1. **`InMemoryMempool`** -- a minimal consensus-layer mempool used during block proposal.
2. **`TransactionPool`** -- a full-featured transaction pool with validation, nonce tracking, and fee ordering.

Both implement the same `Mempool` trait (defined in `crates/node/txpool/src/traits.rs`), meaning the system can use either as the backing store. In practice, the `TransactionPool` is the production implementation that is wired into the `LedgerService`.

---

## Implementation 1: InMemoryMempool (Consensus Layer)

**File**: `crates/node/consensus/src/components/mempool.rs`

```rust
pub struct InMemoryMempool {
    inner: Arc<RwLock<BTreeMap<TxId, Tx>>>,  // line 16
}
```

### Data Structure

A single `BTreeMap<TxId, Tx>` wrapped in `Arc<RwLock<...>>` for thread-safe shared access. `TxId` is a hash-based identifier; `Tx` contains raw transaction bytes (`Bytes`).

### Behavior

| Method | Lines | Behavior |
|--------|-------|----------|
| `insert(tx)` | 44-48 | Inserts raw bytes keyed by hash. Returns `true` if new. No validation whatsoever. |
| `build(max_txs, excluded)` | 50-59 | Collects all non-excluded txs, sorts by `(decode_success, sender, nonce)`, takes first `max_txs`. |
| `prune(tx_ids)` | 61-66 | Removes entries by their TxId hash. |
| `len()` | 68-70 | Returns count of entries. |

### Sorting Key (`tx_order_key`, lines 33-41)

```rust
fn tx_order_key(tx: &Tx) -> (u8, Address, u64) {
    // Attempts to decode the envelope and recover the sender
    // On success: (0, sender_address, nonce)
    // On failure: (1, Address::ZERO, u64::MAX)  -- sorted to the end
}
```

This means decodable transactions sort before non-decodable ones, then by sender address, then by nonce within each sender. There is **no fee-based priority** in this implementation.

### What InMemoryMempool Does NOT Do

- No signature validation
- No nonce validation against state
- No balance checks
- No gas price enforcement
- No size limits (unbounded growth)
- No per-sender limits
- No fee-based priority ordering
- No transaction expiration

---

## Implementation 2: TransactionPool (txpool Crate)

**File**: `crates/node/txpool/src/pool.rs`

```rust
pub struct TransactionPool {
    inner: RwLock<PoolInner>,  // line 87
    config: PoolConfig,        // line 88
}

struct PoolInner {
    by_hash: HashMap<B256, OrderedTransaction>,   // line 62
    by_sender: HashMap<Address, SenderQueue>,     // line 63
    pending_count: usize,                         // line 64
    queued_count: usize,                          // line 65
}
```

### Data Structures

- **`by_hash`**: Direct lookup of any transaction by its keccak256 hash.
- **`by_sender`**: Per-sender queues that track nonce ordering. Each `SenderQueue` (defined in `crates/node/txpool/src/ordering.rs`) contains:
  - `next_nonce: u64` -- the next expected executable nonce
  - `pending: Vec<OrderedTransaction>` -- executable transactions with consecutive nonces starting from `next_nonce`
  - `queued: Vec<OrderedTransaction>` -- future-nonce transactions waiting for gaps to fill

- **`OrderedTransaction`** (file: `crates/node/txpool/src/ordering.rs`, line 10): Contains `hash`, `sender`, `nonce`, `effective_gas_price`, `timestamp`, and the decoded `TxEnvelope`. Ordering is by descending gas price, then ascending timestamp, then hash as tiebreaker (lines 59-67).

### Key Operations

| Method | Lines | Behavior |
|--------|-------|----------|
| `add(tx)` | 99-142 | Checks duplicate, per-sender limit, inserts into sender queue. Warns if pool exceeds size limits (but does not reject). |
| `pending(max_txs)` | 145-154 | Returns all pending txs sorted by `OrderedTransaction::cmp` (gas price priority). |
| `remove_confirmed(sender, nonce)` | 188-216 | Removes all txs for a sender with nonce <= confirmed_nonce. Promotes queued txs. |
| `build(max_txs, excluded)` | 316-356 | Iterates senders round-robin, picking the highest-gas-price next-nonce candidate from each sender per round. Respects nonce continuity. |
| `prune(tx_ids)` | 358-401 | Finds the max confirmed nonce per sender from the given tx_ids, removes all txs with nonce <= that value, and promotes queued txs. |

### Build Algorithm (Block Construction)

The `build` method (lines 316-356) uses a sophisticated approach:

1. Initializes a `BuildSenderState` per sender containing their pending txs and expected next nonce.
2. Iterates in a loop: picks the sender whose next candidate has the lowest `OrderedTransaction` ordering (highest gas price).
3. Advances that sender's nonce and continues until `max_txs` is reached or no candidates remain.
4. Excluded transactions are treated as consumed (nonce advances past them), allowing subsequent txs to still be selected.

---

## Transaction Validation

**File**: `crates/node/txpool/src/validator.rs`

The `TransactionValidator<S>` (line 43) performs comprehensive validation before pool insertion:

| Check | Lines | Rule |
|-------|-------|------|
| Size limit | 57-60 | `tx.bytes.len() <= config.max_tx_size` (default: 128 KB) |
| RLP decode | 64-65 | Must be valid EIP-2718 encoded envelope |
| Chain ID | 67-69 | Must match configured chain ID |
| Signature recovery | 73 | ECDSA signature must be valid, sender recoverable |
| Gas price | 75-79 | `effective_gas_price >= config.min_gas_price` |
| Intrinsic gas | 82-89 | `gas_limit >= intrinsic_gas` (base 21000 + data + create + access list) |
| Nonce floor | 94-95 | `nonce >= state_nonce` (rejects stale) |
| Nonce ceiling | 97-99 | `nonce <= state_nonce + max_txs_per_sender` (rejects far-future) |
| Balance | 102-109 | `balance >= gas_limit * max_fee + value` |

---

## Transaction Flow: End to End

```
1. User sends eth_sendRawTransaction via RPC
       |
       v
2. TransactionValidator.validate(tx)  [crates/node/txpool/src/validator.rs:56]
   - Decode, verify signature, check chain_id, gas, nonce, balance
       |
       v
3. LedgerService.submit_tx(tx)  [crates/node/ledger/src/lib.rs:377]
   - Calls inner mempool.insert(tx)
   - Emits LedgerEvent::TransactionSubmitted
       |
       v
4. TransactionPool.insert(tx) / InMemoryMempool.insert(tx)
   - Adds to pool data structures
       |
       v
5. Leader elected via threshold VRF
       |
       v
6. RevmApplication.build_block(parent)  [crates/node/runner/src/app.rs:84]
   - Calls mempool.build(max_txs, excluded)
   - max_txs = BLOCK_CODEC_MAX_TXS = 10,000
       |
       v
7. executor.execute(parent_state, context, txs)
   - REVM executes each transaction against EVM state
   - Returns ExecutionOutcome with changes, receipts, gas_used
   - If ANY tx fails fatally: entire block proposal fails -> returns None
       |
       v
8. Block proposed to consensus (Simplex BFT)
   - Other validators verify by re-executing
   - Notarized with 2/3+ threshold signatures
       |
       v
9. Block finalized
       |
       v
10. FinalizedReporter.report(block)  [crates/node/reporters/src/lib.rs:414]
    - Persists snapshot to QMDB
    - state.prune_mempool(&block.txs)  [line 226]
    - Acknowledges to consensus
```

---

## Configuration: PoolConfig Defaults

**File**: `crates/node/txpool/src/config.rs`

| Parameter | Default Value | Purpose |
|-----------|--------------|---------|
| `max_pending_txs` | 4,096 | Soft limit on total executable transactions (warns, does not reject) |
| `max_queued_txs` | 1,024 | Soft limit on future-nonce transactions (warns, does not reject) |
| `max_txs_per_sender` | 256 | Hard limit per sender account (rejects at pool insertion) |
| `max_tx_size` | 131,072 (128 KB) | Maximum raw transaction byte size |
| `min_gas_price` | 0 | Minimum effective gas price (any price accepted by default) |
| `replacement_bump_percent` | 10 | Gas price increase required to replace existing tx at same nonce |

**Block-level limit**: `BLOCK_CODEC_MAX_TXS = 10,000` (file: `crates/node/runner/src/runner.rs`, line 44).

---

## Gap Analysis

### Gap 1: No Size Limits on InMemoryMempool

**Severity**: Medium
**File**: `crates/node/consensus/src/components/mempool.rs`, line 16

**Problem**: The `BTreeMap<TxId, Tx>` has no maximum capacity. If `InMemoryMempool` is used as the backing mempool, it grows without bound under sustained load or spam.

**Impact**: Under attack or high load, memory consumption grows linearly with submitted transactions. There is no eviction policy. The node will eventually OOM.

**Note**: When `TransactionPool` is used instead (the production path), the `max_txs_per_sender` limit of 256 provides a per-sender bound, and the pool warns at 4,096 pending / 1,024 queued. However, these are **soft limits** -- the pool still accepts transactions beyond `max_pending_txs` and `max_queued_txs`, it only logs a warning (lines 125-139 in `pool.rs`).

**Proposed Fix**: Add a hard capacity limit to the pool. When the limit is reached, evict the lowest-priority (lowest gas price) transactions. For `InMemoryMempool`, add a `max_size` field and reject inserts beyond it.

---

### Gap 2: No Gossip / Cross-Validator Mempool Synchronization

**Severity**: High
**Files**: All mempool implementations

**Problem**: Each validator maintains a completely independent mempool. There is no transaction gossip protocol between validators. A transaction submitted to validator A's RPC endpoint exists only in validator A's mempool.

**Impact**:
- If validator B becomes leader, it cannot include transactions that were only submitted to validator A.
- Users must broadcast transactions to all validators externally, or rely on a single entry point.
- After finalization on one validator, other validators retain stale copies of transactions that were externally submitted to them but not pruned because they were not in the finalized block's tx list.

**Current Workaround**: The devnet sends all transactions to all validators via external tooling.

**Proposed Fix**: Implement transaction gossip using Commonware's networking layer. When a validator receives a new valid transaction, broadcast it to peers. Peers validate and insert into their own pools.

---

### Gap 3: Pruning Only Removes Block-Included Transactions

**Severity**: Critical
**File**: `crates/node/reporters/src/lib.rs`, line 226

```rust
state.prune_mempool(&block.txs).await;
```

**Problem**: The `prune_mempool` call only removes transactions that were literally included in the finalized block. It does NOT remove transactions that have become stale (nonce < on-chain nonce) due to other transactions from the same sender being finalized.

**Example**:
1. Sender has on-chain nonce 5.
2. Validator A's mempool has tx with nonce 5, 6, 7.
3. Validator B finalizes a block containing nonce 5 from its own pool.
4. Validator A's pruning removes nothing (its nonce-5 tx has a different hash than the one in the block).
5. Validator A's mempool now has nonce 5 (stale), 6, 7.
6. When A becomes leader, `build()` sorts by nonce and returns nonce 5 first.
7. Executor rejects nonce 5 (already consumed) and the entire block fails.

**Impact**: Permanent chain stall if all validators accumulate stale transactions. See the pruning bug document for the full cascade.

**Proposed Fix**: After finalization, query the finalized state for each sender present in the mempool and prune all transactions with `nonce < finalized_nonce`. Alternatively, the `prune` method should accept a `(sender, confirmed_nonce)` map rather than just tx IDs.

---

### Gap 4: Nonce Validation Against Stale State

**Severity**: Medium
**File**: `crates/node/txpool/src/validator.rs`, lines 92-95

```rust
let state_nonce = self.state.nonce(&sender).await...;
if nonce < state_nonce {
    return Err(TxPoolError::NonceTooLow { got: nonce, expected: state_nonce });
}
```

**Problem**: The validator checks the nonce against persisted QMDB state, which only updates after finalization. It does NOT account for:
- Transactions already pending in the mempool for the same sender.
- Transactions in notarized-but-not-yet-finalized blocks.

**Impact**: Duplicate nonces from the same sender can enter the pool. When build() selects both, only one can execute; the duplicate causes wasted block space or (if the executor uses fatal errors) block proposal failure.

**Mitigating Factor**: The `SenderQueue` in `TransactionPool` handles nonce replacement -- if a tx arrives at an already-occupied nonce slot, it must have a 10% higher gas price to replace. So exact duplicates are rejected at the queue level. However, if the same sender submits to multiple validators, each validator's pool independently accepts the same nonce.

**Proposed Fix**: Track "effective nonce" as `max(state_nonce, highest_pending_nonce + 1)` per sender. Or maintain a pending nonce cache that increments as txs are inserted.

---

### Gap 5: No Transaction Expiration / TTL

**Severity**: Low
**Files**: All mempool implementations

**Problem**: Transactions remain in the mempool indefinitely. There is no time-based eviction. The `OrderedTransaction` records a `timestamp` field (line 20 in `ordering.rs`), but it is only used for tie-breaking in ordering, never for expiration.

**Impact**: Stale transactions from previous test runs, abandoned operations, or gas-price-insufficient transactions accumulate forever, consuming memory and polluting block proposals.

**Proposed Fix**: Add a configurable TTL (e.g., 6 hours). Periodically sweep and remove transactions older than the TTL. Use the existing `timestamp` field.

---

### Gap 6: Soft Limits on Pool Size (Warn but Accept)

**Severity**: Medium
**File**: `crates/node/txpool/src/pool.rs`, lines 125-139

```rust
if inner.pending_count > self.config.max_pending_txs {
    warn!(count = inner.pending_count, max = self.config.max_pending_txs, "pool exceeds pending limit");
}
// ... (same for queued)
```

**Problem**: When the pool exceeds `max_pending_txs` (4,096) or `max_queued_txs` (1,024), it only logs a warning. The transaction is still accepted. There is no backpressure or rejection.

**Impact**: Under sustained load, the pool can grow far beyond configured limits. The limits are advisory only.

**Proposed Fix**: Make these hard limits. When exceeded, reject new transactions with a specific error (e.g., `TxPoolError::PoolFull`). Optionally implement eviction of lowest-priority transactions to make room.

---

### Gap 7: Early Returns in Finalization Skip Pruning

**Severity**: Critical
**File**: `crates/node/reporters/src/lib.rs`, lines 105-232

**Problem**: The `handle_finalized_update` function has multiple early-return paths (execution failure at line 143, root computation failure at line 155, state root mismatch at line 166, missing parent snapshot at line 197, persist task failure at line 211, persist data failure at line 216) that all call `ack.acknowledge()` and return WITHOUT calling `prune_mempool`. A block that is consensus-final (has threshold BLS signatures) is irreversible -- its transactions have consumed nonces regardless of local persistence success.

**Impact**: If any error path triggers, finalized transactions remain in the mempool and will be re-proposed in future blocks, causing executor failures and view nullifications.

**Proposed Fix**: Move `prune_mempool` before the persist step, or ensure it is called on every path after the block is known to be finalized. The block is irrevocably part of the canonical chain once consensus finalizes it.

---

## Summary Table

| # | Gap | Severity | Component |
|---|-----|----------|-----------|
| 1 | No size limits on InMemoryMempool | Medium | `consensus/src/components/mempool.rs` |
| 2 | No cross-validator gossip | High | All mempool code |
| 3 | Pruning only removes included txs | Critical | `reporters/src/lib.rs:226` |
| 4 | Nonce validation against stale state | Medium | `txpool/src/validator.rs:92-95` |
| 5 | No transaction expiration | Low | All mempool code |
| 6 | Pool size limits are advisory only | Medium | `txpool/src/pool.rs:125-139` |
| 7 | Early returns skip pruning | Critical | `reporters/src/lib.rs:105-232` |

---

## Relevant Source Files

| File | Purpose |
|------|---------|
| `crates/node/consensus/src/components/mempool.rs` | InMemoryMempool (simple BTreeMap-backed) |
| `crates/node/txpool/src/pool.rs` | TransactionPool (production, nonce-aware, fee-ordered) |
| `crates/node/txpool/src/validator.rs` | TransactionValidator (signature, nonce, balance checks) |
| `crates/node/txpool/src/ordering.rs` | OrderedTransaction and SenderQueue data structures |
| `crates/node/txpool/src/config.rs` | PoolConfig with all tunable parameters |
| `crates/node/txpool/src/traits.rs` | Mempool trait (insert, build, prune, len) |
| `crates/node/reporters/src/lib.rs` | FinalizedReporter (finalization callback, pruning) |
| `crates/node/ledger/src/lib.rs` | LedgerService (submit_tx, prune_mempool delegation) |
| `crates/node/runner/src/app.rs` | RevmApplication (build_block using mempool.build) |
| `crates/node/runner/src/runner.rs` | Runner wiring, BLOCK_CODEC_MAX_TXS=10,000 |
| `crates/node/consensus/src/proposal.rs` | ProposalBuilder (calls mempool.build) |
| `crates/node/consensus/src/execution.rs` | BlockExecution (executor wrapper) |
