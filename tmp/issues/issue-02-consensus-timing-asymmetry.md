# Consensus Timing Asymmetry: 714:1 Penalty Ratio Causes 99.9% Throughput Drop on Single Node Failure

## Summary

The Simplex BFT consensus engine is configured with a `leader_timeout` of 5 seconds, but healthy consensus rounds complete in approximately 7 milliseconds. This 714:1 ratio between the timeout ceiling and the actual round duration means that when any single validator goes offline, every round where the dead node is the designated leader incurs a full 5-second stall. In a 4-validator network, one offline node causes throughput to collapse from ~89 blocks/sec to ~0.08 blocks/sec -- a 99.9% reduction.

## Problem Description

### Observed Healthy-State Performance

On the devnet (4 validators, single Hetzner server, Docker containers), the consensus engine operates dramatically faster than its timeout parameters anticipate:

| Metric | Measured Value |
|--------|---------------|
| View progression rate | ~135 views/sec |
| Block finalization rate | ~89 blocks/sec |
| Nullification rate | ~33.5% of views |
| Average view duration | ~7.4 ms |
| Nullification fast-path duration | ~7 ms |

These numbers mean each consensus round (propose, notarize, finalize or nullify) completes in under 8 milliseconds.

### What Happens When a Node Goes Down

When one of the four validators goes fully offline, two distinct view types emerge:

1. **Healthy leader views (~7 ms):** The three remaining validators take turns proposing and finalizing blocks at the same ~7 ms pace.

2. **Dead leader views (~5,000 ms):** When the offline node is selected as leader, no proposal arrives. The remaining validators must wait for the full `leader_timeout` (5,000 ms) before they can vote to nullify the view and advance.

### Theoretical vs. Observed Throughput Under Failure

A simple model assuming round-robin leader selection and no compounding effects predicts:

```
View N   (healthy leader):  ~7ms   -> finalized block
View N+1 (healthy leader):  ~7ms   -> finalized block
View N+2 (healthy leader):  ~7ms   -> finalized block
View N+3 (dead leader):     ~5000ms -> nullified (timeout)
                            -------
Total cycle:                ~5021ms for 3 blocks
Predicted rate:             3 / 5.021 = 0.60 blocks/sec
```

However, actual devnet measurements (2026-05-22) show an **observed rate of ~0.08 blocks/sec** -- roughly 7.5x worse than the naive prediction. The discrepancy is explained by several compounding factors:

1. **Leader election is VRF-based, not strict round-robin.** The `elector::Random` elector selects leaders using a VRF seed derived from consensus certificates (see "Leader Election" section below). This means the dead node may be selected for consecutive views, compounding the timeout penalty beyond what a uniform 1-in-4 model predicts.

2. **`timeout_retry` adds 2 seconds per failed nullification quorum.** If the initial nullification vote does not gather a quorum on the first attempt, `timeout_retry` (2s) triggers a retransmission. In degraded conditions where message delivery is less reliable, a single dead-leader view can cost `leader_timeout + N * timeout_retry = 5 + N*2` seconds.

3. **Execution lag worsens under intermittent throughput.** When blocks arrive in bursts (3 fast, then 5s stall), the execution pipeline and snapshot store experience cache thrashing. The QMDB state must be rebuilt from cold cache after each stall, causing additional "soft" nullifications even during healthy-leader views.

4. **The 33.5% baseline nullification rate compounds.** Even among the 3 healthy leaders, roughly 1 in 3 views nullifies due to execution lag. This means the effective number of blocks per cycle is closer to 2 than 3, further reducing throughput.

### Why Existing Nullification Is Fast But Dead-Node Nullification Is Not

The ~33.5% nullification rate in healthy state is caused by execution lag: the consensus engine advances views faster than QMDB can commit state snapshots. When a validator is elected leader but the parent block's snapshot has not been persisted yet, `build_block()` returns `None` immediately. This "soft" failure is processed in ~7 ms because the leader actively sends a non-proposal, and the other validators can immediately vote to nullify.

**Relevant code** (`crates/node/runner/src/app.rs`, lines 91-106):
```rust
async fn build_block(&self, parent: &Block, timestamp: u64) -> Option<Block> {
    // ...
    let parent_snapshot = match self.ledger.parent_snapshot(parent_digest).await {
        Some(snap) => snap,
        None => {
            warn!(
                parent_height = parent.height,
                ?parent_digest,
                "build_block: parent snapshot not found — \
                 node has not yet processed this parent block"
            );
            return None;
        }
    };
    // ...
}
```

When a node is completely offline, there is no active response at all. The remaining validators have no way to distinguish "leader is dead" from "leader is slow" until the `leader_timeout` expires.

## Root Cause Analysis

The root cause is a combination of three factors.

### 1. Oversized Leader Timeout

The `leader_timeout` defaults to 5 seconds. The relevant configuration chain is:

**`crates/node/config/src/consensus.rs` (line 31):**
```rust
/// Default Simplex leader timeout in seconds.
pub const DEFAULT_SIMPLEX_LEADER_TIMEOUT_SECS: u64 = 5;
```

This constant feeds into `ConsensusSimplexConfig` (same file, lines 83-85):
```rust
/// Leader timeout in seconds.
#[serde(default = "default_simplex_leader_timeout_secs")]
pub leader_timeout_secs: NonZeroU64,
```

Which is consumed in `crates/node/runner/src/runner.rs` (line 770) to construct the Simplex engine config:
```rust
leader_timeout: Duration::from_secs(simplex_config.leader_timeout_secs.get()),
```

Note: The config field is `leader_timeout_secs: NonZeroU64` (seconds granularity). The Simplex engine itself accepts a `Duration`, so sub-second timeouts are possible at the engine level, but the Kora config layer only supports whole seconds because the field type is `NonZeroU64` interpreted as seconds. **The minimum configurable value is 1 second without code changes.**

There is a separate set of defaults in `crates/node/simplex/src/config.rs` (line 26):
```rust
/// Default leader timeout (1 second).
pub const DEFAULT_LEADER_TIMEOUT: Duration = Duration::from_secs(1);
```

This `DefaultConfig` constructor is used by the e2e test harness and the `kora-simplex` crate's `DefaultConfig::init()`, but **not** by the production runner. The production runner reads from `ConsensusSimplexConfig` which defaults to 5s.

### 2. VRF-Based Leader Election (Not Round-Robin)

The consensus engine uses the `elector::Random` leader elector, configured in `crates/node/runner/src/runner.rs` (line 759):
```rust
elector: Random,
```

The `Random` elector selects leaders using a VRF (Verifiable Random Function) seed. Seeds are derived from threshold BLS signatures on consensus certificates. The `SeedReporter` in `crates/node/reporters/src/lib.rs` (lines 46-69) hashes these seeds and stores them in the ledger:
```rust
Activity::Notarization(notarization) => {
    state
        .set_seed(
            notarization.proposal.payload,
            SeedReporter::<V>::hash_seed(notarization.seed()),
        )
        .await;
}
```

**Important distinction:** The RPC layer (`crates/node/rpc/src/state.rs`, line 82) uses a simple modulo approximation to estimate the current leader for display purposes:
```rust
let leader_index = (view % u64::from(self.inner.validator_count.get())) as u32;
let is_leader = leader_index == self.inner.validator_index;
```

This is **not** the actual leader election used by consensus. The real leader is selected by the `elector::Random` VRF elector inside the Commonware Simplex engine. The RPC approximation happens to be close for monitoring purposes but does not govern consensus behavior. Because VRF selection is pseudo-random rather than strictly round-robin, the dead node may be selected as leader for consecutive views, causing back-to-back 5-second stalls that the naive round-robin model does not predict. This partly explains why the observed throughput (0.08 blocks/sec) is much worse than the round-robin prediction (0.6 blocks/sec).

### 3. Compounding Timeout Factors

The full timeout configuration (all from `crates/node/config/src/consensus.rs`, lines 31-49):

| Parameter | Default Value | Constant Name | Line |
|-----------|--------------|---------------|------|
| `leader_timeout` | 5s | `DEFAULT_SIMPLEX_LEADER_TIMEOUT_SECS` | 31 |
| `certification_timeout` | 10s | `DEFAULT_SIMPLEX_CERTIFICATION_TIMEOUT_SECS` | 34 |
| `timeout_retry` | 2s | `DEFAULT_SIMPLEX_TIMEOUT_RETRY_SECS` | 37 |
| `fetch_timeout` | 5s | `DEFAULT_SIMPLEX_FETCH_TIMEOUT_SECS` | 40 |
| `activity_timeout` | 20 views | `DEFAULT_SIMPLEX_ACTIVITY_TIMEOUT_VIEWS` | 43 |
| `skip_timeout` | 10 views | `DEFAULT_SIMPLEX_SKIP_TIMEOUT_VIEWS` | 46 |
| `fetch_concurrent` | 8 | `DEFAULT_SIMPLEX_FETCH_CONCURRENT` | 49 |

These are wired into the Simplex engine in `crates/node/runner/src/runner.rs` (lines 770-778):
```rust
leader_timeout: Duration::from_secs(simplex_config.leader_timeout_secs.get()),
certification_timeout: Duration::from_secs(
    simplex_config.certification_timeout_secs.get(),
),
timeout_retry: Duration::from_secs(simplex_config.timeout_retry_secs.get()),
fetch_timeout: Duration::from_secs(simplex_config.fetch_timeout_secs.get()),
activity_timeout: ViewDelta::new(simplex_config.activity_timeout_views.get()),
skip_timeout: ViewDelta::new(simplex_config.skip_timeout_views.get()),
fetch_concurrent: simplex_config.fetch_concurrent.get(),
```

In the worst case, a single dead-leader view costs `leader_timeout + timeout_retry = 7 seconds`. If the nullification quorum requires multiple retry rounds, each adds another 2 seconds.

## Impact

### 1. Catastrophic Throughput Loss on Node Failure

| Scenario | Throughput | Reduction |
|----------|-----------|-----------|
| All 4 validators healthy | ~89 blocks/sec | -- |
| 1 validator offline (round-robin prediction) | ~0.6 blocks/sec | 99.3% |
| 1 validator offline (observed, 2026-05-22) | ~0.08 blocks/sec | 99.9% |

The 7.5x gap between prediction and observation is explained by VRF leader clustering, `timeout_retry` compounding, execution lag under intermittent throughput, and the baseline ~33.5% nullification rate reducing useful blocks per healthy-leader cycle.

### 2. Violates BFT Liveness Guarantee

BFT consensus with `n=4` and `f=1` should tolerate one Byzantine or crashed node with graceful degradation. The throughput should scale roughly as `(n-1)/n = 75%` of the healthy rate, yielding ~67 blocks/sec. Instead, the timeout asymmetry turns a tolerable failure into a near-complete outage.

### 3. Wasted Capacity in Healthy State

The ~33.5% nullification rate in healthy state is a separate but related symptom: consensus is advancing views faster than execution can keep up. While nullifications in healthy state are "cheap" (7 ms each), they represent wasted consensus bandwidth that could be eliminated by throttling view progression or pipelining execution.

### 4. Synthetic Block Timestamps Diverge from Reality

Because blocks finalize at ~89/sec but `Block::next_timestamp()` in `crates/node/domain/src/block.rs` (line 47) advances by at minimum `parent_timestamp + 1`:

```rust
pub const fn next_timestamp(now_secs: u64, parent_timestamp: u64) -> Option<u64> {
    match parent_timestamp.checked_add(1) {
        Some(next) => {
            if now_secs > next {
                Some(now_secs)
            } else {
                Some(next)
            }
        }
        None => None,
    }
}
```

When block production outpaces wall-clock time (~89 blocks per real second), the `now_secs > next` branch is never taken. Each block timestamp increments by exactly 1 second, causing the chain's internal clock to drift ahead at ~89x real time. After 5 minutes, the chain's clock is ~7.4 hours ahead. This breaks any time-dependent EVM operations (TIMESTAMP opcode, time-locked contracts).

## Proposed Solution

The fix has three tiers: (1) immediate config tuning, (2) near-term Kora-side improvements, and (3) longer-term upstream changes to the Commonware Simplex protocol.

### Tier 1: Reduce Timeout Values (Configuration + Minor Code Change)

#### Config Granularity Constraint

The current config uses **seconds** (`NonZeroU64` interpreted as seconds). The Simplex engine accepts `Duration`, so it natively supports sub-second values. However, the Kora config layer truncates to whole seconds, making the minimum configurable `leader_timeout` exactly **1 second**.

To support sub-second timeouts, the config fields must be changed from seconds to milliseconds. This requires modifying:

**`crates/node/config/src/consensus.rs`** -- change the constant and field:
```rust
// Before (line 31):
pub const DEFAULT_SIMPLEX_LEADER_TIMEOUT_SECS: u64 = 5;

// After:
pub const DEFAULT_SIMPLEX_LEADER_TIMEOUT_MS: u64 = 1000;
```

```rust
// Before (lines 83-85):
/// Leader timeout in seconds.
#[serde(default = "default_simplex_leader_timeout_secs")]
pub leader_timeout_secs: NonZeroU64,

// After:
/// Leader timeout in milliseconds.
#[serde(default = "default_simplex_leader_timeout_ms")]
pub leader_timeout_ms: NonZeroU64,
```

**`crates/node/runner/src/runner.rs`** -- change `from_secs` to `from_millis` (line 770):
```rust
// Before:
leader_timeout: Duration::from_secs(simplex_config.leader_timeout_secs.get()),

// After:
leader_timeout: Duration::from_millis(simplex_config.leader_timeout_ms.get()),
```

The same seconds-to-milliseconds migration should apply to `certification_timeout`, `timeout_retry`, and `fetch_timeout` for consistency.

#### Recommended Values

| Parameter | Current | Recommended | Rationale |
|-----------|---------|-------------|-----------|
| `leader_timeout` | 5s (5000ms) | **1000ms** | 143x safety margin over 7ms healthy rounds. Allows for real network latency (50-100ms cross-datacenter RTT) with generous headroom. Going lower (e.g. 500ms) risks false nullifications under network jitter in geo-distributed deployments. 1s is the minimum without the ms migration. |
| `certification_timeout` | 10s | **2000ms** | 2x the leader timeout. Certification should complete faster than leader selection. |
| `timeout_retry` | 2s | **500ms** | Must be short to avoid doubling the penalty on retries. 500ms requires the ms migration. Without it, 1s is the minimum. |
| `fetch_timeout` | 5s | **2000ms** | Block fetching is bounded by network RTT; 2s is generous. |
| `activity_timeout` | 20 views | **256 views** | At ~135 views/sec, 20 views = 148ms -- far too aggressive. 256 views = ~1.9 seconds, appropriate for detecting genuinely inactive peers. |
| `skip_timeout` | 10 views | **32 views** | Same reasoning: 10 views = 74ms is too fast at current view rates. 32 views = ~237ms provides adequate time. |

**Why 1000ms (1s) rather than 500ms or 200ms for `leader_timeout`:**

The devnet runs on a single Hetzner server with sub-millisecond inter-container latency. In a production geo-distributed deployment:
- Cross-datacenter RTT: 50-200ms
- Consensus requires multiple message rounds (propose -> notarize -> finalize): 3x RTT = 150-600ms
- Network jitter and packet loss add variance

A 500ms timeout would only provide 2.5x headroom over a 200ms RTT, risking false nullifications that waste capacity. A 1000ms timeout provides 5x headroom, which is the standard safety margin for distributed systems timeouts. If the deployment is known to be single-datacenter (like the current devnet), 500ms is safe and can be set via the config file.

**With `leader_timeout = 1s` and VRF leader selection (worst case):**
```
Assuming 1/4 of views hit the dead node (average case):
3 healthy views: 3 x 7ms = 21ms
1 dead view:     1000ms
Total cycle:     1021ms for ~2 blocks (accounting for 33% nullification)
Effective rate:  ~2 blocks/sec
```

This is 97.8% degradation instead of 99.9% -- still bad, but the chain remains functional and processes transactions.

### Tier 2: Adaptive Leader Timeout (Kora-Side Code Change)

Implement an exponential backoff that starts short and only grows if needed:

1. Start with a base `leader_timeout` of 100ms for the first attempt.
2. If the leader does not propose and the view is nullified, the next time that same leader is up, double the timeout (up to a cap of 2s).
3. If a leader successfully proposes, reset their timeout back to 100ms.

This requires changes to the Simplex engine configuration or a wrapper around the `Application` trait, which may need to be done upstream in Commonware.

### Tier 3: P2P-Aware Leader Skipping (Commonware Upstream Change)

The ideal solution is for the consensus engine to leverage P2P connection state:

1. The P2P oracle (`commonware-p2p`) already tracks which peers are connected.
2. When a leader's P2P connection is known to be down, the elector should skip their slot immediately (0 ms penalty) instead of waiting for the timeout.
3. This requires changes to the `simplex::elector::Random` implementation or a new elector trait that accepts peer liveness hints.

This is an upstream change to the Commonware framework and is tracked separately.

## Implementation Details

### Tier 1: Config Change (Without Millisecond Migration)

If keeping seconds granularity, modify the default constants in `crates/node/config/src/consensus.rs`:

```rust
// Before (lines 31-49):
pub const DEFAULT_SIMPLEX_LEADER_TIMEOUT_SECS: u64 = 5;
pub const DEFAULT_SIMPLEX_CERTIFICATION_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_SIMPLEX_TIMEOUT_RETRY_SECS: u64 = 2;
pub const DEFAULT_SIMPLEX_FETCH_TIMEOUT_SECS: u64 = 5;
pub const DEFAULT_SIMPLEX_ACTIVITY_TIMEOUT_VIEWS: u64 = 20;
pub const DEFAULT_SIMPLEX_SKIP_TIMEOUT_VIEWS: u64 = 10;

// After:
pub const DEFAULT_SIMPLEX_LEADER_TIMEOUT_SECS: u64 = 1;
pub const DEFAULT_SIMPLEX_CERTIFICATION_TIMEOUT_SECS: u64 = 2;
pub const DEFAULT_SIMPLEX_TIMEOUT_RETRY_SECS: u64 = 1;
pub const DEFAULT_SIMPLEX_FETCH_TIMEOUT_SECS: u64 = 2;
pub const DEFAULT_SIMPLEX_ACTIVITY_TIMEOUT_VIEWS: u64 = 256;
pub const DEFAULT_SIMPLEX_SKIP_TIMEOUT_VIEWS: u64 = 32;
```

No changes needed in `runner.rs` because it already uses `Duration::from_secs(...)`.

### Tier 1: Config Change (With Millisecond Migration)

For sub-second support, change all timeout fields from `_secs: NonZeroU64` to `_ms: NonZeroU64` in `crates/node/config/src/consensus.rs`, and update `crates/node/runner/src/runner.rs` to use `Duration::from_millis(...)` instead of `Duration::from_secs(...)`.

This is a breaking change to the config file format. Existing config files with `leader_timeout_secs = 5` will fail to deserialize if the field is renamed to `leader_timeout_ms`. Either support both field names with serde aliases, or document the migration.

### Synchronize Defaults in kora-simplex

The `DefaultConfig` constructor in `crates/node/simplex/src/config.rs` (lines 25-44) has its own defaults:

| Parameter | kora-simplex default | kora-config default |
|-----------|---------------------|---------------------|
| `leader_timeout` | 1s | 5s |
| `certification_timeout` | 2s | 10s |
| `timeout_retry` | 5s | 2s |
| `fetch_timeout` | 1s | 5s |
| `activity_timeout` | 20 views | 20 views |
| `skip_timeout` | 10 views | 10 views |

These two sets of defaults should be synchronized to avoid confusion. The `DefaultConfig` is used by e2e tests and the builder API; the `ConsensusSimplexConfig` is used by the production runner.

### Key Files to Modify

| File | Change |
|------|--------|
| `crates/node/config/src/consensus.rs` (lines 31-49) | Change default timeout constants; optionally switch to millisecond granularity |
| `crates/node/runner/src/runner.rs` (line 770) | Update `Duration::from_secs` to `Duration::from_millis` if millisecond config is adopted |
| `crates/node/simplex/src/config.rs` (lines 25-44) | Synchronize defaults with the production `kora-config` values |
| `docker/compose/devnet.yaml` | No changes needed (uses serde defaults from config structs) |

## Testing Plan

### 1. Baseline Healthy Throughput (Regression Test)

Deploy the 4-validator devnet with the new timeout values. Measure for 5 minutes:
- View rate should remain ~135 views/sec
- Block finalization rate should remain ~89 blocks/sec (or improve slightly due to lower retry overhead)
- Nullification rate should remain ~33.5% (this is caused by execution lag, not timeouts)

### 2. Single-Node Failure Resilience

With the new timeouts deployed:
1. Stop one validator (`docker compose stop validator-node2`)
2. Measure throughput for 60 seconds
3. Expected with `leader_timeout=1s`: throughput drops to ~2-5 blocks/sec instead of ~0.08 blocks/sec
4. Expected with `leader_timeout=500ms` (if ms migration done): throughput drops to ~4-10 blocks/sec
5. Verify: chain continues producing blocks without any multi-second stalls

### 3. Node Recovery

1. After the failure test, restart the stopped validator
2. Measure time to rejoin consensus and resume proposing
3. Verify: no permanent peer blocking (the `NoOpBlocker` in runner.rs line 760 should prevent this)
4. Note: resolver catch-up is a separate issue (PR #131) -- the restarted node may still fail to sync state

### 4. High-Latency Simulation

To validate that the reduced timeout does not cause false nullifications under real network conditions:
1. Use `tc netem` to add 50-100ms latency between Docker containers
2. Deploy with the new timeouts
3. Verify: healthy block rate remains acceptable (some increase in nullifications is expected)
4. If nullification rate exceeds 50%, the `leader_timeout` is too aggressive and should be increased

### 5. Load Test Under Degraded Conditions

1. Start the loadgen with moderate load (10k txs, 30 accounts)
2. Stop one validator mid-test
3. Verify: chain continues processing transactions at a reduced but usable rate
4. Restart the validator and verify recovery

## References

- `crates/node/config/src/consensus.rs` (lines 31-49) -- Default timeout constants
- `crates/node/config/src/consensus.rs` (lines 72-126) -- `ConsensusSimplexConfig` struct with serde defaults
- `crates/node/simplex/src/config.rs` (lines 25-44) -- `DefaultConfig` alternative defaults (used by e2e tests, not production)
- `crates/node/runner/src/runner.rs` (lines 755-782) -- Simplex engine construction with config values
- `crates/node/runner/src/runner.rs` (line 759) -- `elector: Random` (VRF-based leader selection)
- `crates/node/runner/src/runner.rs` (line 760) -- `blocker: NoOpBlocker` (prevents permanent peer blocking)
- `crates/node/runner/src/app.rs` (lines 91-106) -- `build_block()` fast-path `None` return on missing parent snapshot
- `crates/node/reporters/src/lib.rs` (lines 46-69) -- `SeedReporter` stores VRF seeds for leader election
- `crates/node/rpc/src/state.rs` (line 82) -- RPC leader approximation: `view % validator_count` (NOT the real elector)
- `crates/node/domain/src/block.rs` (lines 47-58) -- `Block::next_timestamp()` synthetic increment logic
- `tmp/node-failure-recovery-test.md` -- Devnet failure test data (2026-05-22): 89 -> 0.08 blocks/sec observed
- Commonware `simplex::Config` -- Upstream consensus configuration struct
- Commonware `simplex::elector::Random` -- VRF-based leader elector (no peer-liveness awareness)
