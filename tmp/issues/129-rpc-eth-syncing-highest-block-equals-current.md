# eth_syncing Reports highest_block Equal to current_block, Making Sync Progress Unobservable

## Category
bug/spec -- rpc

## Severity
medium

## Summary
The `eth_syncing` RPC method always sets `highest_block` equal to `current_block` in its response. Per the Ethereum JSON-RPC specification, `highest_block` should represent the highest block the node is aware of from peers, enabling callers to estimate sync progress and remaining time. The current implementation makes it impossible to distinguish a partially-synced node from a fully-synced one.

## Problem
The `syncing()` method in `EthApiImpl` checks whether the node is catching up (via `NodeState::is_catching_up()`). When the node is syncing, it constructs a `SyncInfo` struct where `highest_block` is set to the same value as `current_block`:

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 699-712

```rust
async fn syncing(&self) -> RpcResult<SyncStatus> {
    if let Some(ref state) = self.node_state
        && state.is_catching_up()
    {
        let current_block = self.current_block_number().await;
        Ok(SyncStatus::Syncing(SyncInfo {
            starting_block: U64::from(state.recovered_height()),
            current_block: U64::from(current_block),
            highest_block: U64::from(current_block),  // <-- always equals current_block
        }))
    } else {
        Ok(SyncStatus::NotSyncing(false))
    }
}
```

The `NodeState` type (in `crates/node/rpc/src/state.rs`) tracks `recovered_height` (the height at which the node was recovered from an archive) and `last_verified_height` (the latest fully-verified block), but does not track the network-reported highest block. The `current_block_number()` helper returns the local head, not the peer-reported chain tip.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 699-712:
```rust
async fn syncing(&self) -> RpcResult<SyncStatus> {
    if let Some(ref state) = self.node_state
        && state.is_catching_up()
    {
        let current_block = self.current_block_number().await;
        Ok(SyncStatus::Syncing(SyncInfo {
            starting_block: U64::from(state.recovered_height()),
            current_block: U64::from(current_block),
            highest_block: U64::from(current_block),
        }))
    } else {
        Ok(SyncStatus::NotSyncing(false))
    }
}
```

## Impact
- **Wallets and block explorers** (e.g., MetaMask, Etherscan) that call `eth_syncing` to display sync progress will show 100% complete even when the node is thousands of blocks behind, because `current == highest` always holds.
- **Monitoring tools** that compute sync ETA (`(highest - current) / blocks_per_second`) will calculate zero remaining time, masking the fact that the node is not yet caught up.
- **Deployment automation** that waits for sync completion before routing traffic will prematurely consider the node ready.

The Ethereum JSON-RPC specification (EIP-1474) defines `highest_block` as "the estimated highest block" and expects it to differ from `current_block` during active sync.

## Root Cause
The implementation lacks a mechanism to track the peer-reported or consensus-reported highest block number. The `NodeState` struct does not store a "network highest block" field. The `current_block` value (from `block_height` atomic) is used as a stand-in for `highest_block`, making the two fields redundant.

## Suggested Fix
1. Add a `network_highest_block` field to `NodeState` (or use the consensus engine's `current_view` as a proxy, since in Simplex BFT the view number corresponds to the block height).

2. Update `syncing()` to use this value:

```rust
async fn syncing(&self) -> RpcResult<SyncStatus> {
    if let Some(ref state) = self.node_state
        && state.is_catching_up()
    {
        let current_block = self.current_block_number().await;
        let highest_block = state.current_view(); // or state.network_highest_block()
        Ok(SyncStatus::Syncing(SyncInfo {
            starting_block: U64::from(state.recovered_height()),
            current_block: U64::from(current_block),
            highest_block: U64::from(highest_block),
        }))
    } else {
        Ok(SyncStatus::NotSyncing(false))
    }
}
```

3. Wire the consensus engine to update `network_highest_block` whenever it observes a higher view from peer messages.

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs` -- update `syncing()` to use a distinct `highest_block` value
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/state.rs` -- add `network_highest_block` or `current_view` field to `NodeState`
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- wire consensus view updates to the new `NodeState` field

## Related Issues
- `128-rpc-health-endpoint-always-200.md` -- another RPC endpoint that fails to reflect actual node state
- `133-rpc-net-peer-count-snapshot-never-updated.md` -- `net_peerCount` also returns stale data

## Labels
bug, correctness, rpc
