# P2P: Partition Monitor Reads Static Peer Count -- Always Reports Healthy Regardless of Actual Connectivity

**Category**: bug
**Severity**: medium

## Summary

The partition monitor is designed to detect network degradation by periodically checking the node's peer count against the expected number of peers. However, the `peer_count` value it reads from `NodeState` is set once at startup to the total expected number of peers (validator count minus 1) and is never updated with actual connectivity data. As a result, the monitor always reports "Healthy" regardless of whether the node has any active peer connections.

## Problem

Kora's `spawn_partition_monitor` task runs every 30 seconds and checks `node_state.status().peer_count` against `status.total_expected_peers` to determine network health. It classifies the result as Healthy, Degraded, or Partitioned.

The bug is that `peer_count` is set to a static expected value at startup and is never updated with actual connectivity information:

**Startup** -- `crates/node/runner/src/runner.rs`, line 1178:
```rust
let peer_count = self.scheme.participants().len().saturating_sub(1) as u64;
node_state.set_peer_count(peer_count);
```

This sets `peer_count` to `(number_of_validators - 1)`, which is exactly the expected count. Since the partition monitor compares `peer_count` against `total_expected_peers`, and they are always equal, the monitor always reports "Healthy."

The actual connected peer count would need to come from the transport oracle (which tracks active connections internally), but there is no API or wiring to read live connectivity data into `NodeState`.

## Code Reference

**Peer count set at startup** -- `crates/node/runner/src/runner.rs:1177-1179`:
```rust
if let Some((node_state, addr)) = &self.rpc_config {
    let peer_count = self.scheme.participants().len().saturating_sub(1) as u64;
    node_state.set_peer_count(peer_count);
```

**Partition monitor** -- `crates/node/runner/src/runner.rs:737-767`:
```rust
fn spawn_partition_monitor(node_state: kora_rpc::NodeState, context: cw_tokio::Context) {
    context.child("partition_monitor").shared(false).spawn(move |ctx| async move {
        loop {
            ctx.sleep(PARTITION_CHECK_INTERVAL).await;
            let status = node_state.status();
            match status.partition_status {
                kora_rpc::PartitionStatus::Healthy => {
                    trace!(
                        peer_count = status.peer_count,
                        expected = status.total_expected_peers,
                        "partition check: healthy"
                    );
                }
                kora_rpc::PartitionStatus::Degraded => {
                    warn!(
                        peer_count = status.peer_count,
                        expected = status.total_expected_peers,
                        "partition check: DEGRADED — some peers missing but quorum still possible"
                    );
                }
                kora_rpc::PartitionStatus::Partitioned => {
                    error!(
                        peer_count = status.peer_count,
                        expected = status.total_expected_peers,
                        "partition check: PARTITIONED — below quorum threshold, consensus cannot progress"
                    );
                }
            }
        }
    });
}
```

**Partition check interval** -- `crates/node/runner/src/runner.rs:76`:
```rust
const PARTITION_CHECK_INTERVAL: Duration = Duration::from_secs(30);
```

## Impact

1. **Silent network partition**: A node could lose connectivity to ALL peers and the partition monitor would continue reporting "Healthy." Operators relying on log-based alerting (or the RPC `/health` endpoint) would receive no warning.

2. **Delayed incident response**: In a network partition scenario, the node would produce 100% nullifications but operators would have no clear signal that the issue is connectivity-related. Diagnosis would require manually correlating nullification rate spikes with transport-level logs.

3. **False sense of security**: The partition monitor exists specifically to detect connectivity issues, but since it always reports healthy, it provides a false sense of security. Operators may believe connectivity is being monitored when it is not.

4. **RPC endpoint affected**: If the `/health` or `/status` RPC endpoint exposes `partition_status`, external monitoring systems would also see stale data.

## Root Cause

The `node_state.peer_count` field was designed to reflect actual connectivity but was only ever set to the static expected value at startup. The transport oracle tracks peer connections internally but there is no periodic polling or callback mechanism to update `NodeState` with live data.

## Suggested Fix

**Option A -- Periodic oracle query** (recommended):

Pass a reference to the transport oracle into the partition monitor and query it for the actual connected peer count each cycle:

```rust
fn spawn_partition_monitor(
    node_state: kora_rpc::NodeState,
    oracle: discovery::Oracle<ed25519::PublicKey>,
    context: cw_tokio::Context,
) {
    context.child("partition_monitor").shared(false).spawn(move |ctx| async move {
        loop {
            ctx.sleep(PARTITION_CHECK_INTERVAL).await;
            // Query oracle for actual connected peers
            let actual_peers = oracle.connected_peer_count() as u64;
            node_state.set_peer_count(actual_peers);
            let status = node_state.status();
            // ... existing match on partition_status ...
        }
    });
}
```

This requires the transport oracle to expose a `connected_peer_count()` method. If it does not exist, it would need to be added (the oracle already tracks connected peers internally).

**Option B -- Transport callback**:

Register a callback with the transport oracle that fires when peer count changes, updating `NodeState` immediately rather than polling.

**Option C -- Metric-based** (minimal change):

Instead of fixing the monitor, expose the actual connected peer count as a Prometheus gauge (see issue `060-metrics-missing-peer-count-gauge.md`) and rely on external monitoring. This is less ideal because it doesn't fix the log-based alerting.

## Files to Modify

- `crates/node/runner/src/runner.rs` -- update `spawn_partition_monitor` to accept oracle reference and query live data (lines 737-767)
- `crates/node/runner/src/runner.rs` -- update the call site to pass the oracle (near line 1178)
- Possibly `crates/network/transport/` -- add `connected_peer_count()` method to oracle if it does not exist

## Related Issues

- `060-metrics-missing-peer-count-gauge.md` -- complementary: no Prometheus gauge for peer count
- `163-p2p-network-handle-not-monitored.md` -- complementary: if transport dies, the partition monitor still reports healthy because it reads stale data
- `128-rpc-health-endpoint-always-200.md` -- health endpoint also does not reflect actual connectivity

## Labels

`bug`, `p2p`, `metrics`, `reliability`
