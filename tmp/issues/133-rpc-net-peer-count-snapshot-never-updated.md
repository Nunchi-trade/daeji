# net_peerCount Returns Stale Value (Always 0) Because Peer Count Handle Is Never Wired

## Category
bug/spec -- rpc

## Severity
medium

## Summary
The `net_peerCount` RPC method always returns the initial peer count value (typically 0) because the `NetApiImpl` peer count handle is set once at server construction and never updated thereafter. The `peer_count_handle()` method returns an `Arc<AtomicU64>` that could be used for dynamic updates, but this handle is never exposed to or wired into the consensus engine's peer count tracking.

## Problem
The `RpcServer` (and `JsonRpcServer`) constructs a `NetApiImpl` during `start()` and sets its peer count to the value of `self.peer_count`, which defaults to 0:

**File:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 682-683:
```rust
let net_api = NetApiImpl::new(chain_id);
net_api.set_peer_count(peer_count);
```

The `peer_count` field defaults to 0 in all constructors (lines 436, 458, 487, 591, 820, 842) and can only be set via `with_peer_count()` at construction time:

```rust
pub const fn with_peer_count(mut self, peer_count: u64) -> Self {
    self.peer_count = peer_count;
    self
}
```

The `NetApiImpl` type (in `eth.rs`, lines 973-1001) has a `peer_count_handle()` method that returns an `Arc<AtomicU64>` for dynamic updates:

```rust
pub struct NetApiImpl {
    chain_id: u64,
    peer_count: Arc<std::sync::atomic::AtomicU64>,
}

impl NetApiImpl {
    pub fn new(chain_id: u64) -> Self {
        Self { chain_id, peer_count: Arc::new(std::sync::atomic::AtomicU64::new(0)) }
    }

    pub fn peer_count_handle(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.peer_count.clone()
    }

    pub fn set_peer_count(&self, count: u64) {
        self.peer_count.store(count, std::sync::atomic::Ordering::Relaxed);
    }
}
```

However, `start()` does not call `peer_count_handle()` or return it, and does not include it in any returned handle struct. The handle is created and immediately dropped after `set_peer_count(peer_count)`.

Meanwhile, `NodeState` (in `state.rs`) independently tracks `peer_count` via its own `AtomicU64` and is actively updated by the consensus engine. These two peer count values are completely disconnected.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 682-683 (RpcServer path):
```rust
let net_api = NetApiImpl::new(chain_id);
net_api.set_peer_count(peer_count);
```

`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs`, lines 968-969 (JsonRpcServer path, same pattern):
```rust
let net_api = NetApiImpl::new(self.chain_id);
net_api.set_peer_count(self.peer_count);
```

`/Users/will/dev/nunchi/daeji/crates/node/rpc/src/eth.rs`, lines 987-1001 (NetApiImpl with unused handle):
```rust
impl NetApiImpl {
    pub fn new(chain_id: u64) -> Self {
        Self { chain_id, peer_count: Arc::new(std::sync::atomic::AtomicU64::new(0)) }
    }

    pub fn peer_count_handle(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.peer_count.clone()
    }

    pub fn set_peer_count(&self, count: u64) {
        self.peer_count.store(count, std::sync::atomic::Ordering::Relaxed);
    }
}
```

## Impact
- **Misleading monitoring:** `net_peerCount` always returns `"0x0"`, which monitoring tools and wallets interpret as "no peers connected." This is indistinguishable from a genuinely partitioned node.
- **Combined with issue 128:** Since the `/health` endpoint also always returns 200, there is no RPC-level mechanism to observe whether a node has peer connectivity. Operators must use the non-standard `/status` endpoint or direct metrics scraping.
- **Spec violation:** The Ethereum JSON-RPC specification expects `net_peerCount` to return the current number of connected peers. Returning a constant 0 violates this expectation and breaks compatibility with standard tooling.

## Root Cause
The `peer_count_handle()` method on `NetApiImpl` was designed for exactly this purpose (dynamic updates via a shared atomic), but the wiring was never completed. The `RpcServer::start()` method creates the `NetApiImpl`, merges it into the JSON-RPC module, and discards any reference to the handle. No component calls `peer_count_handle()` or stores the result.

## Suggested Fix
1. **Capture and return the handle** from `RpcServer::start()`:

```rust
// In RpcServer::start():
let net_api = NetApiImpl::new(chain_id);
let peer_count_handle = net_api.peer_count_handle();
// ... register APIs ...

// Include in return value or pass to caller:
// Return peer_count_handle alongside the server handle
```

2. **Wire to NodeState updates** in the runner:

```rust
// In the runner, whenever NodeState.peer_count is updated:
rpc_peer_count_handle.store(new_peer_count, Ordering::Relaxed);
```

Alternatively, **remove the separate atomic** in `NetApiImpl` and have `net_peerCount` read directly from `NodeState`, which is already available via the `node_state` field on `EthApiImpl`.

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/server.rs` -- capture `peer_count_handle()` and return or wire it
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` -- update the RPC peer count handle when the consensus engine reports peer count changes

## Related Issues
- `128-rpc-health-endpoint-always-200.md` -- both findings relate to lack of runtime node-state visibility via RPC
- `129-rpc-eth-syncing-highest-block-equals-current.md` -- another RPC method that returns misleading state information

## Labels
bug, correctness, rpc, metrics, good first issue
