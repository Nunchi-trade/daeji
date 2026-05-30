# Missing kora_peer_count Prometheus Gauge

**Category:** metrics, enhancement
**Severity:** medium

## Summary

The connected peer count is tracked internally by `NodeState::set_peer_count()` and is available via the `/status` JSON-RPC endpoint, but it is not registered as a Prometheus gauge. Peer count is the primary early-warning signal for network partitions: when peers drop below the BFT quorum threshold, consensus cannot progress. Without a Prometheus gauge, operators cannot configure threshold-based alerts for peer connectivity degradation.

## Problem

Kora is an EVM execution client using commonware simplex BFT consensus. Nodes maintain a P2P mesh and track the number of connected peers in the `NodeState` struct. The application already has a partition monitor (`spawn_partition_monitor` in `crates/node/runner/src/runner.rs`) that logs warnings when peer count drops, but this relies on log-based detection rather than Prometheus metric alerting.

The `NodeState` struct (`crates/node/rpc/src/state.rs`, lines 64-82) includes a `peer_count: AtomicU64` field (line 75) that is updated via `set_peer_count()` (line 170):

```rust
pub fn set_peer_count(&self, count: u64) {
    self.inner.peer_count.store(count, Ordering::Relaxed);
}
```

This value is returned in the `status()` method (line 227) and is used by the partition status computation (line 216: `PartitionStatus::from_peer_counts(peer_count, total_expected_peers)`).

However, the `AppMetrics` struct (`crates/node/metrics/src/lib.rs`) does not include a `peer_count` gauge. The `spawn_partition_monitor` function (lines 737-767) already reads the peer count and logs partition status, but log-based alerting is fragile and polling-based compared to Prometheus pull-based metric scraping.

## Code Reference

File: `crates/node/rpc/src/state.rs`, lines 170-172 (peer count stored but not metricsized):

```rust
    /// Update peer count.
    pub fn set_peer_count(&self, count: u64) {
        self.inner.peer_count.store(count, Ordering::Relaxed);
    }
```

File: `crates/node/rpc/src/state.rs`, lines 213-232 (peer count exposed via JSON /status endpoint):

```rust
    pub fn status(&self) -> NodeStatus {
        let peer_count = self.inner.peer_count.load(Ordering::Relaxed);
        let total_expected_peers = u64::from(self.inner.validator_count.get()).saturating_sub(1);
        let partition_status = PartitionStatus::from_peer_counts(peer_count, total_expected_peers);

        NodeStatus {
            // ...
            peer_count,
            total_expected_peers,
            partition_status,
            // ...
        }
    }
```

File: `crates/node/runner/src/runner.rs`, lines 737-767 (partition monitor uses peer count for log-based alerts):

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
                        "partition check: DEGRADED -- some peers missing but quorum still possible"
                    );
                }
                kora_rpc::PartitionStatus::Partitioned => {
                    error!(
                        peer_count = status.peer_count,
                        expected = status.total_expected_peers,
                        "partition check: PARTITIONED -- below quorum threshold, consensus cannot progress"
                    );
                }
            }
        }
    });
}
```

File: `crates/node/metrics/src/lib.rs`, lines 39-112 (the `AppMetrics` struct -- note the ABSENCE of a peer_count gauge):

```rust
pub struct AppMetrics {
    // Transaction Pool, Block Building, Proposal health, Finalization,
    // EVM Execution, RPC, Snapshot Store, Transaction Gossip, Equivocation
    // ... (no peer_count field)
}
```

## Impact

1. **No partition alerts**: Without a `kora_peer_count` gauge, Prometheus cannot be configured with a threshold alert like "peer_count < quorum_threshold for 60 seconds". This is the earliest possible signal that consensus is about to stall. By the time the partition is detected via log scraping or nullification rate, consensus may have already stopped.

2. **No mesh degradation correlation**: In Grafana dashboards, operators cannot correlate peer count drops with nullification spikes. For example, the sequence "peer count drops from 9 to 5 -> nullification rate spikes to 65%" was the signature of the `dialable_addr` bug on the 10-node devnet. Without peer count in Prometheus, this correlation requires manual log analysis.

3. **Redundant partition monitor**: The `spawn_partition_monitor` function essentially reimplements what Prometheus alerting rules would provide natively. With a Prometheus gauge, the partition monitor's log-based approach becomes redundant, and the alerting logic can be moved entirely to Prometheus/Alertmanager rules.

4. **Operational blind spot**: The `/metrics` endpoint includes detailed metrics for txpool, execution, gossip, and equivocation, but is missing the fundamental network health signal. An operator monitoring the Prometheus endpoint sees detailed application metrics but cannot determine if the node is even connected to its peers.

## Root Cause

The peer count was tracked in `NodeState` for the `/status` JSON endpoint and the partition monitor, but was not also registered as a Prometheus metric. The `AppMetrics` struct was built incrementally and this fundamental gauge was not included during initial instrumentation.

## Suggested Fix

Add a `kora_peer_count` gauge to `AppMetrics` and update it wherever `NodeState::set_peer_count()` is called.

**In `crates/node/metrics/src/lib.rs`**, add to the struct:

```rust
    // -- Network --
    /// Number of currently connected peers.
    pub peer_count: Gauge,
```

**In `AppMetrics::new()`**, initialize it:

```rust
    peer_count: Gauge::default(),
```

**In `AppMetrics::register()`**, register it:

```rust
    registry.register(
        "kora_peer_count",
        "Number of currently connected peers",
        self.peer_count.clone(),
    );
```

**Update the caller of `set_peer_count()`** to also update the Prometheus gauge. The peer count is updated from the runner's peer tracking callback. Add alongside the `NodeState::set_peer_count()` call:

```rust
node_state.set_peer_count(count);
metrics.peer_count.set(count as i64);
```

An alternative approach is to add the metrics update inside `NodeState::set_peer_count()` itself, by giving `NodeState` an optional reference to `AppMetrics`:

```rust
pub fn set_peer_count(&self, count: u64) {
    self.inner.peer_count.store(count, Ordering::Relaxed);
    if let Some(ref m) = self.inner.metrics {
        m.peer_count.set(count as i64);
    }
}
```

## Files to Modify

- `crates/node/metrics/src/lib.rs` -- add `peer_count: Gauge` to `AppMetrics` struct, `new()`, and `register()`
- `crates/node/runner/src/runner.rs` -- update the peer count callback to also set the Prometheus gauge (or alternatively, update `NodeState` to hold a metrics reference)

## Related Issues

- `059-metrics-missing-finalized-height-view-nullification.md` -- three other missing Prometheus metrics (finalized height, current view, nullification count)
- `058-docker-dialable-addr-not-set-4node-devnet.md` -- peer count metrics would have immediately revealed the partial mesh caused by the missing `dialable_addr`

## Labels

enhancement, metrics, p2p
