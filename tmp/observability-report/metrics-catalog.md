# Kora Metrics Catalog

Complete catalog of Prometheus metrics exposed by Kora validators on port 9002.
All metrics are scraped by Prometheus at `http://prometheus:9090` with job label
`kora-validators` and `validator_index` label 0-3.

## How to Query

```bash
# From the host machine
curl -sg 'http://localhost:9090/api/v1/query?query=YOUR_QUERY' | python3 -m json.tool

# From CLI health tool
just devnet-health

# Raw metrics from a specific node (host ports 9000-9003 map to container 9002)
curl http://localhost:9000/metrics   # node0
curl http://localhost:9001/metrics   # node1
curl http://localhost:9002/metrics   # node2
curl http://localhost:9003/metrics   # node3
```

---

## Consensus (Simplex Engine)

### Core State
| Metric | Type | Description |
|--------|------|-------------|
| `finalized_height` | gauge | Finalized height of application |
| `processed_height` | gauge | Processed height of application |
| `engine_voter_state_current_view` | gauge | Current consensus view number |
| `engine_voter_state_tracked_views` | gauge | Number of views being tracked in memory |
| `engine_voter_state_timeouts_total` | counter | Timed out views. Labels: `leader`, `reason` |
| `engine_voter_state_nullifications_total` | counter | Nullified views. Labels: `leader` |

### Timeout Reasons (label: `reason`)
| Value | Meaning |
|-------|---------|
| `MissingProposal` | Leader was elected but never sent a proposal |
| `LeaderTimeout` | Leader sent a proposal but it arrived too late |
| `LeaderNullify` | Leader proposed but then sent a nullification |
| `Inactivity` | Peer was marked inactive (not participating in consensus) |

### Voter Messages
| Metric | Type | Description |
|--------|------|-------------|
| `engine_voter_outbound_messages_total` | counter | Outbound consensus messages. Labels: `message` |
| `engine_voter_notarization_latency` | histogram | Time to collect notarization (sum/count/bucket) |
| `engine_voter_finalization_latency` | histogram | Time to collect finalization (sum/count/bucket) |
| `engine_voter_journal_*` | counter/gauge | Journal storage for consensus state |

### Message Types (label: `message`)
| Value | Meaning |
|-------|---------|
| `Notarize` | Vote to notarize a proposal |
| `Notarization` | Aggregated notarization certificate |
| `Finalize` | Vote to finalize a notarized block |
| `Finalization` | Aggregated finalization certificate |
| `Nullify` | Vote to nullify (skip) a view |
| `Nullification` | Aggregated nullification certificate |

### Signature Verification
| Metric | Type | Description |
|--------|------|-------------|
| `engine_batcher_added` | counter | Messages added to the verifier |
| `engine_batcher_verified` | counter | Messages verified |
| `engine_batcher_batch_size` | histogram | Batch size for verification |
| `engine_batcher_verify_latency` | histogram | Signature verification latency |
| `engine_batcher_recover_latency` | histogram | Certificate recover latency |
| `engine_batcher_inbound_messages` | counter | Inbound messages to batcher |
| `engine_batcher_latest_vote` | gauge | View of latest vote per peer |

### Resolver (Consensus Data Fetch)
| Metric | Type | Description |
|--------|------|-------------|
| `engine_resolver_resolver_cancel_total` | counter | Canceled fetches. Labels: `status` |
| `engine_resolver_resolver_fetch_active` | gauge | Currently active fetch requests |
| `engine_resolver_resolver_fetch_pending` | gauge | Pending fetch requests |
| `engine_resolver_resolver_fetch_duration` | histogram | Successful fetch duration |
| `engine_resolver_resolver_serve_duration` | histogram | Successful serve duration |
| `engine_resolver_resolver_serve_processing` | gauge | Currently processing serves |
| `engine_resolver_resolver_peers_blocked` | gauge | Blocked peers count |

---

## Block Production (Marshal)

| Metric | Type | Description |
|--------|------|-------------|
| `marshaled_build_duration` | histogram | Time to build a new block (sum/count/bucket) |

---

## Broadcast (Block Propagation)

| Metric | Type | Description |
|--------|------|-------------|
| `broadcast_get_total` | counter | Get requests. Labels: `status` (Success/Failure/Dropped) |
| `broadcast_receive_total` | counter | Received broadcasts. Labels: `status` |
| `broadcast_subscribe_total` | counter | Subscribe requests. Labels: `status` |
| `broadcast_peer_total` | counter | Broadcasts received per peer. Labels: `sequencer` |
| `broadcast_waiters` | gauge | Number of digests currently being awaited |

---

## Network (P2P Transport)

| Metric | Type | Description |
|--------|------|-------------|
| `runtime_inbound_bandwidth_total` | counter | Bytes received from peers |
| `runtime_outbound_bandwidth_total` | counter | Bytes sent to peers |
| `runtime_inbound_connections_total` | counter | Connections created by peers dialing us |
| `runtime_outbound_connections_total` | counter | Connections created by dialing peers |
| `network_spawner_messages_sent_total` | counter | Messages sent. Labels: `peer`, `message` |
| `network_spawner_messages_received_total` | counter | Messages received. Labels: `peer`, `message` |
| `network_dialer_attempts` | counter | Dial attempts per peer |
| `network_tracker_directory_connected` | gauge | Peer connection timestamps |
| `network_tracker_directory_tracked` | gauge | Total tracked peers |
| `network_tracker_directory_reserved` | gauge | Outstanding reservations |
| `network_tracker_directory_limits` | counter | Rate-limited events per peer |
| `network_tracker_directory_updates` | counter | Updates per peer |

### Network message types (label: `message`)
| Value | Meaning |
|-------|---------|
| `greeting` | Initial peer handshake |
| `peers` | Peer discovery |
| `bit_vec` | Bit vector sync |
| `data_0` through `data_4` | Application data channels |

---

## Storage

### Runtime I/O
| Metric | Type | Description |
|--------|------|-------------|
| `runtime_storage_read_bytes_total` | counter | Total bytes read from disk |
| `runtime_storage_reads_total` | counter | Total read operations |
| `runtime_storage_write_bytes_total` | counter | Total bytes written to disk |
| `runtime_storage_writes_total` | counter | Total write operations |
| `runtime_open_blobs` | gauge | Number of open blob handles |

### Archive (Finalized Blocks / Finalizations)
Prefixed with `finalized_blocks_` or `finalizations_by_height_`:
| Suffix | Type | Description |
|--------|------|-------------|
| `gets` | counter | Number of get operations |
| `has` | counter | Number of has operations |
| `syncs` | counter | Number of sync operations |
| `freezer_puts` | counter | Freezer put operations |
| `freezer_gets` | counter | Freezer get operations |
| `freezer_resizes` | counter | Table resize operations |
| `ordinal_puts` | counter | Ordinal index puts |
| `ordinal_gets` | counter | Ordinal index gets |

### QMDB State (Account/Code/Storage Trees)
Prefixed with `state_qmdb_accounts_`, `state_qmdb_code_`, `state_qmdb_storage_`:
| Suffix | Type | Description |
|--------|------|-------------|
| `index_items` | gauge | Items in the index |
| `index_keys` | gauge | Translated keys |
| `index_pruned` | counter | Pruned items |
| `log_journal_data_*` | counter/gauge | Journal data blob tracking |
| `log_merkle_*` | counter/gauge | Merkle tree journal tracking |

### Cache (Notarizations / Finalizations / Verified / Notarized)
Prefixed with `cache_cache_notarizations_`, `cache_cache_finalizations_`, etc.:
| Suffix | Type | Description |
|--------|------|-------------|
| `gets` | counter | Cache gets |
| `has` | counter | Cache has checks |
| `syncs` | counter | Cache syncs |
| `items_tracked` | gauge | Tracked items |
| `index_items` | gauge | Index items |
| `index_pruned` | counter | Pruned index items |

---

## Runtime

| Metric | Type | Description |
|--------|------|-------------|
| `runtime_process_rss` | gauge | Resident set size (memory) in bytes |
| `runtime_process_virtual_memory` | gauge | Virtual memory size in bytes |
| `runtime_tasks_running` | gauge | Currently running async tasks |
| `runtime_tasks_spawned_total` | counter | Total tasks spawned |

---

## Useful PromQL Queries

### Health
```promql
# Are all validators up?
count(up{job="kora-validators"} == 1)

# Height drift between nodes (should be near 0)
max(finalized_height) - min(finalized_height)

# Skip rate (wasted views, should be near 0)
1 - (rate(finalized_height[5m]) / rate(engine_voter_state_current_view[5m]))
```

### Performance
```promql
# Blocks per second
rate(finalized_height[1m])

# Finalization latency (seconds)
rate(engine_voter_finalization_latency_sum[1m]) / rate(engine_voter_finalization_latency_count[1m])

# Block build time
rate(marshaled_build_duration_sum[1m]) / rate(marshaled_build_duration_count[1m])

# Signature verification latency
rate(engine_batcher_verify_latency_sum[1m]) / rate(engine_batcher_verify_latency_count[1m])
```

### Faults
```promql
# Nullification rate
sum(rate(engine_voter_state_nullifications_total[5m]))

# Timeout rate by reason
sum by (reason)(rate(engine_voter_state_timeouts_total[5m]))

# Broadcast failure rate per node
rate(broadcast_get_total{status="Failure"}[5m])

# Resolver cancellation rate
rate(engine_resolver_resolver_cancel_total[5m])
```

### Resources
```promql
# Memory per node
runtime_process_rss

# Disk write throughput
rate(runtime_storage_write_bytes_total[1m])

# Network bandwidth
rate(runtime_inbound_bandwidth_total[1m])
rate(runtime_outbound_bandwidth_total[1m])
```
