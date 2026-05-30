# BLOCKHASH Opcode Returns Zero During Block Proposal and Verification

**Category**: Bug
**Severity**: High
**Labels**: `bug`, `correctness`, `executor`, `consensus`

## Summary

During block proposal and verification, the EVM's `BLOCKHASH` opcode always returns `B256::ZERO` because the `block_context()` method in `RevmApplication` does not populate recent block hashes. In contrast, the finalization replay path correctly populates them via `RevmContextProvider::context()`. This inconsistency means any smart contract using `BLOCKHASH` will produce different state roots between consensus (proposal/verification) and finalization, triggering a fatal `StateRootMismatch` error that aborts the node process.

## Problem

The Kora node executes each block in multiple contexts. The `BlockContext` struct holds a `recent_block_hashes` map that the EVM uses to service `BLOCKHASH(N)` queries. There are two code paths that construct a `BlockContext`:

1. **Proposal and verification path**: `RevmApplication::block_context()` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:234-251` creates a `BlockContext` but never calls `.with_recent_block_hashes()`. The `recent_block_hashes` map is left empty (default from `BlockContext::new()`), so all `BLOCKHASH(N)` queries return `B256::ZERO`.

2. **Finalization replay path**: `RevmContextProvider::context()` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs:631-664` correctly calls `self.recent_block_hashes(block.height)` and chains `.with_recent_block_hashes(recent_hashes)`.

Since proposal and verification both use the same broken path, they agree with each other (both produce zero hashes), so blocks pass consensus. However, when the `FinalizedReporter` re-executes the block for RPC indexing using the correct path (with real block hashes), it produces a different state root. The `finalize_block` function at `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:540-544` detects this as a `FinalizationError::StateRootMismatch`, which is classified as non-retryable and causes the node to abort.

## Code Reference

**Broken path -- proposal/verification** (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:234-251`):

```rust
fn block_context(
    &self,
    height: u64,
    timestamp: u64,
    prevrandao: B256,
    parent_digest: ConsensusDigest,
) -> BlockContext {
    let base_fee = self.compute_base_fee(parent_digest);
    let header = Header {
        number: height,
        timestamp,
        gas_limit: self.gas_limit,
        beneficiary: self.fee_recipient,
        base_fee_per_gas: Some(base_fee),
        ..Default::default()
    };
    BlockContext::new(header, B256::ZERO, prevrandao)
    // Missing: .with_recent_block_hashes(...)
}
```

**Correct path -- finalization** (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs:631-664`):

```rust
impl BlockContextProvider for RevmContextProvider {
    fn context(&self, block: &Block) -> BlockContext {
        // ... base fee computation ...
        let header = Header {
            number: block.height,
            timestamp: block.timestamp,
            gas_limit: self.gas_limit,
            beneficiary: self.fee_recipient,
            base_fee_per_gas: Some(base_fee),
            ..Default::default()
        };
        let recent_hashes = self.recent_block_hashes(block.height);
        BlockContext::new(header, B256::ZERO, block.prevrandao)
            .with_recent_block_hashes(recent_hashes)  // <-- present here
    }
}
```

**BlockContext with_recent_block_hashes** (`/Users/will/dev/nunchi/daeji/crates/node/executor/src/context.rs:49-60`):

```rust
pub fn with_recent_block_hashes(mut self, hashes: HashMap<u64, B256>) -> Self {
    if hashes.len() > MAX_BLOCK_HASHES {
        self.recent_block_hashes = hashes.into_iter().take(MAX_BLOCK_HASHES).collect();
    } else {
        self.recent_block_hashes = hashes;
    }
    self
}
```

**StateRootMismatch triggers abort** (`/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs:540-544`):

```rust
if state_root != block.state_root {
    return Err(FinalizationError::StateRootMismatch {
        expected: block.state_root,
        computed: state_root,
    });
}
```

## Impact

Any smart contract that uses the `BLOCKHASH` opcode will trigger this bug:
- Randomness contracts using `blockhash(block.number - 1)` as a source of entropy
- Cross-contract verification schemes that validate block hashes
- Any Solidity code using the built-in `blockhash()` function
- Standard DeFi protocols (many use block hashes for pseudo-randomness)

The block will pass consensus (all validators produce the same zero-hash result during proposal/verification), but the finalization pipeline will detect a state root mismatch and abort the node process. This is a hard crash -- the node will restart, hit the same block, and crash again, entering a crash-loop.

Currently no contracts on the devnet use `BLOCKHASH`, so this has not been triggered. However, deploying any standard DeFi or randomness contract will immediately surface this bug.

## Root Cause

The `RevmApplication::block_context()` method was written without access to a `BlockIndex` reference, which is needed to look up recent block hashes. The `RevmApplication` struct does not hold a `block_index` field. The finalization path (`RevmContextProvider`) was written later with its own `BlockContext` construction that correctly includes the block index, but the proposal/verification path was never updated to match.

## Suggested Fix

Add a `block_index: Arc<BlockIndex>` field to the `RevmApplication` struct and populate `recent_block_hashes` in `block_context()`:

```rust
// In RevmApplication struct (app.rs):
pub struct RevmApplication<S, E> {
    // ... existing fields ...
    block_index: Arc<BlockIndex>,  // NEW
}

// In block_context():
fn block_context(
    &self,
    height: u64,
    timestamp: u64,
    prevrandao: B256,
    parent_digest: ConsensusDigest,
) -> BlockContext {
    let base_fee = self.compute_base_fee(parent_digest);
    let header = Header {
        number: height,
        timestamp,
        gas_limit: self.gas_limit,
        beneficiary: self.fee_recipient,
        base_fee_per_gas: Some(base_fee),
        ..Default::default()
    };
    let recent_hashes = self.block_index.recent_block_hashes(height);
    BlockContext::new(header, B256::ZERO, prevrandao)
        .with_recent_block_hashes(recent_hashes)  // FIX
}
```

The `recent_block_hashes()` method on `BlockIndex` already caps output at 256 entries (the EVM BLOCKHASH depth limit), so no additional bounds checking is needed.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (line 90) -- `RevmApplication` struct needs `block_index: Arc<BlockIndex>` field
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` (lines 234-251) -- `block_context()` needs `.with_recent_block_hashes()`
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- constructor call for `RevmApplication::new()` needs to pass the block index

## Related Issues

- `019-dual-base-fee-paths-diverge.md` -- another proposal/finalization BlockContext inconsistency (base fee)
- `014-triple-block-execution.md` -- finalization re-execution is where the mismatch is detected
- `081-executor-block-hash-truncation-ordering.md` -- related block hash handling issue
