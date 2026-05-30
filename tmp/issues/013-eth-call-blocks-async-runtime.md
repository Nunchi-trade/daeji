# eth_call and eth_estimateGas Block the Async Runtime

**Category**: Bug / Performance
**Severity**: High
**Labels**: `bug`, `performance`, `rpc`, `executor`

## Summary

The RPC methods `eth_call` and `eth_estimateGas` perform synchronous EVM execution directly on the async runtime's worker threads without wrapping the work in `tokio::task::spawn_blocking`. While the underlying `StateDbAdapter` uses `block_in_place` to avoid a hard deadlock, it monopolizes an async worker thread for the full duration of EVM execution. Under concurrent RPC load, this can exhaust all worker threads and stall the entire node -- including consensus, P2P networking, and block verification.

## Problem

The `IndexedStateProvider` implementation of `eth_call` and `eth_estimateGas` calls the executor's synchronous EVM methods directly in the async handler context:

**`eth_call`** at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs:176-184`:

```rust
async fn call(
    &self,
    request: CallRequest,
    block: Option<BlockNumberOrTag>,
) -> Result<Bytes, RpcError> {
    self.reject_historical_block(&block)?;
    let block_ctx = self.block_context_for(block)?;
    let params = call_request_to_params(request);
    self.executor.simulate_call(&self.state, params, &block_ctx).map_err(execution_error_to_rpc)
}
```

**`eth_estimateGas`** at `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs:187-195`:

```rust
async fn estimate_gas(
    &self,
    request: CallRequest,
    block: Option<BlockNumberOrTag>,
) -> Result<u64, RpcError> {
    self.reject_historical_block(&block)?;
    let block_ctx = self.block_context_for(block)?;
    let params = call_request_to_params(request);
    self.executor.estimate_gas(&self.state, params, &block_ctx).map_err(execution_error_to_rpc)
}
```

The `simulate_call` method at `/Users/will/dev/nunchi/daeji/crates/node/executor/src/revm.rs:206-259` is fully synchronous -- it builds a REVM context, runs the EVM, and returns. The `estimate_gas` method at `revm.rs:271-303` is worse: it performs up to 25 binary-search iterations, each calling `simulate_call`, meaning a single `eth_estimateGas` for a complex contract can block a worker thread for hundreds of milliseconds.

Internally, the `StateDbAdapter` at `/Users/will/dev/nunchi/daeji/crates/node/executor/src/adapter.rs:32-40` uses `tokio::task::block_in_place` + `Handle::block_on` to bridge async state reads into REVM's synchronous `DatabaseRef` trait. The adapter's own documentation explicitly states:

```rust
/// Callers are expected to run the entire EVM execution inside
/// `tokio::task::spawn_blocking` so that async worker threads remain free for
/// consensus, networking, and RPC.
```

However, the RPC handlers do not follow this guidance.

Note: the *block production* path (`build_block` at `app.rs:355-358`) correctly uses `tokio::task::spawn_blocking` for EVM execution. Only the RPC simulation path is broken.

## Code Reference

**Adapter documentation warning** (`/Users/will/dev/nunchi/daeji/crates/node/executor/src/adapter.rs:1-10`):

```rust
//! State database adapter for REVM.
//!
//! Note: REVM's `DatabaseRef` trait is synchronous, so we bridge async StateDb traits into
//! the sync REVM interface.
//!
//! Callers are expected to run the entire EVM execution inside
//! `tokio::task::spawn_blocking` so that async worker threads remain free for
//! consensus, networking, and RPC.  Inside a `spawn_blocking` thread,
//! `block_in_place` is a no-op (tokio 1.28+) and `Handle::block_on` drives
//! the state DB futures without starving any async workers.
```

**Correct usage in build_block** (`/Users/will/dev/nunchi/daeji/crates/node/runner/src/app.rs:349-384`):

```rust
let outcome = {
    let executor = self.executor.clone();
    let state = parent_snapshot.state.clone();
    match tokio::task::spawn_blocking(move || {
        executor.execute(&state, &context, &txs_bytes)
    })
    .await
    {
        Ok(Ok(outcome)) => outcome,
        // ...
    }
};
```

## Impact

Under concurrent RPC load:

1. Each `eth_call` / `eth_estimateGas` blocks one Tokio worker thread for the full duration of synchronous EVM execution (potentially hundreds of milliseconds for complex contracts).
2. The Kora node defaults to `worker_threads = min(CPU_CORES, 8)` (see `/Users/will/dev/nunchi/daeji/crates/node/config/src/node.rs:207-210`). On the devnet with `DEFAULT_WORKER_THREADS_CAP = 8`, only 8 concurrent simulation requests are needed to exhaust all worker threads.
3. When all worker threads are blocked, every other async task stalls -- including consensus voting, block verification, P2P message handling, snapshot persistence, and all other RPC methods.
4. Consensus stalls can cause nullification cascades if the node fails to vote within the timeout window, degrading network throughput for all validators.
5. A malicious or misconfigured RPC client sending many concurrent `eth_call` requests can effectively DoS the validator.

## Root Cause

The RPC handler calls the executor's simulation methods directly in the async handler context instead of wrapping them in `tokio::task::spawn_blocking`. The block production path was correctly updated to use `spawn_blocking`, but the RPC path was not.

## Suggested Fix

Wrap both `simulate_call` and `estimate_gas` calls in `tokio::task::spawn_blocking`:

```rust
async fn call(
    &self,
    request: CallRequest,
    block: Option<BlockNumberOrTag>,
) -> Result<Bytes, RpcError> {
    self.reject_historical_block(&block)?;
    let block_ctx = self.block_context_for(block)?;
    let params = call_request_to_params(request);
    let executor = Arc::clone(&self.executor);
    let state = self.state.clone();
    tokio::task::spawn_blocking(move || {
        executor.simulate_call(&state, params, &block_ctx)
            .map_err(execution_error_to_rpc)
    })
    .await
    .map_err(|e| RpcError::Internal(e.to_string()))?
}

async fn estimate_gas(
    &self,
    request: CallRequest,
    block: Option<BlockNumberOrTag>,
) -> Result<u64, RpcError> {
    self.reject_historical_block(&block)?;
    let block_ctx = self.block_context_for(block)?;
    let params = call_request_to_params(request);
    let executor = Arc::clone(&self.executor);
    let state = self.state.clone();
    tokio::task::spawn_blocking(move || {
        executor.estimate_gas(&state, params, &block_ctx)
            .map_err(execution_error_to_rpc)
    })
    .await
    .map_err(|e| RpcError::Internal(e.to_string()))?
}
```

This moves EVM execution to the blocking thread pool, which is unlimited by default in Tokio and will not starve the async worker threads. Inside `spawn_blocking`, the `block_in_place` call in `StateDbAdapter::block_on` becomes a no-op (Tokio 1.28+), so there is no performance penalty.

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs` (lines 176-195) -- `call()` and `estimate_gas()` need `spawn_blocking` wrappers
- `/Users/will/dev/nunchi/daeji/crates/node/rpc/src/indexed_provider.rs` (struct definition) -- `executor` field may need to be `Arc<RevmExecutor>` (currently it is)

## Related Issues

- `014-triple-block-execution.md` -- the correct `spawn_blocking` pattern is already used in the block production path
- `035-rpc-gas-oracle-nested-lock-ordering.md` -- another RPC threading issue
- `031-rpc-get-logs-dual-read-locks.md` -- RPC locking issues
