# Migration: `block_time` (seconds) to `block_time_ms` (milliseconds)

> **STATUS: DONE** (fully implemented on `hdc` branch)
>
> | Area | Status |
> |------|--------|
> | Config rename (`DEFAULT_BLOCK_TIME` -> `DEFAULT_BLOCK_TIME_MS`) | DONE |
> | Struct field rename (`block_time` -> `block_time_ms`) | DONE |
> | Serde alias for backward compat | DONE |
> | `ProductionRunner` field + timeout derivation | DONE |
> | CLI wiring (`bin/kora/src/cli.rs`) | DONE |
> | Config README update | DONE |
> | All config tests (7 tests) | DONE |
> | E2e harness `block_time_ms` integration | TODO (optional, tests pass without it) |
> | Docker/devnet env var (`BLOCK_TIME_MS`) | DONE (in `docker/scripts/entrypoint.sh` and `docker/compose/fast-devnet.yaml`) |
> | `timestamp: height` replacement with real wall-clock timestamps | OUT-OF-SCOPE (separate task) |

---

## Verification Commands

```bash
# All config tests pass (7 tests including the alias backward-compat test):
cargo test -p kora-config

# Full workspace build:
cargo build --workspace

# Clippy clean:
cargo clippy --workspace

# Specific test names in kora-config:
#   test_default_execution_config
#   test_execution_config_serde_roundtrip
#   test_execution_config_toml_roundtrip
#   test_execution_config_serde_defaults
#   test_execution_config_partial_defaults
#   test_execution_config_clone_and_eq
#   test_execution_config_old_block_time_alias
```

---

## Remaining Work (optional, non-blocking)

- [ ] Add `block_time_ms: u64` to `TestConfig` in `crates/e2e/src/setup.rs` and derive harness timeouts from it
- [ ] Replace hardcoded `Duration::from_secs(1/2/5/1)` in `crates/e2e/src/harness.rs:386-389` with `TestConfig::block_time_ms`-derived values
- [ ] Add zero-value guard: `assert!(block_time_ms > 0)` in `ProductionRunner::new`
- [ ] Consider adding `BLOCK_TIME_MS` env-var override to the `kora` binary CLI

---

## Rollback Plan

If the migration breaks consensus or causes timeouts:

1. The serde `alias = "block_time"` is **backward-compatible** -- reverting to old config files will still deserialize (the raw seconds value arrives as-is, producing a very short block time, which is detectable in logs).
2. To revert code: revert the rename of `DEFAULT_BLOCK_TIME_MS` -> `DEFAULT_BLOCK_TIME` in `crates/node/config/src/execution.rs` and cascade through `lib.rs`, `runner.rs`, `cli.rs`.
3. The `ProductionRunner` timeout derivation (`Duration::from_millis(self.block_time_ms)`) can be reverted to the old hardcoded `Duration::from_secs(5/10/2/5)` values.
4. No consensus state is affected -- this migration only changes timing parameters, not block data or EVM state.

---

## Goal

Rename `block_time: u64` (seconds) to `block_time_ms: u64` (milliseconds) throughout the
daeji/kora codebase. The default changes from `2` to `2000`. All consensus timeouts that
are currently hardcoded must read from this config value and scale proportionally.

The repo root is `/Users/will/dev/nunchi/daeji/`.

---

## File-by-file changes

### 1. `crates/node/config/src/execution.rs`

This is the canonical definition of the block-time config field.

**Constant rename:**

```rust
// OLD (line 8-9)
/// Default block time in seconds.
pub const DEFAULT_BLOCK_TIME: u64 = 2;

// NEW
/// Default block time in milliseconds.
pub const DEFAULT_BLOCK_TIME_MS: u64 = 2000;
```

**Struct field rename (lines 18-20):**

```rust
// OLD
/// Target block time in seconds.
#[serde(default = "default_block_time")]
pub block_time: u64,

// NEW
/// Target block time in milliseconds.
#[serde(default = "default_block_time_ms", alias = "block_time")]
pub block_time_ms: u64,
```

The `alias = "block_time"` on serde lets old config files (which still say `block_time = 2`)
deserialize without error. **Do not remove the alias** -- it is the backward-compat migration
path for existing TOML/JSON configs.

**Default impl (lines 23-27):**

```rust
// OLD
Self { gas_limit: DEFAULT_GAS_LIMIT, block_time: DEFAULT_BLOCK_TIME }

// NEW
Self { gas_limit: DEFAULT_GAS_LIMIT, block_time_ms: DEFAULT_BLOCK_TIME_MS }
```

**Serde default function (lines 33-35):**

```rust
// OLD
const fn default_block_time() -> u64 {
    DEFAULT_BLOCK_TIME
}

// NEW
const fn default_block_time_ms() -> u64 {
    DEFAULT_BLOCK_TIME_MS
}
```

**Tests (lines 37-90) -- update every occurrence:**

| Line(s) | Old | New |
|---------|-----|-----|
| 45 | `config.block_time, DEFAULT_BLOCK_TIME` | `config.block_time_ms, DEFAULT_BLOCK_TIME_MS` |
| 50 | `block_time: 5` | `block_time_ms: 5000` |
| 58 | `block_time: 1` | `block_time_ms: 1000` |
| 68 | `config.block_time, DEFAULT_BLOCK_TIME` | `config.block_time_ms, DEFAULT_BLOCK_TIME_MS` |
| 76 | `config.block_time, DEFAULT_BLOCK_TIME` | `config.block_time_ms, DEFAULT_BLOCK_TIME_MS` |
| 79 | `{"block_time": 10}` | `{"block_time_ms": 10000}` |
| 81 | `config.block_time, 10` | `config.block_time_ms, 10000` |
| 86 | `block_time: 42` | `block_time_ms: 42000` |

Add a new test to verify backward compatibility with old config files:

```rust
#[test]
fn test_execution_config_old_block_time_alias() {
    // Old configs use "block_time" (seconds). The alias should still deserialize.
    let config: ExecutionConfig =
        serde_json::from_str(r#"{"block_time": 5}"#).expect("alias deserialize");
    assert_eq!(config.block_time_ms, 5);
}
```

Note: the alias deserializes the *raw value*. An old config saying `block_time = 2` will
arrive as `block_time_ms = 2`. This is intentional -- the operator must update the value
to `2000` when they rename the field. The alias exists so the node does not crash on
startup with an unknown-field error; the operator will notice the 2ms block time in logs
and fix it. If you want automatic `* 1000` conversion, you need a custom deserializer --
that is out of scope for this migration.

**Watch out for:** The `kora_config::ExecutionConfig` in this crate (`crates/node/config`)
is a *different type* from `kora_executor::ExecutionConfig` in `crates/node/executor`. The
executor one deals with chain_id, spec_id, gas_limit_bounds. Only the config crate's
`ExecutionConfig` has `block_time`. Do not confuse them.

---

### 2. `crates/node/config/src/lib.rs`

**Re-export rename (line 14):**

```rust
// OLD
pub use execution::{DEFAULT_BLOCK_TIME, DEFAULT_GAS_LIMIT, ExecutionConfig};

// NEW
pub use execution::{DEFAULT_BLOCK_TIME_MS, DEFAULT_GAS_LIMIT, ExecutionConfig};
```

---

### 3. `crates/node/config/README.md`

**Update the example config snippet (line 29):**

```toml
# OLD
block_time = 2

# NEW
block_time_ms = 2000
```

---

### 4. `crates/node/runner/src/runner.rs`

This is where the hardcoded consensus timeouts live. The `ProductionRunner` struct needs a
`block_time_ms` field, and the `simplex::Config` block must read from it.

**Add field to `ProductionRunner` (around line 122):**

```rust
// ADD after `pub gas_limit: u64,`
/// Block time in milliseconds (drives consensus timeouts).
pub block_time_ms: u64,
```

**Update `ProductionRunner::new` (lines 135-150):**

```rust
// OLD signature
pub fn new(
    scheme: ThresholdScheme,
    chain_id: u64,
    gas_limit: u64,
    bootstrap: BootstrapConfig,
) -> Self {

// NEW signature
pub fn new(
    scheme: ThresholdScheme,
    chain_id: u64,
    gas_limit: u64,
    block_time_ms: u64,
    bootstrap: BootstrapConfig,
) -> Self {
```

And in the body, add `block_time_ms` to the struct literal.

**Replace hardcoded timeouts (lines 385-388):**

```rust
// OLD
leader_timeout: Duration::from_secs(5),
certification_timeout: Duration::from_secs(10),
timeout_retry: Duration::from_secs(2),
fetch_timeout: Duration::from_secs(5),

// NEW
leader_timeout: Duration::from_millis(self.block_time_ms),
certification_timeout: Duration::from_millis(self.block_time_ms * 2),
timeout_retry: Duration::from_millis(self.block_time_ms),
fetch_timeout: Duration::from_millis(self.block_time_ms * 2),
```

Rationale for the scaling factors:
- `leader_timeout` = 1x block_time -- how long to wait for the leader to propose.
- `certification_timeout` = 2x block_time -- how long to wait for enough votes.
- `timeout_retry` = 1x block_time -- how often to retry after a timeout.
- `fetch_timeout` = 2x block_time -- how long to wait for a block fetch.

For the default 2000ms: leader=2s, cert=4s, retry=2s, fetch=4s.
For a fast 50ms block: leader=50ms, cert=100ms, retry=50ms, fetch=100ms.

**Watch out for:** The `activity_timeout` and `skip_timeout` are `ViewDelta` values (view
counts, not durations), so they do not change. Leave them at 20 and 10 respectively.

---

### 5. `bin/kora/src/cli.rs`

**Update the `ProductionRunner::new` call (around line 179):**

```rust
// OLD
ProductionRunner::new(scheme, config.chain_id, config.execution.gas_limit, bootstrap)

// NEW
ProductionRunner::new(
    scheme,
    config.chain_id,
    config.execution.gas_limit,
    config.execution.block_time_ms,
    bootstrap,
)
```

---

### 6. `crates/node/simplex/src/config.rs`

The default timeout constants here are used by `DefaultConfig::init()`. Update them to
document that they are now overridable, but keep the current values as the fallback defaults.

**No code changes strictly required** for the migration, because `ProductionRunner` in
`runner.rs` builds its own `simplex::Config` directly and does not call `DefaultConfig::init()`.
The e2e harness also builds its own config inline.

However, if you want `DefaultConfig::init()` to also accept a `block_time_ms` parameter for
future callers, add an optional builder method. This is **not blocking** for the migration.

---

### 7. `crates/node/runner/src/app.rs`

**The `timestamp: height` pattern (lines 69-72 and 83-85).**

Currently both `RevmApplication::block_context()` and the `RevmContextProvider::context()`
set `timestamp: height` (block height as Unix timestamp). This means block 1 has
timestamp=1, block 2 has timestamp=2, etc.

This is **not a real timestamp** -- it is a monotonic counter. The EVM `TIMESTAMP` opcode
will return the block number, not wall-clock time.

**For this migration, do NOT change `timestamp: height`.** Changing it to wall-clock
milliseconds would be a consensus-breaking change (all validators must agree on timestamp)
and is a separate task. The `block_time_ms` migration is purely about consensus timing,
not EVM-visible block header fields.

If a future task requires real timestamps, it should:
1. Add a `timestamp` field to the `Block` struct in `kora_domain`.
2. Have the proposer set it to `SystemTime::now()` (milliseconds since epoch).
3. Have verifiers check it is within an acceptable window.

---

### 8. `crates/node/consensus/src/proposal.rs`

**The `block_context` function (lines 14-24):**

Same pattern -- `timestamp: height`. Same reasoning as above: do not change.

The `ProposalBuilder` does not reference `block_time` for any timing logic. It uses
`kora_config::DEFAULT_GAS_LIMIT` for the gas limit. No changes needed here for this
migration.

---

### 9. `crates/node/ledger/src/lib.rs`

**The test helper `block_context` (around line 500):**

Also uses `timestamp: height`. This is test-only code. No changes needed.

---

### 10. `crates/e2e/src/harness.rs`

**`TestContextProvider::context()` (lines 231-243):** Uses `timestamp: height`. No change.

**Hardcoded consensus timeouts (lines 386-389):**

```rust
// OLD
leader_timeout: Duration::from_secs(1),
certification_timeout: Duration::from_secs(2),
timeout_retry: Duration::from_secs(5),
fetch_timeout: Duration::from_secs(1),

// These are fine for tests. The e2e harness uses simulated networking with 10ms latency.
// Keep them as-is, or optionally read from TestConfig if you add a block_time_ms field.
```

**Optional improvement:** Add `block_time_ms: u64` to `TestConfig` (in `crates/e2e/src/setup.rs`)
with a default of `2000`, and derive the harness timeouts from it. This is nice-to-have
but not strictly required -- the e2e tests currently pass with hardcoded 1s/2s/5s timeouts.

If you do add it to `TestConfig`:

```rust
// In crates/e2e/src/setup.rs, add to the struct:
/// Block time in milliseconds.
pub block_time_ms: u64,

// In Default impl:
block_time_ms: 2000,

// In harness.rs, replace the hardcoded timeouts:
leader_timeout: Duration::from_millis(config.block_time_ms),
certification_timeout: Duration::from_millis(config.block_time_ms * 2),
timeout_retry: Duration::from_millis(config.block_time_ms),
fetch_timeout: Duration::from_millis(config.block_time_ms * 2),
```

**`TestApplication::block_context()` (line 691-701):** Uses `timestamp: height`. No change.

---

### 11. `docker/compose/devnet.yaml`

The devnet currently relies on config-file defaults. The entrypoint script
(`docker/scripts/entrypoint.sh`) passes `--chain-id` but does not set execution config
fields via env vars or config overrides.

**No changes strictly required** if you keep the default at 2000ms (same effective behavior
as `block_time = 2` seconds).

**Optional:** If you want operators to override block time via environment variable, add to
the `x-node-common` environment block:

```yaml
x-node-common: &node-common
  image: kora:local
  networks:
    - kora-net
  environment:
    - RUST_LOG=${RUST_LOG:-info}
    - CHAIN_ID=${CHAIN_ID:-1337}
    - BLOCK_TIME_MS=${BLOCK_TIME_MS:-2000}
```

This would also require the `kora` binary to read `BLOCK_TIME_MS` from the environment
and override `config.execution.block_time_ms`. That is a separate enhancement and not
part of this migration.

---

## Checklist

- [x] `crates/node/config/src/execution.rs` -- rename constant, field, serde default fn, all tests
- [x] `crates/node/config/src/lib.rs` -- update re-export from `DEFAULT_BLOCK_TIME` to `DEFAULT_BLOCK_TIME_MS`
- [x] `crates/node/config/README.md` -- update example TOML snippet
- [x] `crates/node/runner/src/runner.rs` -- add `block_time_ms` to `ProductionRunner`, replace hardcoded timeouts
- [x] `bin/kora/src/cli.rs` -- pass `config.execution.block_time_ms` to `ProductionRunner::new`
- [ ] `crates/e2e/src/setup.rs` -- (optional) add `block_time_ms` to `TestConfig`
- [ ] `crates/e2e/src/harness.rs` -- (optional) derive harness timeouts from `TestConfig::block_time_ms`
- [x] Run `cargo build` -- confirm no compile errors
- [x] Run `cargo test -p kora-config` -- confirm config tests pass
- [x] Run `cargo test -p kora-e2e` -- confirm e2e tests still pass
- [x] Run `cargo clippy --workspace` -- no new warnings

### Files Modified (actual paths)

| File | Status | What Changed |
|------|--------|-------------|
| `crates/node/config/src/execution.rs` | DONE | `DEFAULT_BLOCK_TIME_MS = 2000`, `block_time_ms` field, `alias = "block_time"`, 7 tests |
| `crates/node/config/src/lib.rs:14` | DONE | Re-export `DEFAULT_BLOCK_TIME_MS` |
| `crates/node/config/README.md:29` | DONE | `block_time_ms = 2000` |
| `crates/node/runner/src/runner.rs:124,143,150,440-443` | DONE | `block_time_ms` field, `Duration::from_millis` timeout derivation |
| `bin/kora/src/cli.rs:183` | DONE | Pass `config.execution.block_time_ms` |
| `docker/scripts/entrypoint.sh:95` | DONE | `block_time_ms = ${BLOCK_TIME_MS:-50}` |
| `docker/compose/fast-devnet.yaml:59` | DONE | `block_time_ms` in config generation |
| `deploy/railway/init-config.sh:37` | DONE | `block_time_ms = ${BLOCK_TIME_MS:-50}` |

### Hardcoded Duration::from_secs Values Still Present (NOT part of this migration)

These are **not** block-time-related and should NOT be changed:

| File | Line | Duration | Purpose |
|------|------|----------|---------|
| `crates/e2e/src/harness.rs` | 386-389 | 1s/2s/5s/1s | E2e test harness consensus timeouts (optional to migrate) |
| `crates/e2e/src/setup.rs` | 45 | 30s | Test overall timeout |
| `crates/node/simplex/src/config.rs` | 26-35 | 1s/2s/5s/1s | Default simplex constants (overridden by runner) |
| `crates/node/dkg/src/ceremony.rs` | various | 5s/10s/60s | DKG ceremony timeouts (unrelated to block time) |
| `crates/node/rpc/src/server.rs` | 69 | max_age config | CORS max age (unrelated) |

---

## Anti-patterns

1. **Do not use `f64` for time.** All time values are `u64` milliseconds. Floating point
   introduces rounding errors and is not `Eq`-comparable. The EVM `timestamp` field is
   `u64`. Keep everything `u64`.

2. **Do not break existing config files.** Use `#[serde(alias = "block_time")]` so old
   TOML/JSON configs that still say `block_time = 2` do not cause a deserialization error.
   The node will start (with a 2ms block time, which is conspicuously fast), and the
   operator can update their config at their convenience.

3. **Do not forget to update the Docker env vars** if you add `BLOCK_TIME_MS` support.
   Currently the entrypoint script does not pass execution config overrides, so this is
   low-risk. But if you wire it up, update all service definitions in `devnet.yaml`.

4. **Do not change `timestamp: height` in block headers.** That is a consensus-breaking
   change and is out of scope. The `block_time_ms` migration only affects consensus
   *timing* (how long to wait for leaders/votes), not the EVM-visible block header.

5. **Do not confuse `kora_config::ExecutionConfig` with `kora_executor::ExecutionConfig`.**
   They are different types in different crates. Only `kora_config::ExecutionConfig`
   (in `crates/node/config/src/execution.rs`) has the `block_time` field. The executor's
   `ExecutionConfig` (in `crates/node/executor/src/config.rs`) has `chain_id`,
   `spec_id`, and `gas_limit_bounds`.

6. **Do not set timeouts to zero.** If `block_time_ms` is 0, `Duration::from_millis(0)`
   creates a zero-length timeout and the consensus engine will spin. Consider adding a
   validation check: `assert!(block_time_ms > 0, "block_time_ms must be > 0")` or
   clamping to a minimum (e.g., 10ms).

---

## Testing

### Unit tests

```bash
cargo test -p kora-config
```

Verifies:
- Default config has `block_time_ms = 2000`
- JSON/TOML roundtrip with `block_time_ms`
- Partial defaults fill in `block_time_ms = 2000`
- (If added) Old `block_time` alias deserializes without error

### Integration / E2E tests

```bash
cargo test -p kora-e2e -- --test-threads=1
```

The e2e tests run a 4-validator simulated network. They should pass without changes if
the default remains 2000ms. If you modified the harness timeouts, verify the tests still
finalize within the 30-second timeout.

### Compile check

```bash
cargo build --workspace
cargo clippy --workspace
```

Catches any missed references to `DEFAULT_BLOCK_TIME` or `block_time` field accesses.

### Manual smoke test (Docker devnet)

```bash
cd docker && docker compose -f compose/devnet.yaml up init-config
docker compose -f compose/devnet.yaml up validator-node0 validator-node1 validator-node2 validator-node3
```

Watch logs for:
- `leader_timeout=2000ms` or similar in startup logs
- Blocks finalizing at roughly 2-second intervals
- No panic/crash on startup

---

## Rollback plan

If something breaks after merging:

1. **Revert the commit.** All changes are additive (rename + alias). A `git revert` cleanly
   undoes everything.

2. **Config files are backward-compatible.** Because of the `serde(alias)`, a config file
   can use either `block_time` or `block_time_ms`. Rolling back the code means the field
   name goes back to `block_time`, and any configs already updated to `block_time_ms` will
   get an unknown-field warning but serde's default will kick in with 2 seconds.

3. **No database migration.** This change does not touch persisted state, block format, or
   consensus wire protocol. There is nothing to roll back on disk.

4. **Docker:** If you added `BLOCK_TIME_MS` env vars, simply remove them. The default
   kicks in.

---

## Audit Findings

Audited 2026-05-08 against implementation on branch `hdc`.

### F01 -- Config rename: DONE, correct

`/Users/will/dev/nunchi/daeji/crates/node/config/src/execution.rs`:
- Line 9: `DEFAULT_BLOCK_TIME_MS: u64 = 2000` -- matches spec.
- Line 19-20: `#[serde(default = "default_block_time_ms", alias = "block_time")] pub block_time_ms: u64` -- alias present, matches spec.
- Line 25: `Default` impl uses `DEFAULT_BLOCK_TIME_MS` -- correct.
- Line 33-35: `default_block_time_ms()` function -- correct.
- Lines 42-97: All tests updated to use `block_time_ms` with millisecond values. The alias test at line 92-97 is present and verifies `{"block_time": 5}` deserializes to `block_time_ms = 5`.

No issues.

### F02 -- Re-export rename: DONE, correct

`/Users/will/dev/nunchi/daeji/crates/node/config/src/lib.rs`:
- Line 14: `pub use execution::{DEFAULT_BLOCK_TIME_MS, DEFAULT_GAS_LIMIT, ExecutionConfig};` -- matches spec.
- No stale `DEFAULT_BLOCK_TIME` references remain anywhere in the workspace (confirmed via grep).

No issues.

### F03 -- README update: DONE, correct

`/Users/will/dev/nunchi/daeji/crates/node/config/README.md`:
- Line 29: `block_time_ms = 2000` -- matches spec.

No issues.

### F04 -- ProductionRunner `block_time_ms` field: DONE, correct

`/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`:
- Line 124: `pub block_time_ms: u64` field on `ProductionRunner` -- present.
- Line 139-157: `new()` constructor accepts `block_time_ms: u64` parameter and stores it -- correct.
- Lines 433-436: Timeouts derived from `self.block_time_ms`:
  - `leader_timeout: Duration::from_millis(self.block_time_ms)` -- 1x, matches spec.
  - `certification_timeout: Duration::from_millis(self.block_time_ms * 2)` -- 2x, matches spec.
  - `timeout_retry: Duration::from_millis(self.block_time_ms)` -- 1x, matches spec.
  - `fetch_timeout: Duration::from_millis(self.block_time_ms * 2)` -- 2x, matches spec.
- Lines 437-438: `activity_timeout` and `skip_timeout` remain as `ViewDelta` values (20 and 10) -- correctly unchanged.

No issues.

### F05 -- CLI call site: DONE, correct

`/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs`:
- Lines 178-185: `ProductionRunner::new(scheme, config.chain_id, config.execution.gas_limit, config.execution.block_time_ms, bootstrap)` -- matches spec.

No issues.

### F06 -- Simplex default config: NOT CHANGED (spec says this is acceptable)

`/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs`:
- Lines 26-35: Default constants still hardcoded at 1s/2s/5s/1s. These are only used by `DefaultConfig::init()` which is not called by `ProductionRunner` or the e2e harness, so this is benign.
- The spec explicitly states "No code changes strictly required" for this file.

**Observation**: `DefaultConfig::init()` at line 101 uses `DEFAULT_NULLIFY_RETRY` (5s) for `timeout_retry`, while `ProductionRunner` uses `1x block_time_ms` (2s default). These are different values. If any future caller uses `DefaultConfig::init()`, they will get the old 5s retry, not the proportional timeout. This is a latent divergence but not a bug today.

### F07 -- `timestamp: height` pattern: CORRECTLY LEFT UNCHANGED

Confirmed at all 6 locations:
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs` line 71
- `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` line 85 (`RevmContextProvider`)
- `/Users/will/dev/nunchi/daeji/crates/node/consensus/src/proposal.rs` line 17
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` line 503
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs` lines 235, 694

All use `timestamp: height` (or `timestamp: block.height`). The spec explicitly says do not change these. Correct.

### F08 -- E2E harness timeouts: NOT MIGRATED (spec-optional item)

`/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs` lines 386-389:
- Still hardcoded: `leader_timeout: Duration::from_secs(1)`, `certification_timeout: Duration::from_secs(2)`, `timeout_retry: Duration::from_secs(5)`, `fetch_timeout: Duration::from_secs(1)`.
- `TestConfig` in `/Users/will/dev/nunchi/daeji/crates/e2e/src/setup.rs` has no `block_time_ms` field.
- The spec marks this as "optional improvement" / "nice-to-have".
- A comment in `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/hdc.rs` line 235-236 explicitly notes the limitation: "The harness currently uses 1s leader_timeout. To truly test 50ms blocks, a `.with_block_time_ms(50)` builder method would be needed."

### F09 -- Executor (`revm.rs`): NO CHANGES NEEDED, NONE MADE

`/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs` uses `kora_executor::ExecutionConfig` (chain_id, spec_id, gas_limit_bounds) -- a completely different type from `kora_config::ExecutionConfig`. It has no `block_time` field. The spec correctly identifies this distinction (section 1, "Watch out for") and no changes were required or made.

No issues.

---

## Implementation Status

| Spec Item | File | Status | Notes |
|-----------|------|--------|-------|
| Constant rename `DEFAULT_BLOCK_TIME_MS` | `config/src/execution.rs:9` | DONE | |
| Field rename `block_time_ms` + serde alias | `config/src/execution.rs:19-20` | DONE | |
| Default impl | `config/src/execution.rs:25` | DONE | |
| Serde default fn | `config/src/execution.rs:33-35` | DONE | |
| All unit tests updated | `config/src/execution.rs:37-98` | DONE | |
| Backward-compat alias test | `config/src/execution.rs:92-97` | DONE | |
| Re-export rename | `config/src/lib.rs:14` | DONE | |
| README example TOML | `config/README.md:29` | DONE | |
| `ProductionRunner.block_time_ms` field | `runner/src/runner.rs:124` | DONE | |
| `ProductionRunner::new()` parameter | `runner/src/runner.rs:143` | DONE | |
| Consensus timeouts from `block_time_ms` | `runner/src/runner.rs:433-436` | DONE | |
| CLI call site | `bin/kora/src/cli.rs:178-185` | DONE | |
| Simplex `DefaultConfig::init()` | `simplex/src/config.rs` | SKIPPED | Spec says not required |
| E2E `TestConfig.block_time_ms` | `e2e/src/setup.rs` | SKIPPED | Spec says optional |
| E2E harness timeout derivation | `e2e/src/harness.rs:386-389` | SKIPPED | Spec says optional |
| Docker env var `BLOCK_TIME_MS` | `docker/compose/fast-devnet.yaml` | PARTIAL | See Anti-Patterns A01 |
| `timestamp: height` pattern | multiple files | CORRECTLY UNCHANGED | |

**Overall**: All required items from the spec are implemented. The three optional items are not implemented.

---

## Anti-Patterns & Duct Tape

### A01 -- Docker `BLOCK_TIME_MS` env var is set but never read (dead config)

**Severity**: Medium.

`/Users/will/dev/nunchi/daeji/docker/compose/fast-devnet.yaml` sets `BLOCK_TIME_MS=${BLOCK_TIME_MS:-50}` on lines 33, 76, 100, and 125 for all three validators. However:

1. The entrypoint script (`/Users/will/dev/nunchi/daeji/docker/scripts/entrypoint.sh`) never reads `$BLOCK_TIME_MS`. It passes `--chain-id` to `kora` but has no mechanism to pass `--block-time-ms` or inject it into a config file.
2. The `kora` CLI (`/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs`) has no `--block-time-ms` flag and does not read the `BLOCK_TIME_MS` environment variable.
3. As a result, the fast-devnet validators silently ignore the `BLOCK_TIME_MS=50` setting and start with the compiled default of 2000ms. **The fast-devnet is not actually running with 50ms blocks.**

This is classic dead configuration: the compose file creates an illusion of configurability that does not exist at any lower layer.

### A02 -- No validation of `block_time_ms` value

**Severity**: Medium.

The spec's anti-pattern #6 warns about zero-value block times but no validation was implemented. Currently:
- A config file with `block_time_ms = 0` will produce `Duration::from_millis(0)` for all consensus timeouts (line 433-436 of `runner.rs`), causing the consensus engine to spin-loop.
- A value of `1` is technically accepted but produces a 1ms leader timeout, which is unreachable on any real network.
- There is no minimum clamp, no startup assertion, and no warning log.

The `ExecutionConfig` struct accepts any `u64` including 0. Neither the serde deserialization path nor `ProductionRunner::new()` performs bounds checking.

### A03 -- Serde alias silently accepts wrong-unit values with no warning

**Severity**: Low-Medium.

When an old config file has `block_time = 2` (meaning 2 seconds), the alias deserializes it as `block_time_ms = 2` (meaning 2 milliseconds). The spec acknowledges this explicitly and says "the operator will notice the 2ms block time in logs and fix it."

However, there is **no log message** at startup that prints the effective `block_time_ms` value. Searching for `block_time_ms` or `block_time` in log/tracing output across the codebase yields zero results. The operator would only notice something is wrong when blocks finalize too fast, which could be misattributed to other causes.

The spec's smoke test section says to "Watch logs for `leader_timeout=2000ms` or similar in startup logs" but no such log line exists in the implementation. The closest is `"Starting production validator"` at `runner.rs` line 219, which logs `chain_id` but not `block_time_ms`.

### A04 -- E2E harness and ProductionRunner use fundamentally different timeout ratios

**Severity**: Low.

The production runner (`runner.rs` lines 433-436) uses the scaling:
- `leader_timeout` = 1x block_time
- `certification_timeout` = 2x block_time
- `timeout_retry` = 1x block_time
- `fetch_timeout` = 2x block_time

The e2e harness (`harness.rs` lines 386-389) uses:
- `leader_timeout` = 1s
- `certification_timeout` = 2s
- `timeout_retry` = **5s** (not 1x or 2x anything)
- `fetch_timeout` = 1s

The `timeout_retry` in the e2e harness is 5x the leader_timeout (5s vs 1s), while in production it is 1x. This means the e2e tests exercise a different timeout profile than production. Bugs sensitive to retry timing (e.g., rapid leader rotation, view thrashing) will not surface in tests.

### A05 -- Simplex `DefaultConfig` constants diverge from production defaults

**Severity**: Low.

`/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs` defines:
- `DEFAULT_LEADER_TIMEOUT` = 1s (vs production 2s at default block_time_ms=2000)
- `DEFAULT_NOTARIZATION_TIMEOUT` = 2s (vs production 4s)
- `DEFAULT_NULLIFY_RETRY` = 5s (vs production 2s)
- `DEFAULT_FETCH_TIMEOUT` = 1s (vs production 4s)

These are never used by `ProductionRunner` (which builds `simplex::Config` directly), so this is not a runtime bug. But it creates a documentation and maintenance hazard: a future developer calling `DefaultConfig::init()` will get 1s/2s/5s/1s timeouts instead of the intended proportional-to-block-time values.

### A06 -- `HdcConfig` nested in `NodeConfig` but not plumbed through consistently

**Severity**: Low (not directly related to block-time migration but observed during audit).

`/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs` line 44 adds `pub hdc: HdcConfig` to `NodeConfig`. The `ProductionRunner` at `/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs` line 134 stores `pub hdc_config: Option<kora_config::HdcConfig>` -- it is an `Option` even though the `NodeConfig` always has a (defaulted) `HdcConfig`. The CLI at `/Users/will/dev/nunchi/daeji/bin/kora/src/cli.rs` line 189-191 conditionally calls `.with_hdc(config.hdc.clone())` only if `config.hdc.enabled`, which means when HDC is disabled, `hdc_config` is `None` in the runner. This works but adds an unnecessary Option layer -- the inner `enabled` flag already controls behavior.

---

## Recommended Changes Checklist

### Must fix (correctness / operational safety)

- [ ] **A01**: Wire `BLOCK_TIME_MS` through the Docker pipeline. Either:
  - (a) Add a `--block-time-ms` CLI flag to `bin/kora/src/cli.rs` and pass it in `entrypoint.sh`, or
  - (b) Have `entrypoint.sh` generate/patch a TOML config file with `block_time_ms = $BLOCK_TIME_MS`, or
  - (c) Remove the `BLOCK_TIME_MS` env var from `docker/compose/fast-devnet.yaml` if it is not intended to work yet. Dead configuration is worse than no configuration because it misleads operators.

- [ ] **A02**: Add validation for `block_time_ms`. Suggested approach: validate in `ProductionRunner::new()` or in `NodeConfig::load()`:
  ```rust
  // In ProductionRunner::new() or a dedicated validate() method:
  assert!(block_time_ms >= 10, "block_time_ms must be >= 10 (got {block_time_ms})");
  ```
  Alternatively, add a `validate()` method to `ExecutionConfig` that is called during config loading.

- [ ] **A03**: Add a startup log line in `ProductionRunner::run()` (around `runner.rs` line 219) that prints the effective block time:
  ```rust
  info!(chain_id = self.chain_id, block_time_ms = self.block_time_ms, "Starting production validator");
  ```
  This is essential for diagnosing the serde-alias footgun where old configs get 2ms block times.

### Should fix (test quality / maintainability)

- [ ] **A04**: Add `block_time_ms` field to `TestConfig` in `/Users/will/dev/nunchi/daeji/crates/e2e/src/setup.rs` and derive harness timeouts from it in `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs` lines 386-389. Use the same multiplier ratios as production (1x, 2x, 1x, 2x). This was marked optional in the spec but should be prioritized to ensure test/production parity.

- [ ] **A05**: Either:
  - (a) Update `DefaultConfig::init()` in `/Users/will/dev/nunchi/daeji/crates/node/simplex/src/config.rs` to accept a `block_time_ms` parameter and derive its timeouts, or
  - (b) Add a doc comment to `DefaultConfig::init()` explicitly stating its timeout constants are not synchronized with `kora_config::DEFAULT_BLOCK_TIME_MS` and that callers should override them.

### Nice to have (hygiene)

- [ ] **A06**: Change `ProductionRunner.hdc_config` from `Option<HdcConfig>` to `HdcConfig` and check `.enabled` internally, removing the double-gate pattern in `cli.rs`.

- [ ] Add a serde `deserialize_with` for `block_time_ms` that logs a warning when the value is suspiciously low (< 100ms), hinting that the operator may have forgotten to convert from seconds to milliseconds.

- [ ] Add an integration test in `kora-config` that verifies a TOML config with the old `block_time = 2` key deserializes successfully (the current alias test only covers JSON).

---

## Second-Pass Remediation Detail

Audited 2026-05-08 against the current config, runner, CLI, e2e, and Docker surfaces.
This section turns the vague recommendations above into concrete implementation work. It is
written as remediation guidance only; no implementation code was changed in this audit pass.

### P0 -- Make Docker/Railway `BLOCK_TIME_MS` actually reach `NodeConfig`

Current state:
- `docker/compose/fast-devnet.yaml` sets `BLOCK_TIME_MS=${BLOCK_TIME_MS:-50}` at lines 33,
  76, 100, and 125.
- `deploy/railway/validator-0.env` sets `BLOCK_TIME_MS=50` at line 4; validator-1 and
  validator-2 have the same shape.
- `docker/scripts/entrypoint.sh` reads `CHAIN_ID`, `DATA_DIR`, and `SHARED_DIR` at lines
  7-9, but does not read `BLOCK_TIME_MS`.
- `docker/scripts/entrypoint.sh` invokes `kora ... validator` at lines 84-88 with
  `--data-dir`, `--peers`, and `--chain-id`, but no execution overrides.
- `bin/kora/src/cli.rs::Cli::load_config()` only applies `--chain-id` and `--data-dir`
  overrides at lines 76-86.

Recommended fix: add typed CLI overrides and keep Docker as a thin argument translator.
This keeps validation and precedence in Rust rather than encoding TOML patching logic in
`entrypoint.sh`.

Concrete code changes:

1. In `bin/kora/src/cli.rs`, add global CLI fields to `Cli`:

   ```rust
   #[arg(long, global = true, value_name = "MS")]
   pub block_time_ms: Option<u64>,

   #[arg(long, global = true, value_name = "GAS")]
   pub gas_limit: Option<u64>,

   #[arg(long, global = true, value_name = "BOOL")]
   pub hdc_enabled: Option<bool>,
   ```

2. In `bin/kora/src/cli.rs::Cli::load_config()`, apply overrides in this precedence order:
   config file/defaults, then CLI overrides. Docker/Railway env vars should be translated
   by the entrypoint into these same CLI overrides, so validation and precedence stay in
   one Rust path:

   ```rust
   if let Some(block_time_ms) = self.block_time_ms {
       config.execution.block_time_ms = block_time_ms;
   }
   if let Some(gas_limit) = self.gas_limit {
       config.execution.gas_limit = gas_limit;
   }
   if let Some(hdc_enabled) = self.hdc_enabled {
       config.hdc.enabled = hdc_enabled;
   }
   config.validate()?;
   ```

3. In `docker/scripts/entrypoint.sh`, build an argument array once and reuse it for
   `validator`. Avoid string-concatenated `CONFIG_ARG`; it currently works only for simple
   paths and is easy to break with spaces.

   ```bash
   KORA_ARGS=()
   [[ -n "${CONFIG_FILE:-}" ]] && KORA_ARGS+=(--config "$CONFIG_FILE")
   [[ -n "${BLOCK_TIME_MS:-}" ]] && KORA_ARGS+=(--block-time-ms "$BLOCK_TIME_MS")
   [[ -n "${GAS_LIMIT:-}" ]] && KORA_ARGS+=(--gas-limit "$GAS_LIMIT")
   case "${HDC_ENABLED:-}" in
       1|true|TRUE|yes|YES) KORA_ARGS+=(--hdc-enabled true) ;;
       0|false|FALSE|no|NO) KORA_ARGS+=(--hdc-enabled false) ;;
       "") ;;
       *) error "invalid HDC_ENABLED value: ${HDC_ENABLED}" ;;
   esac

   exec /usr/local/bin/kora "${KORA_ARGS[@]}" validator \
       --data-dir "$DATA_DIR" \
       --peers "${SHARED_DIR}/peers.json" \
       --chain-id "$CHAIN_ID" \
       "$@"
   ```

4. In `docker/compose/devnet.yaml`, add `BLOCK_TIME_MS=${BLOCK_TIME_MS:-2000}` to every
   validator service, not just `x-validator-common`. The services at lines 189, 210, 232,
   and 254 declare their own `environment` list, so Compose replaces the inherited list.
   Adding the value only to `x-validator-common` will not reach the services.

5. In `docker/compose/fast-devnet.yaml`, keep the default `50` only if the operational
   intent is truly a 50ms local devnet. If that is the intended behavior, also keep
   `GAS_LIMIT` and `HDC_ENABLED` wired through the same entrypoint path. Otherwise remove
   the dead env vars entirely. Partial wiring creates misleading operator state.

6. In `deploy/railway/init-config.env` and `deploy/railway/validator-*.env`, document the
   same variable set. Railway uses the Docker image entrypoint (`docker/Dockerfile` lines
   88-90), so fixing `entrypoint.sh` fixes Railway too.

### P0 -- Validate `block_time_ms` before building consensus durations

Current state:
- `crates/node/config/src/execution.rs::ExecutionConfig` accepts any `u64`, including 0.
- `crates/node/runner/src/runner.rs` lines 433-436 multiply `self.block_time_ms * 2`
  directly. This can create zero-duration timeouts or overflow before
  `Duration::from_millis()`.

Recommended hard bounds:

```rust
pub const MIN_BLOCK_TIME_MS: u64 = 10;
pub const WARN_BLOCK_TIME_MS: u64 = 100;
pub const DEFAULT_BLOCK_TIME_MS: u64 = 2000;
pub const MAX_BLOCK_TIME_MS: u64 = 3_600_000; // 1 hour
```

Rationale:
- `0..10ms`: reject. These values can spin consensus or are below realistic timer
  resolution.
- `10..100ms`: allow only for local/simulated networks, but warn at startup.
- `>=100ms`: valid. For non-local deployments, operators should start at `>=500ms` unless
  they have measured network, disk, and CPU headroom.
- `>3_600_000ms`: reject. One-hour block times are already beyond this node's intended
  operating envelope and this cap removes practical overflow risk for 2x timeouts.

Concrete code changes:

1. In `crates/node/config/src/error.rs`, add:

   ```rust
   #[error("invalid block_time_ms {value}: expected {min}..={max}")]
   InvalidBlockTimeMs { value: u64, min: u64, max: u64 },
   ```

2. In `crates/node/config/src/execution.rs`, add:

   ```rust
   impl ExecutionConfig {
       pub const fn validate(&self) -> Result<(), ConfigError> {
           if self.block_time_ms < MIN_BLOCK_TIME_MS || self.block_time_ms > MAX_BLOCK_TIME_MS {
               return Err(ConfigError::InvalidBlockTimeMs {
                   value: self.block_time_ms,
                   min: MIN_BLOCK_TIME_MS,
                   max: MAX_BLOCK_TIME_MS,
               });
           }
           Ok(())
       }
   }
   ```

   If `const fn` cannot be used because of the error construction, make it a normal `fn`.

3. In `crates/node/config/src/node.rs`, add `NodeConfig::validate()` and call it from
   `NodeConfig::load()`, `NodeConfig::from_toml()`, and `NodeConfig::from_json()` after
   deserialization. Also call it at the end of `bin/kora/src/cli.rs::Cli::load_config()`
   after CLI overrides.

4. In `crates/node/runner/src/runner.rs`, add a local checked timeout helper before the
   `simplex::Config` construction:

   ```rust
   let cert_timeout_ms = self
       .block_time_ms
       .checked_mul(2)
       .ok_or_else(|| anyhow::anyhow!("block_time_ms overflow"))?;
   ```

   Then use `cert_timeout_ms` for `certification_timeout` and `fetch_timeout`. This is a
   belt-and-suspenders guard; config validation should reject bad values earlier.

5. In `crates/node/runner/src/runner.rs::ProductionRunner::run()`, expand the startup log:

   ```rust
   info!(
       chain_id = self.chain_id,
       block_time_ms = self.block_time_ms,
       leader_timeout_ms = self.block_time_ms,
       certification_timeout_ms = cert_timeout_ms,
       fetch_timeout_ms = cert_timeout_ms,
       "Starting production validator"
   );
   ```

   If `self.block_time_ms < WARN_BLOCK_TIME_MS`, emit a `warn!` that this is intended only
   for local/simulated networks.

### P1 -- Replace the serde alias footgun with an explicit migration parser

Current state:
- `crates/node/config/src/execution.rs` line 19 uses `#[serde(alias = "block_time")]`.
- That means old TOML `block_time = 2` becomes `block_time_ms = 2`, not 2000.
- Adding the validation above would reject the common old value `2`, which is safer than
  silently running at 2ms, but it still forces manual operator repair.

Better design: implement custom deserialization for `ExecutionConfig` so the old key is
understood as seconds and converted exactly once.

Concrete parser shape:

```rust
#[derive(Deserialize)]
struct RawExecutionConfig {
    #[serde(default = "default_gas_limit")]
    gas_limit: u64,
    block_time_ms: Option<u64>,
    block_time: Option<u64>,
}

impl<'de> Deserialize<'de> for ExecutionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawExecutionConfig::deserialize(deserializer)?;
        let block_time_ms = match (raw.block_time_ms, raw.block_time) {
            (Some(_), Some(_)) => {
                return Err(serde::de::Error::custom(
                    "use only block_time_ms; block_time is deprecated",
                ));
            }
            (Some(ms), None) => ms,
            (None, Some(seconds)) => seconds
                .checked_mul(1000)
                .ok_or_else(|| serde::de::Error::custom("block_time seconds overflow"))?,
            (None, None) => DEFAULT_BLOCK_TIME_MS,
        };
        Ok(Self { gas_limit: raw.gas_limit, block_time_ms })
    }
}
```

Update the existing alias test expectation:
- Old JSON `{"block_time": 5}` should deserialize to `block_time_ms = 5000`, not `5`.
- Add a TOML test for `[execution] block_time = 2` through `NodeConfig::from_toml()`.
- Add a conflict test where both `block_time` and `block_time_ms` are present and parsing
  fails.

### P1 -- Bring e2e timeout behavior into production parity

Current state:
- `crates/e2e/src/setup.rs::TestConfig` has no `block_time_ms` field.
- `crates/e2e/src/harness.rs::start_all_nodes()` accepts `chain_id` and `gas_limit`, but
  not block time.
- `crates/e2e/src/harness.rs` lines 386-389 use 1s/2s/5s/1s timeouts, which do not match
  production's 1x/2x/1x/2x ratios.
- `crates/e2e/src/tests/hdc.rs` lines 235-237 already notes this gap.

Concrete code changes:

1. In `crates/e2e/src/setup.rs::TestConfig`, add:

   ```rust
   pub block_time_ms: u64,
   ```

   Default it to `kora_config::DEFAULT_BLOCK_TIME_MS` if the e2e crate takes a new
   `kora-config` dependency, or `2000` if you want to avoid a dependency edge.

2. Add a builder:

   ```rust
   pub const fn with_block_time_ms(mut self, block_time_ms: u64) -> Self {
       self.block_time_ms = block_time_ms;
       self
   }
   ```

3. Thread `config.block_time_ms` through:
   - `TestHarness::run_inner()` at the `start_all_nodes()` call near lines 154-163.
   - `start_all_nodes()` signature near lines 246-255.
   - `start_single_node()` signature near lines 286-298.

4. Replace `crates/e2e/src/harness.rs` lines 386-389 with production ratios:

   ```rust
   leader_timeout: Duration::from_millis(block_time_ms),
   certification_timeout: Duration::from_millis(block_time_ms * 2),
   timeout_retry: Duration::from_millis(block_time_ms),
   fetch_timeout: Duration::from_millis(block_time_ms * 2),
   ```

   Use the same checked 2x helper as production.

5. Update `crates/e2e/src/tests/hdc.rs::test_hdc_fast_blocks()` to call
   `.with_block_time_ms(50)`. Keep the low-latency `SimLinkConfig` at lines 245-249.

6. Add one non-HDC consensus test with `.with_block_time_ms(50)` and a 5ms simulated link.
   The goal is to prove the timing path itself is not HDC-specific.

### P1 -- De-duplicate timeout derivation

The timeout ratios now exist in prose, production code, and proposed e2e code. Put the
mapping in one helper before adding more call sites.

Minimal option:
- Add a private helper in `crates/node/runner/src/runner.rs`:

  ```rust
  struct ConsensusTimeouts {
      leader: Duration,
      certification: Duration,
      retry: Duration,
      fetch: Duration,
  }

  fn consensus_timeouts(block_time_ms: u64) -> anyhow::Result<ConsensusTimeouts> { ... }
  ```

Better option:
- Move that helper to `crates/node/simplex/src/config.rs` as a public
  `proportional_timeouts(block_time_ms: u64)` helper. Then both `ProductionRunner` and
  `crates/e2e/src/harness.rs` can use the same ratio code. Keep validation constants in
  `kora-config`; the helper should only compute durations after validation.

Do not change `activity_timeout` or `skip_timeout`; they are `ViewDelta` values, not wall
clock durations.

### P2 -- Fix documentation that now contradicts the code

Current stale docs:
- `crates/node/runner/README.md` lines 32-37 and 51 still show
  `ProductionRunner::new(scheme, chain_id, gas_limit, bootstrap)` without
  `block_time_ms`.
- `docker/README.md` lines 163-171 lists Docker environment variables but does not mention
  `BLOCK_TIME_MS`, `GAS_LIMIT`, or `HDC_ENABLED`.
- `bin/kora/README.md` line 38 says only `--chain-id` and `--data-dir` override config
  values.

Concrete doc changes after the implementation lands:
- Update the runner examples to pass `2_000` or `config.execution.block_time_ms`.
- Add `BLOCK_TIME_MS` to Docker docs with defaults: `2000` for `devnet`, `50` only for
  `fast-devnet`.
- Document that `--block-time-ms` is an operational timing parameter, not an EVM header
  timestamp parameter.

### Operational caveats to include in release notes

- `block_time_ms` is not persisted and does not change the block format. Rollback is a
  binary/config rollback, not a database migration.
- Different validators can technically run different local timeouts, but operators should
  treat `block_time_ms` as a network-wide parameter. Mixed values can cause unnecessary
  view changes, delayed certification, or liveness instability even if they do not directly
  change block hashes.
- A 50ms block time is for local fast-devnet or controlled low-latency networks only.
  It should not be the default for public or multi-region deployments.
- `docker/scripts/healthcheck.sh` line 14 checks `/data/.ready` plus P2P port reachability.
  `entrypoint.sh` touches `.ready` at line 64 before the validator process is exec'd, so
  the healthcheck does not prove consensus is finalizing at the configured block time.
  Smoke tests must inspect startup logs and finalized block cadence.
- Old configs using `block_time = 2` must either be converted by the custom parser above
  or fail fast with a clear validation error. Silently accepting a raw `2ms` value is the
  failure mode to avoid.

### Prioritized checklist

- [ ] P0: Add `--block-time-ms`, `--gas-limit`, and `--hdc-enabled` overrides in
  `bin/kora/src/cli.rs::Cli`; apply them in `Cli::load_config()`.
- [ ] P0: Add `ExecutionConfig::validate()` and `NodeConfig::validate()` with
  `MIN_BLOCK_TIME_MS = 10` and `MAX_BLOCK_TIME_MS = 3_600_000`.
- [ ] P0: Call validation from `NodeConfig::from_toml()`, `NodeConfig::from_json()`, and
  after CLI overrides in `Cli::load_config()`.
- [ ] P0: Update `docker/scripts/entrypoint.sh` validator mode to pass
  `--block-time-ms "$BLOCK_TIME_MS"` when set.
- [ ] P0: Update `docker/compose/devnet.yaml`, `docker/compose/fast-devnet.yaml`, and
  `deploy/railway/validator-*.env` so advertised env vars match actual CLI/config inputs.
- [ ] P0: Add startup logging in `ProductionRunner::run()` for `block_time_ms` and derived
  consensus timeout values.
- [ ] P1: Replace the raw serde alias with an explicit old-key parser that converts
  `block_time` seconds to `block_time_ms` milliseconds and rejects both keys together.
- [ ] P1: Add `TestConfig::block_time_ms` and `.with_block_time_ms()` in
  `crates/e2e/src/setup.rs`; thread it through `crates/e2e/src/harness.rs`.
- [ ] P1: Add e2e coverage for 50ms timing in both `crates/e2e/src/tests/hdc.rs` and a
  non-HDC consensus test.
- [ ] P1: Extract a shared timeout derivation helper so production and e2e cannot drift.
- [ ] P2: Update `crates/node/runner/README.md`, `docker/README.md`, and `bin/kora/README.md`
  after the code changes land.
