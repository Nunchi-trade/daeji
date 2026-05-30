# Reduce Leader/Certification Timeouts for Faster Dead-Node Recovery

**Category**: performance -- consensus
**Severity**: medium

**Labels**: `performance`, `consensus`, `config`

---

## Summary

The consensus timeout defaults (leader_timeout=1s, certification_timeout=2s) are 30-60x larger than the typical view completion time (~31ms at 32 blocks/s). When a leader is unavailable (crashed or partitioned), the network wastes the full 1-second timeout before nullifying and advancing to the next view. With 10 validators, a single dead node costs approximately 10% of throughput because its leader slot wastes 1 second every 10 views.

---

## Problem

Kora's Simplex BFT consensus uses configurable timeouts that determine how long the network waits before giving up on a leader and moving to the next view (nullification). The current defaults are:

- **leader_timeout**: 1 second (1000ms)
- **certification_timeout**: 2 seconds (2000ms)
- **timeout_retry**: 1 second (1000ms)

The typical view completion time on the 10-node devnet is approximately 31ms (at 32 blocks/s). This means the leader timeout of 1 second provides 32x headroom over the normal case -- which is unnecessarily conservative now that the network has demonstrated stable operation with near-zero nullification.

The config file also only supports second-granularity timeouts (`NonZeroU64` in seconds), making it impossible to configure sub-second values without changing the config schema.

---

## Code Reference

**Default constants** -- `/Users/will/dev/nunchi/daeji/crates/node/config/src/consensus.rs:30-50`:
```rust
/// Default Simplex leader timeout in seconds.
///
/// Healthy views complete in ~7ms, so even 1 second provides ample margin.
/// A lower timeout limits the throughput penalty when a dead leader's turn
/// is reached in the round-robin schedule.
pub const DEFAULT_SIMPLEX_LEADER_TIMEOUT_SECS: u64 = 1;

/// Default Simplex certification timeout in seconds.
///
/// Healthy views complete in ~7ms, so 2 seconds provides a generous margin
/// for stragglers while avoiding long stalls when certification fails.
pub const DEFAULT_SIMPLEX_CERTIFICATION_TIMEOUT_SECS: u64 = 2;

/// Default Simplex nullification retry timeout in seconds.
pub const DEFAULT_SIMPLEX_TIMEOUT_RETRY_SECS: u64 = 1;
```

**Config struct fields** -- `/Users/will/dev/nunchi/daeji/crates/node/config/src/consensus.rs:96-102`:
```rust
    /// Leader timeout in seconds.
    #[serde(default = "default_simplex_leader_timeout_secs")]
    pub leader_timeout_secs: NonZeroU64,

    /// Certification timeout in seconds.
    #[serde(default = "default_simplex_certification_timeout_secs")]
    pub certification_timeout_secs: NonZeroU64,
```

**Simplex default config** -- `/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs:25-29`:
```rust
/// Default leader timeout (1 second).
pub const DEFAULT_LEADER_TIMEOUT: Duration = Duration::from_secs(1);

/// Default notarization timeout (2 seconds).
pub const DEFAULT_NOTARIZATION_TIMEOUT: Duration = Duration::from_secs(2);
```

---

## Impact

With one dead node out of 10 validators:
- Every 10 views (round-robin), the dead node's leader slot is reached
- That view wastes the full 1-second leader_timeout before nullification
- 1 second out of every ~10.3 seconds (9 * 0.031s + 1s) is wasted
- Effective throughput drops from ~33 blocks/s to ~30 blocks/s (a 10% reduction)

With two dead nodes:
- 2 seconds wasted out of every ~10.6 seconds (8 * 0.031s + 2s)
- Approximately 19% throughput reduction

At quorum boundary (3 dead, 7 alive):
- 3 seconds wasted out of every ~10.9 seconds (7 * 0.031s + 3s)
- Approximately 28% throughput reduction

Reducing the leader timeout to 200-300ms would recover most of this lost throughput while still providing 6-10x headroom over the typical view time.

---

## Root Cause

The timeouts were set conservatively during initial development to avoid false nullifications. The config schema only supports whole-second granularity (`NonZeroU64` for seconds), preventing sub-second tuning. After the network demonstrated stable 34 blocks/s with 0% nullification, the timeouts were not re-tuned.

---

## Suggested Fix

### 1. Add millisecond-granularity timeout fields

Add new config fields that accept millisecond values, deprecating the seconds-based ones:

**Before** (in `consensus.rs`):
```rust
pub leader_timeout_secs: NonZeroU64,
pub certification_timeout_secs: NonZeroU64,
```

**After**:
```rust
/// Leader timeout in milliseconds (overrides leader_timeout_secs if set).
#[serde(default)]
pub leader_timeout_ms: Option<NonZeroU64>,
/// Certification timeout in milliseconds (overrides certification_timeout_secs if set).
#[serde(default)]
pub certification_timeout_ms: Option<NonZeroU64>,

// Keep the old fields for backward compatibility
pub leader_timeout_secs: NonZeroU64,
pub certification_timeout_secs: NonZeroU64,
```

### 2. Reduce default timeouts

Based on observed performance (31ms typical view time):

```rust
pub const DEFAULT_SIMPLEX_LEADER_TIMEOUT_MS: u64 = 300;        // ~10x typical view time
pub const DEFAULT_SIMPLEX_CERTIFICATION_TIMEOUT_MS: u64 = 500;  // ~16x typical view time
```

These values are safe: the devnet has demonstrated stable operation at 34 blocks/s with 0% nullification under both healthy conditions and 3-node-down scenarios. A 300ms leader timeout still provides 10x headroom over the typical 31ms view time, accommodating network jitter and CPU contention spikes.

### 3. Update simplex config translation

In the runner code that translates `ConsensusSimplexConfig` to `simplex::Config`, prefer the millisecond field when set:

```rust
let leader_timeout = config.leader_timeout_ms
    .map(|ms| Duration::from_millis(ms.get()))
    .unwrap_or(Duration::from_secs(config.leader_timeout_secs.get()));
```

---

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/config/src/consensus.rs` -- add millisecond config fields, update defaults
- `/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs` -- update default constants
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` (or wherever simplex::Config is built) -- translate ms fields to Duration

---

## Related Issues

- `029-snapshot-wait-timeout-nullifications.md` -- snapshot wait timeout also contributes to nullification under contention
