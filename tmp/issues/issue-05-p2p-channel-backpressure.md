# P2P Channel Drops: Root-Cause Analysis and Decoupling Proposal

## Summary

Kora's P2P networking layer drops messages when channel buffers overflow, contributing to consensus nullifications on devnet. Live metrics show an 18.5% aggregate message drop rate (`kora:p2p:drop_ratio`), concentrated on the marshal backfill channel (`data_4`) and broadcast blocks channel (`data_3`). However, the original analysis conflated symptoms with root cause. Buffer sizing is a **symptom**: the real bottleneck is the synchronous QMDB commit in the finalization pipeline, which blocks the marshal acknowledgement callback and prevents the node from consuming inbound messages fast enough. Increasing buffers only delays overflows -- it does not solve the fundamental backpressure problem.

This updated issue corrects several errors in the original analysis, identifies the true bottleneck, and proposes a decoupling fix.

## Corrections to Original Analysis

### 1. Recording rules are NOT broken

The original issue claimed that Prometheus recording rules in `docker/config/recording-rules.yml` (introduced by PR #135) overwrite each other because they share the same `record:` name. This is **incorrect**. Each rule filters to a specific `message="data_N"` label and adds a unique `channel` label via `label_replace`. Prometheus recording rules with the same metric name produce **distinct time series** when the output label sets differ. For example:

```
kora:p2p:channel_dropped:rate1m{message="data_0", channel="simplex_votes"}
kora:p2p:channel_dropped:rate1m{message="data_1", channel="simplex_certs"}
kora:p2p:channel_dropped:rate1m{message="data_2", channel="simplex_resolver"}
kora:p2p:channel_dropped:rate1m{message="data_3", channel="broadcast_blocks"}
kora:p2p:channel_dropped:rate1m{message="data_4", channel="marshal_backfill"}
```

All five series coexist. The recording rules work correctly. The Grafana dashboard (`docker/grafana/dashboards/kora-p2p.json`) can query all channels. PR #135 delivered this observability correctly.

### 2. No evidence that consensus votes or certs are being dropped

The observed drop data from devnet metrics shows:

| Channel | Channel ID | Constant Name | Consumer | Drops Observed |
|---------|-----------|---------------|----------|----------------|
| Simplex votes | `data_0` | `CHANNEL_VOTES` | `simplex::Engine` | **Unknown (not measured)** |
| Simplex certs | `data_1` | `CHANNEL_CERTS` | `simplex::Engine` | **Unknown (not measured)** |
| Simplex resolver | `data_2` | `CHANNEL_RESOLVER` | `simplex::Engine` | 2 total (negligible) |
| Broadcast blocks | `data_3` | `CHANNEL_BLOCKS` | `BroadcastInitializer` | 2,784 total |
| Marshal backfill | `data_4` | `CHANNEL_BACKFILL` | `PeerInitializer` (resolver) | 4,845+ (peak 1,405/sec) |

The heavy drops are on `data_4` (backfill) and `data_3` (blocks). These are **marshal-layer** channels, not consensus channels. The claim that "vote or certificate messages are dropped, causing the 26-38% skip rate" is **unsubstantiated**. The skip rate may be caused by other factors (idle nullification from the known commonware bug, slow block verification, QMDB commit latency stalling proposals) rather than dropped consensus messages.

To prove causation, per-channel drop rates for `data_0` and `data_1` must be measured. The recording rules from PR #135 already support this -- simply query:
```promql
kora:p2p:channel_dropped:rate1m{channel=~"simplex_votes|simplex_certs"}
```

### 3. Buffer sizing is a symptom, not the root cause

The devnet already uses a backlog of 2048 (set in `build_local_transport()` at `/Users/will/dev/nunchi/daeji/crates/network/transport/src/ext.rs`, line 60). Despite this 8x increase over the production default of 256, the 18.5% drop rate persists. This proves that larger buffers alone do not solve the problem. The consumer side cannot drain buffers fast enough.

## Root Cause: Synchronous QMDB Commit Blocks Marshal Acknowledgement

The true bottleneck is visible in the finalization pipeline. When a block is finalized, the following happens synchronously in `handle_finalized_update()` (`/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs`, lines 112-161):

```rust
Update::Block(block, ack) => {
    // 1. Execute the block (REVM execution + state root computation)
    let result = finalize_block(&state, &context, &executor, &provider, ..., &block).await;
    // ... indexing ...

    // 2. Prune mempool
    state.prune_mempool(&block.txs).await;

    // 3. ONLY THEN acknowledge to marshal
    ack.acknowledge();  // <-- Marshal cannot advance until this returns
}
```

Inside `finalize_block()` (`/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs`, lines 169-279), the QMDB persist is **awaited synchronously**:

```rust
let persist_handle = context.clone().shared(true)
    .spawn(move |_| async move { persist_state.persist_snapshot(digest).await });
let persist_result = persist_handle.await;  // <-- blocks until QMDB writes complete
```

And `persist_snapshot` in turn calls `qmdb.commit_changes(changes).await` (`/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs`, line 415), which writes to all three QMDB partitions (accounts, storage, code) sequentially (`/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs`, lines 340-365).

The result is this critical path:

```
Block finalized
  -> REVM execution (CPU-bound)
    -> QMDB state root computation
      -> QMDB commit to 3 partitions (I/O-bound, sequential)
        -> mempool prune
          -> ack.acknowledge()  <-- marshal can finally deliver next block
```

Until `ack.acknowledge()` is called, the marshal actor is blocked. This means:
- New finalized blocks queue up in the marshal's internal buffer
- The broadcast engine (consuming `data_3`) backs up because marshal cannot process acknowledgements
- The resolver/peer handler (consuming `data_4`) backs up because the backfill pipeline shares the marshal actor
- Channel buffers fill and messages are dropped

This explains why drops concentrate on `data_3` (blocks) and `data_4` (backfill) -- these are both marshal-layer channels whose consumer is gated behind the QMDB commit.

## Channel Architecture Reference

All five channels are registered in `/Users/will/dev/nunchi/daeji/crates/network/transport/src/builder.rs` (lines 74-87) with a uniform backlog:

```rust
let backlog = self.backlog;

// Register simplex channels
let votes = network.register(CHANNEL_VOTES, quota, backlog);    // data_0
let certs = network.register(CHANNEL_CERTS, quota, backlog);    // data_1
let resolver = network.register(CHANNEL_RESOLVER, quota, backlog); // data_2

// Register marshal channels
let blocks = network.register(CHANNEL_BLOCKS, quota, backlog);  // data_3
let backfill = network.register(CHANNEL_BACKFILL, quota, backlog); // data_4
```

Channel constants are defined in `/Users/will/dev/nunchi/daeji/crates/network/transport/src/channels.rs` (lines 10-22).

The channels are consumed in the runner (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`):
- **Simplex channels** (votes, certs, resolver) go to `simplex::Engine` (line 783-787)
- **Marshal blocks channel** goes to `BroadcastInitializer` -> broadcast engine (line 711)
- **Marshal backfill channel** goes to `PeerInitializer` -> resolver (line 702)

The two build paths use different backlogs:

| Build Path | Backlog | Function | File |
|------------|---------|----------|------|
| `build_local_transport()` | **2048** | Devnet/standalone | `ext.rs:60` |
| `build_transport()` | **256** (DEFAULT_BACKLOG) | Production | `ext.rs:66-86` |

The rate quota is 1,000 messages/sec/channel for all channels (`default_quota()` in `builder.rs`, line 22).

## Impact Assessment (Revised)

### What IS affected
- **Marshal-layer throughput**: Backfill and block broadcast are directly impacted by QMDB commit latency
- **Node recovery**: During catch-up, the backfill channel (`data_4`) drops 782 msg/sec on node0, preventing efficient state sync
- **Production transport gap**: `build_transport()` still uses DEFAULT_BACKLOG=256, 8x smaller than devnet

### What is NOT proven
- **Consensus vote/cert drops**: No measured data shows `data_0` or `data_1` drops. The 26-38% skip rate may be caused by other factors (idle nullification, slow verification) rather than dropped votes
- **Direct causation between drops and nullifications**: Correlation is not causation. The skip rate could be independent of P2P drops

### Severity: MEDIUM (revised from HIGH)

The drops degrade marshal-layer throughput and node recovery, but the impact on consensus liveness is unproven. The skip rate correlation needs investigation with per-channel metrics before assigning higher severity.

## Proposed Solution

### Part 1 (Root Cause): Decouple QMDB Commit from Marshal Acknowledgement

The `ack.acknowledge()` call should happen immediately after the snapshot is inserted into the in-memory store, **before** the QMDB disk commit. The QMDB commit can be deferred to a background task since the in-memory overlay already holds the correct state for subsequent block verification.

In `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs`, change `handle_finalized_update()`:

```rust
Update::Block(block, ack) => {
    let result = finalize_block_fast(&state, &context, &executor, &provider, ..., &block).await;

    // Acknowledge immediately after in-memory state is ready.
    // Marshal can now deliver the next finalized block without waiting for QMDB.
    state.prune_mempool(&block.txs).await;
    ack.acknowledge();

    // QMDB persist happens in background -- does not block marshal.
    let persist_state = state.clone();
    let digest = block.commitment();
    context.clone().shared(true).spawn(move |_| async move {
        if let Err(err) = persist_state.persist_snapshot(digest).await {
            error!(?digest, error = ?err, "background QMDB persist failed");
            // The in-memory state is still correct; the node can continue.
            // On restart, the archive will replay this block.
        }
    });
}
```

This requires splitting `finalize_block()` so that snapshot insertion and QMDB commit are separate steps. The snapshot insertion (overlay merge) is fast and CPU-only. The QMDB commit (3-partition sequential I/O) is slow and can safely lag behind.

**Risk**: If the node crashes after `ack.acknowledge()` but before the QMDB commit completes, the persisted QMDB state will be behind the archive head. On restart, `recover_finalized_state()` replays from the archive, which already handles this case. The commit marker validation (`validate_commit_marker()` in `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, lines 231-260) will detect and log the mismatch, and the node will re-apply the missing blocks from the archive.

**Bounded concurrency**: To prevent unbounded background tasks, limit concurrent QMDB commits (e.g., a semaphore with permits=2). If the background queue grows too large, the acknowledgement path should start awaiting again as backpressure.

### Part 2 (Mitigation): Production Transport Backlog Parity

Update `build_transport()` in `/Users/will/dev/nunchi/daeji/crates/network/transport/src/ext.rs` to match the devnet backlog:

```rust
fn build_transport<E>(&self, crypto: ed25519::PrivateKey, context: E)
    -> Result<NetworkTransport<ed25519::PublicKey, E>, TransportError>
{
    let transport_config = TransportConfig::recommended(...)
        .with_backlog(2048);  // Match local transport
    Ok(transport_config.build(context))
}
```

This does not fix the root cause but prevents the production path from being 8x worse than devnet.

### Part 3 (Observability): Measure Vote/Cert Drops

Use the existing recording rules from PR #135 to measure `data_0` and `data_1` drops on the next devnet run. If votes/certs are being dropped at significant rates, escalate severity and consider per-channel backlog differentiation. If they are not being dropped, the consensus skip rate investigation should focus elsewhere (idle nullification, verification latency).

Query to run:
```promql
sum by (channel) (kora:p2p:channel_dropped:rate1m{channel=~"simplex_votes|simplex_certs"})
```

## Previously Proposed Changes (Deferred)

The following proposals from the original issue are deferred pending root-cause validation:

- **Per-channel buffer sizing**: Only worthwhile after confirming that the QMDB decoupling in Part 1 does not eliminate drops entirely. Increasing buffers for marshal channels without fixing the consumer bottleneck just delays the inevitable.
- **Drop alerting rule**: The aggregate drop ratio alert is useful but less urgent than fixing the root cause. Can be added as part of a general alerting pass.

## Testing Plan

- [ ] **Root cause validation**: On devnet, measure `finalize_block` latency breakdown (REVM execution vs. QMDB commit vs. total) using the existing `debug!` logs. If QMDB commit dominates, confirm the hypothesis.
- [ ] **Vote/cert drop measurement**: Query `kora:p2p:channel_dropped:rate1m` for `data_0` and `data_1` to determine whether consensus messages are actually being dropped.
- [ ] **Decoupling test**: After implementing Part 1, measure whether the aggregate drop rate on `data_3` and `data_4` decreases.
- [ ] **Skip rate correlation**: Compare skip rate before and after the QMDB decoupling to determine whether drops were actually causing nullifications.
- [ ] **Production backlog**: After Part 2, verify `build_transport()` uses 2048 backlog (check transport config at startup via `info!` log).
- [ ] **Crash recovery**: Test that a crash between `ack.acknowledge()` and QMDB commit completes recovery correctly via archive replay on restart.

## References

- **Channel registration**: `/Users/will/dev/nunchi/daeji/crates/network/transport/src/builder.rs` (lines 80-87)
- **Channel constants**: `/Users/will/dev/nunchi/daeji/crates/network/transport/src/channels.rs` (lines 10-22)
- **Default backlog constant**: `/Users/will/dev/nunchi/daeji/crates/network/transport/src/config.rs` (line 15, `DEFAULT_BACKLOG = 256`)
- **Local transport backlog**: `/Users/will/dev/nunchi/daeji/crates/network/transport/src/ext.rs` (line 60, `.with_backlog(2048)`)
- **Production transport (no override)**: `/Users/will/dev/nunchi/daeji/crates/network/transport/src/ext.rs` (lines 66-86)
- **Rate quota**: `/Users/will/dev/nunchi/daeji/crates/network/transport/src/builder.rs` (line 22, 1000 msg/s)
- **Channel consumers in runner**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` (lines 697-787)
- **Finalization pipeline (acknowledge)**: `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` (lines 112-161)
- **QMDB persist in finalize_block**: `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` (lines 261-276)
- **LedgerView::persist_snapshot**: `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` (lines 401-450)
- **QMDB commit_changes (3-partition write)**: `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs` (lines 373-379)
- **Commit marker validation**: `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` (lines 231-260)
- **Recording rules (all channels)**: `/Users/will/dev/nunchi/daeji/docker/config/recording-rules.yml` (lines 86-184)
- **P2P Grafana dashboard**: `/Users/will/dev/nunchi/daeji/docker/grafana/dashboards/kora-p2p.json`
- **PR #135**: P2P channel metrics and Grafana dashboard (recording rules, dashboard -- already merged)
- **PR #131**: NoOpBlocker to prevent resolver peer blocking after restart (already merged)
