# Kora Devnet Investigation Overview

## What Is Kora?

Kora is an EVM-compatible blockchain using BFT consensus (Commonware Simplex) with BLS12-381 threshold VRF for leader election. The devnet runs 4 validators + 1 secondary (follower) node + Prometheus/Grafana observability.

## Architecture Overview

```
                        ┌─────────────────────────────────────────┐
                        │              Validator Node              │
                        │                                         │
  RPC (eth_sendRawTx)   │  ┌─────────┐    ┌──────────────────┐   │
  ─────────────────────►│  │   RPC   │───►│  InMemoryMempool  │   │
                        │  └─────────┘    └────────┬─────────┘   │
                        │                          │              │
                        │  ┌───────────────────────▼──────────┐  │
                        │  │    Simplex Consensus Engine       │  │
                        │  │  (leader election, voting, certs) │  │
                        │  └───────────────────────┬──────────┘  │
                        │                          │              │
                        │  ┌───────────────────────▼──────────┐  │
                        │  │   RevmApplication (build/verify)  │  │
                        │  │   → RevmExecutor (EVM execution)  │  │
                        │  └───────────────────────┬──────────┘  │
                        │                          │              │
                        │  ┌───────────────────────▼──────────┐  │
                        │  │   FinalizedReporter               │  │
                        │  │   → persist to QMDB               │  │
                        │  │   → prune mempool                 │  │
                        │  └──────────────────────────────────┘  │
                        │                                         │
                        │  Storage: OverlayState<QmdbState>       │
                        │  P2P: 5 channels (votes/certs/resolver/ │
                        │       blocks/backfill)                  │
                        └─────────────────────────────────────────┘
```

## Network Stall Root Cause

The devnet stalls under load due to a cascading failure across multiple subsystems:

### 1. Stale Nonce Validation on Ingress
`TransactionValidator` (txpool/validator.rs:91-100) checks nonces against **QMDB persisted state only**, not pending/in-flight transactions. Under load, the loadgen tool rapidly increments nonces and broadcasts transactions concurrently. Many transactions arrive with nonces that are valid against persisted state but already consumed by pending blocks.

### 2. Wrong Mempool in Production
Production uses `InMemoryMempool` (a simple `BTreeMap<TxId, Tx>`) instead of the sophisticated `TransactionPool` (per-sender queues, nonce ordering, gas-price prioritization). The simple mempool has:
- No nonce awareness or ordering
- No size limits (unbounded growth)
- No per-sender caps
- O(n) ECDSA recovery on every `build()` call

### 3. Executor Aborts Entire Block on Single Failure
`RevmExecutor::execute()` (executor/revm.rs:391,395) uses the `?` operator that propagates any single transaction failure (NonceTooHigh, NonceTooLow, decode error) as a fatal error, aborting the **entire block**. No skip-and-continue logic exists.

### 4. build_block Returns None → Nullification
When the executor fails, `RevmApplication::build_block()` (runner/app.rs:135) returns `None`. This causes the Simplex consensus engine to **nullify the round** — the proposal slot is wasted.

### 5. Poisoned Mempool → Permanent Stall
The `prune_mempool()` call only happens after **successful** finalization (reporters/lib.rs:226). When blocks fail to build, stale transactions are never pruned. They get re-proposed every round, fail again, and the cycle continues indefinitely.

### 6. Early Return Bug in FinalizedReporter
Even when a block IS finalized, if `persist_snapshot()` fails (reporters/lib.rs:216-219), the function returns early **without calling `prune_mempool()`**, leaking transactions.

### Contributing Factors
- **Loadgen race condition**: Nonces incremented atomically before async send completes (Ordering::Relaxed)
- **No Prometheus metrics registered**: The `/metrics` endpoint returns empty data despite infrastructure being ready
- **Rate limiting configured but not enforced**: RPC accepts unlimited requests
- **Resolver peer blocking**: Commonware resolver blocks peers on "invalid data", hampering catch-up after restarts

## Key Files

| Component | File | Key Lines |
|-----------|------|-----------|
| InMemoryMempool | `crates/node/consensus/src/components/mempool.rs` | 13-70 |
| TransactionPool (unused) | `crates/node/txpool/src/pool.rs` | 23-401 |
| TransactionValidator | `crates/node/txpool/src/validator.rs` | 56-122 |
| RevmExecutor | `crates/node/executor/src/revm.rs` | 354-412 |
| RevmApplication | `crates/node/runner/src/app.rs` | 84-246 |
| FinalizedReporter | `crates/node/reporters/src/lib.rs` | 105-232 |
| LedgerState | `crates/node/ledger/src/lib.rs` | 56-340 |
| Runner startup | `crates/node/runner/src/runner.rs` | 329-578 |
| Loadgen | `bin/loadgen/src/main.rs` | 65-327 |
| Docker compose | `docker/compose/devnet.yaml` | Full file |
| Grafana dashboard | `docker/grafana/dashboards/kora-overview.json` | Full file |

## Investigation Files Index

| File | Contents |
|------|----------|
| [01-mempool-analysis.md](./01-mempool-analysis.md) | InMemoryMempool vs TransactionPool comparison |
| [02-executor-block-building.md](./02-executor-block-building.md) | RevmExecutor, block building pipeline, fatal error |
| [03-consensus-integration.md](./03-consensus-integration.md) | Simplex config, failure modes, finalization |
| [04-metrics-and-observability.md](./04-metrics-and-observability.md) | Prometheus metrics, gaps, recommended additions |
| [05-rpc-p2p-docker.md](./05-rpc-p2p-docker.md) | RPC endpoints, P2P channels, Docker infrastructure |
| [06-storage-and-state.md](./06-storage-and-state.md) | OverlayState, QMDB, state root computation |
| [07-configuration-reference.md](./07-configuration-reference.md) | All configurable parameters with defaults |
| [08-logging-and-error-handling.md](./08-logging-and-error-handling.md) | Logging gaps, error patterns, dangerous code |
| [09-grafana-dashboards.md](./09-grafana-dashboards.md) | Dashboard panels, monitoring scripts, evidence |
| [10-loadgen-and-benchmarking.md](./10-loadgen-and-benchmarking.md) | Load generator, race conditions, benchmarking gaps |
| [11-recommendations.md](./11-recommendations.md) | Prioritized fixes and improvements |

## Fix Priority

1. **[CRITICAL]** Make executor skip failed transactions instead of aborting the block
2. **[CRITICAL]** Wire `TransactionPool` into production instead of `InMemoryMempool`
3. **[HIGH]** Fix `FinalizedReporter` early return bug — always prune mempool
4. **[HIGH]** Validate nonces against pending state, not just QMDB
5. **[MEDIUM]** Register actual Prometheus metrics (infrastructure exists, no metrics registered)
6. **[MEDIUM]** Enforce RPC rate limiting (configured but not applied)
7. **[LOW]** Fix loadgen nonce race condition (use SeqCst ordering, await sends before incrementing)
