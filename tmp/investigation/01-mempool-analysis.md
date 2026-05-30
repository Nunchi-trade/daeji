# Mempool Analysis: InMemoryMempool vs TransactionPool

## Summary

Production uses `InMemoryMempool` — a bare `BTreeMap<TxId, Tx>` with zero validation, zero size limits, and O(n) ECDSA recovery per block build. The codebase also contains an unused `TransactionPool` with per-sender queues, nonce tracking, gas-price ordering, and configurable limits. The sophisticated pool is fully implemented but not wired into the runner.

---

## 1. InMemoryMempool (PRODUCTION — ACTIVE)

**File:** `crates/node/consensus/src/components/mempool.rs`

### Data Structure (Lines 13-17)
```rust
pub struct InMemoryMempool {
    inner: Arc<RwLock<BTreeMap<TxId, Tx>>>,
}
```

Single `BTreeMap` keyed by `TxId` (keccak256 hash). Protected by `Arc<RwLock>`.

### Methods

#### `insert(tx: Tx) -> bool` (Lines 44-48)
```rust
fn insert(&self, tx: Tx) -> bool {
    let id = tx.id();
    let mut inner = self.inner.write();
    inner.insert(id, tx).is_none()
}
```
- O(log n) BTreeMap insertion
- Returns `true` if newly inserted
- **No validation**: accepts any bytes as a transaction
- **No size limits**: unbounded growth
- **No deduplication logic**: relies solely on BTreeMap key uniqueness

#### `build(max_txs, excluded) -> Vec<Tx>` (Lines 50-59)
```rust
fn build(&self, max_txs: usize, excluded: &BTreeSet<TxId>) -> Vec<Tx> {
    let inner = self.inner.read();
    let mut candidates: Vec<_> = inner
        .iter()
        .filter(|(id, _)| !excluded.contains(id))
        .map(|(id, tx)| (tx_order_key(tx), *id, tx.clone()))
        .collect();
    candidates.sort_by_key(|(order, id, _)| (*order, *id));
    candidates.into_iter().take(max_txs).map(|(_, _, tx)| tx).collect()
}
```
- **O(n log n)**: clones ALL transactions, sorts, then takes top `max_txs`
- Every call triggers ECDSA recovery for every transaction (via `tx_order_key`)

#### `tx_order_key()` (Lines 33-41) — PERFORMANCE CRITICAL
```rust
fn tx_order_key(tx: &Tx) -> (u8, Address, u64) {
    let Ok(envelope) = TxEnvelope::decode_2718(&mut tx.bytes.as_ref()) else {
        return (1, Address::ZERO, u64::MAX);
    };
    let Ok(sender) = envelope.recover_signer() else {
        return (1, Address::ZERO, u64::MAX);
    };
    (0, sender, envelope.nonce())
}
```
- **ECDSA signature recovery per transaction per build() call**
- No caching of recovered senders
- Failed recoveries sorted last with sentinel values

#### `prune(tx_ids)` (Lines 61-66)
```rust
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();
    for id in tx_ids { inner.remove(id); }
}
```
- O(m log n) where m = tx_ids length
- Only called after successful finalization

#### `len()` (Lines 68-70)
- O(1) read

### Critical Issues

| Issue | Impact |
|-------|--------|
| No validation | Malformed/invalid transactions accepted into pool |
| No size limits | Unbounded memory growth under load |
| No per-sender caps | Single sender can flood the pool |
| No nonce awareness | Stale nonces stay in pool forever (until pruned) |
| O(n) ECDSA per build | CPU bottleneck on every block proposal |
| No fee prioritization | No way to prefer higher-value transactions |
| No replacement logic | Can't replace pending tx with higher gas price |

---

## 2. TransactionPool (UNUSED — COMPLETE IMPLEMENTATION)

**Files:**
- `crates/node/txpool/src/pool.rs` (348 lines)
- `crates/node/txpool/src/ordering.rs` (248 lines)
- `crates/node/txpool/src/validator.rs` (858 lines)
- `crates/node/txpool/src/config.rs`

### Data Structures (pool.rs:60-89)

```rust
struct PoolInner {
    by_hash: HashMap<B256, OrderedTransaction>,
    by_sender: HashMap<Address, SenderQueue>,
    pending_count: usize,
    queued_count: usize,
}

pub struct TransactionPool {
    inner: RwLock<PoolInner>,
    config: PoolConfig,
}
```

Dual-indexed: by hash (for lookups) and by sender (for nonce ordering).

### SenderQueue (ordering.rs:69-158)

```rust
pub struct SenderQueue {
    pub sender: Address,
    pub next_nonce: u64,
    pub pending: Vec<OrderedTransaction>,   // Consecutive nonces from next_nonce
    pub queued: Vec<OrderedTransaction>,    // Future nonces with gaps
}
```

**Insert Logic:**
- Nonce < `next_nonce` → rejected (stale)
- Nonce == `next_nonce + pending.len()` → added to pending, then promotes queued
- Nonce > expected → added to queued (sorted by nonce)
- Same nonce with higher gas price → replaces existing

### Ordering (ordering.rs:53-66)

```rust
impl Ord for OrderedTransaction {
    fn cmp(&self, other: &Self) -> Ordering {
        other.effective_gas_price.cmp(&self.effective_gas_price)  // High to low
            .then_with(|| self.timestamp.cmp(&other.timestamp))   // FIFO within same price
            .then_with(|| self.hash.cmp(&other.hash))             // Deterministic tiebreaker
    }
}
```

### Configuration (config.rs)

| Parameter | Default | Purpose |
|-----------|---------|---------|
| `max_pending_txs` | 4096 | Global pending limit |
| `max_queued_txs` | 1024 | Global queued limit |
| `max_txs_per_sender` | 256 | Per-sender cap |
| `max_tx_size` | 128 KiB | Per-transaction size limit |
| `min_gas_price` | 0 | Minimum gas price |
| `replacement_bump_percent` | 10% | Required gas increase for replacement |

### Key Methods

- **`add()`** (pool.rs:98-142): Deduplicates, enforces limits, handles replacement
- **`build()`** (pool.rs:316-356): Multi-sender fair ordering, no ECDSA recovery needed (pre-validated), O(k) single pass
- **`prune()`** (pool.rs:358-401): Advances `next_nonce`, promotes queued, cleans empty queues
- **`pending()`** (pool.rs:144-154): Returns executable transactions sorted by gas price

---

## 3. TransactionValidator

**File:** `crates/node/txpool/src/validator.rs` (Lines 56-122)

### Validation Checks Performed

| # | Check | Lines | Error |
|---|-------|-------|-------|
| 1 | Transaction size ≤ max_tx_size | 57-62 | `TxTooLarge` |
| 2 | RLP/EIP-2718 decoding | 64-65 | `DecodeError` |
| 3 | Chain ID matches | 67-70 | `InvalidChainId` |
| 4 | ECDSA signature recovery | 72 | `InvalidSignature` |
| 5 | Gas price ≥ min_gas_price | 74-80 | `GasPriceTooLow` |
| 6 | Gas limit ≥ intrinsic gas | 82-89 | `IntrinsicGasTooLow` |
| 7 | Nonce ≥ state_nonce (QMDB only) | 91-100 | `NonceTooLow` / `NonceGap` |
| 8 | Balance ≥ max_cost | 102-110 | `InsufficientBalance` |

### CRITICAL ISSUE: Stale Nonce Validation (Lines 91-100)

```rust
let state_nonce = self.state.nonce(&sender).await?;  // Reads from QMDB ONLY
if nonce < state_nonce {
    return Err(TxPoolError::NonceTooLow { got: nonce, expected: state_nonce });
}
```

**Problem:** Nonces are checked against **persisted QMDB state only**. If a transaction is in a pending (not yet finalized) block, its nonce is still "valid" according to the validator. This creates duplicate-nonce transactions in the mempool.

**Fix needed:** Check against both QMDB state AND pending transactions in the pool.

---

## 4. Why TransactionPool Is Not Wired

**File:** `crates/node/runner/src/runner.rs` (Lines 406-429)

The RPC submission callback:
1. Creates `TransactionValidator` (line 413) — validator IS used
2. Calls `validator.validate(tx)` (line 414) — validation IS performed
3. Calls `ledger.submit_tx(tx)` (line 418) — submits to `InMemoryMempool` directly

The `LedgerView` (ledger/src/lib.rs:148) hardcodes `InMemoryMempool::new()`. The `TransactionPool` is never instantiated in production code.

**Root cause:** `LedgerView` is not generic over the mempool type. It's monomorphized with `InMemoryMempool`.

---

## 5. Feature Comparison

| Feature | InMemoryMempool | TransactionPool |
|---------|----------------|-----------------|
| Data Structure | BTreeMap | HashMap×2 + SenderQueue |
| Per-sender tracking | None | Yes |
| Nonce ordering | On-demand (ECDSA per build) | Maintained in pending/queued |
| Fee prioritization | None | Gas price descending |
| ECDSA cost | O(n) per build() | Once during validation |
| Max pending txs | **Unlimited** | 4,096 |
| Max queued txs | **Unlimited** | 1,024 |
| Max per-sender | **Unlimited** | 256 |
| Replacement logic | None | Higher gas price replaces |
| build() complexity | O(n log n) + ECDSA | O(k) single pass |
| Gap handling | None | Separated pending/queued |
| Stale nonce check | None | In SenderQueue.insert() |
| Size limits | None | Enforced (128 KiB/tx) |

---

## 6. Integration Path

To switch production to `TransactionPool`:

1. Make `LedgerView` generic over mempool type (or use a trait object)
2. Instantiate `TransactionPool` in runner instead of `InMemoryMempool`
3. Pre-validate all transactions and store `OrderedTransaction` (avoid double ECDSA recovery)
4. Handle broadcast transactions through the same validation pipeline (currently bypassed for P2P-received txs)
5. Update nonce checking to include pending pool state, not just QMDB
