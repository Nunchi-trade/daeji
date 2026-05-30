# 142: QMDB Page Cache Size Not Configurable by Operators

**Category**: enhancement
**Severity**: low
**Component**: storage / config

## Summary

The QMDB page cache defaults (16 KB page size, 4,096 pages = 64 MB total) are hardcoded in `QmdbBackendConfig` and cannot be changed through any user-facing configuration surface (CLI flags, config file, or environment variables). Although a `with_page_cache()` builder method exists in the code, it is never wired to any operator-accessible setting. Operators cannot tune QMDB caching for their specific hardware and workload without modifying source code.

## Problem

Kora uses QMDB (a Merkle database) with three partitions (accounts, storage, code) as its persistent state store. The page cache configuration is defined in `crates/storage/backend/src/config.rs` with hardcoded defaults:

```rust
const DEFAULT_PAGE_SIZE: NonZeroU16 = NZU16!(16 * 1024);       // 16 KB pages
const DEFAULT_PAGE_CACHE_SIZE: NonZeroUsize = NZUsize!(4_096);  // 4096 pages = 64 MB
```

The `QmdbBackendConfig` struct has a `with_page_cache()` builder method that accepts custom values, but no code path ever calls this method with user-provided values. The `QmdbConfig` type alias is used in `crates/storage/qmdb-ledger/src/ledger.rs` to initialize the backend, but it always uses `QmdbBackendConfig::new(partition_prefix)` which applies the defaults.

All three partitions (accounts, storage, code) share the same 64 MB cache configuration, despite having vastly different access patterns. The storage partition sees far more random reads than the code partition and would benefit from a larger cache.

**File**: `crates/storage/backend/src/config.rs` (lines 7-8)

## Code Reference

```rust
// crates/storage/backend/src/config.rs:1-42
const DEFAULT_PAGE_SIZE: NonZeroU16 = NZU16!(16 * 1024);
const DEFAULT_PAGE_CACHE_SIZE: NonZeroUsize = NZUsize!(4_096);

#[derive(Clone)]
pub struct QmdbBackendConfig {
    pub partition_prefix: String,
    pub page_size: NonZeroU16,
    pub page_cache_size: NonZeroUsize,
}

impl QmdbBackendConfig {
    pub fn new(partition_prefix: impl Into<String>) -> Self {
        Self {
            partition_prefix: partition_prefix.into(),
            page_size: DEFAULT_PAGE_SIZE,
            page_cache_size: DEFAULT_PAGE_CACHE_SIZE,
        }
    }

    /// Override the QMDB page cache settings.
    #[must_use]
    pub const fn with_page_cache(
        mut self,
        page_size: NonZeroU16,
        page_cache_size: NonZeroUsize,
    ) -> Self {
        self.page_size = page_size;
        self.page_cache_size = page_cache_size;
        self
    }
}
```

## Impact

1. **Suboptimal I/O performance**: The 64 MB default may be too small for production workloads where the account/storage trees exceed this cache size, causing excessive disk reads on every state lookup.
2. **Wasted memory on devnet**: On memory-constrained devnet nodes (4 GB RAM, 10 nodes), 64 MB per partition x 3 partitions = 192 MB of page cache per node may be too much, leaving less headroom for execution and consensus.
3. **No per-partition tuning**: The storage partition processes orders of magnitude more random reads than the code partition, but both get the same cache size. This means the code partition wastes cache while the storage partition is cache-starved.
4. **Deployment friction**: Operators must rebuild the binary to change cache settings, which is impractical for production deployments.

## Root Cause

The `with_page_cache()` builder method exists but is not connected to any configuration surface. No CLI flag, TOML config field, or environment variable is parsed to override the defaults.

## Suggested Fix

1. Add fields to the node configuration struct in `crates/node/config/src/node.rs`:

```rust
/// QMDB page cache size in MB (total across all partitions).
#[serde(default = "default_qmdb_cache_mb")]
pub qmdb_cache_mb: usize,
```

2. Wire the config value through to `QmdbBackendConfig::with_page_cache()` during ledger initialization in the runner.

3. (Optional) Add per-partition cache sizes:

```toml
[storage]
accounts_cache_mb = 96
storage_cache_mb = 128
code_cache_mb = 32
```

4. Add an environment variable override `KORA_QMDB_CACHE_MB` for container deployments where config files are less convenient.

## Files to Modify

- `crates/node/config/src/node.rs` -- add `qmdb_cache_mb` config field
- `crates/node/runner/src/runner.rs` -- wire config value to `QmdbBackendConfig::with_page_cache()`
- `crates/storage/backend/src/config.rs` -- no changes needed (builder already exists)

## Related Issues

None.

## Labels

`enhancement`, `config`, `storage`, `good first issue`
