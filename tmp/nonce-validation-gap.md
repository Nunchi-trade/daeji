# Nonce Validation Gap in Kora Transaction Pool

## Background: What is Kora?

Kora is a blockchain node implementation built in Rust. It processes Ethereum-compatible transactions using a multi-validator BFT consensus system. Like Ethereum, Kora requires each account to include a sequential **nonce** with every transaction. The nonce starts at 0 and must increment by exactly 1 for each successive transaction from the same account. This mechanism prevents replay attacks and establishes strict ordering of transactions per account.

Kora's transaction pipeline works as follows:

1. A user submits a signed transaction via `eth_sendRawTransaction` (JSON-RPC).
2. The RPC layer invokes a `TxSubmitCallback` that validates the transaction using `TransactionValidator`.
3. If validation passes, the transaction is inserted into the mempool via `ledger.submit_tx(tx)`.
4. A consensus proposer builds a block by pulling transactions from the mempool.
5. The executor applies transactions against state; if any transaction has an invalid nonce at execution time, it fails.

## The Problem

The `TransactionValidator` validates a transaction's nonce **only against the persisted state database (QMDB)**. It does not check whether the mempool already contains a pending transaction occupying the same nonce for the same sender. This creates a window where multiple transactions with the same nonce can coexist in the mempool, leading to execution failures and wasted block space.

## Exact Validation Code

From `crates/node/txpool/src/validator.rs`, lines 91-100:

```rust
let nonce = envelope.nonce();
let state_nonce =
    self.state.nonce(&sender).await.map_err(|e| TxPoolError::StateError(e.to_string()))?;
if nonce < state_nonce {
    return Err(TxPoolError::NonceTooLow { got: nonce, expected: state_nonce });
}
let max_accepted_nonce = state_nonce.saturating_add(self.config.max_txs_per_sender as u64);
if nonce > max_accepted_nonce {
    return Err(TxPoolError::NonceGap { got: nonce, expected: state_nonce });
}
```

This code checks exactly two conditions:
1. The transaction nonce must not be **below** the state nonce (would mean it was already executed).
2. The transaction nonce must not exceed `state_nonce + max_txs_per_sender` (256 by default), which prevents unbounded nonce gaps.

What it does **not** check:
- Whether a transaction with the same nonce from the same sender already exists in the mempool.
- Whether pending (not yet finalized) transactions in the pool have already "claimed" a range of nonces for this sender.
- Whether the nonce has been consumed by a block that has been proposed but not yet persisted to QMDB.

## How the Validator is Instantiated

From `crates/node/runner/src/runner.rs`:

```rust
let tx_state = state.qmdb_state().await;
let chain_id = self.chain_id;
let tx_submit: kora_rpc::TxSubmitCallback = Arc::new(move |data| {
    let ledger = tx_ledger.clone();
    let state = tx_state.clone();
    Box::pin(async move {
        let tx = Tx::new(data);
        let validator =
            TransactionValidator::new(chain_id, state, PoolConfig::default());
        validator.validate(tx.clone()).await.map_err(|err| { ... })?;
        if ledger.submit_tx(tx).await {
            Ok(())
        } else {
            Err(...)
        }
    })
});
```

The validator is constructed with a `QmdbState` snapshot cloned at RPC server initialization time. A new `TransactionValidator` is created for every incoming transaction, but they all share the same cloned `QmdbState` handle. The QMDB state reflects only finalized/persisted blocks, meaning there is a lag between when a transaction is executed and when the validator "knows" about the resulting nonce increment.

## The Mempool Layer

After the validator accepts a transaction, it enters the `InMemoryMempool`:

```rust
// From crates/node/consensus/src/components/mempool.rs
pub struct InMemoryMempool {
    inner: Arc<RwLock<BTreeMap<TxId, Tx>>>,
}

fn insert(&self, tx: Tx) -> bool {
    let id = tx.id();  // keccak256 of raw bytes
    let mut inner = self.inner.write();
    inner.insert(id, tx).is_none()  // returns false only if exact hash exists
}
```

The `InMemoryMempool` deduplicates by transaction hash only. If two transactions have **different content** (different calldata, value, gas settings, etc.) but target the **same nonce** from the **same sender**, the mempool will accept both because they have different hashes.

## Concrete Exploit Scenario

```
Timeline (single node):

T0: Account 0xABC has state nonce = 5 in QMDB
T1: User submits TX_A (nonce=5, sends 1 ETH to Bob)
    - Validator checks: 5 >= 5 (state_nonce) --> PASS
    - Mempool insert: hash(TX_A) not in map --> accepted
T2: User submits TX_B (nonce=5, sends 1 ETH to Eve, different calldata)
    - Validator checks: 5 >= 5 (state_nonce) --> PASS
    - Mempool insert: hash(TX_B) != hash(TX_A) --> accepted
T3: Mempool now contains TWO transactions for nonce=5 from the same sender
T4: Block proposer includes both TX_A and TX_B in a block
T5: Executor runs TX_A (nonce=5): succeeds, account nonce becomes 6
T6: Executor runs TX_B (nonce=5): FAILS -- nonce too low (expected 6, got 5)
```

The outcome of T6 depends on the executor's error handling. In Kora's current implementation, the executor's use of the `?` operator means a nonce mismatch during execution can abort the entire block, causing consensus to produce empty or failed blocks.

## Multi-Node Timing Gap

The problem is exacerbated in a multi-validator setup:

```
T0: Account has nonce=0 in QMDB on all 4 validators
T1: TX(nonce=0) submitted to validator 0 --> accepted into validator 0's mempool
T2: Validator 0 proposes block containing TX(nonce=0), consensus finalizes
T3: Validator 0's QMDB updated: nonce=1
T4: Validators 1,2,3 QMDB still at nonce=0 (finalization propagation in progress)
T5: Same TX(nonce=0) resubmitted to validator 1
    - Validator checks: 0 >= 0 --> PASS (stale QMDB state)
    - Mempool insert: accepted (it's a different node's mempool)
T6: Validator 1 proposes block containing this stale TX(nonce=0)
T7: Executor rejects: nonce too low --> block building fails
```

## How Ethereum Clients Handle This

Mature Ethereum execution clients (geth, reth, erigon) compute an "effective nonce" when validating incoming transactions:

```
effective_nonce = max(state_nonce, highest_pending_nonce_in_pool + 1)
```

This means:
- If the state nonce is 5, and the pool already contains a pending transaction with nonce=5, the next valid nonce for acceptance is 6.
- Submitting a second transaction with nonce=5 (and a different hash) results in a "replacement" scenario where the new transaction must offer higher gas to displace the existing one.
- Submitting a transaction with nonce=5 that offers lower gas is rejected outright.

In geth specifically, the `TxPool` maintains per-account pending lists indexed by nonce, and the validation step checks against the pool-local nonce watermark before accepting.

## Proposed Fix

### Option A: Inject Pool State into Validation (Minimal Change)

Pass the mempool's per-sender highest pending nonce to the validator:

```rust
pub async fn validate(
    &self,
    tx: Tx,
    pool_nonce_for_sender: Option<u64>,  // NEW PARAMETER
) -> Result<ValidatedTransaction, TxPoolError> {
    // ... existing checks ...

    let nonce = envelope.nonce();
    let state_nonce = self.state.nonce(&sender).await?;

    // Use the higher of state nonce or pool's next expected nonce
    let effective_nonce = match pool_nonce_for_sender {
        Some(pool_next) => state_nonce.max(pool_next),
        None => state_nonce,
    };

    if nonce < effective_nonce {
        return Err(TxPoolError::NonceTooLow { got: nonce, expected: effective_nonce });
    }

    // For replacement: if nonce == an existing pending nonce, require higher gas price
    // (not shown -- would need pool query for existing tx at same nonce)

    // ... rest of validation ...
}
```

### Option B: Replace InMemoryMempool with TransactionPool (Recommended)

The codebase already contains a fully-featured `TransactionPool` (in `crates/node/txpool/src/pool.rs`) with:

- Per-sender `SenderQueue` with `next_nonce` tracking
- Nonce-ordered pending/queued classification
- Replacement-by-gas-price for same-nonce transactions
- Size limits per sender (configurable via `PoolConfig.max_txs_per_sender`)
- Proper pruning via `remove_confirmed(sender, confirmed_nonce)`

The `TransactionPool` already implements the `Mempool` trait. Switching production to use it instead of `InMemoryMempool` would provide nonce-aware insertion for free:

```rust
// In SenderQueue::insert (crates/node/txpool/src/ordering.rs:91-116):
pub fn insert(&mut self, tx: OrderedTransaction) -> Option<OrderedTransaction> {
    if tx.nonce < self.next_nonce {
        return Some(tx);  // REJECT: stale nonce
    }
    if tx.nonce == self.next_nonce + self.pending.len() as u64 {
        self.pending.push(tx);  // Append to pending sequence
        self.promote_queued();
        None
    } else if tx.nonce > self.next_nonce + self.pending.len() as u64 {
        self.queued.insert(pos, tx);  // Queue as future
        None
    } else {
        // Same nonce slot -- only replace if higher gas price
        let idx = (tx.nonce - self.next_nonce) as usize;
        if tx.effective_gas_price > existing.effective_gas_price {
            let old = std::mem::replace(&mut self.pending[idx], tx);
            return Some(old);  // Return displaced tx
        }
        Some(tx)  // REJECT: lower gas than incumbent
    }
}
```

This would require changing the `LedgerState` struct from:
```rust
mempool: InMemoryMempool,
```
to:
```rust
mempool: TransactionPool,
```

### Option C: Refresh Validator State After Each Finalization

After each block finalization, update the state reference used by the validator so it always reflects the latest persisted nonce:

```rust
// After finalization in the runner:
validator_state.refresh(ledger.qmdb_state().await);
```

This reduces the window for stale-nonce acceptance but does **not** eliminate the pending-state gap (the time between mempool acceptance and block finalization).

## Severity Assessment

**Severity: HIGH**

- **Correctness impact**: Multiple transactions for the same nonce enter the mempool, causing deterministic execution failures when a block includes both.
- **Liveness impact**: On the current executor (which aborts on error via `?`), a single nonce conflict in a proposed block can halt block production until the stale transaction is manually pruned or ages out.
- **Economic impact**: In a fee market, this gap allows an attacker to cheaply spam conflicting same-nonce transactions, wasting proposer resources and delaying legitimate transactions.
- **Ease of exploitation**: Trivially exploitable -- any user can submit two transactions with the same nonce and different payloads. No special access required; standard `eth_sendRawTransaction` suffices.

The `TransactionPool` already exists and handles this correctly. The fix is primarily a wiring change to replace `InMemoryMempool` with `TransactionPool` in the `LedgerState` struct.
