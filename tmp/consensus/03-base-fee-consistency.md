# 03 — Base Fee Consistency

## Problem

Blocks report `baseFeePerGas: "0x0"` but `eth_feeHistory` returns `1 gwei`
(`0x3b9aca00`). These should agree.

## Root Cause

Two independent code paths produce conflicting values:

**Path A — Block production** (`crates/node/runner/src/app.rs`):
When building a block, the `base_fee_per_gas` field comes from
`calculate_base_fee()` in `crates/node/executor/src/revm.rs:355-382`.
However, the genesis block (or the initial block state) may not have a
`base_fee_per_gas` set, so the first block inherits `None` or `0`, and since
all blocks have `gas_used = 0` (empty chain), the EIP-1559 formula keeps
reducing it toward zero:

```
parent_gas_used (0) < parent_gas_target → decrease base fee
```

With an empty chain, every block's `base_fee_per_gas` trends to zero over time.

**Path B — `eth_feeHistory`** (`crates/node/rpc/src/eth.rs:374-402`):
Hardcoded to return `1_000_000_000` (1 gwei) regardless of actual block data:

```rust
let base_fee = U256::from(1_000_000_000u64); // Hardcoded!
Ok(FeeHistory {
    base_fee_per_gas: vec![base_fee; count + 1],
    // ...
})
```

## Design

### Fix `eth_feeHistory` to Use Real Block Data

The `eth_feeHistory` implementation should read actual block data from the
`BlockIndex` instead of returning hardcoded values. This is the primary fix —
`eth_feeHistory` must reflect reality.

```rust
async fn fee_history(
    &self,
    block_count: U64,
    newest_block: BlockNumberOrTag,
    reward_percentiles: Option<Vec<f64>>,
) -> RpcResult<FeeHistory> {
    let provider = self.state_provider.read().await;
    let head = provider.block_number().await
        .unwrap_or_else(|_| self.block_height.load(Ordering::Relaxed));

    let newest = match newest_block {
        BlockNumberOrTag::Number(n) => n.to::<u64>().min(head),
        BlockNumberOrTag::Tag(_) | BlockNumberOrTag::Latest => head,
    };

    let requested = block_count.to::<u64>().min(1024);
    let count = requested.min(newest.saturating_add(1)) as usize;
    let oldest = newest.saturating_add(1).saturating_sub(count as u64);

    // Read actual base fees from indexed blocks
    let mut base_fees = Vec::with_capacity(count + 1);
    let mut gas_ratios = Vec::with_capacity(count);

    for i in 0..count {
        let block_num = oldest + i as u64;
        if let Some(block) = provider.block_by_number(
            BlockNumberOrTag::Number(U64::from(block_num)), false
        ).await.map_err(|e| /* ... */)? {
            let base_fee = block.base_fee_per_gas.unwrap_or(U256::ZERO);
            base_fees.push(base_fee);
            let gas_limit = block.gas_limit.to::<u64>();
            let gas_used = block.gas_used.to::<u64>();
            gas_ratios.push(if gas_limit > 0 { gas_used as f64 / gas_limit as f64 } else { 0.0 });
        } else {
            base_fees.push(U256::ZERO);
            gas_ratios.push(0.0);
        }
    }

    // Next block's predicted base fee (count + 1 entries)
    let next_base_fee = base_fees.last().copied().unwrap_or(U256::ZERO);
    base_fees.push(next_base_fee);

    // Rewards: for an empty chain, all percentiles get zero
    let rewards = reward_percentiles.map(|percentiles| {
        vec![vec![U256::ZERO; percentiles.len()]; count]
    });

    Ok(FeeHistory {
        base_fee_per_gas: base_fees,
        gas_used_ratio: gas_ratios,
        oldest_block: U64::from(oldest),
        reward: rewards,
    })
}
```

This requires `eth_feeHistory` to have access to the `StateProvider` (for
`block_by_number`). Currently it does — the `EthApiImpl` holds the
`state_provider` behind an `RwLock`.

### Fix Genesis Base Fee

Ensure the genesis block is initialized with `base_fee_per_gas: Some(1_000_000_000)`
(1 gwei). This is the standard Ethereum post-London initial base fee and
provides a reasonable starting point for the EIP-1559 formula.

This ties into workstream 01 (genesis indexing) — when constructing the genesis
`IndexedBlock`, set `base_fee_per_gas: Some(1_000_000_000)`.

If block production already reads `base_fee_per_gas` from the parent block's
indexed state, then having genesis set to 1 gwei will propagate correctly. On
an empty chain (0 gas used), the EIP-1559 formula will gradually decrease it,
which is correct behavior — it means gas is cheap when nobody's using the chain.

### Alternative Considered: Floor the Base Fee

One could add a minimum base fee floor (e.g., never go below 1 gwei). This is
a protocol-level decision and **not** part of this fix. The immediate issue is
that `eth_feeHistory` lies about the base fee. Making it truthful is the fix.
Whether the protocol should enforce a floor is a separate concern.

## Files to Change

| File | Change |
|------|--------|
| `crates/node/rpc/src/eth.rs` | Rewrite `fee_history()` to read real block data |
| `crates/node/runner/src/runner.rs` | Set genesis `base_fee_per_gas` to `Some(1_000_000_000)` (ties into 01) |

## Tests

1. **Unit test**: Create a `BlockIndex` with 5 blocks, each having different
   `base_fee_per_gas` values. Call `fee_history(5, "latest", [25, 75])`.
   Verify `base_fee_per_gas` array matches the actual block values (+ 1 for
   the predicted next block).

2. **Unit test**: Verify `gas_used_ratio` is computed correctly (e.g., a block
   with 50% gas usage returns 0.5).

3. **Unit test**: Verify that on an empty `BlockIndex`, `fee_history` returns
   a reasonable response (not an error).

## Verification

```bash
# After fix: baseFeePerGas in feeHistory should match the block's value
BLOCK_FEE=$(curl -s -X POST $RPC_URL -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}' \
  | jq -r '.result.baseFeePerGas')

HISTORY_FEE=$(curl -s -X POST $RPC_URL -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_feeHistory","params":["0x1","latest",[]],"id":1}' \
  | jq -r '.result.baseFeePerGas[-2]')

echo "Block: $BLOCK_FEE  History: $HISTORY_FEE"
# These should now match
```
