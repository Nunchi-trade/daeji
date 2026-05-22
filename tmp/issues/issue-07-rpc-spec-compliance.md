# RPC Spec Compliance: Block Tag Regression, logsBloom, gasUsed, and JSON-RPC Deviations

## Summary

Kora's JSON-RPC responses deviate from the [Ethereum JSON-RPC specification](https://ethereum.github.io/execution-apis/api-documentation/) in several ways that break compatibility with standard Ethereum tooling. Verified findings as of 2026-05-22:

1. **REGRESSION (PR #121):** `reject_historical_block` incorrectly rejects `safe` and `finalized` block tags, breaking clients that use these standard tags for state queries
2. **Block-level `logsBloom` serializes as `"0x"` instead of the full 256-byte zero bloom** -- breaks Foundry/alloy-rs deserialization
3. **Block-level `gasUsed` returns `0x0` for recovered blocks** even when they contain transactions
4. **Recovered blocks have no receipts or transactions indexed** -- `eth_getTransactionReceipt` returns `null` after restart

These are not execution bugs -- EVM execution via revm is correct -- but they prevent standard Ethereum clients from parsing Kora's RPC responses.

### Resolved from original report

- **`eth_subscribe` error code**: The original report claimed `eth_subscribe` was unimplemented and returned `-32603` over HTTP. This is no longer accurate. `eth_subscribe` is fully registered as a jsonrpsee subscription in `crates/node/rpc/src/subscription.rs` (lines 62-129), supporting `newPendingTransactions` over WebSocket. The subscription module is merged into both `RpcServer` and `JsonRpcServer` in `crates/node/rpc/src/server.rs` (lines 493-530 and 732-742). Over HTTP, jsonrpsee's framework-level handling of subscription methods is outside our control and is acceptable behavior.

### Already addressed by merged PRs

- **PR #120**: Block gas limit enforcement (merged)
- **PR #121**: Reject historical state queries (merged, but introduced the regression in finding 1)
- **PR #123**: BLOCKHASH opcode with recent block hashes (merged)

---

## Issue 1: REGRESSION -- `reject_historical_block` Rejects Safe and Finalized Tags

### Severity: HIGH (regression from PR #121)

### Problem

The `reject_historical_block` function introduced by PR #121 correctly rejects requests for historical state that QMDB cannot serve, but it incorrectly rejects `safe` and `finalized` block tags. In a single-finality BFT chain like Kora, both `safe` and `finalized` are semantically equivalent to `latest` and should resolve to the head block.

**File:** `crates/node/rpc/src/indexed_provider.rs`, lines 234-258

```rust
fn reject_historical_block(&self, block: &Option<BlockNumberOrTag>) -> Result<(), RpcError> {
    match block {
        None
        | Some(BlockNumberOrTag::Latest)
        | Some(BlockNumberOrTag::Tag(BlockTag::Latest | BlockTag::Pending)) => Ok(()),
        Some(BlockNumberOrTag::Number(n)) => {
            let head = self.index.head_block_number();
            let requested = n.to::<u64>();
            if requested == head {
                Ok(())
            } else if requested > head {
                Err(RpcError::InvalidBlockNumber(format!(
                    "block not yet available (requested {requested}, head {head})",
                )))
            } else {
                Err(RpcError::Unsupported(format!(
                    "historical state not available (block {requested})",
                )))
            }
        }
        // BUG: This catch-all matches Safe, Finalized, AND Earliest.
        // Safe and Finalized should be accepted (they mean "latest" in BFT).
        Some(BlockNumberOrTag::Tag(tag)) => {
            Err(RpcError::Unsupported(format!("historical state not available (tag {tag:?})",)))
        }
    }
}
```

The catch-all arm `Some(BlockNumberOrTag::Tag(tag))` matches `Safe`, `Finalized`, and `Earliest`. While rejecting `Earliest` is correct (it refers to genesis, which is truly historical state), rejecting `Safe` and `Finalized` is wrong.

Note that `resolve_block_number` (line 297) and `resolve_tag` (line 305) correctly map these tags to the head block number:

```rust
fn resolve_tag(&self, tag: BlockTag) -> Result<u64, RpcError> {
    match tag {
        BlockTag::Latest | BlockTag::Safe | BlockTag::Finalized | BlockTag::Pending => {
            Ok(self.index.head_block_number())
        }
        BlockTag::Earliest => Ok(0),
    }
}
```

So `reject_historical_block` rejects tags that the rest of the code would correctly handle. This creates an inconsistency: `eth_getBlockByNumber("finalized", false)` works (it uses `resolve_block_number` directly), but `eth_getBalance(addr, "finalized")` fails (it calls `reject_historical_block` first).

### Impact

- `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt`, `eth_call`, and `eth_estimateGas` all fail when called with `"safe"` or `"finalized"` block tags
- Many Ethereum tools and libraries (ethers.js, viem, Foundry) use `"finalized"` as the default block tag for safety-critical reads
- Hardhat defaults to `"latest"` but can be configured to use `"finalized"`
- This regression was introduced by PR #121 and does not affect block retrieval methods (`eth_getBlockByNumber`)

### Fix

Add `Safe` and `Finalized` to the accepted tags in `reject_historical_block`:

```rust
fn reject_historical_block(&self, block: &Option<BlockNumberOrTag>) -> Result<(), RpcError> {
    match block {
        None
        | Some(BlockNumberOrTag::Latest)
        | Some(BlockNumberOrTag::Tag(
            BlockTag::Latest | BlockTag::Pending | BlockTag::Safe | BlockTag::Finalized,
        )) => Ok(()),
        Some(BlockNumberOrTag::Number(n)) => {
            let head = self.index.head_block_number();
            let requested = n.to::<u64>();
            if requested == head {
                Ok(())
            } else if requested > head {
                Err(RpcError::InvalidBlockNumber(format!(
                    "block not yet available (requested {requested}, head {head})",
                )))
            } else {
                Err(RpcError::Unsupported(format!(
                    "historical state not available (block {requested})",
                )))
            }
        }
        // Only Earliest remains -- correctly rejected as historical.
        Some(BlockNumberOrTag::Tag(tag)) => {
            Err(RpcError::Unsupported(format!("historical state not available (tag {tag:?})",)))
        }
    }
}
```

**Files to modify:** `crates/node/rpc/src/indexed_provider.rs` (line 238)

---

## Issue 2: Block `logsBloom` Returns `"0x"` Instead of Full Zero Bloom

### Severity: HIGH

### Problem

The Ethereum JSON-RPC spec requires `logsBloom` to be a **256-byte (512 hex character) bloom filter**, always formatted as `"0x"` followed by exactly 512 hex characters. For blocks with no log-producing transactions, this should be `"0x"` followed by 512 zeros (514 characters total).

Kora returns `"0x"` (3 characters) -- an empty byte array -- because the block-to-RPC conversion uses `Bytes::new()`:

**File:** `crates/node/rpc/src/indexed_provider.rs`, line 280

```rust
fn indexed_block_to_rpc(&self, block: IndexedBlock, full_transactions: bool) -> RpcBlock {
    // ...
    RpcBlock {
        // ...
        logs_bloom: Bytes::new(),  // BUG: serializes as "0x" (empty)
        // ...
    }
}
```

The `RpcBlock` struct in `crates/node/rpc/src/types.rs` (line 66) declares `logs_bloom` as `Bytes`:

```rust
pub struct RpcBlock {
    // ...
    pub logs_bloom: Bytes,
    // ...
}
```

`Bytes` is `alloy_primitives::Bytes`, which serializes with a `"0x"` prefix. An empty `Bytes` serializes as just `"0x"`.

### Root Cause

The `IndexedBlock` struct in `crates/storage/indexer/src/types.rs` does **not** contain a `logs_bloom` field at all. Block-level bloom is never computed or stored. When the block is converted to an RPC response, an empty `Bytes` is used as a placeholder.

By contrast, **receipt-level** `logs_bloom` IS correctly computed. In `crates/node/reporters/src/lib.rs`, the per-receipt bloom is computed using `alloy_primitives::logs_bloom` and stored in `IndexedReceipt.logs_bloom` as a `Bloom` (256 bytes). So receipt blooms are correct; only the block-level bloom is broken.

### Impact

Foundry's `cast` tool (built on alloy-rs) fails to deserialize blocks:

```
ERROR alloy_provider::blocks: failed to fetch block number=3187
  err=deserialization error: invalid string length at line 1 column 798
```

alloy-rs expects `logsBloom` to be exactly 514 characters (`"0x"` + 512 hex chars). Receiving `"0x"` (3 characters) causes a strict deserialization failure. This breaks:

- `cast block` commands
- Any alloy-rs/ethers-rs client polling for blocks
- Block explorers that validate field lengths
- Transaction confirmation polling in Foundry workflows

### Spec Reference

From the [Ethereum JSON-RPC spec](https://ethereum.github.io/execution-apis/api-documentation/), `logsBloom` in block objects is defined as:

> `DATA`, 256 Bytes - the bloom filter for the logs of the block.

This is always 256 bytes (512 hex characters), zero-padded when no logs are present.

### Fix

**Option A (minimal):** Replace `Bytes::new()` with a 256-byte zero array in `indexed_block_to_rpc`:

```rust
// In crates/node/rpc/src/indexed_provider.rs, line 280
logs_bloom: Bytes::from(vec![0u8; 256]),
```

**Option B (correct, recommended):** Change the `logs_bloom` field type from `Bytes` to `alloy_primitives::Bloom` in both `RpcBlock` and `RpcTransactionReceipt` (in `crates/node/rpc/src/types.rs`). Add a `logs_bloom: Bloom` field to `IndexedBlock`, compute the block-level bloom as the bitwise OR of all receipt blooms during indexing, and use it in the RPC response:

```rust
// In crates/storage/indexer/src/types.rs, add to IndexedBlock:
pub logs_bloom: Bloom,

// In crates/node/reporters/src/lib.rs, index_finalized_block():
let block_logs_bloom = outcome.receipts
    .iter()
    .fold(Bloom::ZERO, |acc, receipt| acc | logs_bloom(receipt.logs()));

// In crates/node/rpc/src/types.rs:
pub logs_bloom: Bloom,  // was: Bytes (in both RpcBlock and RpcTransactionReceipt)

// In crates/node/rpc/src/indexed_provider.rs:
logs_bloom: block.logs_bloom,  // Bloom serializes correctly as 514-char hex
```

Option B is the spec-correct solution; the block bloom should be the union of all receipt blooms so that clients can quickly filter blocks by log topics without fetching individual receipts. Using `Bloom` as the type makes it structurally impossible to produce a truncated bloom.

**Files to modify:**
- `crates/node/rpc/src/types.rs` -- change `logs_bloom` type from `Bytes` to `Bloom`
- `crates/node/rpc/src/indexed_provider.rs` -- use `Bloom::ZERO` or computed bloom
- `crates/storage/indexer/src/types.rs` -- add `logs_bloom: Bloom` to `IndexedBlock`
- `crates/node/reporters/src/lib.rs` -- compute block-level bloom during indexing
- `crates/node/runner/src/runner.rs` -- use `Bloom::ZERO` in `seed_genesis_block_index` and `index_recovered_block`

---

## Issue 3: Block-Level `gasUsed` Is Zero for Recovered Blocks

### Severity: MEDIUM

### Problem

Block-level `gasUsed` is hardcoded to `0` in `index_recovered_block`, which is called for every block replayed from the archive after a node restart.

**File:** `crates/node/runner/src/runner.rs`, lines 141-160

```rust
fn index_recovered_block(
    index: &kora_indexer::BlockIndex,
    block: &Block,
    provider: &RevmContextProvider,
) {
    let block_context = provider.context(block);
    let transaction_hashes = block.txs.iter().map(|tx| keccak256(&tx.bytes)).collect();
    let indexed_block = kora_indexer::IndexedBlock {
        hash: block.id().0,
        number: block.height,
        parent_hash: block.parent.0,
        state_root: block.state_root.0,
        timestamp: block_context.header.timestamp,
        gas_limit: block_context.header.gas_limit,
        gas_used: 0,  // BUG: always 0, even for blocks with transactions
        base_fee_per_gas: block_context.header.base_fee_per_gas,
        transaction_hashes,
    };
    index.insert_block(indexed_block, Vec::new(), Vec::new());
}
```

Note that transaction hashes ARE indexed during recovery (unlike receipts), so `eth_getBlockByNumber` returns the correct transaction hash list. But `gas_used` is wrong, and passing `Vec::new()` for both transactions and receipts means full transaction objects and receipts are unavailable.

During normal finalization (`crates/node/reporters/src/lib.rs`), `gas_used` is correctly set from `outcome.gas_used`, which is the cumulative gas from `revm::ExecutionResult::tx_gas_used()`.

### Impact

- After a node restart, all previously-finalized blocks report `gasUsed: "0x0"` at the block level
- `eth_feeHistory` computes `gasUsedRatio` from block-level `gasUsed`, so it returns `0.0` for all recovered blocks
- EIP-1559 base fee calculations using `eth_feeHistory` data will be wrong
- Block explorers show 0% gas utilization for historical blocks
- `eth_getTransactionReceipt` returns `null` for all pre-restart transactions (no receipts indexed)
- `eth_getTransactionByHash` returns `null` for all pre-restart transactions (no full tx objects indexed)

### Fix

Store `gas_used` (and ideally receipts and full transaction data) alongside the block in the archive, so recovery can populate them without re-execution. The minimal approach is to add `gas_used` to the archived block metadata. A fuller fix would persist receipts and transaction data to enable complete post-restart query support.

**Files to modify:**
- `crates/node/runner/src/runner.rs` -- `index_recovered_block` function
- Block archive format (if adding persistent gas_used/receipt storage)

---

## Summary of All Verified Deviations

| # | Field / Issue | Location | Current Behavior | Spec Requirement | Severity |
|---|---------------|----------|-----------------|-----------------|----------|
| 1 | `safe`/`finalized` block tags rejected | `indexed_provider.rs:254` | Returns error for state queries | Should map to latest | **HIGH** |
| 2 | Block `logsBloom` | `indexed_provider.rs:280` | `"0x"` (empty) | `"0x0000...0000"` (514 chars) | **HIGH** |
| 3 | Block `gasUsed` (recovered) | `runner.rs:155` | Always `"0x0"` | Actual gas consumed | MEDIUM |
| 3 | Receipts (recovered) | `runner.rs:159` | Not indexed | Full receipt data | MEDIUM |
| 3 | Transactions (recovered) | `runner.rs:159` | Not indexed (hashes only) | Full tx objects | MEDIUM |
| -- | Block `transactionsRoot` | `indexed_provider.rs:278` | `B256::ZERO` | MPT root of tx list | LOW |
| -- | Block `receiptsRoot` | `indexed_provider.rs:279` | `B256::ZERO` | MPT root of receipts | LOW |
| -- | Block `size` | `indexed_provider.rs:292` | `U64::ZERO` | RLP-encoded block size | LOW |
| -- | Block `miner` | `indexed_provider.rs:288` | `Address::ZERO` | Block proposer address | LOW |

---

## Implementation Priority

### Phase 1: Fix the regression (small, high-impact)

1. **Fix `reject_historical_block`** to accept `Safe` and `Finalized` tags
   - File: `crates/node/rpc/src/indexed_provider.rs`
   - One-line change to the match arm at line 238

### Phase 2: Fix logsBloom (small, high-impact)

2. **Change `logs_bloom` type** from `Bytes` to `Bloom` in `RpcBlock` and `RpcTransactionReceipt`
   - File: `crates/node/rpc/src/types.rs`
3. **Use `Bloom::ZERO`** in `indexed_block_to_rpc` and update `indexed_receipt_to_rpc`
   - File: `crates/node/rpc/src/indexed_provider.rs`
4. **Add `logs_bloom: Bloom`** to `IndexedBlock` and compute it during finalization
   - Files: `crates/storage/indexer/src/types.rs`, `crates/node/reporters/src/lib.rs`
5. **Use `Bloom::ZERO`** in `seed_genesis_block_index` and `index_recovered_block`
   - File: `crates/node/runner/src/runner.rs`

### Phase 3: Recovery completeness (larger, medium-impact)

6. **Persist gas_used, receipts, and full transactions** in the archive or re-execute during recovery
   - Files: `crates/node/runner/src/runner.rs`, archive format, `crates/storage/indexer/`

---

## How to Reproduce

```bash
# Deploy devnet (see project README)

# 1. Verify logsBloom is truncated:
curl -s -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}' \
  | python3 -c "import sys,json; b=json.load(sys.stdin)['result']; print(f'logsBloom length: {len(b[\"logsBloom\"])} (expected 514)')"
# Output: logsBloom length: 3 (expected 514)

# 2. Verify Foundry fails to parse blocks:
cast block latest --rpc-url http://localhost:8545
# ERROR alloy_provider::blocks: failed to fetch block number=NNNN

# 3. Verify "finalized" tag is rejected for state queries:
curl -s -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","finalized"],"id":1}'
# Returns: {"error":{"code":-32602,"message":"unsupported: historical state not available (tag Finalized)"}}

# 4. Verify "finalized" tag DOES work for block retrieval (inconsistency):
curl -s -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["finalized",false],"id":1}'
# Returns: {"result": { ... block data ... }}
```
