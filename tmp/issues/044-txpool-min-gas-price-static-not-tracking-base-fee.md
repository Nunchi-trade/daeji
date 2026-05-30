# 044: min_gas_price Is Static and Does Not Track Dynamic EIP-1559 Base Fee

**Category:** bug
**Severity:** medium

**Labels:** `bug`, `txpool`, `correctness`, `security`

## Summary

The transaction pool's minimum gas price is hardcoded as a static value of 1 gwei, matching `INITIAL_BASE_FEE`. The validator rejects any transaction whose effective gas price is below this threshold. However, the EIP-1559 base fee changes dynamically based on block gas utilization, and the pool's minimum does not follow these changes. This means the pool accepts transactions that can never be executed when the base fee rises above 1 gwei, wasting pool capacity and enabling a DoS vector.

## Problem

The pool config defines `min_gas_price` as a static constant of 1 gwei (1,000,000,000 wei) in its `Default` and `new()` implementations. The transaction validator compares each incoming transaction's effective gas price against this static value. Meanwhile, the EIP-1559 base fee is computed dynamically per block in `RevmApplication::compute_base_fee()` and can rise significantly above 1 gwei under sustained load.

The static minimum at `crates/node/txpool/src/config.rs:26-31` (in the `Default` impl):

```rust
fn default() -> Self {
    Self {
        max_pending_txs: 4096,
        max_queued_txs: 1024,
        max_txs_per_sender: 256,
        max_tx_size: 128 * 1024,      // 128 KB
        min_gas_price: 1_000_000_000, // 1 gwei, matches INITIAL_BASE_FEE
        replacement_bump_percent: 10,
        pending_ttl_secs: 30 * 60,
        queued_ttl_secs: 60 * 60,
    }
}
```

The validator check at `crates/node/txpool/src/validator.rs:86-91`:

```rust
let effective_gas_price = effective_gas_price(&envelope);
if effective_gas_price < self.config.min_gas_price {
    return Err(TxPoolError::GasPriceTooLow {
        price: effective_gas_price,
        min: self.config.min_gas_price,
    });
}
```

The dynamic base fee calculation happens in `crates/node/runner/src/app.rs:215-224`, using `calculate_base_fee()` from `kora_executor`, but this value is never fed back to the pool configuration.

## Impact

Under high gas utilization (the gas limit is 250M per block), the EIP-1559 base fee adjusts upward. When the base fee rises above 1 gwei:

1. **Pool pollution**: Transactions with `max_fee_per_gas` between 1 gwei and the current base fee are accepted into the pool but can never be included in blocks (their `max_fee_per_gas` is below the block's base fee). These transactions consume pool capacity -- up to 4096 pending slots.
2. **DoS vector**: An attacker can submit thousands of transactions at 1 gwei that permanently occupy pool slots, reducing the usable pool size for legitimate transactions. These transactions never expire from the pending pool unless the TTL (30 minutes) kicks in.
3. **Inverse problem**: If a future configuration lowers the initial base fee below 1 gwei, legitimate cheap transactions would be incorrectly rejected.

## Root Cause

The `min_gas_price` was set to match the initial base fee at chain genesis but was never connected to the dynamic base fee computation. The pool and the execution layer have no shared mechanism for communicating the current base fee.

## Suggested Fix

**Option 1 (recommended):** Update the pool's minimum gas price to track the current base fee. Add a method to update the validator's minimum on each new block:

```rust
// In TransactionPool or a wrapper:
pub fn update_base_fee(&self, current_base_fee: u64) {
    // Allow one block's max decrease (12.5%) to avoid rejecting
    // transactions that will be valid in the next block
    let min_acceptable = (current_base_fee as u128)
        .saturating_mul(875)
        .saturating_div(1000);
    self.min_gas_price.store(min_acceptable, Ordering::Relaxed);
}
```

Then in the validator:

```rust
if effective_gas_price < self.dynamic_min_gas_price() {
    return Err(TxPoolError::GasPriceTooLow {
        price: effective_gas_price,
        min: self.dynamic_min_gas_price(),
    });
}
```

**Option 2:** Filter out underpriced transactions at block-build time rather than at insertion time. Accept all transactions above an absolute floor (e.g., 1 wei) but skip those with `max_fee_per_gas < current_base_fee` during the block build loop. This simplifies validation but requires build-time access to the current base fee.

**Option 3:** Add a periodic pool maintenance task that evicts transactions whose `max_fee_per_gas` is below the current base fee, similar to the existing TTL-based eviction.

## Files to Modify

- `crates/node/txpool/src/config.rs` -- `min_gas_price` field (line 15, defaults at lines 31 and 47)
- `crates/node/txpool/src/validator.rs` -- comparison against static minimum (line 86)
- `crates/node/runner/src/app.rs` -- wire `compute_base_fee()` output to pool (line 215)
- `crates/node/runner/src/runner.rs` -- pass base fee updates to pool after block finalization

## Related Issues

- `041-txpool-in-memory-mempool-no-limits.md` -- InMemoryMempool has no gas price checks at all
- `042-txpool-eviction-cascades-to-high-fee-txs.md` -- Eviction strategy for when pool is full
- `040-txpool-no-periodic-cleanup-ttl-dead-code.md` -- TTL-based cleanup is not wired up
