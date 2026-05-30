# QmdbLedger::init() and LedgerView::init() Always Apply Genesis With No Restart Guard

## Category
bug -- storage / data integrity

## Severity
medium

## Summary
The public APIs `QmdbLedger::init()`, `LedgerView::init()`, and `LedgerView::init_with_genesis_timestamp()` all hardcode `apply_genesis = true`, meaning they unconditionally apply the genesis allocation to QMDB on every call. While the production runner correctly guards against this with a `!has_finalized_history` check, the public API is a footgun: any new caller using the simple constructors will silently re-apply genesis on restart, potentially overwriting modified account state.

## Problem
The `QmdbLedger::init()` method passes `apply_genesis: true` unconditionally:

**File:** `/Users/will/dev/nunchi/daeji/crates/storage/qmdb-ledger/src/ledger.rs`, lines 53-58:
```rust
pub async fn init(
    context: Context,
    config: QmdbConfig,
    genesis_alloc: Vec<(Address, U256)>,
) -> Result<Self, Error> {
    Self::init_with_genesis(context, config, genesis_alloc, true).await
}
```

The `LedgerView::init()` and `LedgerView::init_with_genesis_timestamp()` methods both flow through to `init_with_config_and_genesis_options()` with `apply_genesis: true`:

**File:** `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs`, lines 126-131:
```rust
pub async fn init(
    context: tokio::Context,
    partition_prefix: String,
    genesis_alloc: Vec<(Address, U256)>,
) -> LedgerResult<Self> {
    Self::init_with_genesis_timestamp(context, partition_prefix, genesis_alloc, 0).await
}
```

**File:** `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs`, lines 135-148:
```rust
pub async fn init_with_genesis_timestamp(
    context: tokio::Context,
    partition_prefix: String,
    genesis_alloc: Vec<(Address, U256)>,
    genesis_timestamp: u64,
) -> LedgerResult<Self> {
    let config = QmdbConfig::new(partition_prefix);
    Self::init_with_config_and_genesis_timestamp(
        context,
        config,
        genesis_alloc,
        genesis_timestamp,
    )
    .await
}
```

And `init_with_config_and_genesis_timestamp()` at lines 191-205 also passes `true`:
```rust
pub async fn init_with_config_and_genesis_timestamp(
    context: tokio::Context,
    config: QmdbConfig,
    genesis_alloc: Vec<(Address, U256)>,
    genesis_timestamp: u64,
) -> LedgerResult<Self> {
    Self::init_with_config_and_genesis_options(
        context,
        config,
        genesis_alloc,
        true,           // <-- always applies genesis
        genesis_timestamp,
    )
    .await
}
```

The production runner correctly guards this at lines 1008-1013 of `runner.rs`:

```rust
let has_finalized_history = finalized_blocks.last_index().is_some();
let state = LedgerView::init_with_genesis_options(
    context.child("state"),
    format!("{}-qmdb", self.partition_prefix),
    self.bootstrap.genesis_alloc.clone(),
    !has_finalized_history,   // only apply genesis on first run
    self.bootstrap.genesis_timestamp,
)
```

However, the test code in `ledger/src/lib.rs` line 867 uses `LedgerView::init()` directly (which always applies genesis), and any future code path or integration using the simple constructors would have the same problem.

## Code Reference
`/Users/will/dev/nunchi/daeji/crates/storage/qmdb-ledger/src/ledger.rs`, lines 53-58:
```rust
pub async fn init(
    context: Context,
    config: QmdbConfig,
    genesis_alloc: Vec<(Address, U256)>,
) -> Result<Self, Error> {
    Self::init_with_genesis(context, config, genesis_alloc, true).await
}
```

`/Users/will/dev/nunchi/daeji/crates/node/runner/src/runner.rs`, lines 1008-1013 (correct guard):
```rust
let has_finalized_history = finalized_blocks.last_index().is_some();
let state = LedgerView::init_with_genesis_options(
    context.child("state"),
    format!("{}-qmdb", self.partition_prefix),
    self.bootstrap.genesis_alloc.clone(),
    !has_finalized_history,
    self.bootstrap.genesis_timestamp,
)
```

## Impact
- **Silent state corruption on restart:** If a new code path (test harness, CLI tool, alternative runner) uses `QmdbLedger::init()` or `LedgerView::init()` on an existing database, the genesis allocation will be re-applied. This overwrites the nonce and balance of every genesis account with their initial values, silently rolling back any state changes that occurred after genesis.
- **Currently mitigated:** The production runner uses the correct `init_with_genesis_options()` variant with the `!has_finalized_history` guard. The risk is from future misuse of the simpler API.
- **No defensive guard at QMDB layer:** There is no genesis marker stored in QMDB itself that would allow the storage layer to detect and skip redundant genesis application. Safety depends entirely on the caller passing the correct `apply_genesis` flag.

## Root Cause
The `init()` constructors default to applying genesis unconditionally for convenience. There is no genesis marker stored in QMDB (unlike the `commit_seq` markers that track cross-partition consistency). The QMDB layer has no way to know whether genesis was already applied.

## Suggested Fix
**Option 1 (preferred): Add a genesis marker to QMDB.** Store a sentinel key (similar to `COMMIT_SEQ_ACCOUNT_KEY`) after the first genesis application. On subsequent calls, check for this marker and skip genesis if present:

```rust
pub async fn init_with_genesis(
    context: Context,
    config: QmdbConfig,
    genesis_alloc: Vec<(Address, U256)>,
    apply_genesis: bool,
) -> Result<Self, Error> {
    // ... open backend, verify consistency ...
    let handle = Handle::from_store(store).with_root_provider(Arc::new(RwLock::new(root_provider)));

    if apply_genesis {
        // Check for genesis marker before applying
        let already_initialized = handle.has_genesis_marker().await?;
        if !already_initialized {
            handle.init_genesis(genesis_alloc).await?;
            handle.set_genesis_marker().await?;
        }
    }
    Ok(Self { handle })
}
```

**Option 2: Deprecate the always-apply constructors.** Mark `init()`, `init_with_genesis_timestamp()`, and `init_with_config_and_genesis_timestamp()` as `#[deprecated]`, forcing callers to explicitly specify the `apply_genesis` flag.

## Files to Modify
- `/Users/will/dev/nunchi/daeji/crates/storage/qmdb-ledger/src/ledger.rs` -- add genesis marker check or deprecate `init()`
- `/Users/will/dev/nunchi/daeji/crates/node/ledger/src/lib.rs` -- deprecate `init()`, `init_with_genesis_timestamp()`, `init_with_config_and_genesis_timestamp()`
- `/Users/will/dev/nunchi/daeji/crates/storage/qmdb/src/store.rs` -- add genesis marker sentinel key and read/write methods

## Related Issues
- `137-commit-seq-sentinel-key-collision.md` -- the commit sequence sentinel keys use the same pattern that a genesis marker would need

## Labels
bug, correctness, storage, reliability
