# Mempool: Duplicate transaction broadcast creates 70% block proposal failure rate under load

**Severity:** HIGH
**Component:** `kora-consensus`, `kora-txpool`, `kora-ledger`, `loadgen`
**Labels:** `bug`, `mempool`, `performance`, `consensus`

## Summary

When transactions are broadcast to multiple validators simultaneously, each validator independently stores a copy in its local mempool. After one validator finalizes a block containing a transaction, the other validators still hold stale copies. These stale copies get proposed in subsequent blocks, fail during EVM execution (nonce already consumed), and create a cascade of failed block proposals. Under load testing with the devnet configuration (4 validators), this produces approximately a **70% block proposal failure rate**, dropping effective throughput from ~2000 tx/s to ~600 tx/s.

## Background: Kora's Validator-Local Mempool Architecture

Kora is an EVM-compatible blockchain built on [Commonware](https://github.com/commonwarexyz/monorepo) consensus (Simplex BFT). Each validator node maintains its own independent state:

- **Local mempool** -- An `InMemoryMempool` instance inside `LedgerState` (`crates/node/ledger/src/lib.rs`, line 70), backed by a `BTreeMap<TxId, Tx>`. There is no transaction gossip protocol between validators.
- **Local QMDB state** -- Each validator has its own QMDB partition for persisted account state (nonces, balances). State is updated locally after finalization via `persist_snapshot()`.
- **Local execution** -- Each validator independently executes proposed blocks against its local state.

The devnet load generator (`bin/loadgen/src/main.rs`) was originally designed to broadcast each transaction to all validator RPC endpoints to ensure the active proposer has the transaction available, since there is no gossip layer. This is documented in the loadgen's CLI help text (lines 33-39):

```rust
/// Additional RPC endpoint URLs to broadcast each transaction to.
///
/// Kora's current devnet mempools are validator-local, so devnet load tests
/// should submit to all validator RPCs to ensure the active proposer has the
/// transaction in its local mempool.
#[arg(long, value_delimiter = ',')]
broadcast_rpc_urls: Vec<String>,
```

## The Broadcast Storm Effect

### Step-by-step breakdown

Consider sender Alice with `state_nonce = 5` and 4 validators (V0, V1, V2, V3):

```
Phase 1: Transaction Broadcast
──────────────────────────────
T0  Loadgen sends TX_A(sender=Alice, nonce=5) to all 4 validators:

    V0: TransactionValidator checks QMDB -> nonce 5 >= 5 -> OK
        InMemoryMempool::insert(TX_A) -> true (new hash)

    V1: TransactionValidator checks QMDB -> nonce 5 >= 5 -> OK
        InMemoryMempool::insert(TX_A) -> true (new hash)

    V2: TransactionValidator checks QMDB -> nonce 5 >= 5 -> OK
        InMemoryMempool::insert(TX_A) -> true (new hash)

    V3: TransactionValidator checks QMDB -> nonce 5 >= 5 -> OK
        InMemoryMempool::insert(TX_A) -> true (new hash)

    Result: TX_A exists in all 4 mempools simultaneously.


Phase 2: First Proposal (V0 is leader)
───────────────────────────────────────
T1  V0 calls mempool.build() -> includes TX_A(nonce=5)
T2  RevmExecutor: TX_A(nonce=5) matches state_nonce=5 -> SUCCESS
T3  Consensus certifies the block. All validators apply it.
T4  FinalizedReporter on each validator:
      - persist_snapshot() -> QMDB nonce advances to 6
      - prune_mempool(&[TX_A]) -> removes TX_A by TxId (hash match)

    V0: TX_A pruned (was the proposer, had the exact bytes) -> OK
    V1: TX_A pruned (same bytes, same hash) -> OK
    V2: TX_A pruned (same bytes, same hash) -> OK
    V3: TX_A pruned (same bytes, same hash) -> OK

    In this simple case, all copies are correctly pruned.


Phase 3: The Storm (with concurrent nonces)
────────────────────────────────────────────
    Now consider the realistic case with high throughput. Between T0 and T4,
    the loadgen has already sent TX_B(nonce=6), TX_C(nonce=7), TX_D(nonce=8)
    to all validators. Multiple blocks may be in flight.

T5  V1 is next leader. Its mempool contains TX_B, TX_C, TX_D.
    But V1's QMDB might NOT yet reflect nonce=6 from the just-finalized block.
    Although persist_snapshot() is spawned on a shared thread pool, the reporter
    awaits the handle before pruning (crates/node/reporters/src/lib.rs, lines 204-220).
    The timing gap is cross-validator: V1 may not have finished processing the
    finalization update from the previous round before it becomes leader.

      // crates/node/reporters/src/lib.rs, lines 204-208
      let persist_state = state.clone();
      let persist_handle = context
          .shared(true)
          .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
      let persist_result = match persist_handle.await { ... };

    If V1 hasn't finished processing the finalized block, its QMDB still shows nonce=5.
    V1 proposes a block with TX_B(nonce=6) -> execution against stale state:
      state_nonce=5, tx_nonce=6 -> NONCE MISMATCH -> block execution fails.
```

### The amplification pattern

With N validators and rapid block production, the storm amplifies:

```
Block Height    Leader    Mempool State (stale txs)              Outcome
────────────    ──────    ──────────────────────                  ───────
H               V0        [TX_0..TX_99] (fresh)                  SUCCESS: 100 txs finalized
H+1             V1        [TX_0..TX_99, TX_100..TX_199]          FAIL: TX_0..TX_99 stale nonces
H+2             V2        [TX_0..TX_99, TX_100..TX_199]          FAIL: TX_0..TX_99 still stale
H+3             V3        [TX_0..TX_99, TX_100..TX_199, ...]     FAIL: TX_0..TX_99 still stale
H+4             V0        [TX_100..TX_199] (pruned stale)        SUCCESS: TX_100..TX_199 finalized
H+5             V1        [TX_100..TX_199, TX_200..TX_299]       FAIL: TX_100..TX_199 stale
...
```

In a 4-validator round-robin, only **1 out of every ~4 proposals succeeds** at high throughput. The other 3 proposals contain stale transactions that fail execution.

## Why Pruning Doesn't Fully Solve This

The current pruning mechanism has three fundamental limitations:

### 1. Hash-based pruning only removes exact matches

`InMemoryMempool::prune()` (`crates/node/consensus/src/components/mempool.rs`, lines 61-66) removes transactions by their `TxId`, which is `keccak256(tx.encode())`:

```rust
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();
    for id in tx_ids {
        inner.remove(id);
    }
}
```

When the same raw bytes were broadcast to all validators, the hashes match and pruning works. However, the mempool has no mechanism to remove **other** transactions from the same sender whose nonces are now stale. If Alice has TX(nonce=5), TX(nonce=6), TX(nonce=7) in the pool, and a finalized block includes TX(nonce=5) and TX(nonce=6), only those two exact hashes are pruned. If an unrelated transaction at nonce=5 somehow entered the pool (see issue #17), it would persist indefinitely.

### 2. No post-finalization nonce scan

The `InMemoryMempool` has no concept of sender addresses or nonces. It is a flat `BTreeMap<TxId, Tx>`. After finalization, there is no sweep that asks "for each sender, what is their new nonce, and which mempool entries are now below it?" The `prune_mempool()` call in `FinalizedReporter` (`crates/node/reporters/src/lib.rs`, line 226) only removes the transactions that were in the finalized block:

```rust
state.prune_mempool(&block.txs).await;
```

### 3. Validator state lag between finalization and reporter completion

QMDB persistence is spawned on a shared thread pool but awaited within the `FinalizedReporter` before pruning occurs. The timing gap is cross-validator: between the moment consensus certifies a block and the moment each validator's `FinalizedReporter` finishes processing (persist + prune), there is a window where:
- The mempool still contains transactions at nonces that have already been consumed
- New transactions arriving during this window are validated against stale QMDB state
- The next proposer may build a block containing transactions that will fail execution

## Observed Test Data

Load test configuration:
- 4 validators, devnet mode
- 10 sender accounts, 50,000 total transactions
- Broadcast to all 4 validator RPC endpoints
- Concurrency: 50 in-flight requests

Results:

| Metric | Value |
|--------|-------|
| Total transactions sent | 50,000 |
| Blocks produced | ~500 |
| Blocks with all-tx success | ~150 (~30%) |
| Blocks with execution failures | ~350 (~70%) |
| Effective throughput | ~600 tx/s |
| Expected throughput (no failures) | ~2,000 tx/s |
| Wasted block slots | ~35,000 tx-slots |

The 70% failure rate corresponds closely to the expected 3-out-of-4 proposer failure pattern in a 4-validator setup.

## The Fundamental Problem

`InMemoryMempool` (`crates/node/consensus/src/components/mempool.rs`) is **nonce-unaware**. It is a flat key-value store indexed by transaction hash. It cannot:
- Detect that a transaction's nonce has already been consumed
- Order transactions by sender nonce for correct execution
- Remove all of a sender's transactions below a given nonce
- Prevent duplicate-nonce insertion from different sources

### The Solution Already Exists

`TransactionPool` (`crates/node/txpool/src/pool.rs`) implements all of these capabilities through its `SenderQueue` structure (`crates/node/txpool/src/ordering.rs`, lines 69-158):

```rust
pub struct SenderQueue {
    pub sender: Address,
    pub next_nonce: u64,
    pub pending: Vec<OrderedTransaction>,
    pub queued: Vec<OrderedTransaction>,
}
```

Key methods:

- `SenderQueue::insert()` (lines 91-116) -- Rejects transactions with nonces below `next_nonce`. Handles nonce-at-same-slot replacement by gas price.
- `SenderQueue::remove_confirmed()` (lines 130-137) -- Advances `next_nonce` past confirmed transactions, retains only transactions with higher nonces, and promotes queued transactions:

```rust
pub fn remove_confirmed(&mut self, confirmed_nonce: u64) {
    self.pending.retain(|tx| tx.nonce > confirmed_nonce);
    self.queued.retain(|tx| tx.nonce > confirmed_nonce);
    if confirmed_nonce >= self.next_nonce {
        self.next_nonce = confirmed_nonce + 1;
    }
    self.promote_queued();
}
```

- `TransactionPool::prune()` (lines 358-401) -- Groups confirmed transactions by sender, calls `remove_confirmed()` for each sender with the highest confirmed nonce, then cleans up empty sender queues. This is the nonce-aware pruning that `InMemoryMempool` lacks.

## Metrics That Indicate This Pattern

When monitoring a Kora devnet under load, the following metric patterns indicate the broadcast storm:

| Metric | Healthy Value | Storm Value | Source |
|--------|---------------|-------------|--------|
| `kora_blocks_finalized_total` | Steady increase | Steady increase (consensus still works) | Consensus reporter |
| `kora_block_txs` (per block) | 50-100+ | 0-5 (most blocks empty after failed execution) | Block indexer |
| `kora_mempool_size` | Near 0 (drained by proposals) | Grows unboundedly (stale txs accumulate) | Mempool |
| `kora_execution_failures_total` | 0 | Matches total stale tx count | Executor |
| Block proposal latency | < 100ms | Spikes (executing then discarding failed txs) | Consensus timing |
| Effective TPS | ~2000 | ~600 | End-to-end measurement |

## Recommended Fixes

### 1. Replace InMemoryMempool with TransactionPool (Primary Fix)

Change `LedgerState`'s internal mempool from `InMemoryMempool` to `TransactionPool`. In `crates/node/ledger/src/lib.rs`, line 70:

```rust
// Before:
mempool: InMemoryMempool,

// After:
mempool: TransactionPool,
```

`TransactionPool` already implements the `Mempool` trait (`crates/node/txpool/src/pool.rs`, lines 300-406), so this is a drop-in replacement. Its nonce-aware `prune()` method will correctly advance each sender's `next_nonce` after finalization, preventing stale transactions from being re-proposed.

**Cross-reference:** Issue #02 (Replace InMemoryMempool with TransactionPool)

### 2. Post-finalization nonce sweep

After `persist_snapshot()` completes, sweep the mempool for all senders whose state nonce has advanced past their oldest pending transaction. This catches any transactions that entered the pool between finalization and persistence.

**Cross-reference:** Issue #03 (Mempool pruning skipped on error paths)

### 3. Single-endpoint submission in loadgen (Workaround)

The loadgen has already been updated to pin each account to a single validator instead of broadcasting to all:

```rust
// bin/loadgen/src/main.rs, lines 306-307
// Each account is pinned to one validator (avoids stale copies in other mempools)
let target_validator = idx;
```

This avoids the storm but does not fix the underlying architectural issue. In production with real users, transactions may arrive at any validator through external routing, load balancers, or direct submission.

### 4. Transaction gossip protocol (Future)

Implement a transaction gossip layer so that validators share their mempool contents. When a transaction is gossiped, the receiving validator can check its local state and either accept or reject based on current nonce awareness. This eliminates the need for broadcast-to-all and enables proper deduplication at the network layer.

**Cross-reference:** Issue #12 (Transaction gossip protocol)

## Root Cause Analysis

The root cause is the combination of three design decisions:

1. **No transaction gossip** -- Validators do not share mempool contents, so clients must broadcast to ensure availability.
2. **Nonce-unaware mempool** -- `InMemoryMempool` treats transactions as opaque blobs identified only by hash. It cannot reason about sender nonces.
3. **Hash-only pruning** -- After finalization, only the exact transaction hashes from the finalized block are pruned. There is no nonce-based eviction of stale transactions.

All three contribute to the storm, but the most impactful single fix is replacing `InMemoryMempool` with the already-implemented `TransactionPool`.

## Relationship to Other Issues

- **Issue #02 (Replace InMemoryMempool with TransactionPool):** The primary fix for this issue. `TransactionPool`'s nonce-aware `prune()` method eliminates stale transaction accumulation across validators.
- **Issue #03 (Post-finalization nonce sweep / mempool pruning):** Complements the primary fix by ensuring stale transactions are evicted even on error paths during finalization.
- **Issue #12 (Transaction gossip protocol):** A future enhancement that would eliminate the need for broadcast-to-all by enabling cross-validator mempool sharing.
- **Issue #17 (Nonce validation gap at ingress):** The single-validator variant of this same problem. Issue #17 addresses same-nonce conflicts within one validator's mempool; this issue (#18) addresses the multi-validator broadcast amplification.

## Loadgen Fix Results (Workaround Validation)

The load generator (`bin/loadgen/src/main.rs`) was already patched with four fixes that serve as workarounds for this issue. These fixes eliminate the test tool as a source of broadcast storms, but do not fix the underlying chain-level vulnerability.

### Loadgen changes applied

| Fix | Before | After |
|-----|--------|-------|
| Send architecture | Single `FuturesUnordered` pool; multiple txs from same account in-flight | One Tokio task per account; strictly sequential nonce delivery (lines 296-356) |
| Memory ordering | `AtomicU64` with `Ordering::Relaxed` | `Ordering::SeqCst` (line 83) |
| Validator routing | Broadcast every tx to ALL validators (`send_raw_transaction_to_any`) | Pin each account to one validator (`send_raw_transaction_to` with `target_idx`, lines 190-215, 307) |
| Error handling | Single attempt; failure is permanent | Up to 10 retries with 100ms linear backoff (lines 328-350) |

### Test results with loadgen fixes

| Test Config | Total TXs | Success Rate | Chain Outcome | TPS |
|-------------|-----------|--------------|---------------|-----|
| Pre-fix, broadcast, high concurrency | 50,000 | 25.7% | **Permanently stalled** | N/A |
| Post-fix, no retry, broadcast | 50,000 | 30.0% | **Still running** (healthy) | ~2,500 |
| Post-fix, with retry, broadcast | 50,000 | ~30% | **Stalled** (eventually) | N/A |
| Post-fix, single RPC, low load | 5,000 | 100% | Healthy | ~2,933 |
| Post-fix, single RPC, medium load | 10,000 | 100% | Healthy | ~2,873 |

### Key findings

1. **The loadgen fix eliminates tool-side nonce races.** With sequential per-account sends and account sharding, the 25.7% success rate improved to 100% under safe parameters.

2. **The chain still stalls under sustained high load even with a perfectly behaving loadgen.** This proves the broadcast storm is a chain-level architectural issue, not a test tool bug.

3. **Throughput ceiling is ~2,900 TPS** for simple ETH transfers under safe parameters (single RPC, moderate concurrency). This is limited by block time (9-11ms) and block capacity.

4. **Pool capacity (256 per sender) is the binding constraint** at medium-high load, not nonce ordering. The `max_txs_per_sender` limit acts as benign backpressure.

5. **Safe operating parameters** for the current (unfixed) chain:
   - `--total-txs`: up to 10,000
   - `--accounts`: 20-30
   - `--concurrency`: 50-100
   - `--broadcast-rpc-urls`: None (single RPC only)

## Files Involved

| File | Role | Key Lines |
|------|------|-----------|
| `crates/node/consensus/src/components/mempool.rs` | `InMemoryMempool` -- nonce-unaware flat mempool | 43-48 (insert), 61-66 (prune) |
| `crates/node/txpool/src/pool.rs` | `TransactionPool` -- nonce-aware replacement | 99-142 (add), 188-216 (remove_confirmed), 300-406 (Mempool impl), 358-401 (prune) |
| `crates/node/txpool/src/ordering.rs` | `SenderQueue` -- per-sender nonce tracking | 69-80 (struct), 91-116 (insert), 130-137 (remove_confirmed) |
| `crates/node/ledger/src/lib.rs` | `LedgerState` holds the `InMemoryMempool` -- swap point | 68-70 (struct field), 148 (construction), 175 (submit_tx) |
| `crates/node/reporters/src/lib.rs` | `FinalizedReporter` -- QMDB persistence + pruning | 204-220 (persist_snapshot spawn + await), 226 (prune_mempool call) |
| `crates/node/runner/src/runner.rs` | RPC tx_submit callback -- creates validator per request | 406-431 |
| `crates/node/txpool/src/validator.rs` | `TransactionValidator` -- nonce validation against QMDB only | 91-100 |
| `bin/loadgen/src/main.rs` | Load generator (already fixed with account sharding) | 82-88 (SeqCst nonces), 190-215 (targeted send), 296-356 (per-account tasks) |

## Testing Plan

1. **Unit test: Nonce-aware pruning** -- Insert TX(nonce=0), TX(nonce=1), TX(nonce=2) into `TransactionPool` for one sender. Prune with only TX(nonce=0)'s hash. Verify that `next_nonce` advances to 1 and TX(nonce=1), TX(nonce=2) remain as pending.

2. **Unit test: Stale nonce rejection** -- Insert TX(nonce=0) into `TransactionPool`. Call `remove_confirmed(0)`. Attempt to insert another TX(nonce=0). Verify it is rejected.

3. **Integration test: Multi-validator broadcast** -- In the e2e harness with 4 validators, send the same transaction to all 4 RPC endpoints. Wait for finalization. Verify that all 4 validators prune the transaction and that subsequent proposals do not contain stale copies.

4. **Load test regression** -- Run `loadgen --accounts 10 --total_txs 10000 --broadcast-rpc-urls <all-validators>`. Measure block proposal success rate. Target: >95% (up from ~30%).

5. **Throughput benchmark** -- Compare effective TPS before and after the fix:
   - Before: ~600 tx/s (with broadcast storm)
   - Target: ~2000 tx/s (matching single-validator throughput)

6. **Mempool size monitoring** -- During the load test, verify that mempool size remains bounded and trends toward zero as transactions are finalized, rather than growing unboundedly with stale entries.

## Verification Steps

After implementing the fix, run these concrete verification commands:

### Step 1: Unit tests pass

```bash
cargo test -p kora-txpool
cargo test -p kora-consensus
```

### Step 2: Verify nonce-aware pruning in TransactionPool

The existing test `pool_prune_advances_sender_nonce` in `crates/node/txpool/src/pool.rs` (line 528) already validates this behavior. After the swap, run:

```bash
cargo test -p kora-txpool -- pool_prune_advances_sender_nonce
```

### Step 3: Safe load test (baseline, must not regress)

```bash
just trusted-devnet
sleep 30
cargo run --release --bin loadgen -- \
  --total-txs 5000 --accounts 20 --concurrency 50 \
  --rpc-url http://127.0.0.1:8545
```

Expected: 100% success, ~2,900 TPS, chain healthy.

### Step 4: Broadcast mode stress test (the storm scenario)

```bash
just trusted-devnet
sleep 30
cargo run --release --bin loadgen -- \
  --total-txs 10000 --accounts 20 --concurrency 50 \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

Expected: Block proposal success rate >95%. Chain does not stall. Verify:

```bash
# Block number should keep advancing
curl -s http://127.0.0.1:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
sleep 5
curl -s http://127.0.0.1:8545 -X POST \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

### Step 5: Heavy sustained load (the previously-stalling config)

```bash
just trusted-devnet
sleep 30
cargo run --release --bin loadgen -- \
  --total-txs 50000 --accounts 50 --concurrency 200 \
  --rpc-url http://127.0.0.1:8545
```

Expected: Chain does NOT permanently stall. Some transaction rejections are acceptable (pool capacity), but block production must continue throughout.
