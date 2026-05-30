# 075: State Queries with Explicit Block Numbers Always Fail Due to Race Condition

**Category:** rpc / correctness
**Severity:** high

## Summary

State queries (`eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt`, `eth_call`, `eth_estimateGas`) work correctly with named block tags (`latest`, `safe`, `finalized`, `pending`) but always fail when an explicit block number is passed (e.g., `"0x5cbe3"`). The `reject_historical_block()` function uses strict equality against the current head block number, but at ~33 blocks/s the head has already advanced past the requested number by the time the check executes. This means any client that resolves `latest` to a number and passes it to a subsequent call will get an error.

## Problem

Kora is a minimal Ethereum-compatible execution client that stores only the latest state (no historical snapshots) using QMDB. The RPC layer includes a guard function `reject_historical_block()` that validates block number parameters before serving state queries. This function is intended to reject requests for historical state that Kora cannot serve, but its strict equality check makes it reject virtually *all* explicit block number requests.

**File:** `crates/node/rpc/src/indexed_provider.rs`, lines 281-307

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
            if requested == head {          // <-- strict equality: almost never true
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
        Some(BlockNumberOrTag::Tag(tag)) => {
            Err(RpcError::Unsupported(format!("historical state not available (tag {tag:?})",)))
        }
    }
}
```

This function is called from six RPC methods:
- `balance()` (line 84)
- `nonce()` (line 97)
- `code()` (line 110)
- `storage()` (line 135)
- `call()` (line 181)
- `estimate_gas()` (line 192)

Reproduction on the 10-node devnet (10 out of 10 attempts failed):

```bash
HEAD=$(curl -s -X POST http://127.0.0.1:8545 \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}' | jq -r '.result')

curl -s -X POST http://127.0.0.1:8545 \
  -H 'Content-Type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"eth_getBalance\",\"params\":[\"0xEb1Ba7Fc58b3416361a0EE07d140c91410c0AA8c\",\"$HEAD\"],\"id\":2}"
# => {"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"historical state not available (block 360690)"}}
```

At ~33 blocks/s, even a 1ms network round-trip between `eth_blockNumber` and the state query is enough for the head to advance, making `requested < head` and triggering the "historical state not available" error.

## Code Reference

**File:** `crates/node/rpc/src/indexed_provider.rs:281-307`
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
        Some(BlockNumberOrTag::Tag(tag)) => {
            Err(RpcError::Unsupported(format!("historical state not available (tag {tag:?})",)))
        }
    }
}
```

## Impact

Any client or tool that passes explicit block numbers to state queries will receive an error. This breaks:

- **Hardhat**: uses explicit block numbers during `hardhat_impersonateAccount` and test assertions
- **Foundry (cast/forge)**: `cast call --block <number>` fails
- **Block explorers** (Blockscout, Etherscan-like): query state at specific block heights for display
- **ethers.js / viem / web3.js**: libraries that resolve named tags client-side and pass the resulting number to subsequent calls for consistency
- **Multicall patterns**: first call gets block number, subsequent calls pass it to ensure consistent state across queries

All six state query methods are affected. Named tags (`latest`, `safe`, `finalized`, `pending`) work correctly.

## Root Cause

The `reject_historical_block()` function uses strict equality (`requested == head`) to determine if a block number request should be served. Since Kora only stores latest state, the intent was to allow only the current head block. However, at 33+ blocks/s, the head advances so fast that by the time the RPC handler reads the head block number, it has already moved past the block number the client requested moments earlier. The request falls into the `requested < head` branch and is rejected as "historical."

## Suggested Fix

Since QMDB only stores the latest state (no historical snapshots), accept any block number that is not in the future:

```rust
// Before:
Some(BlockNumberOrTag::Number(n)) => {
    let head = self.index.head_block_number();
    let requested = n.to::<u64>();
    if requested == head {
        Ok(())
    } else if requested > head {
        Err(RpcError::InvalidBlockNumber(...))
    } else {
        Err(RpcError::Unsupported(...))
    }
}

// After:
Some(BlockNumberOrTag::Number(n)) => {
    let head = self.index.head_block_number();
    let requested = n.to::<u64>();
    if requested <= head {
        Ok(())  // Serve latest state for any known block
    } else {
        Err(RpcError::InvalidBlockNumber(format!(
            "block not yet available (requested {requested}, head {head})",
        )))
    }
}
```

This matches the behavior of other nodes that only maintain latest state (pruned nodes, light clients). The semantics are: "we know this block exists, so we serve the best state we have (latest)." This is standard behavior and explicitly permitted by the Ethereum JSON-RPC specification for nodes without full historical state.

## Files to Modify

- `crates/node/rpc/src/indexed_provider.rs` (lines 281-307) -- `reject_historical_block()` strict equality check

## Related Issues

- `078-rpc-missing-standard-methods.md` -- other RPC method gaps

## Labels

bug, correctness, rpc
