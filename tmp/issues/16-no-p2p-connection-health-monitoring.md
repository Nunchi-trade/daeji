# P2P: No connection health monitoring or per-peer metrics

**Severity:** Medium
**Component:** `crates/network/transport/`, Commonware P2P layer
**Labels:** observability, networking, p2p

## Summary

Kora has no heartbeat, liveness check, or connection-state metrics for the P2P layer. Operators have zero visibility into per-peer network health until consensus-level symptoms appear (leader timeouts, nullifications, stalls). During chain stalls, operators cannot distinguish between network partition issues and consensus bugs without manually digging through logs.

## Background

Kora's P2P stack is built on Commonware's `authenticated::discovery` module. The transport is configured in `crates/network/transport/src/builder.rs` and creates a single authenticated discovery network with 5 multiplexed channels. The validator set is static (no dynamic membership changes) -- all 4 validators and any secondary peers are registered with the oracle at startup in `crates/node/runner/src/runner.rs:337`:

```rust
// crates/node/runner/src/runner.rs:334-337
let validators = self.scheme.participants().clone();
let secondary = Set::from_iter_dedup(self.secondary_peers.iter().cloned());
let secondary_count = secondary.len();
transport.oracle.track(0, TrackedPeers::new(validators, secondary)).await;
```

The oracle uses epoch 0 (single epoch, never changes -- `EPOCH_LENGTH = u64::MAX` at line 55). All peers are identified by ed25519 public keys. The oracle maintains connection state internally but does not expose per-peer connection status via any public API or metric.

### Channel Layout

The 5 channels are defined in `crates/network/transport/src/channels.rs:9-22`:

```rust
/// Channel ID for vote messages.
pub const CHANNEL_VOTES: u64 = 0;

/// Channel ID for certificate messages.
pub const CHANNEL_CERTS: u64 = 1;

/// Channel ID for resolver messages.
pub const CHANNEL_RESOLVER: u64 = 2;

/// Channel ID for block broadcast messages.
pub const CHANNEL_BLOCKS: u64 = 3;

/// Channel ID for backfill messages.
pub const CHANNEL_BACKFILL: u64 = 4;
```

These channels serve distinct consensus and data-dissemination roles:

| Channel | ID | Purpose | Consumer |
|---------|-----|---------|----------|
| Votes | 0 | Leader proposals, vote messages | Simplex consensus engine |
| Certificates | 1 | Notarization and finalization certificate gossip | Simplex consensus engine |
| Resolver | 2 | Fetch requests for missing consensus dependencies | Simplex resolver |
| Blocks | 3 | Full block broadcast (reliable dissemination) | Marshal broadcast layer |
| Backfill | 4 | Historical block requests for catch-up | Marshal resolver/peer layer |

### Rate Quotas

All channels share the same default rate quota, defined in `crates/network/transport/src/builder.rs:21-24`:

```rust
/// Default rate quota for channels (1000 messages per second).
const fn default_quota() -> Quota {
    Quota::per_second(NonZeroU32::new(1000).expect("1000 is non-zero"))
}
```

Each channel is registered with a backlog of 256 messages (from `crates/network/transport/src/config.rs:15`):

```rust
/// Default channel backlog size.
pub const DEFAULT_BACKLOG: usize = 256;
```

The registration happens in `crates/network/transport/src/builder.rs:81-87` (inside the `build_with_quota` method):

```rust
let votes = network.register(CHANNEL_VOTES, quota, backlog);
let certs = network.register(CHANNEL_CERTS, quota, backlog);
let resolver = network.register(CHANNEL_RESOLVER, quota, backlog);
let blocks = network.register(CHANNEL_BLOCKS, quota, backlog);
let backfill = network.register(CHANNEL_BACKFILL, quota, backlog);
```

### Mailbox Sizes

The internal consensus mailboxes (between the network layer and the consensus/marshal actors) are sized at 1024 messages each:

```rust
// crates/node/simplex/src/config.rs:17
pub const DEFAULT_MAILBOX_SIZE: usize = 1024;

// crates/network/marshal/src/broadcast.rs:15
pub const DEFAULT_MAILBOX_SIZE: usize = 1024;

// crates/network/marshal/src/peers.rs:34
pub const DEFAULT_MAILBOX_SIZE: usize = 1024;
```

The production runner imports and uses the simplex default (`crates/node/runner/src/runner.rs:37` and `559`):

```rust
// Line 37: import
use kora_simplex::{DEFAULT_MAILBOX_SIZE as MAILBOX_SIZE, DefaultPool};

// Line 559: usage in simplex engine config
mailbox_size: MAILBOX_SIZE,
```

The broadcast and peer resolver also use their own 1024-message defaults (see `BroadcastInitializer::DEFAULT_MAILBOX_SIZE` at `broadcast.rs:15` and `PeerInitializer::DEFAULT_MAILBOX_SIZE` at `peers.rs:34`).

---

## What Is Missing

### 1. No per-peer connection state metric

There is no Kora-level metric that reports whether a specific validator peer is connected or disconnected. The Commonware runtime exposes aggregate counters:

```
runtime_inbound_connections_total   (counter) - Connections created by peers dialing us
runtime_outbound_connections_total  (counter) - Connections created by dialing peers
```

And the network tracker exposes:

```
network_tracker_directory_connected  (gauge) - Peer connection timestamps
network_tracker_directory_tracked    (gauge) - Total tracked peers
```

However, there is no per-peer connected/disconnected gauge that an operator can use in a Grafana dashboard to see "node2 lost connectivity to node3 at timestamp X." The `network_tracker_directory_connected` metric tracks timestamps, not boolean state, and requires knowing specific peer public keys to interpret.

Note: The `NodeStatus` struct (in `crates/node/rpc/src/state.rs:99-118`) includes a `peer_count` field (line 115), but this is set via `NodeState::set_peer_count()` and in practice always reports 0 due to a separate bug (see Issue #11: `net_peerCount` always returns zero). Even when fixed, this field provides only an aggregate count, not per-peer state.

### 2. No messages sent/received per channel counter

The Commonware runtime provides `network_spawner_messages_sent_total` and `network_spawner_messages_received_total` counters with labels `{peer, message}` where `message` is one of `greeting`, `peers`, `bit_vec`, `data_0` through `data_4`. While `data_0` through `data_4` correspond to the five application channels, these are labeled by the raw Commonware channel ID, not by the Kora-meaningful channel name (votes, certs, resolver, blocks, backfill).

The mapping from Commonware's internal `data_N` labels to Kora's logical channels is:

| `message` label | Kora channel | Purpose |
|-----------------|-------------|---------|
| `data_0` | Votes (CHANNEL_VOTES) | Consensus vote messages |
| `data_1` | Certificates (CHANNEL_CERTS) | Notarization/finalization certificates |
| `data_2` | Resolver (CHANNEL_RESOLVER) | Simplex resolver fetch requests |
| `data_3` | Blocks (CHANNEL_BLOCKS) | Block broadcast via marshal layer |
| `data_4` | Backfill (CHANNEL_BACKFILL) | Marshal resolver backfill requests |

There is no Kora-level wrapper that:
- Maps `data_0` to "votes", `data_1` to "certs", etc. in a human-readable way
- Provides per-channel throughput dashboards
- Alerts when a specific channel's message rate drops to zero

### 3. No bandwidth utilization per peer

The runtime provides aggregate bandwidth counters:

```
runtime_inbound_bandwidth_total    (counter) - Total bytes received
runtime_outbound_bandwidth_total   (counter) - Total bytes sent
```

These are not broken down by peer or by channel. Operators cannot determine if a single peer is responsible for disproportionate bandwidth usage, or if a specific channel (e.g., block broadcast) is consuming most bandwidth.

### 4. No message delivery latency histogram

There is no metric measuring the time between a message being sent on one node and received on another. While Commonware provides `network_spawner_messages_sent_total` and `network_spawner_messages_received_total`, there is no round-trip or one-way latency measurement.

At the consensus level, `engine_voter_notarization_latency` and `engine_voter_finalization_latency` histograms exist, but these measure end-to-end consensus round time, not raw P2P delivery time. A network delay of 100ms and a CPU bottleneck of 100ms appear identical in these metrics.

### 5. No mailbox fill level gauge

The channel backlog (256 messages per channel, per `DEFAULT_BACKLOG`) and internal mailboxes (1024 messages) have no fill-level gauge. There is no way to detect that a channel's inbound buffer is 90% full and about to start dropping messages.

### 6. No heartbeat/ping-pong mechanism

There is no application-level heartbeat between validators. Peer liveness is inferred entirely from consensus activity. If a validator's P2P connection silently fails (e.g., TCP half-open due to a network partition), it will not be detected until a consensus timeout fires.

---

## Current Detection Mechanisms

Without P2P-level metrics, peer health is detected only through consensus-level symptoms:

| Failure | Detection Mechanism | Detection Latency |
|---------|---------------------|-------------------|
| Validator crash | Leader timeout (`CONSENSUS_LEADER_TIMEOUT = 2s`, `crates/node/runner/src/runner.rs:48`) | 2-4 seconds |
| Network partition | Certification timeout (`CONSENSUS_CERTIFICATION_TIMEOUT = 4s`, line 49) | 4-8 seconds |
| Slow peer | Activity timeout (`CONSENSUS_ACTIVITY_TIMEOUT = 256 views`, line 52) | Minutes |
| Silent disconnect | Leader timeout on affected leader's turn | Up to `2s * num_validators` between turns |

The production runner configures these timeouts at `crates/node/runner/src/runner.rs:48-53`:

```rust
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_secs(2);    // L48
const CONSENSUS_CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(4);  // L49
const CONSENSUS_TIMEOUT_RETRY: Duration = Duration::from_secs(1);     // L50
const CONSENSUS_FETCH_TIMEOUT: Duration = Duration::from_secs(1);     // L51
const CONSENSUS_ACTIVITY_TIMEOUT: ViewDelta = ViewDelta::new(256);    // L52
const CONSENSUS_SKIP_TIMEOUT: ViewDelta = ViewDelta::new(32);         // L53
```

Additional constants relevant to P2P configuration (same file):
```rust
const SIGNATURE_THREADS: usize = 2;           // L54
const BLOCK_CODEC_MAX_TXS: usize = 10_000;    // L44
const BLOCK_CODEC_MAX_TX_BYTES: usize = 8 * 1024 * 1024;  // L47 (8 MB)
```

### Existing Prometheus Scrape Configuration

Prometheus (`docker/config/prometheus.yml`) scrapes validator metrics every 10 seconds from the `kora-validators` job targeting `validator-node0:9002` through `validator-node3:9002` (internal port). A `validator_index` label is extracted via relabeling. The secondary peer is scraped separately as `kora-secondary` from `secondary-node0:9002`. Any new metrics added to the Kora binary will be automatically scraped without Prometheus configuration changes.

### The Only P2P-Adjacent Metric

The single metric that provides any insight into P2P-level issues is `engine_resolver_resolver_peers_blocked` -- a gauge of peers blocked from providing blocks for catch-up. This is defined by Commonware's resolver module and reported in raw metrics as:

```
# HELP engine_resolver_resolver_peers_blocked Current number of blocked peers.
# TYPE engine_resolver_resolver_peers_blocked gauge
engine_resolver_resolver_peers_blocked 0
```

An alert exists for this in `docker/config/alerts.yml:181-188` (in the `performance_alerts` group):

```yaml
- alert: ResolverPeersBlocked
  expr: engine_resolver_resolver_peers_blocked > 0
  for: 1m
  labels:
    severity: warning
  annotations:
    summary: "Node {{ $labels.instance }} has {{ $value }} blocked resolver peers"
```

This is specific to the resolver's block-fetch subsystem and does not reflect general P2P connectivity.

### Mailbox and Buffer Overflow Behavior

The full buffer configuration is:

| Buffer | Size | Source |
|--------|------|--------|
| Channel backlog (per channel) | 256 messages | `DEFAULT_BACKLOG` in `crates/network/transport/src/config.rs:15` |
| Broadcast mailbox | 1024 messages | `BroadcastInitializer::DEFAULT_MAILBOX_SIZE` in `crates/network/marshal/src/broadcast.rs:15` |
| Broadcast deque | 256 messages | `BroadcastInitializer::DEFAULT_DEQUE_SIZE` in `crates/network/marshal/src/broadcast.rs:18` |
| Resolver mailbox | 1024 messages | `PeerInitializer::DEFAULT_MAILBOX_SIZE` in `crates/network/marshal/src/peers.rs:34` |
| Simplex mailbox | 1024 messages | `DEFAULT_MAILBOX_SIZE` in `crates/node/simplex/src/config.rs:17` |
| Max message size | 1 MB | `DEFAULT_MAX_MESSAGE_SIZE` in `crates/network/transport/src/config.rs:12` |
| Fetch concurrent | 32 | Hardcoded in `crates/node/runner/src/runner.rs:569` |

Additional resolver timing configuration (all from `crates/network/marshal/src/peers.rs`):
- Initial delay: 200ms (`DEFAULT_INITIAL_DELAY`, line 37)
- Timeout: 200ms (`DEFAULT_TIMEOUT`, line 40)
- Fetch retry timeout: 100ms (`DEFAULT_FETCH_RETRY_TIMEOUT`, line 43)

When inbound buffers are full (256 messages per channel backlog), messages may be silently dropped by the Commonware network layer. There is no Kora-level logging or metric for this event. The only signal is an increase in consensus timeouts or nullifications as dropped vote/certificate messages cause round failures.

The internal mailboxes (1024 messages each for simplex, broadcast, and peer initializer) also have no overflow instrumentation. If a slow consumer cannot keep up with incoming messages, the mailbox fills and new messages are dropped without any metric being incremented.

---

## Impact

During the chain stalls that have been observed in production:

1. **Root cause ambiguity** -- Operators could not determine whether stalls were caused by network issues (peer disconnection, message loss) or consensus bugs (voter panic, executor failure). The `VoterCrash` and `ConsensusStall` alerts fire, but they do not differentiate between "node lost network connectivity" and "node's voter actor panicked."

2. **Blind restart decisions** -- Without per-peer connection state, operators restart all nodes rather than targeting the specific failed peer. This causes unnecessary state loss (especially with the tmpfs journal issue from Issue #15).

3. **No early warning** -- Network degradation (increased latency, packet loss, buffer pressure) provides no warning before it manifests as consensus failures. By the time a `HighNullificationRate` alert fires, the damage is already done.

4. **Post-mortem difficulty** -- After a stall, there is no historical data on per-peer connectivity, message rates, or buffer pressure. The only available data is consensus-level metrics (views, finalizations, nullifications) and unstructured logs.

---

## Proposed Fixes

### 1. Add a periodic P2P health check task

Create a background task that periodically queries the Commonware oracle for connected peers and publishes their state:

```rust
// In crates/node/runner/src/runner.rs, after oracle.track()

let oracle_clone = transport.oracle.clone();
let expected_peers = self.scheme.participants().len();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        // Query oracle for connected peer count
        // Publish as a Prometheus gauge: kora_p2p_connected_peers
        // Also publish per-peer connection state if oracle API supports it
    }
});
```

### 2. Expose Prometheus metrics for P2P health

Add the following metrics to the Kora application layer:

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `kora_p2p_connected_peers` | Gauge | `role` (validator/secondary) | Number of connected peers by role |
| `kora_p2p_peer_connected` | Gauge | `peer_index` | 1 if connected, 0 if disconnected |
| `kora_p2p_channel_messages_sent_total` | Counter | `channel` (votes/certs/resolver/blocks/backfill) | Messages sent per logical channel |
| `kora_p2p_channel_messages_received_total` | Counter | `channel` | Messages received per logical channel |
| `kora_p2p_channel_backlog_depth` | Gauge | `channel` | Current number of messages in the channel backlog |
| `kora_p2p_mailbox_depth` | Gauge | `component` (simplex/broadcast/resolver) | Current fill level of internal mailboxes |

### 3. Add per-channel rate counters

Wrap the channel senders/receivers in `crates/network/transport/src/builder.rs` with instrumented wrappers that count messages per channel:

```rust
// After registering channels, wrap with counters
let votes = network.register(CHANNEL_VOTES, quota, backlog);
let votes = InstrumentedChannel::new(votes, "votes");
// ... repeat for each channel
```

This requires either a wrapper type around the Commonware `Sender`/`Receiver` or a middleware layer that intercepts `send()` and `recv()` calls.

### 4. Add a lightweight heartbeat between validators

Implement a heartbeat mechanism that:
- Sends a small ping message every 5 seconds to all connected peers
- Measures round-trip time to each peer
- Publishes `kora_p2p_peer_rtt_seconds` histogram per peer
- Detects silent disconnections within 15 seconds (3 missed heartbeats)

This could reuse one of the existing channels (e.g., a sub-message type on the resolver channel) or use a dedicated 6th channel. The heartbeat should be separate from consensus messages to avoid interfering with rate quotas.

### 5. Add Grafana dashboard panels

Create a new "P2P Health" dashboard (or add a section to the existing kora-overview dashboard at `docker/grafana/dashboards/kora-overview.json`) with:

- **Peer connectivity matrix** -- 4x4 grid showing connection state between each validator pair
- **Per-channel message rates** -- Time series of messages/sec per channel
- **Bandwidth by peer** -- Stacked area chart of bytes sent/received per peer
- **Mailbox fill levels** -- Gauge panels for each component's mailbox depth
- **RTT heatmap** -- Latency distribution per peer pair (if heartbeat is implemented)

### 6. Document the current channel layout

Add a Prometheus recording rule that maps Commonware's `data_N` labels to Kora channel names. The existing `docker/config/recording-rules.yml` already contains 22 recording rules in two groups (`performance_recording` and `throughput_recording`). Add a new group or append to the existing file:

```yaml
# Add to docker/config/recording-rules.yml as a new group:
- name: kora_p2p
  interval: 10s
  rules:
    # Map data_N labels to human-readable channel names
    - record: kora:channel_messages_sent:rate5m
      expr: |
        label_replace(
        label_replace(
        label_replace(
        label_replace(
        label_replace(
          rate(network_spawner_messages_sent_total{message=~"data_.*"}[5m]),
          "channel", "votes", "message", "data_0"),
          "channel", "certs", "message", "data_1"),
          "channel", "resolver", "message", "data_2"),
          "channel", "blocks", "message", "data_3"),
          "channel", "backfill", "message", "data_4")

    - record: kora:channel_messages_received:rate5m
      expr: |
        label_replace(
        label_replace(
        label_replace(
        label_replace(
        label_replace(
          rate(network_spawner_messages_received_total{message=~"data_.*"}[5m]),
          "channel", "votes", "message", "data_0"),
          "channel", "certs", "message", "data_1"),
          "channel", "resolver", "message", "data_2"),
          "channel", "blocks", "message", "data_3"),
          "channel", "backfill", "message", "data_4")
```

Alternatively, use Grafana dashboard variable value mappings for simpler presentation:

```
0 -> votes
1 -> certs
2 -> resolver
3 -> blocks
4 -> backfill
```

After adding the recording rules, reload Prometheus without restart:
```bash
curl -X POST http://localhost:9090/-/reload
```

---

## Testing Plan

1. **Per-peer connection metric** -- Start 4-node devnet. Verify `kora_p2p_connected_peers` reports 3 for each validator. Stop `validator-node2`. Verify the metric drops to 2 on remaining nodes within 15 seconds. Restart node2 and verify it returns to 3.

2. **Channel message counters** -- Run the load generator (`bin/loadgen`) at 100 TPS. Verify that:
   - `votes` channel shows ~4 msg/s (one per view from each validator)
   - `certs` channel shows ~4 msg/s (certificate gossip)
   - `blocks` channel shows ~1 msg/s (block broadcast from leader)
   - `resolver` and `backfill` channels show near-zero under normal operation

3. **Mailbox pressure** -- Introduce artificial latency (using `tc netem` or Docker network settings) on one node. Verify that mailbox depth gauges increase for that node's inbound channels. Verify that the mailbox depth returns to near-zero when latency is removed.

4. **Silent disconnect detection** -- If heartbeat is implemented: use `iptables` to block traffic between two specific validators. Verify that the heartbeat detects the partition within 15 seconds and the `kora_p2p_peer_connected` gauge transitions to 0, before any consensus timeout fires.

5. **Dashboard verification** -- Verify that all new Grafana panels display data correctly. Test the peer connectivity matrix with various failure scenarios (1 node down, 2 nodes down, network partition).

6. **Existing metric mapping** -- Query `network_spawner_messages_sent_total{message="data_0"}` in Prometheus and verify it corresponds to vote channel activity. Verify the same for `data_1` through `data_4`.

---

## Verification Steps

After implementing the proposed fixes, verify with these steps:

```bash
# 1. Start the devnet
cd docker && just trusted-devnet

# 2. Verify new metrics are being emitted by scraping a validator directly
curl -s http://localhost:9000/metrics | grep kora_p2p
# Expected: kora_p2p_connected_peers, kora_p2p_peer_connected, etc.

# 3. Verify Prometheus is scraping the new metrics
curl -s "http://localhost:9090/api/v1/query?query=kora_p2p_connected_peers" | jq '.data.result'
# Expected: 4 results (one per validator), each showing 3 connected peers

# 4. Verify channel message counters map correctly
curl -s "http://localhost:9090/api/v1/query?query=kora_p2p_channel_messages_sent_total" | jq '.data.result[] | {channel: .metric.channel, value: .value[1]}'
# Expected: counters for votes, certs, resolver, blocks, backfill channels

# 5. Verify existing Commonware data_N metrics still work
curl -s "http://localhost:9090/api/v1/query?query=network_spawner_messages_sent_total" | jq '.data.result[] | {message: .metric.message, value: .value[1]}'
# Expected: data_0 through data_4, plus greeting, peers, bit_vec

# 6. Verify recording rule maps data_N to channel names
curl -s "http://localhost:9090/api/v1/query?query=kora:channel_messages_sent:rate5m" | jq '.data.result'

# 7. Test peer disconnect detection
docker compose -f compose/devnet.yaml stop validator-node2
sleep 15
curl -s "http://localhost:9090/api/v1/query?query=kora_p2p_connected_peers" | jq '.data.result'
# Expected: remaining validators show 2 connected peers

# 8. Verify the recording rule in Prometheus
curl -s "http://localhost:9090/api/v1/rules" | jq '.data.groups[] | select(.name | contains("kora")) | .rules[] | select(.name | contains("channel"))'
```

---

## Related Files

| File | Description |
|------|-------------|
| `crates/network/transport/src/channels.rs` | Channel ID constants (L9-22): `CHANNEL_VOTES=0` through `CHANNEL_BACKFILL=4`. Also has `SimplexChannels` and `MarshalChannels` structs. |
| `crates/network/transport/src/builder.rs` | Transport builder (101 lines): `default_quota()` at L21-24 (1000 msg/s), `build_with_quota` at L70 registers all 5 channels (L81-87) and starts network (L90) |
| `crates/network/transport/src/config.rs` | Transport config: `DEFAULT_BACKLOG = 256` (L15), `DEFAULT_MAX_MESSAGE_SIZE = 1MB` (L12), `DEFAULT_NAMESPACE` (L18). Also has `TransportParsing` for bootstrap peer format `PK_HEX@HOST:PORT` (L144) |
| `crates/network/transport/src/transport.rs` | `NetworkTransport` struct: holds oracle, handle, simplex channels, marshal channels |
| `crates/node/runner/src/runner.rs` | Production runner (600+ lines): consensus timeout constants (L48-53), `oracle.track()` call (L337), `fetch_concurrent: 32` (L569), `MAILBOX_SIZE` usage (L559) |
| `crates/node/simplex/src/config.rs` | Simplex config defaults: `DEFAULT_MAILBOX_SIZE = 1024` (L17). Also has `DEFAULT_LEADER_TIMEOUT`, `DEFAULT_NOTARIZATION_TIMEOUT` |
| `crates/network/marshal/src/broadcast.rs` | Broadcast initializer (62 lines): `DEFAULT_MAILBOX_SIZE = 1024` (L15), `DEFAULT_DEQUE_SIZE = 256` (L18), `DEFAULT_PRIORITY = true` (L21) |
| `crates/network/marshal/src/peers.rs` | Peer initializer (98 lines): `DEFAULT_MAILBOX_SIZE = 1024` (L34), `DEFAULT_INITIAL_DELAY = 200ms` (L37), `DEFAULT_TIMEOUT = 200ms` (L40), `DEFAULT_FETCH_RETRY_TIMEOUT = 100ms` (L43) |
| `crates/node/rpc/src/state.rs` | `NodeState`/`NodeStatus` (L99-118): has `peer_count` field (L115) but no per-peer breakdown. Uses `camelCase` serialization. |
| `docker/config/alerts.yml` | 22 alerting rules across 5 groups. `ResolverPeersBlocked` (L181-188) is the only P2P-adjacent alert |
| `docker/config/recording-rules.yml` | 22 pre-computed recording rules. New channel-mapping rules should be added here |
| `docker/config/prometheus.yml` | Scrape config (35 lines): 10s interval, `kora-validators` job targets `validator-nodeN:9002`, `kora-secondary` targets `secondary-node0:9002` |
| `docker/scripts/devnet-health.sh` | Health diagnostic script: queries `runtime_inbound_bandwidth_total` and `runtime_outbound_bandwidth_total` (L108, L110) |
| `docker/grafana/dashboards/kora-overview.json` | Overview dashboard: Network section has bandwidth and message rate panels. New P2P panels could be added here |
| `docker/grafana/dashboards/kora-stall-diagnostics.json` | Stall diagnostics dashboard: "Network & P2P Health" section with broadcast success/failure panels |
