# Dependency Version Splits: jsonrpsee 0.24 and alloy v1/v2 Cause Duplicate Crates

**Category**: dependencies
**Severity**: medium

**Labels**: `enhancement`, `dependencies`, `performance`

## Summary

The Kora project has two significant dependency version splits that cause duplicate crate compilations, increased binary size, and maintenance burden. First, `jsonrpsee 0.24` requires `tower 0.4`, while the rest of the stack uses `tower 0.5`, forcing a renamed duplicate dependency. Second, the workspace pins `alloy-consensus` and `alloy-eips` at v1 while `alloy-evm 0.34.0` transitively pulls in v2, causing two copies of core Ethereum type hierarchies. Additionally, `cargo-deny` is configured with `multiple-versions = "allow"`, so CI never flags new duplicates.

## Problem

### 1. jsonrpsee 0.24 forces tower 0.4 duplicate

The RPC crate depends on `jsonrpsee 0.24.10`, which requires `tower 0.4`. However, `axum 0.8` and `reqwest 0.12` (used elsewhere in the project) use `tower 0.5`. The RPC crate works around this by keeping both versions with a renamed import:

```toml
# /Users/will/dev/nunchi/daeji/crates/node/rpc/Cargo.toml:16-23
tower = { version = "0.5", features = ["limit", "util"] }
tower-http = { version = "0.6", features = ["cors"] }
# jsonrpsee 0.24 depends on tower 0.4; its `set_http_middleware` expects
# tower 0.4's `ServiceBuilder`, so we keep a renamed 0.4 dependency.
tower_04 = { package = "tower", version = "0.4" }

# JSON-RPC
jsonrpsee = { version = "0.24", features = ["server", "macros"] }
```

### 2. alloy-consensus / alloy-eips v1/v2 split

The workspace `Cargo.toml` declares v1 for Alloy types:

```toml
# /Users/will/dev/nunchi/daeji/Cargo.toml:93-95
alloy-primitives = "1.0"
alloy-consensus = { version = "1.0", features = ["k256"] }
alloy-eips = "1.0"
```

But `alloy-evm 0.34.0` transitively requires `alloy-consensus 2.0.1` and `alloy-eips 2.0.1`:

```toml
# /Users/will/dev/nunchi/daeji/Cargo.toml:99-100
revm = { version = "38.0.0", default-features = false, features = ["optional_balance_check", "optional_no_base_fee"] }
alloy-evm = { version = "0.34.0", default-features = false }
```

This means both type hierarchies coexist in the binary.

### 3. cargo-deny does not detect duplicates

The `deny.toml` configuration explicitly allows all duplicate versions:

```toml
# /Users/will/dev/nunchi/daeji/deny.toml:27-33
[bans]
multiple-versions = "allow"
wildcards = "allow"
skip = [
    "getrandom",
    "windows-link",
]
```

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/crates/node/rpc/Cargo.toml`, lines 16-23
**File**: `/Users/will/dev/nunchi/daeji/Cargo.toml`, lines 93-100
**File**: `/Users/will/dev/nunchi/daeji/deny.toml`, lines 27-33

## Impact

- **Compile time**: Each duplicate crate adds to build duration. Docker builds under QEMU take approximately 45 minutes; reducing duplicates could save 5-10 minutes per build.
- **Binary size**: Two full copies of consensus types, EIP types, and tower middleware stack are linked into the final binary.
- **Maintenance burden**: Version drift between duplicates makes auditing harder. Type-mismatch errors at crate boundaries become possible when the wrong version's types are accidentally used.
- **Invisible to CI**: With `multiple-versions = "allow"`, new dependency duplicates introduced by any PR will never trigger a CI warning, allowing the problem to grow silently.

## Root Cause

1. `jsonrpsee 0.24` was released before tower 0.5 and has not been updated to support it.
2. `alloy-evm 0.34.0` depends on alloy v2, while the workspace pins alloy at v1.
3. `deny.toml` uses `"allow"` instead of `"warn"` for `multiple-versions`, so no CI check flags duplicates.

## Suggested Fix

### Immediate

1. **Upgrade `jsonrpsee` from 0.24 to 0.26**: jsonrpsee 0.26 supports tower 0.5, eliminating the 0.4 duplicate and the `tower_04` workaround in `crates/node/rpc/Cargo.toml`. This may require API changes if jsonrpsee's `set_http_middleware` signature changed.

2. **Upgrade workspace `alloy-consensus` and `alloy-eips` to version 2.0** in the root `Cargo.toml`:
   ```toml
   alloy-consensus = { version = "2.0", features = ["k256"] }
   alloy-eips = "2.0"
   ```
   This will likely require code changes wherever alloy consensus/eips types are used, as v2 may have breaking API changes.

### Follow-up

3. **Change `[bans] multiple-versions`** from `"allow"` to `"warn"` in `deny.toml`, with an expanded `skip` list for known-acceptable duplicates (hashbrown, getrandom, windows-sys, ark-ff).

4. **Add non-workspace dependencies to workspace `Cargo.toml`**: `tower`, `tower-http`, and `jsonrpsee` should be workspace dependencies for consistent version management across all crates.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/Cargo.toml` -- Workspace `alloy-consensus`/`alloy-eips` v1 declarations (lines 94-95), `alloy-evm` dependency (line 100)
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/Cargo.toml` -- jsonrpsee 0.24, tower 0.4/0.5 workaround (lines 16-23)
- `/Users/will/dev/nunchi/daeji/deny.toml` -- `multiple-versions = "allow"` (line 28), `skip` list (lines 30-33)
- All crates using `alloy-consensus` or `alloy-eips` types -- API migration to v2

## Related Issues

- `095-docker-build-profile-unused.md` -- Docker build improvements (compile time reduction)
- `096-ci-docker-build-smoke-test.md` -- CI improvements (cargo-deny should catch duplicates)
