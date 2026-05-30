# TxPool: No pending-nonce awareness at transaction ingress allows same-nonce conflicts

**Severity:** HIGH
**Component:** `kora-txpool`, `kora-runner`, `kora-consensus`
**Labels:** `bug`, `txpool`, `nonce`, `security`

## Summary

The `TransactionValidator` validates transaction nonces only against the persisted QMDB state. It does not check whether the mempool already contains a pending transaction at the same nonce from the same sender. This allows two (or more) transactions with identical nonces but different payloads to both pass validation and enter the mempool. When a block is proposed containing both, one succeeds during EVM execution and the other fails with a nonce mismatch, wasting block space and causing silent execution failures.

## Background: How Transactions Enter Kora

Kora is an EVM-compatible blockchain built on [Commonware](https://github.com/commonwarexyz/monorepo) consensus primitives. Transactions flow through the system as follows:

1. **RPC ingress** -- An external client submits a signed EIP-1559 (or legacy) transaction via `eth_sendRawTransaction`.
2. **Validation** -- The RPC handler instantiates a `TransactionValidator` and calls `validator.validate(tx)`. This checks chain ID, signature, gas intrinsics, nonce range, and balance against the persisted QMDB state.
3. **Mempool insertion** -- If validation passes, the transaction is forwarded to `LedgerService::submit_tx()`, which calls `InMemoryMempool::insert()`.
4. **Block proposal** -- When a validator is elected leader, it calls `mempool.build(max_txs, excluded)` to collect transactions for a new block.
5. **Execution** -- The `RevmExecutor` executes each transaction against the parent block's state. The EVM enforces strict sequential nonces: if a transaction's nonce does not match the sender's current nonce in the execution state, it fails.
6. **Finalization** -- After consensus certifies the block, the `FinalizedReporter` persists the state changes to QMDB and prunes the mempool of finalized transactions by their hash-based `TxId`.

The **nonce** is the sender's sequential transaction counter. It serves two critical purposes: (a) preventing replay attacks (each nonce can only be consumed once), and (b) ordering a sender's transactions deterministically. The EVM requires `tx.nonce == state_nonce` at execution time.

## The Validation Gap

### Validator code

In `crates/node/txpool/src/validator.rs`, lines 91-100:

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

This check validates that:
- `nonce >= state_nonce` (not already consumed in persisted state)
- `nonce <= state_nonce + max_txs_per_sender` (not too far in the future)

It does **not** check whether a pending transaction at the same nonce from the same sender already exists in the mempool.

### Validator instantiation

In `crates/node/runner/src/runner.rs`, lines 406-413, the RPC submit callback creates a new `TransactionValidator` per request:

```rust
let tx_submit: kora_rpc::TxSubmitCallback = Arc::new(move |data| {
    let ledger = tx_ledger.clone();
    let state = tx_state.clone();
    Box::pin(async move {
        let tx = Tx::new(data);
        let tx_id = tx.id();
        let validator =
            TransactionValidator::new(chain_id, state, PoolConfig::default());
        validator.validate(tx.clone()).await.map_err(|err| { ... })?;
        // ...
        ledger.submit_tx(tx).await
    })
});
```

Each invocation shares the same cloned `QmdbState` handle. `QmdbState` reflects only **finalized** state -- it is not updated until `persist_snapshot()` completes in `FinalizedReporter`. The validator has no visibility into what transactions are already sitting in the mempool.

### InMemoryMempool deduplication

In `crates/node/consensus/src/components/mempool.rs`, lines 43-48:

```rust
impl Mempool for InMemoryMempool {
    fn insert(&self, tx: Tx) -> bool {
        let id = tx.id();
        let mut inner = self.inner.write();
        inner.insert(id, tx).is_none()
    }
```

The `TxId` is computed as `keccak256(tx.encode())` -- it is a hash of the raw transaction bytes. Two transactions from the same sender with the same nonce but **different data** (different recipient, value, calldata, or gas parameters) produce different `TxId` values. Both are accepted into the `BTreeMap`.

## Concrete Exploit Scenario

Consider sender Alice with `state_nonce = 5`:

```
Time   Action                                              Result
────   ──────                                              ──────
T0     Alice submits TX_A(nonce=5, to=Bob, value=100)      Validator: 5 >= 5 -> OK
       via eth_sendRawTransaction                          InMemoryMempool: hash_A not present -> inserted

T1     Alice submits TX_B(nonce=5, to=Carol, value=200)    Validator: 5 >= 5 -> OK (checks same QMDB state)
       via eth_sendRawTransaction                          InMemoryMempool: hash_B != hash_A -> inserted

T2     Leader builds block proposal                        mempool.build() returns [TX_A, TX_B, ...]
                                                           (BTreeMap ordering, both present)

T3     RevmExecutor executes block:
       - TX_A(nonce=5): state_nonce=5, matches -> SUCCESS  state_nonce advances to 6
       - TX_B(nonce=5): state_nonce=6, mismatch -> FAIL    Execution error, tx silently dropped

T4     Block is finalized with TX_A only                   TX_B remains in mempool (different hash)
       prune_mempool removes TX_A by hash                  TX_B proposed again in next block -> fails again
```

This is not merely a theoretical concern. Any user or dApp that rapidly retries a transaction with adjusted parameters (common in MEV, DEX trading, or contract deployment scripts) will trigger this pattern.

### Multi-Node Timing Gap

The problem is amplified in a multi-validator setup. Kora runs a 4-validator devnet where each validator has its own local mempool and its own QMDB state handle:

```
Time   Action                                              Result
────   ──────                                              ──────
T0     Validator 0 finalizes block at height N              Validator 0's QMDB: sender nonce -> 1
       containing TX(nonce=0) from sender

T1     Client submits TX(nonce=0) to Validator 1            Validator 1's QMDB still shows nonce=0
                                                            Validator: 0 >= 0 -> OK, inserted

T2     Client submits TX(nonce=0) to Validator 2            Validator 2's QMDB still shows nonce=0
                                                            Validator: 0 >= 0 -> OK, inserted
```

Between finalization on Validator 0 and QMDB persistence propagation to Validators 1-3, there is a window where stale nonces are accepted. The QMDB state on each validator only advances after `persist_snapshot()` completes locally via the `FinalizedReporter` callback.

## How Ethereum Clients Handle This

Mature Ethereum execution clients (Geth, Reth, Nethermind) solve this with **pending nonce tracking**:

```
effective_nonce = max(state_nonce, highest_pending_nonce_in_pool + 1)
```

When a new transaction arrives at nonce N for sender S:
1. If N < effective_nonce: reject as `NonceTooLow`
2. If N == existing pending nonce: allow **replacement** only if gas price exceeds the existing transaction by a configurable bump percentage (typically 10%)
3. If N > effective_nonce: queue as a future transaction (nonce gap)

This prevents two transactions at the same nonce from coexisting unless one explicitly replaces the other with a higher gas price.

## Proposed Fixes

### Option A: Inject pool state into validation

Add a `pool_nonce_for_sender` query to the validation path:

```rust
// In TransactionValidator::validate():
let pool_nonce = self.pool.highest_nonce_for_sender(&sender);
let effective_nonce = state_nonce.max(pool_nonce.map(|n| n + 1).unwrap_or(0));
if nonce < effective_nonce {
    return Err(TxPoolError::NonceTooLow { got: nonce, expected: effective_nonce });
}
```

This requires passing a reference to the pool (or a nonce-lookup trait) into the validator. Moderate refactor.

### Option B: Replace InMemoryMempool with TransactionPool (Recommended)

The codebase already contains `TransactionPool` in `crates/node/txpool/src/pool.rs`. This pool:
- Maintains per-sender `SenderQueue` with nonce ordering (`crates/node/txpool/src/ordering.rs`)
- Tracks `next_nonce` per sender and rejects/replaces duplicate nonces via `SenderQueue::insert()` (lines 91-116)
- Implements nonce-aware pruning via `remove_confirmed()` (lines 130-137) which advances `next_nonce` and promotes queued transactions
- Already implements the `Mempool` trait (lines 300-406)

Switching from `InMemoryMempool` to `TransactionPool` in `LedgerState` (`crates/node/ledger/src/lib.rs`, line 70) resolves both the duplicate-nonce insertion problem and the pruning gap. This is closely related to issue #02 (Replace InMemoryMempool with TransactionPool) -- implementing that fix resolves this issue as well.

### Option C: Refresh validator state after each finalization

Subscribe to `LedgerEvent::SnapshotPersisted` and refresh the QMDB state handle used by the validator. This narrows the window but does not eliminate the same-nonce conflict within a single finalization interval.

## Relationship to Other Issues

- **Issue #02 (Replace InMemoryMempool with TransactionPool):** Directly resolves this issue. The `TransactionPool` handles same-nonce conflicts via `SenderQueue`.
- **Issue #03 (Post-finalization nonce sweep):** Complements this fix by ensuring stale transactions are evicted after state advances.
- **Issue #18 (Duplicate transaction broadcast storm):** The same missing nonce awareness causes the broadcast storm problem on multi-validator deployments.

## Observed Load Test Data

This issue was confirmed during load testing of the Kora 4-validator devnet. The loadgen tool (`bin/loadgen/src/main.rs`) exercises the transaction submission path and exposes the nonce validation gap under sustained load.

### Pre-fix loadgen behavior (nonce race condition active)

| Metric | Value |
|--------|-------|
| Total transactions | 50,000 |
| Success rate | 25.7% (12,850 accepted) |
| Chain outcome | **Permanently stalled** |
| Root cause | Out-of-order nonce delivery + broadcast-to-all-validators created stale transactions in all mempools |

### Post-fix loadgen behavior (sequential sends, account sharding)

| Metric | Value |
|--------|-------|
| Total transactions (safe config) | 5,000 |
| Success rate | 100% |
| Throughput | ~2,933 TPS |
| Chain outcome | Healthy |

| Metric | Value |
|--------|-------|
| Total transactions (medium config) | 10,000 |
| Success rate | 100% |
| Throughput | ~2,873 TPS |
| Chain outcome | Healthy |

The loadgen was fixed with four changes (see `bin/loadgen/src/main.rs`):
1. **Per-account sequential sends** -- One Tokio task per account; nonce N completes before N+1 is assigned (lines 296-356)
2. **SeqCst memory ordering** -- `AtomicU64::fetch_add(1, Ordering::SeqCst)` instead of `Ordering::Relaxed` (line 83)
3. **Account sharding** -- Each account is pinned to one validator via `let target_validator = idx` (line 307), using `send_raw_transaction_to()` (lines 190-215) instead of the old broadcast-to-all function
4. **Retry with backoff** -- Up to 10 retries with 100ms linear backoff (lines 328-350)

These loadgen fixes are **workarounds**, not protocol-level solutions. They eliminate the test tool as a source of nonce conflicts, making it possible to isolate chain-level bugs. However, the underlying nonce validation gap remains exploitable by any external client.

### Key finding from load test results

Even with a perfectly behaving loadgen, the chain still stalls under sustained high load (50,000 txs, concurrency 200) because stale transactions can enter the pool through the QMDB persistence lag window. The loadgen fix proves the chain-level nonce validation gap is the root cause, not the test tool.

## Files Involved

| File | Role | Key Lines |
|------|------|-----------|
| `crates/node/txpool/src/validator.rs` | Transaction validation (nonce check against QMDB only) | 91-100 |
| `crates/node/runner/src/runner.rs` | RPC callback creates `TransactionValidator` per request | 406-431 |
| `crates/node/consensus/src/components/mempool.rs` | `InMemoryMempool` -- hash-only deduplication, no nonce awareness | 43-48 (insert), 61-66 (prune) |
| `crates/node/txpool/src/pool.rs` | `TransactionPool` -- nonce-aware pool with `Mempool` trait impl | 99-142 (add), 188-216 (remove_confirmed), 300-406 (Mempool impl) |
| `crates/node/txpool/src/ordering.rs` | `SenderQueue` -- per-sender nonce tracking | 91-116 (insert), 130-137 (remove_confirmed) |
| `crates/node/ledger/src/lib.rs` | `LedgerState` holds the `InMemoryMempool` instance | 68-70 |
| `crates/node/reporters/src/lib.rs` | `FinalizedReporter` calls `prune_mempool()` after finalization | 226 |
| `bin/loadgen/src/main.rs` | Load generator (already fixed with account sharding) | 82-88, 190-215, 296-356 |

## Severity Justification

**HIGH** -- This vulnerability is trivially exploitable via standard `eth_sendRawTransaction` calls. Any user can submit two transactions with the same nonce, causing one to fail silently during execution. In a multi-validator setup, the timing gap between finalization and QMDB persistence makes this even easier to trigger without malicious intent (e.g., a wallet retrying a stuck transaction).

## Testing Plan

1. **Unit test:** Submit two `ValidatedTransaction` instances with the same sender and nonce but different hashes to `InMemoryMempool::insert()`. Verify both are accepted (confirming the bug). Repeat with `TransactionPool::add()` and verify the second is rejected or replaces the first.

2. **Integration test:** In the e2e harness, submit two conflicting nonce transactions to the same validator's RPC. Assert that only one is included in the finalized block and the other is either rejected at ingress or evicted from the pool.

3. **Multi-validator test:** Submit a transaction to Validator 0, wait for finalization, then submit a stale-nonce transaction to Validator 1. Verify that it is rejected (after the fix, Validator 1's pool should be nonce-aware even before its QMDB catches up).

4. **Load test regression:** Run the load generator with `--accounts 10 --total_txs 10000` and verify zero execution failures due to nonce conflicts.

## Verification Steps

After implementing the fix, run these concrete verification commands:

### Step 1: Unit tests pass

```bash
cargo test -p kora-txpool
cargo test -p kora-consensus
```

### Step 2: Safe load test (must achieve 100% success, no stall)

```bash
just trusted-devnet
sleep 30
cargo run --release --bin loadgen -- \
  --total-txs 5000 --accounts 20 --concurrency 50 \
  --rpc-url http://127.0.0.1:8545
```

Expected: 100% success, ~2,900 TPS, chain healthy after test.

### Step 3: Stress test (must not stall the chain)

```bash
just trusted-devnet
sleep 30
cargo run --release --bin loadgen -- \
  --total-txs 50000 --accounts 50 --concurrency 200 \
  --rpc-url http://127.0.0.1:8545
```

Expected: Chain does NOT stall permanently. Some transactions may be rejected (pool capacity), but blocks continue to be produced. Verify with:

```bash
curl -s http://127.0.0.1:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

### Step 4: Broadcast mode test (must not cause 70% failure rate)

```bash
just trusted-devnet
sleep 30
cargo run --release --bin loadgen -- \
  --total-txs 10000 --accounts 20 --concurrency 50 \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

Expected: Block proposal success rate >95% (up from ~30% before the fix). Chain remains healthy.

---

## Additional Detail: Nonce Validation Against Stale State

The `TransactionValidator` in `crates/node/txpool/src/validator.rs` (lines 92-95) checks the nonce against QMDB state:

```rust
let state_nonce = self.state.nonce(&sender).await...;
if nonce < state_nonce {
    return Err(TxPoolError::NonceTooLow { got: nonce, expected: state_nonce });
}
```

This check uses persisted (finalized) state, which lags behind the actual pending state. It does NOT account for:
- Transactions already pending in the mempool for the same sender
- Transactions in notarized-but-not-yet-finalized blocks

**Mitigating factor**: The `SenderQueue` in `TransactionPool` handles nonce replacement at the queue level -- if a tx arrives at an already-occupied nonce slot, it must have a strictly higher gas price to replace (see `ordering.rs` line 109: `tx.effective_gas_price > existing.effective_gas_price`). Note: `PoolConfig` defines a `replacement_bump_percent = 10` field, but `SenderQueue::insert()` does not currently consult it -- it only checks strict `>`. So exact duplicates and same-price replacements are rejected. However, if the same sender submits to multiple validators, each validator's pool independently accepts the same nonce.

The fix described above (tracking `pending_nonce` as `max(state_nonce, highest_pending_nonce + 1)`) addresses this window. The `TransactionPool`'s `SenderQueue.next_nonce` field provides the mechanism -- the missing piece is updating it atomically with the state nonce on finalization.

## Related Issues

- **Issue #02 (Replace InMemoryMempool)**: Wiring TransactionPool provides the `SenderQueue` infrastructure needed for pending nonce tracking
- **Issue #18 (Duplicate transaction broadcast storm)**: Same-nonce submissions to multiple validators are a symptom of this gap
- **Issue #26 (Transaction TTL)**: Stale transactions that bypass nonce validation should still expire via TTL
- **Issue #27 (Pool size limits)**: Under sustained nonce-conflict load, pool can grow unbounded without hard limits
