# Loadgen lacks progress reporting, nonce recovery, and transaction confirmation -- silent hangs under stress

## Summary

The Kora load generator (`bin/loadgen/src/main.rs`) has several resilience gaps that make it unreliable for stress testing and difficult to debug when things go wrong. Under moderate load (1,000-5,000 transactions) it completes successfully, but at higher volumes (10,000+ transactions) it silently hangs with no diagnostic output, no mechanism to recover from nonce desynchronization with the chain, and no way to verify whether submitted transactions were actually included in blocks. The tool also lacks periodic progress reporting, so operators cannot distinguish between "running normally" and "stuck" without killing the process and inspecting chain state manually. These issues were exposed during devnet load testing on a 4-validator Simplex BFT network.

## Problem Description

### 1. No periodic progress reporting during execution

The loadgen produces a startup log (account addresses, configuration) and a final summary log, but emits zero output during the actual transaction-sending phase. For a 10,000-transaction test that may run for several minutes, there is no way to observe:

- How many transactions have been submitted so far
- How many have succeeded vs. failed
- Which accounts (if any) are stuck in retry loops
- The current submission throughput (TPS)
- How much time has elapsed

**File**: `bin/loadgen/src/main.rs`, lines 362-414

The per-account task loop runs silently unless `--verbose` is passed, in which case it logs every single transaction hash (too noisy for long runs). There is no middle ground -- no periodic summary that reports aggregate progress every N seconds or every N transactions.

**Observed impact**: During a 10,000-transaction stress test, the loadgen appeared to hang after approximately 3 minutes. Without any progress output, it was impossible to tell whether it was stuck in retry loops, waiting on slow RPC responses, or making forward progress. The process had to be killed manually.

### 2. No nonce resynchronization on chain-state divergence

When the loadgen's internal nonce counter diverges from the on-chain nonce (which happens under high load when validators reject transactions for nonce gaps), the retry loop retries the same failing nonce repeatedly without querying the chain for the current nonce.

**File**: `bin/loadgen/src/main.rs`, lines 377-405

```rust
let mut attempts = 0u32;
let mut succeeded = false;
loop {
    let _permit = semaphore.acquire().await.expect("semaphore closed");
    let result =
        send_raw_transaction_to(&clients, tx.clone(), target_validator).await;
    drop(_permit);

    match result {
        Ok(hash) => {
            // ...
            succeeded = true;
            break;
        }
        Err(e) => {
            attempts += 1;
            if u64::from(attempts) >= MAX_RETRY_ATTEMPTS {
                warn!(nonce, error = %e, account = %account.address, "tx failed after retries");
                break;
            }
            // Exponential backoff: 100ms, 200ms, 400ms, ...
            let delay = RETRY_BASE_DELAY * 2u32.saturating_pow(attempts - 1);
            tokio::time::sleep(delay).await;
        }
    }
}
```

The retry logic treats all errors identically -- it retries the exact same signed transaction (same nonce, same payload) with exponential backoff. But there are at least four distinct error classes that require different recovery strategies:

| Error type | RPC error message | Correct recovery |
|-----------|-------------------|-----------------|
| Nonce too low | `"nonce too low: got N, expected at least M"` | Re-query `eth_getTransactionCount`, advance local nonce to match chain state |
| Nonce gap | `"nonce gap: got N, expected M"` | Re-query nonce, wait for pending transactions to drain, then resume from the chain's current nonce |
| Nonce already in pool | `"nonce N already in pool for sender 0x..."` | Increment nonce and retry with new transaction (the previous one is pending) |
| Transient failure | Connection refused, timeout, HTTP 503 | Retry the same transaction (current behavior) |
| Pool full | `"transaction rejected by mempool"` | Back off and retry, or skip and move on |

When a validator responds with "nonce gap: got 339, expected 57", the loadgen has raced far ahead of the chain's inclusion rate. Retrying nonce 339 will always fail because the chain is only at nonce 57. The correct action is to re-query `eth_getTransactionCount` and reset the local nonce counter, but the code never does this.

**Observed impact**: During the 10,000-transaction test, validator logs showed cascading nonce gap rejections across all 4 validators:
```
WARN kora_runner::runner: rpc submit: validator rejected tx
  tx_id=TxId(0x8cc0...) error=nonce gap: got 339, expected 57
WARN kora_runner::runner: rpc submit: validator rejected tx
  tx_id=TxId(0x...) error=nonce gap: got 470, expected 189
```

The loadgen's accounts had submitted nonces 339 and 470, but the chain had only confirmed up to nonces 57 and 189 respectively. Every retry of the high-nonce transactions failed with the same error, burning through all 10 retry attempts, after which the account moved to the next nonce (which was even further ahead), making the problem worse.

### 3. Missing `NonceAlreadyInPool` error handling

Since PR #134, the transaction pool rejects same-nonce duplicates at ingress. The validator returns this as an RPC error:

**File**: `crates/node/txpool/src/error.rs`, lines 92-99
```rust
/// A transaction with the same sender and nonce already exists in the pool.
#[error("nonce {nonce} already in pool for sender {sender}")]
NonceAlreadyInPool {
    /// Sender address.
    sender: Address,
    /// Conflicting nonce.
    nonce: u64,
},
```

This surfaces through the RPC layer (`crates/node/runner/src/runner.rs`, line 590-591) as:
```
RPC error: {"code":-32602,"message":"invalid transaction: nonce 42 already in pool for sender 0x..."}
```

The loadgen treats this as a generic error and retries the same transaction with backoff. But this error means the transaction is already pending in the pool -- retrying with the same nonce will always fail. The correct action is to treat this as a success (the nonce is covered), increment the local nonce, and move on.

**Observed impact**: When the loadgen's `send_raw_transaction_to` falls back to multiple validators (lines 230-255), the first validator may accept the transaction while the second rejects with `NonceAlreadyInPool`. The fallback logic does not distinguish this from other errors, so the transaction is counted as failed when it was actually accepted.

### 4. No transaction confirmation or inclusion verification

The loadgen operates in a fire-and-forget model. After `eth_sendRawTransaction` returns a transaction hash, the loadgen considers the transaction "successful" and moves on. It never checks whether the transaction was actually included in a block.

**File**: `bin/loadgen/src/main.rs`, lines 386-392

```rust
Ok(hash) => {
    success.fetch_add(1, Ordering::Relaxed);
    if verbose {
        info!(nonce, hash = %hash, account = %account.address, "tx sent");
    }
    succeeded = true;
    break;
}
```

The `success_count` counter tracks submissions, not confirmations. There is no `eth_getTransactionReceipt` polling, no block-height tracking, and no final reconciliation step that compares expected nonces against on-chain nonces.

**Observed impact**: In the multi-validator simultaneous load test, both loadgen instances reported 100% success (200 total transactions submitted). But on-chain nonces showed only 100 were included -- exactly 50% were silently dropped due to nonce conflicts between the two instances submitting to different validators with the same accounts. The loadgen's summary output gave no indication of this.

### 5. No adaptive concurrency based on chain throughput

The loadgen sends transactions as fast as the concurrency semaphore allows, regardless of the chain's actual inclusion rate. With `--concurrency 100` and 30 accounts, the loadgen submits transactions far faster than the chain includes them, creating a large nonce gap between submitted and confirmed nonces.

**File**: `bin/loadgen/src/main.rs`, lines 339-343

```rust
// Global concurrency limiter — bounds total in-flight HTTP requests
if args.concurrency == 0 {
    eyre::bail!("--concurrency must be >= 1");
}
let semaphore = Arc::new(Semaphore::new(args.concurrency));
```

The semaphore limits concurrent HTTP requests, not the gap between submitted and confirmed nonces. Even with per-account sequential sending (which correctly prevents nonce reordering within a single account), the per-account pipeline fills up faster than the chain drains it.

**Observed impact**: At 10,000 transactions across 30 accounts, each account had ~333 transactions to send. The loadgen submitted them sequentially per account, but so quickly that the chain fell hundreds of nonces behind. This triggered the nonce gap rejections described above.

### 6. Nonce initialization queries only the primary RPC

On startup, the loadgen queries `eth_getTransactionCount` from only the first RPC client (index 0) to initialize nonces:

**File**: `bin/loadgen/src/main.rs`, lines 302-307

```rust
if !args.dry_run {
    for account in &accounts {
        let nonce = clients[0].get_transaction_count(account.address).await?;
        account.set_nonce(nonce);
    }
}
```

If the primary RPC endpoint is down or returning stale data, the loadgen either crashes immediately (due to the `?` propagation) or starts with incorrect nonces. There is no fallback to broadcast URLs for nonce initialization.

### 7. The `send_raw_transaction_to` fallback does not distinguish error types

**File**: `bin/loadgen/src/main.rs`, lines 230-255

```rust
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
            let mut errors = vec![e.to_string()];
            for (i, client) in clients.iter().enumerate() {
                if i == idx {
                    continue;
                }
                match client.send_raw_transaction(&raw_tx).await {
                    Ok(hash) => return Ok(hash),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            eyre::bail!("all RPC endpoints rejected transaction: {}", errors.join("; "))
        }
    }
}
```

When the target validator rejects a transaction with "nonce too low" or "nonce gap", falling back to other validators with the same stale transaction is pointless -- they will reject it for the same reason. The fallback logic should only activate for connection-level errors (timeouts, refused connections, HTTP errors), not for semantic rejections that apply regardless of which validator receives the transaction.

## Proposed Solution

### Phase 1: Progress reporting (low effort, high impact)

Add a background task that periodically logs aggregate statistics:

```rust
// Spawn a progress reporter that runs every 5 seconds
let progress_success = success_count.clone();
let progress_failure = failure_count.clone();
let progress_total = args.total_txs;
let progress_start = start;

let progress_handle = tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.tick().await; // skip first immediate tick
    loop {
        interval.tick().await;
        let s = progress_success.load(Ordering::Relaxed);
        let f = progress_failure.load(Ordering::Relaxed);
        let elapsed = progress_start.elapsed().as_secs_f64();
        let tps = if elapsed > 0.0 { s as f64 / elapsed } else { 0.0 };
        info!(
            sent = s + f,
            success = s,
            failed = f,
            total = progress_total,
            elapsed_secs = format!("{:.1}", elapsed),
            tps = format!("{:.1}", tps),
            pct = format!("{:.1}%", (s + f) as f64 / progress_total as f64 * 100.0),
            "progress"
        );
        if s + f >= progress_total {
            break;
        }
    }
});
```

This provides a live heartbeat during long-running tests, making it immediately obvious whether the loadgen is making progress or stuck.

### Phase 2: Error-aware retry with nonce resynchronization

#### CRITICAL BUG in naive approach

A straightforward implementation of nonce recovery inside the existing inner retry loop has a **fatal flaw** due to the failure recovery code that follows it. Here is the structure of the current per-account task loop (`bin/loadgen/src/main.rs`, lines 362-414):

```rust
for _ in 0..count {
    let nonce = account.next_nonce();                        // line 364
    let tx = sign_eip1559_transfer(/* ... nonce ... */);     // line 365-372

    let mut attempts = 0u32;
    let mut succeeded = false;
    loop {                                                   // inner retry loop (line 379)
        // ... send tx, retry on error ...
        // On nonce gap: resync nonce and break              // <-- proposed fix
    }

    if !succeeded {                                          // line 407
        account.set_nonce(nonce);     // <-- OVERWRITES the resynced nonce!  // line 410
        failure.fetch_add(1, Ordering::Relaxed);             // line 411
    }
}
```

**The bug**: If you add nonce gap handling that calls `account.set_nonce(chain_nonce)` inside the inner loop and then `break`s (with `succeeded = false`, since the tx was not actually sent), the code at **line 410** immediately overwrites the resynced nonce back to the old failed value. The failure recovery path at lines 407-411 was designed to "rewind" the nonce so the next outer loop iteration retries the same nonce -- but after a nonce resync, this rewind destroys the correction.

**Concrete example of the bug in action**:

1. Account's `AtomicU64` nonce is at 340
2. Line 364: `next_nonce()` returns 339, advances atomic to 340
3. Transaction signed with nonce 339 is rejected: `"nonce gap: got 339, expected 57"`
4. Nonce gap handler resyncs: `account.set_nonce(57)` -- atomic is now 57
5. Inner loop breaks with `succeeded = false`
6. Line 410: `account.set_nonce(nonce)` where `nonce` is the local variable 339 -- **atomic is now 339 again**
7. Next outer loop iteration: `next_nonce()` returns 339 -- the exact same failing nonce

The nonce recovery never takes effect. The loadgen continues to hammer the same failing nonce.

#### Root cause: the loop structure conflates two concerns

The outer `for _ in 0..count` loop assumes each iteration corresponds to exactly one nonce. But nonce recovery (resync to chain state) may need to **skip** or **repeat** nonces, which does not fit the `for` loop's "one iteration = one nonce" model.

Additionally, the transaction is signed with a specific nonce **before** the retry loop (line 365-372), so after a nonce resync, the pre-signed `tx` is stale. Recovery requires re-signing with the new nonce, which means the signing must move inside the retry loop or the outer loop must restart from nonce acquisition.

#### Correct implementation: restructured outer loop

The fix requires restructuring the per-account task to use a `while` loop that tracks transactions sent (not nonces attempted), and moves transaction signing inside the retry-capable scope. Here is the corrected pseudocode:

```
let mut sent = 0u64;
while sent < count {
    let nonce = account.next_nonce();
    let tx = sign_eip1559_transfer(&account.key, chain_id, receiver, value, nonce, gas_limit);

    let mut attempts = 0u32;
    let mut result = TxOutcome::Pending;

    loop {
        let send_result = send_raw_transaction_to(&clients, tx.clone(), target).await;

        match send_result {
            Ok(hash) => {
                success.fetch_add(1, Relaxed);
                sent += 1;
                result = TxOutcome::Sent;
                break;
            }
            Err(e) => {
                let err_msg = e.to_string();
                attempts += 1;

                if err_msg.contains("nonce too low") {
                    // Transaction was already included (e.g. via broadcast copy).
                    // Re-query chain nonce and advance local counter.
                    if let Ok(chain_nonce) = get_nonce_from_any(&clients, account.address).await {
                        account.set_nonce(chain_nonce);
                    }
                    // Count as success -- the nonce was consumed on-chain.
                    success.fetch_add(1, Relaxed);
                    sent += 1;
                    result = TxOutcome::Sent;
                    break;
                }
                else if err_msg.contains("already in pool") {
                    // Transaction with this nonce is already pending in the pool.
                    // The nonce is covered. Count as success and move on.
                    success.fetch_add(1, Relaxed);
                    sent += 1;
                    result = TxOutcome::Sent;
                    break;
                }
                else if err_msg.contains("nonce gap") {
                    // We are ahead of the chain. Wait, resync, and RESTART
                    // the outer loop (do NOT fall through to the set_nonce rewind).
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if let Ok(chain_nonce) = get_nonce_from_any(&clients, account.address).await {
                        account.set_nonce(chain_nonce);
                    }
                    // Do NOT increment `sent` -- this nonce was never consumed.
                    // Break inner loop and let the outer while-loop re-acquire nonce.
                    result = TxOutcome::NeedsResync;
                    break;
                }
                else {
                    // Transient error -- exponential backoff
                    if u64::from(attempts) >= MAX_RETRY_ATTEMPTS {
                        warn!(nonce, error = %e, account = %account.address,
                              "tx failed after retries");
                        failure.fetch_add(1, Relaxed);
                        sent += 1;
                        result = TxOutcome::Failed;
                        break;
                    }
                    let delay = RETRY_BASE_DELAY * 2u32.saturating_pow(attempts - 1);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    // NO nonce rewind here. The nonce is either consumed (Sent/Failed)
    // or already corrected by the resync (NeedsResync).
    // The old `if !succeeded { account.set_nonce(nonce); }` is REMOVED.
}
```

Key changes from the current code:

1. **`while sent < count` replaces `for _ in 0..count`**: The outer loop counts successful sends (or exhausted retries), not nonce attempts. A nonce-gap resync does not consume a "send slot".

2. **No post-loop nonce rewind**: The `if !succeeded { account.set_nonce(nonce); }` block at lines 407-411 is removed entirely. Nonce management is handled exclusively inside the error handlers.

3. **`nonce too low` = implicit success**: The transaction was already included on-chain. Resync the local counter and count it as sent.

4. **`already in pool` = implicit success**: The nonce is already covered by a pending transaction. Count it and move on. This handles the `NonceAlreadyInPool` rejection from PR #134.

5. **`nonce gap` = resync and retry**: Resync the nonce counter and let the outer `while` loop re-acquire a fresh nonce and re-sign a new transaction. The old transaction (signed with the wrong nonce) is discarded.

6. **Transient errors**: Exhaust retries with backoff, then count as failed and move on. The nonce is "consumed" as a failed attempt (no rewind), which avoids infinite loops on permanently-failing nonces.

### Phase 3: Post-run confirmation verification

After all transactions are submitted, add a reconciliation step that verifies on-chain nonces match expectations:

```rust
// After all account tasks complete
info!("All transactions submitted. Verifying on-chain inclusion...");

let mut total_expected = 0u64;
let mut total_confirmed = 0u64;
let mut total_gap = 0u64;

for account in &accounts {
    let expected_nonce = account.nonce.load(Ordering::Relaxed);
    // Try all clients for the nonce query, in case the primary is down
    let chain_nonce = get_nonce_from_any(&clients, account.address).await?;

    let gap = expected_nonce.saturating_sub(chain_nonce);
    if gap > 0 {
        warn!(
            account = %account.address,
            expected = expected_nonce,
            confirmed = chain_nonce,
            pending = gap,
            "account has unconfirmed transactions"
        );
    }
    total_expected += expected_nonce;
    total_confirmed += chain_nonce;
    total_gap += gap;
}

info!(
    total_expected,
    total_confirmed,
    total_pending = total_gap,
    "Inclusion verification complete"
);
```

This does not block waiting for inclusion (which could take many seconds depending on block rate), but it provides an immediate sanity check against the chain state when the loadgen finishes.

### Phase 4: Resilient nonce initialization with fallback

Replace the single-client nonce query at startup with a fallback across all available RPC endpoints:

```rust
if !args.dry_run {
    for account in &accounts {
        let nonce = get_nonce_from_any(&clients, account.address)
            .await
            .wrap_err_with(|| format!(
                "failed to query nonce for {} from any RPC endpoint",
                account.address
            ))?;
        account.set_nonce(nonce);
    }
}

/// Query `eth_getTransactionCount` from any available RPC client,
/// trying each in order until one succeeds.
async fn get_nonce_from_any(clients: &[RpcClient], address: Address) -> Result<u64> {
    let mut last_err = None;
    for client in clients {
        match client.get_transaction_count(address).await {
            Ok(nonce) => return Ok(nonce),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| eyre::eyre!("no RPC clients configured")))
}
```

### Phase 5: Differentiate fallback by error class in `send_raw_transaction_to`

Only fall back to other validators on connection-level errors, not semantic rejections:

```rust
async fn send_raw_transaction_to(
    clients: &[RpcClient],
    raw_tx: Bytes,
    target_idx: usize,
) -> Result<String> {
    let idx = target_idx % clients.len();

    match clients[idx].send_raw_transaction(&raw_tx).await {
        Ok(hash) => Ok(hash),
        Err(e) => {
            let err_str = e.to_string();
            // Semantic rejections (nonce errors, pool errors) will fail
            // on all validators. Only fall back for transport errors.
            let is_transport_error = err_str.contains("error sending request")
                || err_str.contains("Connection refused")
                || err_str.contains("timed out")
                || err_str.contains("connection closed");

            if !is_transport_error {
                return Err(e);
            }

            // Transport error: try other clients
            let mut errors = vec![err_str];
            for (i, client) in clients.iter().enumerate() {
                if i == idx { continue; }
                match client.send_raw_transaction(&raw_tx).await {
                    Ok(hash) => return Ok(hash),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            eyre::bail!("all RPC endpoints failed: {}", errors.join("; "))
        }
    }
}
```

## Testing

1. **Progress reporting**: Run `loadgen --total-txs 5000 --accounts 20` and verify that progress lines appear on stdout every 5 seconds with sent/success/failed/tps counters.

2. **Nonce recovery**: Start loadgen with `--total-txs 10000 --accounts 30 --concurrency 100` against a 4-validator devnet. Verify that when nonce gap errors occur, the loadgen re-queries the chain nonce and continues sending rather than exhausting all retries with the stale nonce.

3. **NonceAlreadyInPool handling**: Verify that when a transaction is rejected with "nonce N already in pool", the loadgen counts it as a success and moves to the next nonce rather than retrying the same nonce.

4. **Post-run verification**: After any loadgen run, verify the final output includes an "Inclusion verification" log line comparing expected nonces against on-chain nonces for each account.

5. **Fallback on primary failure**: Stop the primary RPC validator mid-test. Verify that the loadgen continues sending via broadcast URLs rather than crashing.

6. **Graceful completion at high volume**: Run `loadgen --total-txs 10000 --accounts 30 --concurrency 50` (reduced concurrency to avoid overwhelming the chain). Verify the loadgen completes without hanging and the final summary shows reasonable success/failure counts.

7. **No nonce rewind regression**: Verify that the removal of the `if !succeeded { account.set_nonce(nonce); }` block does not cause nonce gaps on transient failures (the new code counts exhausted-retry transactions as "sent" so the outer loop does not re-attempt them).

## Impact

- **Operator experience**: Progress reporting transforms the loadgen from a black box into an observable tool. Operators can immediately tell whether the tool is making progress or stuck.
- **Test reliability**: Nonce resynchronization prevents the cascading failure mode where the loadgen races ahead of the chain and then wastes all retry attempts on doomed transactions.
- **Result accuracy**: Post-run verification catches silent transaction drops (like the 50% loss observed in multi-instance tests) that the current success counter misses entirely.
- **Fault tolerance**: RPC fallback during nonce initialization and error-classified retry prevent single-validator failures from crashing the entire load test.
- **Pool rejection handling**: Proper `NonceAlreadyInPool` handling prevents false failure counts when the pool correctly deduplicates transactions that were already accepted via broadcast.
