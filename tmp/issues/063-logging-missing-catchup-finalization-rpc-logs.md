# Missing Structured Logs for Catch-Up Completion, Finalization Timing, and RPC Submission Failures

**Category**: enhancement -- logging
**Severity**: medium

**Labels**: `enhancement`, `reliability`, `recovery`, `rpc`, `metrics`

---

## Summary

Three important operational events either have no log output or lack structured fields, making it difficult to monitor node health without Prometheus. Specifically: (1) there is no log when catch-up mode ends, (2) finalization timing is not logged as a single summary line, and (3) the RPC layer does not log when the transaction submission callback is not wired (causing silently dropped transactions).

---

## Problem

### 1. No Log When Catch-Up Mode Ends

The `is_catching_up()` method at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:452-467` transitions from `true` to `false` when `last_verified_height >= recovered_height + CATCH_UP_THRESHOLD` (where `CATCH_UP_THRESHOLD = 64`). There is a log at line 702-706 for "first full-execution verification past recovery point", but this fires when the first block past `recovered_height` is verified -- not when the catch-up window actually closes (which requires 64 additional blocks). Operators need to know the exact moment the node transitions from certificate-trust mode to full-verification mode.

### 2. No Finalization Timing Log

The `handle_finalized_update` function at `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:236-346` processes finalized blocks but does not log the wall-clock duration of the entire finalization pipeline for a single block. While individual sub-operations log timing (retry delays, etc.), there is no single log line with `finalize_duration_ms` covering the complete processing of a finalized block. This makes it difficult to diagnose slow finalization without Prometheus histograms.

### 3. Silent Transaction Drops When No Callback Is Wired

The `send_raw_transaction` method at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs:504-513` accepts transactions even when no `tx_submit` callback is configured. When the callback is `None`, the transaction is silently dropped -- no error is returned to the user, the hash is cached in `pending_txs` (making it look successful via `getTransactionByHash`), but the transaction never reaches the mempool or consensus pipeline. The existing test at line 2427 documents this as a known regression scenario but there is no warning log.

---

## Code Reference

**Catch-up transition** -- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:452-467`:
```rust
    fn is_catching_up(&self, block_height: u64) -> bool {
        let recovered = self.recovered_height.load(Ordering::Relaxed);
        if recovered == 0 {
            return false;
        }
        if block_height <= recovered {
            return false;
        }
        let verified = self.last_verified_height.load(Ordering::Relaxed);
        verified < recovered.saturating_add(CATCH_UP_THRESHOLD)
    }
```

There is a log when the first block past recovery is verified (line 699-706):
```rust
        if prev_verified < self.recovered_height.load(Ordering::Relaxed)
            && block.height >= self.recovered_height.load(Ordering::Relaxed)
        {
            info!(
                height = block.height,
                recovered_height = self.recovered_height.load(Ordering::Relaxed),
                "catch-up: first full-execution verification past recovery point"
            );
        }
```

But no log fires when `is_catching_up()` finally returns `false` (i.e., when `last_verified_height` reaches `recovered_height + 64`).

**Finalization with no timing** -- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:255-270`:
```rust
        Update::Block(block, ack) => {
            if let Some(ref ns) = node_state {
                ns.set_finalized_height(block.height);
            }
            let persist_checkpoint =
                checkpoint_interval <= 1 || block.height.is_multiple_of(checkpoint_interval);
            let result = finalize_with_retry(
                &state,
                &context,
                &executor,
                &provider,
                block_index.as_ref(),
                &block,
                persist_checkpoint,
            )
            .await;
            // No timing log for the entire finalization pipeline
```

**Silent tx drop** -- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs:504-513`:
```rust
    async fn send_raw_transaction(&self, data: Bytes) -> RpcResult<B256> {
        let tx_hash = alloy_primitives::keccak256(&data);
        let pending_tx = raw_tx_to_pending_rpc(&data)?;

        let accepted = if let Some(ref submit) = self.tx_submit {
            submit(data).await?;
            true
        } else {
            false  // Transaction silently dropped -- no warning, no error to caller
        };
```

---

## Impact

- **Without catch-up completion logging**: After a node restart, operators cannot determine when the node has finished recovery and is participating normally in consensus. They must poll the `/status` endpoint or watch metrics, which may not be configured on every deployment.
- **Without finalization timing**: Slow finalization caused by QMDB I/O contention, mutex congestion, or state root computation is invisible in logs. On a 10-node devnet sharing one NVMe, this is a likely operational concern. Operators must rely on Prometheus histograms, which may not be scraped or alerting.
- **Without RPC submission warning**: A misconfigured node (missing `tx_submit` callback wiring) silently drops every transaction submitted via `eth_sendRawTransaction`. The user sees a success response with a transaction hash, but the transaction never enters a block. This exact failure mode was observed on a previous devnet deployment.

---

## Root Cause

These logging gaps were not identified during incremental development. The catch-up transition and finalization timing were not treated as first-class observable events. The silent callback-missing case was discovered as a devnet regression but a warning log was not added.

---

## Suggested Fix

### Catch-up completion log

In `verify_block` (app.rs), after updating `last_verified_height`, check if catch-up just ended:

```rust
// After: self.last_verified_height.fetch_max(block.height, Ordering::Relaxed);
let recovered = self.recovered_height.load(Ordering::Relaxed);
if recovered > 0
    && prev_verified < recovered.saturating_add(CATCH_UP_THRESHOLD)
    && block.height >= recovered.saturating_add(CATCH_UP_THRESHOLD)
{
    info!(
        recovered_height = recovered,
        current_height = block.height,
        catch_up_threshold = CATCH_UP_THRESHOLD,
        "Node catch-up complete. Now requiring full execution for all blocks."
    );
}
```

### Finalization timing log

In `handle_finalized_update` (reporters/src/lib.rs), wrap the entire block processing:

```rust
Update::Block(block, ack) => {
    let finalize_start = Instant::now();
    // ... existing finalization logic ...
    info!(
        height = block.height,
        finalize_duration_ms = finalize_start.elapsed().as_millis(),
        "Block finalized"
    );
}
```

### RPC submission warning

In `send_raw_transaction` (eth.rs), log when no callback is wired:

```rust
let accepted = if let Some(ref submit) = self.tx_submit {
    submit(data).await?;
    true
} else {
    warn!(
        tx_hash = %tx_hash,
        "send_raw_transaction: no tx_submit callback wired -- \
         transaction accepted but will NOT be proposed to consensus"
    );
    false
};
```

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` -- add catch-up completion log in `verify_block`
- `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` -- add finalization timing log in `handle_finalized_update`
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs` -- add warning log in `send_raw_transaction` when callback is missing

---

## Related Issues

- `027-catch-up-mode-persists-indefinitely.md` -- catch-up mode behavior and transitions
- `061-metrics-missing-gas-persist-rpc-latency.md` -- related observability gaps in metrics
- `069-perf-pipeline-finalization-execution-persistence.md` -- finalization pipeline performance
