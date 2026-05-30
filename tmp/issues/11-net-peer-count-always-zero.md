# RPC: kora_nodeStatus peerCount always returns 0; net_peerCount returns a static snapshot

## Summary

The `kora_nodeStatus` RPC always reports `"peerCount": 0` because the `NodeState.peer_count` field is never updated after initialization. Separately, `net_peerCount` returns a static value set once at startup (equal to `participants - 1`, e.g. 3 for a 4-validator network) but never reflects runtime connectivity changes. Both are **reporting bugs only** -- actual P2P connectivity and consensus are completely unaffected. The node communicates normally over its 5 multiplexed channels (votes, certs, resolver, blocks, backfill); only the reported peer counts are wrong.

**Severity:** Low-Medium

## Background

Kora is an EVM-compatible blockchain built on Commonware's consensus primitives. Its RPC server lives in `crates/node/rpc/` and exposes both standard Ethereum JSON-RPC methods (`eth_*`, `net_*`, `web3_*`) and Kora-specific methods (`kora_*`).

The `net_peerCount` method is part of the standard Ethereum `net_*` namespace. It should return the number of peers currently connected to the node. Ethereum tooling (wallets, block explorers, monitoring dashboards) uses this to assess node health and network connectivity.

Kora also exposes `kora_nodeStatus`, which returns a richer status object including a `peerCount` field. This is used by Kora's own monitoring and Grafana dashboards.

The `kora_nodeStatus` endpoint always returns `"peerCount": 0`. The `net_peerCount` endpoint returns a static value (e.g. `0x3`) set at startup but never updated to reflect actual peer connectivity.

## The Bug

There are **two separate peer count tracking systems** in the codebase, and neither one is properly updated at runtime.

### System 1: `NetApiImpl` (used by `net_peerCount`)

**File:** `crates/node/rpc/src/eth.rs`, lines 423-466

```rust
/// Net API implementation.
pub struct NetApiImpl {
    chain_id: u64,
    peer_count: Arc<std::sync::atomic::AtomicU64>,
}

impl NetApiImpl {
    /// Create a new Net API implementation.
    pub fn new(chain_id: u64) -> Self {
        Self { chain_id, peer_count: Arc::new(std::sync::atomic::AtomicU64::new(0)) }
    }

    /// Get a handle to update the peer count.
    pub fn peer_count_handle(&self) -> Arc<std::sync::atomic::AtomicU64> {
        self.peer_count.clone()
    }

    /// Update the peer count.
    pub fn set_peer_count(&self, count: u64) {
        self.peer_count.store(count, std::sync::atomic::Ordering::Relaxed);
    }
}

impl NetApiServer for NetApiImpl {
    fn listening(&self) -> RpcResult<bool> {
        Ok(true) // always returns true
    }

    fn peer_count(&self) -> RpcResult<U64> {
        let count = self.peer_count.load(std::sync::atomic::Ordering::Relaxed);
        Ok(U64::from(count))
    }
}
```

The `AtomicU64` is initialized to 0. At RPC server startup, `with_peer_count()` sets it once:

**File:** `crates/node/runner/src/runner.rs`, line 439

```rust
let rpc = kora_rpc::RpcServer::with_state_provider(
    node_state.clone(),
    *addr,
    self.chain_id,
    indexed_provider,
)
.with_tx_submit(tx_submit)
.with_peer_count(self.scheme.participants().len().saturating_sub(1) as u64);
drop(rpc.start());
```

This flows into `crates/node/rpc/src/server.rs` line 255:

```rust
let net_api = NetApiImpl::new(chain_id);
net_api.set_peer_count(peer_count);
```

The value is set to `participants - 1` (e.g., 3 for a 4-validator network) and then the `NetApiImpl` is consumed by `net_api.into_rpc()` on line 264, which moves it into the jsonrpsee `RpcModule`. The `peer_count_handle()` method exists to return an `Arc<AtomicU64>` clone, but **nobody calls `peer_count_handle()` or stores the returned Arc**. The value becomes a static snapshot frozen at startup time.

So `net_peerCount` returns a plausible value (participants - 1, e.g. 3 for a 4-validator network) in production because `with_peer_count()` is called. However, this is a **static snapshot** frozen at startup -- it never reflects actual runtime connectivity changes. If a peer disconnects, the count stays the same. If `with_peer_count()` is not called (e.g. when using `JsonRpcServer::new()` directly, as in tests), the value remains 0 forever.

There is also a standalone `JsonRpcServer` type (at `crates/node/rpc/src/server.rs`, lines 323-415) that has its own `with_peer_count` builder method (line 385) and `set_peer_count` call (line 403), following the same pattern.

### System 2: `NodeState` (used by `kora_nodeStatus`)

**File:** `crates/node/rpc/src/state.rs`, lines 21-31

```rust
struct NodeStateInner {
    chain_id: u64,
    validator_index: u32,
    started_at: Instant,
    current_view: AtomicU64,
    finalized_count: AtomicU64,
    proposed_count: AtomicU64,
    nullified_count: AtomicU64,
    peer_count: AtomicU64,       // <-- initialized to 0
    is_leader: RwLock<bool>,
}
```

The `peer_count` field is initialized to 0 in `NodeState::new()` (line 46). A setter exists at line 76:

```rust
/// Update peer count.
pub fn set_peer_count(&self, count: u64) {
    self.inner.peer_count.store(count, Ordering::Relaxed);
}
```

**This setter is NEVER CALLED anywhere in production code.** The only call is in the unit test at line 211-214.

The `NodeStateReporter` (which receives consensus activity updates) does NOT update the peer count:

**File:** `crates/node/reporters/src/lib.rs`, lines 453-475

```rust
impl<S> Reporter for NodeStateReporter<S>
where
    S: commonware_cryptography::certificate::Scheme + Clone + Send + 'static,
{
    type Activity = Activity<S, ConsensusDigest>;

    fn report(&mut self, activity: Self::Activity) -> impl std::future::Future<Output = ()> + Send {
        match &activity {
            Activity::Notarization(n) => {
                self.state.set_view(n.proposal.round.view().get());
            }
            Activity::Finalization(f) => {
                self.state.set_view(f.proposal.round.view().get());
                self.state.inc_finalized();
            }
            Activity::Nullification(_) => {
                self.state.inc_nullified();
            }
            _ => {}
        }
        async {}
    }
}
```

Notice: `set_view`, `inc_finalized`, and `inc_nullified` are all called, but there is no `self.state.set_peer_count(...)` call. The `peer_count` in `NodeState` remains 0 forever.

As a result, `kora_nodeStatus` always returns:

```json
{
  "peerCount": 0,
  ...
}
```

## Impact

1. **Dashboards show all nodes as isolated.** The Grafana dashboards (in `docker/grafana/dashboards/`) that display peer counts always show 0, making it look like every node is disconnected.
2. **False alerts.** Any monitoring that alerts on low peer count will fire constantly.
3. **Cannot detect actual network partitions.** Since the reported count is always 0, there is no way to distinguish a healthy node from one that has genuinely lost connectivity to all peers.
4. **Misleading to operators.** Someone checking node health via `curl` against the RPC will see 0 peers and may unnecessarily restart nodes or escalate incidents.

## Proposed Fixes

### Option A: Set static count from validator set on NodeState (5 lines, ~5 min)

Add one line in `crates/node/runner/src/runner.rs` inside the existing `if let Some((node_state, addr)) = &self.rpc_config` block (line 395), right after line 440 (`drop(rpc.start())`):

```rust
// After drop(rpc.start()) on line 440, still inside the existing if-let block:
node_state.set_peer_count(self.scheme.participants().len().saturating_sub(1) as u64);
```

This gives `kora_nodeStatus` the same static count that `net_peerCount` already has. It does not reflect runtime connectivity changes, but it is accurate for a permissioned validator set that is expected to be fully connected.

### Option B: Live updates from P2P layer (~15 lines, ~30 min)

Spawn a background task in `runner.rs` (near line 440, after the RPC server is started) that periodically queries the transport oracle for the actual connected peer count. The oracle is a `discovery::Oracle<P>` stored on the `NetworkTransport` struct (defined at `crates/network/transport/src/transport.rs`, line 28):

```rust
/// Oracle for peer management and Byzantine blocking.
pub oracle: discovery::Oracle<P>,
```

The oracle is already cloned and used throughout `runner.rs` (e.g. line 337 for `transport.oracle.track()`, line 495-496 for resolver init, line 503 for broadcast init). Add the background task inside the existing `if let Some((node_state, addr)) = &self.rpc_config` block (line 395), after line 440 (`drop(rpc.start())`):

```rust
// After drop(rpc.start()) on line 440, still inside the existing if-let block:
let state = node_state.clone();
let oracle = transport.oracle.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        // Query the Commonware oracle for current connected peer count
        let connected = oracle.connected_peers().len() as u64;
        state.set_peer_count(connected);
    }
});
```

Note: You will need to verify the exact Commonware oracle API for querying connected peers. The `discovery::Oracle<P>` implements `commonware_p2p::Manager` and `commonware_p2p::Blocker` traits. Check the `TrackedPeers` and `Manager` trait documentation for the correct method name (it may be `peers()`, `connected()`, or similar rather than `connected_peers()`).

This gives a live, accurate peer count that reflects actual network conditions. It also requires making `NetApiImpl` read from the same source (see Option C).

### Option C: Unify both systems (~20 lines, ~1 hr)

Eliminate the separate `AtomicU64` in `NetApiImpl` and make it read from `NodeState` instead. This way both `net_peerCount` and `kora_nodeStatus` return the same value from a single source of truth, updated by the background task from Option B.

This involves:
1. Adding `NodeState` as a field on `NetApiImpl`
2. Having `peer_count()` read from `NodeState::status().peer_count`
3. Removing the standalone `AtomicU64` and `peer_count_handle()` method
4. Implementing the background update task from Option B

**Recommendation:** Option B is the best balance of effort and correctness. Option A is a reasonable stopgap if time is limited.

## Related: Other Stub Values in RPC

The peer count issue is part of a broader pattern of hardcoded/stub RPC responses:

| Method | Current behavior | File | Line |
|--------|-----------------|------|------|
| `net_listening` | Always returns `true` | `crates/node/rpc/src/eth.rs` | 459-461 |
| `eth_syncing` | Always returns `false` | `crates/node/rpc/src/eth.rs` | 412-413 |
| `eth_gasPrice` | Hardcoded `1_000_000_000` (1 gwei) | `crates/node/rpc/src/eth.rs` | 366-367 |
| `eth_maxPriorityFeePerGas` | Hardcoded `1_000_000_000` (1 gwei) | `crates/node/rpc/src/eth.rs` | 370-371 |
| `eth_feeHistory` | Uniform base fee, zero gas usage ratios | `crates/node/rpc/src/eth.rs` | 374-401 |

These are less impactful than the peer count issue since Kora does not currently have syncing or dynamic gas pricing, but they are worth noting for future work.

## Testing Plan

1. **Unit test for NodeState peer count update:** Already exists at `crates/node/rpc/src/state.rs` line 211 (`node_state_set_peer_count`). Verify it still passes after changes.

2. **Integration test:** Start a multi-validator devnet and query both RPC endpoints:
   ```bash
   # Check net_peerCount
   curl -s -X POST http://127.0.0.1:8545 \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}'
   # Expected: {"result":"0x3"} for a 4-validator network

   # Check kora_nodeStatus
   curl -s -X POST http://127.0.0.1:8545 \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}'
   # Expected: "peerCount": 3
   ```

3. **Peer disconnect test (for Option B/C):** Stop one validator and verify the peer count on remaining validators decreases after the polling interval (5 seconds).

4. **Dashboard verification:** Check that Grafana panels displaying peer count show non-zero values after the fix.

## Verification Steps (Post-Implementation)

After implementing a fix, run through these checks to confirm correctness:

1. **Compile check:** `cargo build` passes with no errors in `kora_rpc`, `kora_runner`, and `kora_reporters` crates.

2. **Existing unit tests pass:**
   ```bash
   cargo test -p kora-rpc -- node_state_set_peer_count
   cargo test -p kora-rpc -- node_status_serde_roundtrip
   cargo test -p kora-rpc -- node_status_json_uses_camel_case
   ```

3. **Verify `set_peer_count` is called in production code:**
   ```bash
   # Should show at least one call site outside of test code
   grep -rn 'set_peer_count' crates/node/runner/src/ crates/node/reporters/src/
   ```

4. **Start devnet and verify both endpoints return non-zero values:**
   ```bash
   # After ~10 seconds for peers to connect:
   curl -s -X POST http://127.0.0.1:8545 \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}'
   # Expected: {"result":"0x3"} (not "0x0")

   curl -s -X POST http://127.0.0.1:8545 \
     -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","method":"kora_nodeStatus","params":[],"id":1}'
   # Expected: "peerCount":3 (not 0)

   # Also check the HTTP status endpoint:
   curl -s http://127.0.0.1:8546/status | jq .peerCount
   # Expected: 3 (not 0)
   ```

5. **For Option B/C -- verify dynamic updates:** Stop one validator and re-query after 5+ seconds. The peer count should decrease by 1.

## Files Referenced

| File | Lines | Description |
|------|-------|-------------|
| `crates/node/rpc/src/eth.rs` | 422-467 | `NetApiImpl` struct, methods, and `NetApiServer` impl with `peer_count` `AtomicU64` |
| `crates/node/rpc/src/state.rs` | 21-31 | `NodeStateInner` struct with `peer_count: AtomicU64` field (line 29) |
| `crates/node/rpc/src/state.rs` | 36-50 | `NodeState::new()` -- initializes `peer_count` to 0 (line 46) |
| `crates/node/rpc/src/state.rs` | 76-78 | `NodeState::set_peer_count()` -- the setter that is never called in production |
| `crates/node/rpc/src/state.rs` | 211-214 | `node_state_set_peer_count` unit test -- only place `set_peer_count` is called |
| `crates/node/rpc/src/server.rs` | 72-82 | `RpcServer` struct with `peer_count: u64` field (line 81) |
| `crates/node/rpc/src/server.rs` | 179-182 | `RpcServer::with_peer_count()` builder method |
| `crates/node/rpc/src/server.rs` | 254-255 | Where `NetApiImpl::set_peer_count()` is called in `RpcServer::start()` |
| `crates/node/rpc/src/server.rs` | 264 | Where `net_api.into_rpc()` consumes the `NetApiImpl` |
| `crates/node/rpc/src/server.rs` | 323-415 | `JsonRpcServer` -- standalone server with its own `with_peer_count` (line 385) and `set_peer_count` (line 403) |
| `crates/node/runner/src/runner.rs` | 432-440 | Where `with_peer_count` is called on the `RpcServer` builder at startup |
| `crates/node/reporters/src/lib.rs` | 426-475 | `NodeStateReporter` -- updates view, finalized, nullified but NOT peer count |
| `crates/node/rpc/src/kora.rs` | 34-37 | `KoraApiImpl::node_status()` returns `self.state.status()` (which includes `peer_count: 0`) |
| `crates/network/transport/src/transport.rs` | 23-40 | `NetworkTransport` struct with `oracle: discovery::Oracle<P>` for peer management |
