# Block Timestamp Validation Allows Same Timestamp as Parent Block

**Category**: bug
**Severity**: medium
**Labels**: bug, correctness, executor

## Summary

The timestamp validation in `validate_header_against_parent` uses a strict less-than comparison (`<`), which only rejects timestamps that go backwards in time. This allows two consecutive blocks to have identical timestamps (`header.timestamp == parent.timestamp`). The Ethereum Yellow Paper (Section 4.3.4) requires that block timestamps must be strictly greater than the parent's timestamp, meaning the comparison should use `<=`.

## Problem

In `crates/node/executor/src/revm.rs`, the `validate_header_against_parent` method checks the block timestamp at line 86 using `<` instead of `<=`:

```rust
if header.timestamp < parent.timestamp {
```

This check correctly rejects timestamps that go backwards (e.g., parent=100, child=99), but it accepts timestamps that are equal (e.g., parent=100, child=100). Per the Ethereum specification, each block must have a strictly greater timestamp than its parent.

In Kora's consensus (Commonware simplex), blocks are proposed rapidly (33+ blocks/second on the 10-node devnet). At this rate, it is entirely possible for a proposer to generate a block with the same second-precision Unix timestamp as its parent, since multiple blocks can be produced within the same second.

## Code Reference

**File**: `crates/node/executor/src/revm.rs`, lines 86-91

```rust
if header.timestamp < parent.timestamp {
    return Err(ExecutionError::BlockValidation(format!(
        "timestamp moved backwards: parent {}, current {}",
        parent.timestamp, header.timestamp
    )));
}
```

## Impact

- **Time-dependent smart contracts**: Solidity contracts that use `block.timestamp` for time-sensitive logic (vesting schedules, auctions, Dutch auctions, time-locked governance proposals) assume timestamps strictly increase. Equal timestamps could cause these contracts to behave incorrectly -- for example, a Dutch auction price that should decrease over time would stall, or a time lock could expire one block early.
- **Base fee calculation edge cases**: While Kora's current EIP-1559 base fee implementation does not use the timestamp delta directly, some base fee algorithms use the time delta between blocks as an input. A zero delta could cause unexpected behavior or division by zero in future algorithm changes.
- **EVM opcode behavior**: The `TIMESTAMP` EVM opcode would return the same value for consecutive blocks, which breaks the assumption in many deployed contracts that each new block has a unique, increasing timestamp.
- **Block ordering ambiguity**: With equal timestamps, block ordering becomes ambiguous for any system that uses timestamps to determine recency. This could affect external indexers, block explorers, or analytics tools that consume Kora blocks.

## Root Cause

The comparison operator is `<` (strictly less than) instead of `<=` (less than or equal). The code rejects timestamps that move backwards but does not enforce the Ethereum-standard requirement that timestamps must strictly increase from one block to the next.

## Suggested Fix

Change the comparison from `<` to `<=`:

**Before:**
```rust
if header.timestamp < parent.timestamp {
    return Err(ExecutionError::BlockValidation(format!(
        "timestamp moved backwards: parent {}, current {}",
        parent.timestamp, header.timestamp
    )));
}
```

**After:**
```rust
if header.timestamp <= parent.timestamp {
    return Err(ExecutionError::BlockValidation(format!(
        "timestamp must be strictly greater than parent: parent {}, current {}",
        parent.timestamp, header.timestamp
    )));
}
```

Additionally, the block proposal logic (in the runner) should ensure it generates a timestamp that is at least `parent.timestamp + 1`. If the wall clock has the same second as the parent, the proposer should use `parent.timestamp + 1` as the minimum.

## Files to Modify

- `crates/node/executor/src/revm.rs` -- line 86, change `<` to `<=`
- `crates/node/runner/src/runner.rs` -- ensure the block proposer generates timestamps that are strictly greater than the parent's (may already be handled, but should be verified)

## Related Issues

- `066-perf-reduce-consensus-timeouts.md` -- consensus timing parameters that affect how quickly blocks are produced, which determines how often same-second timestamps could occur
