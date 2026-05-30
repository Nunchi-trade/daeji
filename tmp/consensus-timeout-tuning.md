# Consensus Timeout Configuration and Tuning

## What Is Kora?

Kora is a minimal, high-performance EVM execution client built in Rust. It combines:

- **Commonware Simplex BFT** for consensus (BLS12-381 threshold signatures)
- **REVM** for EVM transaction execution
- **QMDB** for persistent state storage

The consensus layer uses the **Simplex** protocol from the Commonware framework. Simplex is a streamlined BFT consensus protocol that achieves finality in a single round under normal operation.

---

## How Simplex Consensus Works

Simplex operates in a sequence of numbered **views**. Each view has exactly one designated **leader** selected by VRF-based random election (`elector: Random`).

### Normal Flow (Happy Path)

```
View N:
  1. Leader Selection    - VRF determines which validator leads this view
  2. Block Proposal      - Leader calls propose() -> build_block() -> broadcasts block
  3. Verification        - Other validators call verify() -> re-execute transactions
  4. Notarization        - Validators cast notarize votes (BLS threshold signatures)
  5. Certification       - 2/3+ notarization votes collected -> certified
  6. Finalization        - Validators cast finalize votes -> 2/3+ -> block finalized
  7. View Advance        - View N+1 begins immediately
```

In normal operation, the view completes as fast as network propagation and BLS signature aggregation allow. Timeouts are never hit.

### Failed Flow (Nullification)

```
View N:
  1. Leader Selection    - VRF selects a leader
  2. Leader Fails        - propose() returns None, or no proposal arrives
  3. Leader Timeout      - After LEADER_TIMEOUT seconds, validators vote to nullify
  4. Nullification       - 2/3+ nullify votes -> view is nullified (no block produced)
  5. View Advance        - View N+1 begins
```

A nullified view produces no block. The chain advances to the next view, where a different leader is selected.

### Key Terminology

| Term | Meaning |
|------|---------|
| **View** | A single round of the consensus protocol |
| **Notarization** | 2/3+ validators attest that a proposed block is valid |
| **Certification** | The aggregated threshold signature proving notarization |
| **Finalization** | 2/3+ validators confirm the certified block as final |
| **Nullification** | 2/3+ validators agree to skip a view without a block |
| **Epoch** | A group of views sharing the same validator set (set to u64::MAX = infinite in Kora) |

---

## Timeout Parameters

### Production Values (crates/node/runner/src/runner.rs)

These are compile-time constants defined in `crates/node/runner/src/runner.rs` (lines 48-53):

```rust
const CONSENSUS_LEADER_TIMEOUT: Duration = Duration::from_secs(2);
const CONSENSUS_CERTIFICATION_TIMEOUT: Duration = Duration::from_secs(4);
const CONSENSUS_TIMEOUT_RETRY: Duration = Duration::from_secs(1);
const CONSENSUS_FETCH_TIMEOUT: Duration = Duration::from_secs(1);
const CONSENSUS_ACTIVITY_TIMEOUT: ViewDelta = ViewDelta::new(256);
const CONSENSUS_SKIP_TIMEOUT: ViewDelta = ViewDelta::new(32);
```

### Default Values (crates/node/simplex/src/config.rs)

The library defaults (lines 25-41) differ from production:

```rust
pub const DEFAULT_LEADER_TIMEOUT: Duration = Duration::from_secs(1);
pub const DEFAULT_NOTARIZATION_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_NULLIFY_RETRY: Duration = Duration::from_secs(5);
pub const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(1);
pub const DEFAULT_ACTIVITY_TIMEOUT: ViewDelta = ViewDelta::new(20);
pub const DEFAULT_SKIP_TIMEOUT: ViewDelta = ViewDelta::new(10);
```

### E2E Test Values (crates/e2e/src/harness.rs)

The e2e test harness uses the library defaults (lines 385-390):

```rust
leader_timeout: Duration::from_secs(1),
certification_timeout: Duration::from_secs(2),
timeout_retry: Duration::from_secs(5),
activity_timeout: ViewDelta::new(20),
skip_timeout: ViewDelta::new(10),
```

### Comparison Table

| Parameter | Production | Library Default | E2E Test | Purpose |
|-----------|-----------|-----------------|----------|---------|
| `leader_timeout` | 2s | 1s | 1s | Time leader has to propose a block before validators nullify |
| `certification_timeout` | 4s | 2s | 2s | Time to collect 2/3+ certification votes after notarization |
| `timeout_retry` | 1s | 5s | 5s | Retry interval for re-broadcasting nullification votes |
| `fetch_timeout` | 1s | 1s | 1s | Timeout for fetching missing blocks from peers (resolver) |
| `activity_timeout` | 256 views | 20 views | 20 views | Views of inactivity before a validator is considered offline |
| `skip_timeout` | 32 views | 10 views | 10 views | Views without progress before skipping ahead |

---

## What Each Timeout Controls

### leader_timeout (2 seconds in production)

**When it fires:** A validator has not received a block proposal from the current view's leader within 2 seconds of the view starting.

**What happens:** The validator casts a nullification vote. If 2/3+ validators agree, the view is nullified and the chain advances.

**Relationship to block build time:** The leader_timeout must be greater than the worst-case `build_block()` duration. Currently, block build completes in under 1ms for typical blocks, so 2s provides enormous headroom.

**Logged as:** The Commonware engine emits a timeout event. In the application layer, `Activity::Nullify` is traced and `Activity::Nullification` is logged at debug level (see `crates/node/service/src/stubs.rs` lines 117-121).

**Prometheus metric:** `engine_voter_state_timeouts_total{reason="leader"}`

### certification_timeout (4 seconds in production)

**When it fires:** After a block is notarized (2/3+ notarize votes received), the system waits up to 4 seconds for finalization votes to be collected into a full certification.

**What happens:** If certification is not achieved within 4 seconds, the view is considered timed out and the protocol may retry or move forward.

**Relationship to network latency:** This timeout must account for: (a) BLS signature aggregation time, (b) network propagation delay for all validator votes, and (c) any congestion on the P2P layer.

**Prometheus metric:** `engine_voter_state_timeouts_total{reason="certification"}`

### timeout_retry (1 second in production)

**When it fires:** After an initial nullification vote is cast but the view has not yet been nullified (2/3+ nullify votes not yet collected), the validator re-broadcasts its nullification vote every 1 second.

**What happens:** Helps ensure nullification converges even if some messages are lost or delayed.

**Production vs default:** Production uses 1s (aggressive retry) versus the library default of 5s. This makes Kora's production configuration recover faster from failed proposals at the cost of slightly more network traffic during failures.

### fetch_timeout (1 second in production)

**When it fires:** When a validator needs a block it does not have (e.g., it joined late or missed a broadcast), it requests the block from peers. If no response arrives within 1 second, the fetch is retried.

**What happens:** The resolver component re-requests the block from a different peer.

**Related config:** `fetch_concurrent: 32` (production) controls how many concurrent fetch requests are outstanding.

### activity_timeout (256 views in production)

**When it fires:** If a validator has not participated meaningfully in 256 consecutive views, it is considered inactive.

**What happens:** The protocol may exclude the validator from leader election or apply penalties. This protects against permanently disconnected validators consuming leader slots.

**Production vs default:** Production uses 256 (very lenient) versus the library default of 20. This is because production deployments may experience transient network issues or maintenance windows.

### skip_timeout (32 views in production)

**When it fires:** If the chain makes no progress (no blocks finalized) for 32 consecutive views, validators attempt to skip ahead.

**What happens:** The consensus engine attempts to advance past a stuck state, potentially skipping leaders that appear to be permanently offline.

**Production vs default:** Production uses 32 versus the library default of 10, providing more patience before triggering skip behavior.

---

## Other Throughput-Related Constants

### Block Production (crates/node/runner/src/runner.rs)

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `BLOCK_CODEC_MAX_TXS` | 10,000 | Maximum transactions per block at the codec level |
| `BLOCK_CODEC_MAX_TX_BYTES` | 8 MiB | Maximum total serialized block size |
| `SIGNATURE_THREADS` | 2 | BLS signature verification parallelism |
| `EPOCH_LENGTH` | u64::MAX | Effectively infinite (no epoch rotations) |
| `fetch_concurrent` | 32 | Parallel resolver block fetch requests |
| `replay_buffer` | 16 MiB | Journal replay buffer (recovery) |
| `write_buffer` | 16 MiB | Journal write buffer (persistence) |
| `forwarding` | SilentLeader | Leader does not broadcast proposal back to itself |

### Application Layer (crates/node/runner/src/app.rs)

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `max_txs` (ProposalBuilder) | Configurable (default 1000) | Maximum transactions pulled from mempool per proposal |

### Execution Configuration (crates/node/config/src/execution.rs)

| Parameter | Value | Purpose |
|-----------|-------|---------|
| `DEFAULT_GAS_LIMIT` | 250,000,000 | Maximum gas per block |
| `DEFAULT_BLOCK_TIME` | 2s | Target block time (used for EIP-1559 base fee calculation) |

---

## Relationship Between Timeouts

```
                         TIMING DIAGRAM (Happy Path)

    |-------- build_block() --------|
    |   (must finish before         |
    |    leader_timeout)             |
    |                                |
    0s                          LEADER_TIMEOUT (2s)
    |================================|

    |--- notarize votes ---|--- finalize votes ---|
    |                      |                      |
    0s            ~25-47ms (BLS)          CERTIFICATION_TIMEOUT (4s)
    |==============|========================|======|


                         TIMING DIAGRAM (Failed Proposal)

    |--- leader does nothing ---|--- nullify votes ---|--- retry ---|
    0s                     LEADER_TIMEOUT   TIMEOUT_RETRY intervals
    |============================|===================|============|
                  2s                    +1s              +1s ...
```

### Critical Invariants

1. **leader_timeout > max(build_block_time)** - If build_block takes longer than the leader timeout, the block will be nullified even though the leader was working. Currently: build_block is ~0.03ms vs 2000ms timeout (66,000x headroom).

2. **certification_timeout > leader_timeout** - Certification cannot complete until after the proposal is made, so the certification budget must exceed the leader timeout.

3. **timeout_retry < leader_timeout** - The retry interval should be shorter than the initial timeout so that missed nullification votes get resent before the next timeout period begins.

4. **activity_timeout > skip_timeout** - A node should be skipped before it is declared inactive.

---

## How Timeouts Affect Throughput

### Normal Operation (Timeouts Never Hit)

In normal operation, the block pipeline runs at the speed of BLS signature aggregation:

```
Actual block time: ~9-10ms (pipelined finalization)
Build duration:    ~0.03ms (negligible)
Network:           < 1ms (same machine devnet)
BLS aggregation:   25-47ms (dominates latency but is pipelined)
```

Effective throughput: ~108 blocks/sec observed on devnet.

### When Timeouts Fire

Every nullified view wastes time equal to `leader_timeout`:

```
Throughput loss per nullification = leader_timeout / effective_block_time
                                  = 2000ms / 9.28ms
                                  = ~215 potential blocks lost per nullification
```

However, in practice, nullified views are overlapped (the 2-second wait happens in parallel with other views progressing through the pipeline), so the actual cost is measured as a reduction in the percentage of views that produce blocks.

### Aggressive vs Conservative Tuning

| Profile | leader_timeout | cert_timeout | retry | Effect |
|---------|---------------|--------------|-------|--------|
| Current (conservative) | 2s | 4s | 1s | Tolerates slow proposals; costs 2s per failure |
| Aggressive | 500ms | 2s | 250ms | Fast failure recovery; risks cutting off slow but valid proposals |
| Ultra-aggressive | 200ms | 1s | 100ms | Sub-second recovery; only viable with co-located validators |
| Geo-distributed | 5s | 10s | 2s | Accounts for cross-region latency |

---

## Current Production Values vs Theoretical Optimal

### Observed Performance (Devnet, 4 Validators, Same Machine)

```
Block time:          9.28ms effective
Views/sec:           ~148
Blocks/sec:          ~108
Skip rate:           ~26% (startup drift, see idle-nullification doc)
Build duration:      0.03ms
Finalization:        25-47ms (BLS signature aggregation)
```

### Theoretical Maximum

If all views produced blocks (0% skip rate):
```
Max throughput: 148 views/sec * 1 block/view = 148 blocks/sec
```

### Timeout Optimization Opportunities

| Optimization | Impact | Notes |
|--------------|--------|-------|
| Reduce leader_timeout 2s -> 500ms | Faster recovery from failed proposals | Safe given 0.03ms build times |
| Reduce cert_timeout 4s -> 2s | Faster timeout on missed votes | Safe for same-machine deployments |
| Reduce timeout_retry 1s -> 250ms | More aggressive nullification convergence | Minimal downside |
| Eliminate startup drift | +35% throughput (108 -> 148 blocks/sec) | Requires protocol changes |

---

## When Timeouts Fire: Logs and Metrics

### Log Messages

From `crates/node/service/src/stubs.rs` and `crates/node/reporters/src/lib.rs`:

```
TRACE  "nullify vote"                        -- This validator voted to nullify
DEBUG  nullification round=<view>            -- 2/3+ validators agreed to nullify
WARN   "build_block: execution failed"       -- Executor error triggered nullification
WARN   "missing parent snapshot"             -- Parent not available, verify returns false
```

From `crates/node/runner/src/app.rs`:

```
DEBUG  "built block" height=N total_ms=M     -- Successful proposal
DEBUG  "propose complete" build_ms=M         -- Proposal timing
DEBUG  "verify complete" verify_ms=M         -- Verification timing
WARN   "state root mismatch"                 -- Block rejected during verification
```

### Prometheus Metrics

| Metric | Description |
|--------|-------------|
| `engine_voter_state_timeouts_total` | Counter of timeouts, labeled by reason (leader, certification) |
| `engine_voter_state_nullifications_total` | Counter of completed nullifications |
| `engine_voter_state_current_view` | Current view number (monotonically increasing) |
| `finalized_height` | Latest finalized block height |
| `engine_voter_notarization_latency_sum/count` | Histogram of notarization latency |
| `engine_voter_finalization_latency_sum/count` | Histogram of finalization latency |
| `marshaled_build_duration_sum/count` | Histogram of block build duration |
| `engine_batcher_verify_latency_sum/count` | Histogram of BLS signature verification |

### Derived Metrics (Grafana)

```promql
# Skip rate (fraction of views that do not produce a block)
1 - (rate(finalized_height[5m]) / rate(engine_voter_state_current_view[5m]))

# Consensus efficiency (inverse of skip rate)
avg(rate(finalized_height[5m])) / avg(rate(engine_voter_state_current_view[5m]))

# Blocks per second
avg(rate(finalized_height[1m]))

# Nullification rate
sum(rate(engine_voter_state_nullifications_total[1m]))

# Stall detection
rate(engine_voter_state_current_view[1m]) > 0 and rate(finalized_height[1m]) == 0
```

---

## Tuning Recommendations

### For Faster Block Times (Reduce Empty-View Overhead)

| Change | Rationale | Risk |
|--------|-----------|------|
| `LEADER_TIMEOUT` 2s -> 500ms | Faster recovery from failed proposals; current build times are 0.03ms so 500ms still provides 16,000x headroom | Legitimate slow proposals (large blocks with expensive state root computation) could be cut off |
| `CERTIFICATION_TIMEOUT` 4s -> 2s | Faster timeout on missed certification votes | Multi-machine deployments may need more time for vote propagation |
| `TIMEOUT_RETRY` 1s -> 500ms | Faster convergence of nullification | More messages during failure, but they are small |

### For Higher Throughput (Process More Transactions)

| Change | Rationale | Risk |
|--------|-----------|------|
| `BLOCK_CODEC_MAX_TXS` 10k -> 1k | Less ECDSA recovery overhead per `mempool.build()` call | Limits TPS if demand exceeds 1k/block |
| `SIGNATURE_THREADS` 2 -> 4 | More BLS verification parallelism | Uses more CPU cores |
| `gas_limit` 250M -> 500M | Allow more computation per block | Longer execution time; must still fit within leader_timeout |

### For Better Stall Recovery

| Change | Rationale | Risk |
|--------|-----------|------|
| `ACTIVITY_TIMEOUT` 256 -> 64 | Faster detection of permanently offline nodes | May trigger false positives during slow catch-up |
| `SKIP_TIMEOUT` 32 -> 16 | Faster skip past stuck leaders | May skip leaders that are merely slow, not stuck |

---

## Code References

### Where Timeouts Are Defined

- **Production constants:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` lines 48-53
- **Library defaults:** `/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs` lines 25-41
- **E2E test values:** `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs` lines 385-390

### Where Timeouts Are Applied

- **Simplex engine config:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` lines 548-573 (the `simplex::Config` struct passed to `simplex::Engine::new()`)
- **Default config builder:** `/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs` lines 86-108 (the `DefaultConfig::init()` method)

### Where Proposals Are Built

- **Application propose():** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` lines 285-318
- **Block building:** `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` lines 84-164 (`build_block()`)
- **Proposal builder:** `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/proposal.rs` lines 83-122 (`build_proposal()`)

### Where Nullification Is Reported

- **NodeStateReporter:** `/Users/will/dev/nunchi/daeji/crates/node/reporters/src/lib.rs` lines 453-475 (increments `nullified_count`)
- **RPC state:** `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/state.rs` lines 70-72 (`inc_nullified()`)
- **Grafana dashboard:** `/Users/will/dev/nunchi/daeji/docker/grafana/dashboards/kora-overview.json` (nullification panels)

---

## Monitoring These Parameters

The devnet provides three monitoring interfaces:

1. **Grafana Dashboards** (http://localhost:3000)
   - `kora-overview`: Cluster health, height drift, nullification rate, timeout reasons
   - `kora-performance`: Block time, throughput, efficiency, capacity vs actual
   - `kora-transaction-flow`: Per-node skip rate, stall detection

2. **Prometheus** (http://localhost:9090)
   - Raw metrics from all validators on ports 9000-9003
   - Recording rules for derived metrics (e.g., `kora:nullification_rate`)

3. **devnet-health.sh** script
   - Queries Prometheus for a structured text report
   - Shows: height drift, blocks/sec, nullification rate, skip rate, latency percentiles

### Alert Thresholds (Recommended)

| Alert | Condition | Severity |
|-------|-----------|----------|
| `SlowBlockBuild` | `marshaled_build_duration` p95 > 1s | Warning |
| `CriticalBlockBuild` | `marshaled_build_duration` p99 > 1.8s | Critical (approaching leader_timeout) |
| `HighFinalizationLatency` | `engine_voter_finalization_latency` p95 > 2s | Warning |
| `LowConsensusEfficiency` | efficiency < 50% for 5 minutes | Critical |
| `ChainStalled` | `rate(finalized_height[5m]) == 0` and `rate(engine_voter_state_current_view[5m]) > 0` | Critical |
| `HighNullificationRate` | `sum(rate(engine_voter_state_nullifications_total[5m])) > 10` | Warning |
