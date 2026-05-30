# Load Generator Nonce Race Condition

## Background: Kora and the Loadgen Tool

Kora is a high-performance blockchain built on Commonware primitives, targeting sub-second finality with a validator-local mempool architecture. Each validator maintains its own transaction pool; there is no global gossip layer for pending transactions in the devnet configuration.

The load generator (`bin/loadgen/src/main.rs`) is a CLI stress-testing tool that sends large volumes of EIP-1559 Ethereum transactions to Kora's JSON-RPC endpoints. It creates multiple accounts (each derived from a deterministic seed), tracks per-account nonces locally, signs transactions with `alloy` and `k256`, and submits them via `eth_sendRawTransaction`. Its purpose is to measure throughput (TPS), identify bottlenecks, and exercise the chain under sustained high concurrency.

---

## The Original Bug

### Architecture Before the Fix

The original loadgen used a single `FuturesUnordered` collection with a global concurrency limit to manage in-flight transactions:

```rust
// ORIGINAL CODE (before fix)
use futures::stream::{FuturesUnordered, StreamExt};

struct Account {
    key: SigningKey,
    address: Address,
    nonce: AtomicU64,
}

impl Account {
    fn next_nonce(&self) -> u64 {
        self.nonce.fetch_add(1, Ordering::Relaxed)  // THE BUG
    }
}

// Main send loop
for i in 0..args.total_txs {
    let account = accounts[i as usize % accounts.len()].clone();
    let nonce = account.next_nonce();                    // Capture nonce
    let tx = sign_eip1559_transfer(..., nonce, ...);     // Sign immediately

    let fut = async move {
        send_raw_transaction_to_any(&clients, tx).await  // Send (ASYNC, unordered)
    };

    futures.push(fut);

    if futures.len() >= args.concurrency {
        futures.next().await;  // Wait for ONE to complete, any order
    }
}
while futures.next().await.is_some() {}  // Drain remaining
```

This architecture had three compounding problems that created a nonce race condition.

---

### Problem 1: AtomicU64 with Relaxed Ordering

```rust
fn next_nonce(&self) -> u64 {
    self.nonce.fetch_add(1, Ordering::Relaxed)
}
```

`Ordering::Relaxed` guarantees only that the `fetch_add` operation itself is atomic (no torn reads/writes). It provides **no guarantees** about:

- Visibility of the updated value to other threads/tasks
- Ordering of the atomic operation relative to surrounding memory accesses

In practice, on x86 architectures, Relaxed often behaves like SeqCst due to the strong memory model, but on ARM (and in the Tokio work-stealing scheduler where tasks migrate between threads), one task can observe a stale nonce value even after another task has incremented it. This is a correctness hazard that makes the bug platform-dependent and intermittent — the worst kind.

---

### Problem 2: Concurrent In-Flight Transactions Per Account

With `concurrency=200` and 50 accounts, the math is straightforward:

- Up to 200 futures are buffered in `FuturesUnordered` before any single one is awaited
- Each account may have `200 / 50 = 4` transactions in-flight simultaneously
- Nonces are assigned sequentially (100, 101, 102, 103) but the HTTP sends race against each other

The Tokio runtime, operating system network stack, and remote RPC server all introduce non-deterministic latency. The result: transaction with nonce 103 may arrive at the RPC endpoint before nonce 100 from the same account.

When a transaction pool receives nonce 103 before nonce 100, it must either:
- Accept it and hold it as a "future" transaction (if the pool supports this)
- Reject it as having a nonce gap

Kora's mempool rejects transactions with nonce gaps (the pending nonce for that account is N, and the incoming nonce is > N). This means the first arrival "wins" and subsequent arrivals are rejected, but since arrival order is random, the ordering is corrupted.

---

### Problem 3: Broadcast-to-All-Validators Pattern

The original `send_raw_transaction_to_any()` function broadcast every transaction to ALL validator RPC endpoints simultaneously:

```rust
// ORIGINAL: sends to ALL clients
async fn send_raw_transaction_to_any(clients: &[RpcClient], raw_tx: Bytes) -> Result<String> {
    for client in clients {
        client.send_raw_transaction(&raw_tx).await;  // Fan-out to every validator
    }
}
```

Because Kora's devnet uses validator-local mempools (no gossip), this was intended to ensure the active proposer had the transaction. But it creates a critical problem:

1. Account A sends nonce 50 to validators {V1, V2, V3, V4}
2. Validator V1 proposes a block containing nonce 50 — it gets finalized
3. The on-chain nonce for account A is now 51
4. Validators V2, V3, V4 still have nonce 50 in their local mempools
5. These are now **stale** (NonceTooLow) but the mempools do not proactively prune them
6. When V2 becomes proposer, it may include stale nonce 50 in its block proposal

This creates "stale copies" — transactions that were valid when submitted but became invalid after another validator finalized them first.

---

## The Cascade: From Race Condition to Permanent Chain Stall

```
Loadgen assigns nonces 100, 101, 102, 103 to account A (concurrency=200)
    |
    v
HTTP sends race: nonce 102 arrives at validator V1 before nonces 100, 101
    |
    v
V1 accepts nonce 102 (it was "next" from V1's perspective)
V1 proposes block with nonce 102 --> finalized
On-chain nonce for account A is now 103
    |
    v
V2 still has nonces 100, 101 in its mempool (stale copies from broadcast)
    |
    v
V2 becomes proposer, includes nonces 100, 101 in its block proposal
    |
    v
Executor processes nonce 100: NonceTooLow error
Executor uses `?` operator on the error --> ABORT entire block execution
    |
    v
Block proposal fails, view is nullified
Stale transactions are NOT pruned (pruning requires successful finalization)
    |
    v
V2 proposes again, same stale txs are included again --> same failure
    |
    v
PERMANENT STALL: no block can ever finalize while stale txs exist in the pool
```

The critical insight is that the executor's `?` operator treats ANY transaction execution failure as fatal, aborting the entire block. Combined with the fact that stale transactions are only pruned on successful finalization (which can never happen because the stale txs prevent finalization), this creates a deadlock.

---

## Why This Was Hard to Diagnose

This bug was particularly insidious because:

1. **It appeared as a chain bug, not a test tool bug.** The symptom was "chain stalls permanently under load" — which naturally directed investigation toward the consensus protocol, executor, and mempool logic rather than the load generator itself.

2. **It was non-deterministic.** At low concurrency (50 or fewer concurrent sends), nonces were unlikely to arrive out of order because fewer transactions from the same account were in-flight. The bug only manifested at high concurrency with many accounts.

3. **Relaxed ordering works on x86.** Developers running tests on x86 laptops may never observe the memory ordering issue because x86's TSO (Total Store Order) memory model provides stronger guarantees than Relaxed requires. The bug is more likely on ARM or under heavy Tokio work-stealing.

4. **The broadcast pattern masked the root cause.** Because transactions were sent to all validators, the "stale copy" issue was conflated with the nonce-ordering issue. Both contributed to the stall but through different mechanisms.

5. **Success rate was misleading.** A 25.7% success rate under high load could be attributed to "RPC overload" or "pool capacity limits" without recognizing the structural nonce ordering problem.

---

## The Four Fixes Applied

### Fix 1: Per-Account Sequential Sends

**Before:** A single `FuturesUnordered` managed all transactions across all accounts. Multiple transactions from the same account could be in-flight simultaneously.

**After:** One dedicated Tokio task per account. Within each task, transactions are sent strictly sequentially — nonce N must complete (receive RPC acceptance or final rejection) before nonce N+1 is even assigned. Cross-account parallelism is preserved because all account tasks run concurrently.

```rust
// AFTER: One task per account, sequential within each
for (idx, account) in accounts.iter().enumerate() {
    let handle = tokio::spawn(async move {
        for _ in 0..count {
            let _permit = semaphore.acquire().await.expect("semaphore closed");
            let nonce = account.next_nonce();
            let tx = sign_eip1559_transfer(&account.key, chain_id, receiver, transfer_amount, nonce, gas_limit);

            // ... send with retry ...
            // Nonce N completes before nonce N+1 is assigned for this account
        }
    });
    handles.push(handle);
}
```

A global `Semaphore` (with permits equal to the `--concurrency` flag) bounds total in-flight HTTP requests across all accounts, preventing RPC overload while maintaining ordering guarantees within each account.

### Fix 2: SeqCst Memory Ordering

**Before:**
```rust
fn next_nonce(&self) -> u64 {
    self.nonce.fetch_add(1, Ordering::Relaxed)
}
```

**After:**
```rust
fn next_nonce(&self) -> u64 {
    self.nonce.fetch_add(1, Ordering::SeqCst)
}

fn set_nonce(&self, nonce: u64) {
    self.nonce.store(nonce, Ordering::SeqCst);
}
```

`Ordering::SeqCst` establishes a single total order of all sequentially-consistent operations across all threads. Every thread observes the same global ordering of atomic operations. This eliminates any possibility of stale nonce reads, regardless of platform or task migration.

While Fix 1 (sequential per-account sends) makes the ordering largely moot for correctness (since each account only has one task), SeqCst provides defense-in-depth and makes the code correct even if the concurrency model changes in the future.

### Fix 3: Account Sharding Across Validators

**Before:** `send_raw_transaction_to_any()` broadcast every transaction to ALL validator endpoints simultaneously, creating stale copies in multiple mempools.

**After:** `send_raw_transaction_to()` pins each account to a specific validator using deterministic assignment:

```rust
// Each account is pinned to one validator (avoids stale copies in other mempools)
let target_validator = idx;  // account index determines validator

async fn send_raw_transaction_to(
    clients: &[RpcClient],
    raw_tx: Bytes,
    target_idx: usize,
) -> Result<String> {
    let idx = target_idx % clients.len();

    // Try the target client first
    match clients[idx].send_raw_transaction(&raw_tx).await {
        Ok(hash) => return Ok(hash),
        Err(e) => {
            // If target rejects, try remaining clients as fallback
            // ...
        }
    }
}
```

This ensures each account's transactions exist in only ONE validator's mempool. When that validator finalizes a transaction, no stale copies remain elsewhere. Fallback to other validators is only attempted on rejection (pool full, timeout), not by default.

### Fix 4: Retry with Exponential Backoff

**Before:** A single send attempt per transaction. If the pool rejects the transaction (pool full, rate limit), it is counted as a permanent failure.

**After:** Up to 10 retry attempts with linear backoff (100ms x attempt_number):

```rust
let mut attempts = 0;
loop {
    match send_raw_transaction_to(&clients, tx.clone(), target_validator).await {
        Ok(hash) => {
            success.fetch_add(1, Ordering::Relaxed);
            break;
        }
        Err(e) => {
            attempts += 1;
            if attempts >= 10 {
                failure.fetch_add(1, Ordering::Relaxed);
                warn!(nonce, error = %e, account = %account.address, "tx failed after retries");
                break;
            }
            // Back off to let the chain finalize pending txs
            tokio::time::sleep(Duration::from_millis(100 * attempts)).await;
        }
    }
}
```

The backoff (100ms, 200ms, 300ms, ... up to 1000ms) gives the chain time to finalize pending transactions and drain pool capacity. This handles transient rejections due to `max_txs_per_sender` limits without creating permanent nonce gaps.

---

## Code Diff Summary

```diff
- use futures::stream::{FuturesUnordered, StreamExt};
+ use tokio::sync::Semaphore;

  struct Account {
      key: SigningKey,
      address: Address,
      nonce: AtomicU64,
  }

  impl Account {
-     fn next_nonce(&self) -> u64 {
-         self.nonce.fetch_add(1, Ordering::Relaxed)
-     }
+     fn next_nonce(&self) -> u64 {
+         self.nonce.fetch_add(1, Ordering::SeqCst)
+     }
+
+     fn set_nonce(&self, nonce: u64) {
+         self.nonce.store(nonce, Ordering::SeqCst);
+     }
  }

- async fn send_raw_transaction_to_any(clients: &[RpcClient], raw_tx: Bytes) -> Result<String> {
-     // Sends to ALL clients simultaneously (creates stale copies)
- }
+ async fn send_raw_transaction_to(
+     clients: &[RpcClient],
+     raw_tx: Bytes,
+     target_idx: usize,
+ ) -> Result<String> {
+     // Sends to specific client; fallback to others only on rejection
+ }

- // Single FuturesUnordered with global concurrency limit
- for i in 0..args.total_txs {
-     let account = accounts[i as usize % accounts.len()].clone();
-     let nonce = account.next_nonce();
-     let tx = sign_eip1559_transfer(...);
-     futures.push(send_raw_transaction_to_any(&clients, tx));
-     if futures.len() >= args.concurrency { futures.next().await; }
- }
+ // Per-account Tokio tasks with global Semaphore + retry loop
+ for (idx, account) in accounts.iter().enumerate() {
+     tokio::spawn(async move {
+         for _ in 0..count {
+             let _permit = semaphore.acquire().await;
+             let nonce = account.next_nonce();
+             let tx = sign_eip1559_transfer(...);
+             // Retry loop with backoff
+             send_raw_transaction_to(&clients, tx, idx).await;
+             // N completes before N+1 assigned
+         }
+     });
+ }
```

---

## Broader Implications

This bug is not merely a loadgen-specific issue. It exposes a fundamental vulnerability in Kora's transaction processing pipeline:

- **Any external system** that submits transactions concurrently from a single account can trigger the same stale-nonce chain stall: DApp backends with multiple workers, bridge relayers with parallel submissions, MEV bots, or batch transaction processors.

- **The loadgen fix alone is insufficient.** Even with perfectly ordered nonce delivery, the chain can still stall if the executor aborts on encountering a stale transaction (from any source). Two chain-level fixes are needed:
  1. **Executor skip-and-continue**: Skip invalid transactions during block execution rather than aborting the entire block
  2. **Proactive mempool pruning**: Remove transactions with nonce < on-chain nonce after each block finalization

The loadgen fixes eliminate the tool as a source of invalid transactions, making it possible to test the chain without the test infrastructure itself being the cause of failures.
