# Block Timestamps and Gas Accounting -- Split Into Separate Issues

## Status: SPLIT

This issue was originally filed as a single combined issue. Audit review on 2026-05-22 determined it should be split into three independent issues with different complexity levels. Each sub-issue can be implemented and merged independently.

| Sub-Issue | Summary | Complexity | Dependencies |
|-----------|---------|------------|--------------|
| A | Timestamp drift: blocks run 107x slower than wall-clock | High (requires commonware changes) | None |
| B | Gas accounting disabled (base_fee = 0) | Medium | None |
| C | EIP-1559 dynamic base fee | Medium-High | Depends on B |

---

# Sub-Issue A: Block Timestamp Drift

## Summary

Block timestamps increment by exactly 1 second per block regardless of actual wall-clock time. At the current devnet throughput of ~107 blocks/second, the chain's internal clock runs 107x slower than reality. Any smart contract relying on `block.timestamp` will observe incorrect time.

## Root Cause

The timestamp logic in `crates/node/domain/src/block.rs` (lines 47-58) computes `max(now_secs, parent_timestamp + 1)`:

```rust
pub const fn next_timestamp(now_secs: u64, parent_timestamp: u64) -> Option<u64> {
    match parent_timestamp.checked_add(1) {
        Some(next) => {
            if now_secs > next {
                Some(now_secs)
            } else {
                Some(next)
            }
        }
        None => None,
    }
}
```

This function is called during block proposal in `crates/node/runner/src/app.rs` (lines 327-337):

```rust
let now_secs = unix_timestamp_secs(&env);
let timestamp = match Block::next_timestamp(now_secs, parent.timestamp) {
    Some(ts) => ts,
    None => { /* error: overflow */ return None; }
};
```

And in `crates/node/consensus/src/proposal.rs` (line 20).

The function itself is correct in principle -- it returns wall-clock time when the clock is ahead of the monotonic floor. The problem is that Simplex consensus produces blocks at ~107/sec, so `now_secs` (second-resolution) is always behind `parent_timestamp + 1`. The `now_secs > next` branch is never taken during normal operation.

### Why the Previously-Proposed "Option C" Fix Is a No-Op

The original issue recommended replacing `next_timestamp` with:

```rust
pub const fn next_timestamp(now_secs: u64, parent_timestamp: u64) -> Option<u64> {
    let floor = match parent_timestamp.checked_add(1) {
        Some(f) => f,
        None => return None,
    };
    if now_secs >= floor {
        Some(now_secs)
    } else {
        Some(floor)
    }
}
```

This is semantically identical to the current code. The only difference is `now_secs >= floor` vs `now_secs > next` where `floor == next`. When `now_secs == next`, both variants return the same value (`next` / `floor`). The rewrite does not change any behavior.

### Actual Fix Required

The real fix must happen at the **consensus level**: block production must be throttled so that blocks are not produced faster than 1 per second.

`crates/node/config/src/execution.rs` defines `DEFAULT_BLOCK_TIME: u64 = 2` (line 9), and `ExecutionConfig` has a `block_time` field (line 20), but this value is **not used by Simplex to pace block production**. Simplex advances views as fast as validators can propose and notarize, with no minimum block interval.

**Implementation path:**

1. Add a minimum block interval to the consensus/proposal phase. Before proposing, if `now_secs <= parent.timestamp`, the proposer should sleep until `parent.timestamp + block_time` seconds have elapsed.
2. This requires changes in commonware's Simplex implementation or in the `Application::propose` method (which already receives the runtime `Env` that implements `Clock`). The delay could be added at the start of `propose()` in `crates/node/runner/src/app.rs` (line 313).
3. Alternatively, commonware could expose a configurable minimum view duration.

**Side benefit:** throttling block production would also reduce the ~27% nullification rate observed on devnet, caused by consensus outpacing execution.

### Observed Impact

| Metric | Value |
|--------|-------|
| Block timestamp delta | Exactly 1 second between consecutive blocks |
| Actual wall-clock block time | ~9.3ms |
| Time drift factor | ~107x (chain time runs 107x slower than real time) |

After 5 minutes of real time, the chain's internal clock advances by ~32,000 seconds (~8.9 hours). Contracts using `block.timestamp` for timelocks, vesting, interest accrual, or oracle staleness checks will all produce incorrect results.

### Related: Non-Deterministic Genesis Hash

The genesis block timestamp is set from `BootstrapConfig.genesis_timestamp` in `crates/node/domain/src/bootstrap.rs` (line 38). The `new()` constructor defaults this to 0, and `load()` reads it from the genesis JSON file's `timestamp` field. As long as the genesis JSON uses a fixed timestamp, the genesis hash will be deterministic. However, if any deployment path sets the timestamp from wall-clock time, the genesis hash will differ between deployments. This should be documented as a deployment constraint.

---

# Sub-Issue B: Gas Accounting Disabled (base_fee = 0)

## Summary

`base_fee_per_gas` is hardcoded to `Some(0)` in all block construction paths. This means no ETH is ever deducted for gas, no fees are burned, and the chain has no economic mechanism to price block space. Enabling gas charging requires a migration strategy because existing clients and tooling assume free transactions.

## Current State

The `base_fee_per_gas: Some(0)` hardcoding appears in three places:

1. **`crates/node/runner/src/app.rs` line 81** -- `RevmApplication::block_context()`:
   ```rust
   fn block_context(&self, height: u64, timestamp: u64, prevrandao: B256) -> BlockContext {
       let header = Header {
           number: height,
           timestamp,
           gas_limit: self.gas_limit,
           beneficiary: Address::ZERO,
           base_fee_per_gas: Some(0),
           ..Default::default()
       };
       BlockContext::new(header, B256::ZERO, prevrandao)
   }
   ```

2. **`crates/node/consensus/src/proposal.rs` line 20**:
   ```rust
   base_fee_per_gas: Some(0),
   ```

3. **`crates/node/runner/src/runner.rs` line 304** -- `RevmContextProvider::context()`:
   ```rust
   impl BlockContextProvider for RevmContextProvider {
       fn context(&self, block: &Block) -> BlockContext {
           let header = Header {
               number: block.height,
               timestamp: block.timestamp,
               gas_limit: self.gas_limit,
               beneficiary: Address::ZERO,
               base_fee_per_gas: Some(0),
               ..Default::default()
           };
           // ...
       }
   }
   ```

Additionally, `beneficiary` is `Address::ZERO` in all three places, meaning even if fees were collected, priority fees would go to the zero address.

### What Actually Happens During Execution

The revm executor (`crates/node/executor/src/revm.rs`) sets `blk.basefee` to 0 (line 383):
```rust
blk.basefee = context.header.base_fee_per_gas.unwrap_or_default();
```

REVM still computes gas usage internally -- `result_and_state.result.tx_gas_used()` returns actual computational gas (line 424). Gas is tracked in receipts (line 428) and accumulated in `outcome.gas_used` (line 446). The block gas limit is enforced (lines 409-412, from PR #120). But because `basefee = 0`, the effective gas price is 0, so no ETH is deducted from senders.

### The Txpool Also Defaults to min_gas_price = 0

`crates/node/txpool/src/config.rs` line 31:
```rust
min_gas_price: 0,
```

The validator in `crates/node/txpool/src/validator.rs` (line 86) checks `effective_gas_price < self.config.min_gas_price`, but with `min_gas_price = 0`, all transactions pass regardless of gas price.

### RPC Gas Price Oracle Mismatch

The gas oracle in `crates/node/rpc/src/eth.rs` has a `default_base_fee()` fallback of 1 gwei (line 1068) and `min_price` / `min_priority_fee` both default to 1 gwei (lines 260-262). When no transactions exist in recent blocks, `eth_gasPrice` returns `base_fee + min_priority_fee = 0 + 1 gwei = 1 gwei`. Clients querying `eth_gasPrice` see 1 gwei, but transactions with `gasPrice = 0` are equally valid. This misleads clients about fee requirements.

### EIP-1559 Algorithm Exists But Is Never Engaged

The `calculate_base_fee()` function in `crates/node/executor/src/revm.rs` (lines 327-353) correctly implements the EIP-1559 base fee adjustment algorithm. It is used by `validate_base_fee()` (line 140) for block verification, but never for block construction. Since the initial base fee is 0 and blocks report 0 effective gas usage against the target, the base fee would remain at 0 forever even if the calculation were wired in.

## Migration Strategy

Enabling gas charging is a **breaking change** for all existing clients and tooling. The migration must be planned carefully:

### Phase 1: Add Configuration (Non-Breaking)
- Add `initial_base_fee` field to `ExecutionConfig` (or the node-level config), defaulting to 0.
- Add `fee_recipient` field for the beneficiary address, defaulting to `Address::ZERO`.
- Wire `min_gas_price` in `PoolConfig` to be configurable from the node config file.
- No behavior change yet.

### Phase 2: Announce and Coordinate
- Document the transition timeline. All clients and the loadgen (`bin/loadgen/src/main.rs`) must be updated to:
  - Query `eth_gasPrice` and use the returned value.
  - Set `maxFeePerGas` and `maxPriorityFeePerGas` on EIP-1559 transactions.
  - Ensure funded accounts have sufficient balance for gas costs.
- The loadgen currently sends transactions with `max_fee_per_gas: 0` and `max_priority_fee_per_gas: 0`. This will fail once gas is enforced.

### Phase 3: Enable Gas Charging
- Set `initial_base_fee` to a non-zero value (e.g., 1 gwei = `1_000_000_000`).
- Set `min_gas_price` in `PoolConfig` to match.
- Set `beneficiary` to the validator's address or a configured fee recipient.
- All three `base_fee_per_gas: Some(0)` sites must read from config instead of hardcoding.

### Phase 4: Genesis Considerations
- The genesis JSON should include an `initial_base_fee` field.
- Existing devnets with free-transaction history cannot retroactively charge gas. The base fee activates from the block where the config change takes effect.
- Genesis allocations must account for gas costs in funded account balances.

---

# Sub-Issue C: EIP-1559 Dynamic Base Fee

## Summary

Even after enabling a non-zero base fee (Sub-Issue B), the base fee will remain static unless it is dynamically computed from parent block utilization. This sub-issue tracks wiring the existing `calculate_base_fee()` into block construction.

## Depends On

Sub-Issue B (gas accounting must be enabled first with a non-zero initial base fee, otherwise the calculation always returns 0).

## Implementation

1. **Compute base fee dynamically during block construction**: Instead of hardcoding `Some(0)`, derive each block's base fee from the parent block's `base_fee_per_gas`, `gas_used`, and `gas_limit` using `calculate_base_fee()` (already implemented in `crates/node/executor/src/revm.rs` lines 327-353).

2. **Persist parent block metadata**: The `RevmContextProvider` in `crates/node/runner/src/runner.rs` (line 297) and `RevmApplication::block_context()` in `crates/node/runner/src/app.rs` (line 75) need access to the parent block's `gas_used` and `base_fee_per_gas`. Currently these are not carried in the `Block` struct or easily accessible. Options:
   - Read the parent block's indexed data from `BlockIndex` (which stores `gas_used` and `base_fee_per_gas` via `crates/node/reporters/src/lib.rs`).
   - Add `gas_used` and `base_fee_per_gas` fields to the `Block` struct itself.

3. **Base fee parameters**: `BaseFeeParams` is already defined in `crates/node/executor/src/config.rs` (lines 30-46) with Ethereum defaults: `elasticity_multiplier = 2`, `max_change_denominator = 8`. These should be made configurable from the node config.

4. **Validation already works**: The `validate_base_fee()` method (lines 140-166) already verifies block headers against the expected base fee. Once construction uses the same algorithm, validation will pass.

### Gas Oracle Alignment

Once the base fee is dynamic, the RPC gas oracle (`crates/node/rpc/src/eth.rs`) will naturally return accurate prices because it already samples `base_fee_per_gas` from indexed blocks (line 1004) and computes the next base fee for `eth_feeHistory` (line 1174, `calculate_next_base_fee`).

---

## Test Plan

### Sub-Issue A (Timestamps)
- [ ] Add a configurable minimum block interval to the proposal phase
- [ ] Verify that `block.timestamp` values track wall-clock time within the configured block interval
- [ ] Deploy a contract that reads `block.timestamp` and verify it matches `Date.now() / 1000` within tolerance
- [ ] Verify that the nullification rate decreases after throttling block production

### Sub-Issue B (Gas Accounting)
- [ ] Add `initial_base_fee` config field; verify default of 0 preserves existing behavior
- [ ] Set non-zero base fee; verify `baseFeePerGas` in block headers is non-zero
- [ ] Verify sender balances decrease after submitting transactions
- [ ] Verify `eth_gasPrice` returns a value consistent with the configured base fee
- [ ] Update the loadgen to use non-zero gas prices before enabling
- [ ] Verify block gas limit enforcement (PR #120) still works with real gas accounting

### Sub-Issue C (Dynamic Base Fee)
- [ ] Wire `calculate_base_fee()` into block construction
- [ ] Verify base fee increases when blocks are above 50% gas utilization
- [ ] Verify base fee decreases when blocks are below 50% gas utilization
- [ ] Verify `eth_feeHistory` returns accurate base fee predictions
- [ ] Run loadgen under sustained load and verify base fee adjusts appropriately
