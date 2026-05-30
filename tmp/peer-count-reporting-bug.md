# Bug: `net_peerCount` Always Returns 0

## Summary

The `net_peerCount` JSON-RPC method always returns `0x0` on Kora nodes, even when the node is actively connected to peers and participating in consensus. This is a reporting bug -- actual P2P connectivity is unaffected.

---

## What is Kora?

Kora is a blockchain that uses a BFT consensus protocol among a fixed set of validators. It communicates over a P2P network built on the Commonware framework. The node exposes an Ethereum-compatible JSON-RPC interface, including the `net_peerCount` method that should report the number of connected peers.

---

## The Bug

### Symptom

```bash
$ curl -s http://localhost:8545 -X POST \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}'

{"jsonrpc":"2.0","result":"0x0","id":1}
```

All validators in a 4-node network report 0 peers, despite actively finalizing blocks (which requires P2P communication).

### Root Cause

There are **two separate peer count tracking systems** in the codebase, and one of them is never updated after initialization.

---

## System 1: `NetApiImpl` (used by `net_peerCount`)

**File:** `crates/node/rpc/src/eth.rs`, lines 423-466

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

impl NetApiServer for NetApiImpl {
    fn peer_count(&self) -> RpcResult<U64> {
        let count = self.peer_count.load(std::sync::atomic::Ordering::Relaxed);
        Ok(U64::from(count))
    }
}
```

The `AtomicU64` is initialized to `0`. It is set once at startup in the runner:

**File:** `crates/node/runner/src/runner.rs`, line 439

```rust
.with_peer_count(self.scheme.participants().len().saturating_sub(1) as u64);
```

This passes the initial peer count into the `RpcServer` builder, which then calls:

**File:** `crates/node/rpc/src/server.rs`, line 255

```rust
net_api.set_peer_count(peer_count);
```

### The Problem with System 1

The `peer_count` field on `RpcServer` defaults to `0`:

```rust
pub fn new(state: NodeState, addr: SocketAddr) -> Self {
    Self {
        // ...
        peer_count: 0,   // <-- default
    }
}
```

When `with_peer_count` is called in the runner (line 439), it sets this to `participants - 1` (e.g., 3 for a 4-validator network). This value is then passed to `NetApiImpl` at server start.

However, `peer_count_handle()` returns an `Arc<AtomicU64>` that **nobody holds a reference to after server startup**. The `NetApiImpl` is moved into the jsonrpsee `RpcModule`, and the handle is never stored or updated again. The count becomes a static snapshot set once at startup.

If `with_peer_count` is not called (e.g., in tests or when using `JsonRpcServer::new()` directly), the value remains 0 forever.

---

## System 2: `NodeState` (used by `kora_nodeStatus`)

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
    peer_count: AtomicU64,     // <-- initialized to 0
    is_leader: RwLock<bool>,
}
```

The `NodeState::set_peer_count` method exists (line 76):

```rust
pub fn set_peer_count(&self, count: u64) {
    self.inner.peer_count.store(count, Ordering::Relaxed);
}
```

But it is **never called** anywhere in production code. The `NodeStateReporter` (which bridges consensus events to `NodeState`) updates view, finalized count, and nullified count -- but not peer count:

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

No call to `self.state.set_peer_count(...)` appears here or anywhere else in the codebase.

---

## Impact

### Misleading Monitoring

The `kora_nodeStatus` endpoint reports `"peerCount": 0` on all nodes. This means:
- Dashboards show all nodes as isolated
- Alerting rules based on peer count fire false alarms
- During actual network partition events, the metric provides no signal (it was already 0)

### Cannot Detect Network Partitions via RPC

Since the value is always 0 regardless of connectivity, operators cannot use `net_peerCount` or `kora_nodeStatus.peerCount` to detect when a node has actually lost its peers. The metric is indistinguishable between "healthy" and "partitioned."

### Diagnostic Confusion

When investigating chain stalls, seeing `peers: 0` initially suggests a P2P connectivity problem. This is misleading and wastes investigation time.

---

## Proposed Fix

### Option A: Static Count from Validator Set (Minimal, ~5 lines)

Set the `NodeState` peer count at initialization, same as what `NetApiImpl` already does:

```rust
// In runner.rs, where node_state is created:
let peer_count = self.scheme.participants().len().saturating_sub(1) as u64;
node_state.set_peer_count(peer_count);
```

This gives a correct-at-startup count (e.g., 3 for a 4-validator network) but does not reflect runtime connectivity changes.

### Option B: Live Updates from P2P Layer (Better, ~15 lines)

Query the Commonware P2P oracle for actual connected peer count and update periodically:

```rust
// Spawn a background task:
let oracle = transport.oracle.clone();
let state = node_state.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        let connected = oracle.connected_peers().len() as u64;
        state.set_peer_count(connected);
    }
});
```

This provides real-time visibility into P2P health and makes the metric useful for partition detection.

### Option C: Unify the Two Systems

The `NetApiImpl` and `NodeState` both track peer count independently. They should share a single source of truth:

```rust
// Make NetApiImpl read from NodeState:
impl NetApiServer for NetApiImpl {
    fn peer_count(&self) -> RpcResult<U64> {
        // Read from shared NodeState instead of private AtomicU64
        Ok(U64::from(self.node_state.status().peer_count))
    }
}
```

This eliminates the dual-tracking confusion entirely.

---

## Effort Estimate

- Option A: ~5 lines, 5 minutes
- Option B: ~15 lines, 30 minutes (need to identify correct oracle API)
- Option C: ~20 lines refactor, 1 hour (touches trait boundaries)

---

## Related: Other Stub/Placeholder Values in RPC

The `net_peerCount` bug is part of a broader pattern where RPC methods return plausible but incorrect values:

| Method | Returns | Reality |
|--------|---------|---------|
| `net_peerCount` | `0x0` | Should be 3 (for 4-validator network) |
| `net_listening` | `true` always | Never changes, no actual check |
| `eth_syncing` | `false` always | No sync protocol implemented |
| `eth_gasPrice` | `1000000000` (1 gwei) | Hardcoded, no market pricing |
| `eth_maxPriorityFeePerGas` | `1000000000` (1 gwei) | Hardcoded |
| `eth_feeHistory` | Uniform values | No actual fee variance tracking |

These are not bugs per se (the chain currently works without dynamic fees or sync), but they represent areas where the RPC interface provides an incomplete picture of node state.

---

## Relevant Source Files

| File | Line(s) | Description |
|------|---------|-------------|
| `crates/node/rpc/src/eth.rs` | 423-466 | `NetApiImpl` with `peer_count` AtomicU64 |
| `crates/node/rpc/src/state.rs` | 29, 46, 76-78 | `NodeState.peer_count` field and setter |
| `crates/node/rpc/src/server.rs` | 179, 255 | `with_peer_count` builder and initial set |
| `crates/node/runner/src/runner.rs` | 439 | Where `with_peer_count` is called |
| `crates/node/reporters/src/lib.rs` | 453-475 | `NodeStateReporter` (missing peer count update) |
