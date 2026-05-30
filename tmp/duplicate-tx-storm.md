# Duplicate Transaction Broadcast Storm

## Background: What is Kora?

Kora is an EVM-compatible blockchain built on a multi-validator BFT consensus system. In a typical Kora devnet deployment, 4 validator nodes participate in consensus. Each validator maintains its own local mempool of unconfirmed transactions. Unlike Ethereum mainnet clients, Kora does **not** implement a P2P transaction gossip protocol -- transactions only propagate to other validators when they are included in a proposed block. This means external tools (load generators, deployment scripts) must submit transactions directly to individual validator RPC endpoints.

## The Problem

When a load generator broadcasts the same transaction to multiple validators simultaneously, each validator independently accepts and stores that transaction in its local mempool. When one validator finalizes the transaction (nonce consumed, state updated), the other validators still hold the now-stale copy. These stale copies subsequently get proposed in new blocks, where they fail during execution because the nonce has already been consumed. This creates a "storm" of failed transactions that cascades through the network.

## How Transactions Enter the System

The transaction submission path in Kora (from `crates/node/runner/src/runner.rs`):

```rust
let tx_submit: TxSubmitCallback = Arc::new(move |data| {
    let ledger = tx_ledger.clone();
    let state = tx_state.clone();
    Box::pin(async move {
        let tx = Tx::new(data);
        let validator = TransactionValidator::new(chain_id, state, PoolConfig::default());
        validator.validate(tx.clone()).await?;  // Check nonce against QMDB
        if ledger.submit_tx(tx).await {
            Ok(())
        } else {
            Err(RpcError::InvalidTransaction("transaction rejected by mempool"))
        }
    })
});
```

Key architectural facts:
- Each validator has its own `TransactionValidator` instance checking against its own local QMDB state.
- Each validator has its own independent `InMemoryMempool`.
- There is **no** cross-validator transaction deduplication.
- There is **no** transaction gossip protocol -- transactions do not propagate between validators via P2P.

## The Broadcast Pattern

In Kora's devnet test infrastructure, the load generator sends transactions to all 4 validators for redundancy and load balancing:

```
Loadgen ---> Validator 0 (RPC :8545)
         |-> Validator 1 (RPC :8546)
         |-> Validator 2 (RPC :8547)
         |-> Validator 3 (RPC :8548)
```

When the loadgen submits a transaction (e.g., `TX_A` with nonce=5 from account 0xABC):
1. All 4 validators receive `TX_A`.
2. All 4 validators independently validate `TX_A` against their local QMDB (all see state nonce <= 5).
3. All 4 validators accept `TX_A` into their local mempools.
4. Now `TX_A` exists in 4 separate mempools simultaneously.

## The Storm Effect

Here is the sequence that produces the failure cascade:

```
T0: TX_A (nonce=5, account 0xABC) is in all 4 mempools

T1: Validator 0 is elected proposer for round N
    - Pulls TX_A from its mempool
    - Proposes block containing TX_A
    - Consensus finalizes block N
    - Executor runs TX_A: SUCCESS (nonce 5 -> 6 in state)

T2: Validators 0,1,2,3 all persist the finalized block
    - Validator 0 prunes TX_A from its mempool (proposer-side pruning)
    - Validators 1,2,3 call prune() with TX_A's TxId
    - IF prune correctly identifies TX_A by hash, it is removed

T3: Validator 1 is elected proposer for round N+1
    - Problem: If pruning was incomplete (see below), TX_A is still in validator 1's mempool
    - Validator 1 proposes block containing TX_A (stale copy)
    - Executor attempts TX_A: FAIL (nonce too low: got 5, expected 6)
    - Block execution aborts

T4: Validator 2 proposes next round -- same problem repeats
T5: Validator 3 proposes next round -- same problem repeats
```

## Why Pruning Does Not Fully Solve This

The `InMemoryMempool` prune mechanism works by transaction ID (keccak256 hash of raw bytes):

```rust
// From crates/node/consensus/src/components/mempool.rs
fn prune(&self, tx_ids: &[TxId]) {
    let mut inner = self.inner.write();
    for id in tx_ids {
        inner.remove(id);
    }
}
```

And from the ledger layer (`crates/node/ledger/src/lib.rs`):

```rust
pub async fn prune_mempool(&self, txs: &[Tx]) {
    let inner = self.inner.lock().await;
    let tx_ids: Vec<TxId> = txs.iter().map(Tx::id).collect();
    inner.mempool.prune(&tx_ids);
}
```

Pruning works correctly **if** the finalized block's transactions are propagated back to all validators for pruning. However, the critical issue is one of **timing and scope**:

1. **Nonce-based staleness is invisible to hash-based pruning**: If account 0xABC had transactions `TX_A` (nonce=5) and `TX_B` (nonce=5, different data/hash) both in the pool, pruning `TX_A` by hash does not remove `TX_B`. The nonce=5 slot has been consumed, but `TX_B` remains.

2. **No post-finalization nonce scan**: After a block is finalized, there is no sweep of the mempool to remove transactions whose nonces have become stale relative to the new state. The `InMemoryMempool` has no concept of sender nonces -- it is a flat key-value map indexed by transaction hash.

3. **Validator state lag**: Between finalization and QMDB persistence, there is a window where new incoming transactions are validated against stale state, allowing more stale transactions to enter.

## Real Test Data: The 70% Failure Rate

During a 50,000-transaction load test on a 4-validator devnet:

- The loadgen submitted transactions to all 4 validators (round-robin or broadcast).
- Each validator accumulated a full copy of all pending transactions.
- When validator 0 finalized a block containing 500 transactions, validators 1-3 still held those 500 transactions.
- The next 3 proposers (validators 1, 2, 3) each attempted to include the same already-executed transactions.
- Result: approximately **70% of block proposals failed** during execution due to nonce-too-low errors.
- Net throughput dropped from the expected ~2000 tx/s to ~600 tx/s.
- The executor's error propagation (via Rust's `?` operator) caused entire blocks to abort rather than skipping individual failed transactions.

Observed log pattern across nodes:

```
WARN rpc submit: ledger.submit_tx returned false (duplicate or pool error)
WARN rpc submit: validator rejected tx ... nonce too low: got 0, expected at least 1
```

The first message indicates the same raw bytes hitting the same node twice (hash collision in `InMemoryMempool`). The second indicates a transaction arriving at a node whose QMDB has already advanced past that nonce.

## Relationship to Account Sharding in Loadgen

A partial mitigation was introduced in the load generator: **account sharding**. Instead of sending all transactions from a single account to all validators, the loadgen assigns different sender accounts to different validators:

```
Account A (nonces 0-999)  --> only to Validator 0
Account B (nonces 0-999)  --> only to Validator 1
Account C (nonces 0-999)  --> only to Validator 2
Account D (nonces 0-999)  --> only to Validator 3
```

This eliminates the broadcast duplication because each transaction only exists in one validator's mempool. Each proposer pulls from its own shard of accounts and there is no cross-validator nonce conflict.

**However**, account sharding is a loadgen-level workaround, not a protocol-level fix. It does not help with:
- Real-world usage where users submit to any available RPC endpoint.
- Wallet retry logic that resubmits to multiple endpoints after timeout.
- Deployment scripts (like Foundry Forge) that broadcast contract creation transactions to multiple nodes.
- Any scenario where the same signed transaction reaches more than one validator.

## The Fundamental Problem: No Post-Finalization Mempool Invalidation

Even with sharding, the core architectural gap remains:

**The `InMemoryMempool` is nonce-unaware.** It stores transactions as opaque blobs indexed by hash. After a block is finalized and account nonces advance, the mempool has no mechanism to identify and evict transactions that have become invalid due to nonce advancement.

The `TransactionPool` (in `crates/node/txpool/src/pool.rs`) solves this with per-sender queues:

```rust
// From crates/node/txpool/src/ordering.rs
pub struct SenderQueue {
    pub sender: Address,
    pub next_nonce: u64,           // Tracks the next valid nonce
    pub pending: Vec<OrderedTransaction>,  // Nonce-consecutive executable txs
    pub queued: Vec<OrderedTransaction>,   // Future txs waiting for gaps
}

pub fn remove_confirmed(&mut self, confirmed_nonce: u64) {
    self.pending.retain(|tx| tx.nonce > confirmed_nonce);
    self.queued.retain(|tx| tx.nonce > confirmed_nonce);
    if confirmed_nonce >= self.next_nonce {
        self.next_nonce = confirmed_nonce + 1;
    }
    self.promote_queued();
}
```

With `TransactionPool`, after finalization:
1. `remove_confirmed(sender, confirmed_nonce)` evicts all transactions with nonce <= confirmed_nonce.
2. `next_nonce` advances, so any future insertions with stale nonces are immediately rejected by `SenderQueue::insert()`.
3. Queued transactions with now-valid nonces are promoted to pending.

The `InMemoryMempool` has none of this logic. Its `prune()` method only removes transactions by exact hash match -- it cannot invalidate "all transactions from sender X with nonce <= N".

## Metrics That Indicate This Pattern

When the duplicate transaction storm is occurring, the following observable symptoms appear:

| Metric / Log | Indicates |
|---|---|
| `ledger.submit_tx returned false` at high rate | Same raw bytes hitting the same node repeatedly |
| `validator rejected tx: nonce too low` | Transaction arriving after QMDB advanced past its nonce |
| Block execution failures / empty blocks | Proposer included stale transactions that fail at execution |
| Mempool size remains high despite blocks being produced | Stale transactions are not being pruned |
| High ratio of proposed-to-finalized transaction count | Many proposed txs are being discarded |
| `pending_count` on mempool stays constant across finalizations | Pruning is not removing invalidated transactions |

## Impact and Severity

**Severity: HIGH**

**Liveness**: In the observed load test, the storm reduced effective throughput by ~70%. Blocks were proposed with stale transactions, failed during execution, and produced empty or short blocks. The chain continued to advance (liveness was not fully lost) but at drastically reduced capacity.

**Correctness**: No incorrect state transitions occur -- the executor correctly rejects stale-nonce transactions. The issue is purely one of efficiency and resource waste.

**Resource waste**:
- Network bandwidth consumed broadcasting blocks that will partially fail.
- CPU time spent re-executing transactions that are known-stale.
- Consensus rounds "wasted" on blocks that produce fewer finalized transactions than capacity allows.

**User experience**: Transactions that were already finalized may appear "stuck" to users because the same tx hash shows up in other validators' mempools. Users may resubmit, exacerbating the problem.

## Recommended Fixes

### 1. Replace InMemoryMempool with TransactionPool (Primary Fix)

The `TransactionPool` already exists, implements the `Mempool` trait, and provides:
- Per-sender nonce tracking that prevents stale insertions
- `remove_confirmed()` for nonce-based bulk eviction after finalization
- Hash-based deduplication (rejects exact duplicates)
- Gas-price-based replacement for same-nonce conflicts

This is a wiring change in `crates/node/ledger/src/lib.rs`:
```rust
// Before:
mempool: InMemoryMempool,

// After:
mempool: TransactionPool,
```

### 2. Post-Finalization Nonce Sweep

After each finalization, iterate affected senders and remove all pool transactions with nonces that are now stale:

```rust
// After block finalization:
for (sender, new_nonce) in affected_accounts {
    pool.remove_confirmed(&sender, new_nonce);
}
```

This is already how `TransactionPool::prune()` works (it resolves per-sender max confirmed nonces), making fix #1 sufficient.

### 3. Single-Endpoint Submission (Loadgen Workaround)

For controlled test environments, ensure each transaction is only submitted to a single validator endpoint. This is what account sharding achieves, but it should be combined with fix #1 for production correctness.

### 4. Transaction Gossip Protocol (Future Enhancement)

Implementing P2P transaction gossip would allow validators to share mempool contents, enabling cross-validator deduplication before proposal time. This is a larger architectural change but would align Kora with how Ethereum mainnet clients operate.
