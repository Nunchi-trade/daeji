# Load Generator and Benchmarking

## 1. Loadgen Tool

**File:** `bin/loadgen/src/main.rs`

### Overview

The loadgen tool generates and broadcasts EIP-1559 transfer transactions to the devnet. It uses multiple accounts with atomic nonce tracking and concurrent broadcast to multiple RPC endpoints.

### Account Structure (Lines 65-88)

```rust
struct Account {
    key: SigningKey,
    address: Address,
    nonce: AtomicU64,  // Shared across concurrent futures
}

impl Account {
    fn new(seed: u8) -> Self {
        let mut secret = [0u8; 32];
        secret[31] = seed;  // Deterministic keys from seed byte
        let key = SigningKey::from_bytes((&secret).into()).expect("valid key");
        let address = address_from_key(&key);
        Self { key, nonce: AtomicU64::new(0), address }
    }

    fn next_nonce(&self) -> u64 {
        self.nonce.fetch_add(1, Ordering::Relaxed)  // ISSUE: Relaxed ordering
    }

    fn set_nonce(&self, nonce: u64) {
        self.nonce.store(nonce, Ordering::Relaxed)
    }
}
```

### Transaction Construction

- **Type:** EIP-1559 transfer (no contract calls)
- **Gas price:** Zero (`max_fee_per_gas: 0, max_priority_fee_per_gas: 0`)
- **Gas limit:** 21000 (standard transfer)
- **Value:** Small configurable amount
- **Receiver:** Single deterministic address

### Concurrent Broadcast (Lines 282-327)

```rust
let mut futures = FuturesUnordered::new();

for i in 0..args.total_txs {
    let account = accounts[i as usize % accounts.len()].clone();
    let nonce = account.next_nonce();  // Increment BEFORE send
    let tx = sign_eip1559_transfer(&account.key, chain_id, receiver, amount, nonce, gas_limit);

    let fut = async move {
        match send_raw_transaction_to_any(&clients, tx).await {
            Ok(hash) => { success.fetch_add(1, Ordering::Relaxed); }
            Err(e) => { failure.fetch_add(1, Ordering::Relaxed); }
        }
    };

    futures.push(fut);

    if futures.len() >= args.concurrency {
        futures.next().await;  // Wait for ONE future to complete
    }
}
```

### Nonce Initialization (Lines 253-258)

```rust
for account in &accounts {
    let nonce = clients[0].get_transaction_count(account.address).await?;
    account.set_nonce(nonce);
}
```

---

## 2. Race Condition: How Loadgen Triggers Stalls

### The Problem

```
Timeline:
  t=0: next_nonce() returns N, atomically increments to N+1
  t=0: Transaction with nonce N signed and queued (NOT sent yet)
  t=0: next_nonce() returns N+1, atomically increments to N+2
  t=0: Transaction with nonce N+1 signed and queued
  ...repeat up to `concurrency` times...
  t=1: One future completes, next one starts
  t=2: Transaction N+3 sent to RPC and received
  t=3: Transaction N sent to RPC (finally)

Result: RPC receives N+3 before N. If N+3 is included in a block first,
        N becomes stale (NonceTooLow) and poisons the mempool.
```

### Contributing Factors

1. **Relaxed memory ordering**: `Ordering::Relaxed` provides no cross-thread guarantees. The `set_nonce()` (initialization) and subsequent `fetch_add()` calls may reorder across threads.

2. **Fire-and-forget futures**: Up to `concurrency - 1` futures are buffered before any is awaited. All their nonces are pre-incremented.

3. **Round-robin account selection**: `accounts[i % accounts.len()]` — multiple transactions for the same account are interleaved with other accounts' transactions, making ordering non-deterministic.

4. **Multi-endpoint broadcast**: `send_raw_transaction_to_any` selects a random RPC endpoint. Different transactions from the same account may go to different validators.

### How This Causes Stalls

1. Loadgen sends transactions with nonces N, N+1, N+2, ... concurrently
2. Network reordering causes N+2 to arrive before N
3. If N+2 is included in a block (executor runs all tx in proposal):
   - N's nonce becomes stale (NonceTooLow)
4. N remains in the `InMemoryMempool` forever (no nonce validation, no pruning on failure)
5. Next leader proposes a block containing N
6. Executor hits NonceTooLow → `?` operator → aborts block → `build_block()` returns None
7. View nullified, transactions not pruned
8. Repeat forever

### Fixes

1. **Use `Ordering::SeqCst`** for nonce operations (prevents reordering)
2. **Await each send before incrementing nonce** (sequential per-account)
3. **Or:** Use per-account send queues that ensure ordering
4. **Separate from loadgen:** Fix the executor to skip bad transactions instead of aborting

---

## 3. Loadgen CLI Arguments

```
Usage: loadgen [OPTIONS]

Options:
    --total-txs <N>                Total transactions to send (default: 100)
    --accounts <N>                 Number of accounts to use (default: 10)
    --concurrency <N>              Max concurrent in-flight txs (default: 10)
    --chain-id <N>                 Chain ID (default: 1337)
    --broadcast-rpc-urls <URLs>    Comma-separated RPC URLs
    --verbose                      Enable verbose logging
    --dry-run                      Sign but don't send
```

### Example Commands

```bash
# Quick load test (1000 txs)
cargo run --release -p loadgen --bin loadgen -- \
    --total-txs 1000 \
    --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548

# Stress test (10000 txs, 50 accounts)
cargo run --release -p loadgen --bin loadgen -- \
    --total-txs 10000 \
    --accounts 50 \
    --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

---

## 4. Benchmarking Infrastructure — What Exists

### Timing Logs in Application

The block building pipeline has built-in timing (app.rs):

```rust
// build_block timing breakdown:
debug!(
    snapshot_ms = snapshot_elapsed.as_millis(),  // Snapshot retrieval time
    exec_ms = exec_elapsed.as_millis(),          // EVM execution time
    root_ms = root_elapsed.as_millis(),          // State root computation
    total_ms = total_elapsed.as_millis(),        // Total block build time
    "built block"
);
```

Same pattern for `verify_block()` and `propose()`.

### Cargo Profile

```toml
# Cargo.toml
[profile.maxperf]
inherits = "release"
lto = "fat"
codegen-units = 1
```

### External Load Testing

- `bin/loadgen/` provides external transaction generation
- `repro-logs/chaos_monitor.py` provides monitoring during tests
- `docker/scripts/devnet-stats.sh` provides real-time metrics

---

## 5. Benchmarking Infrastructure — What's Missing

### No Microbenchmarks

| Missing Benchmark | Target Function | Purpose |
|-------------------|-----------------|---------|
| ECDSA recovery throughput | `tx_order_key()` via `recover_signer()` | Measure mempool build() bottleneck |
| Transaction decoding | `decode_tx_env()` | Decode throughput |
| EVM execution throughput | `evm.replay()` per tx | Single tx execution cost |
| State root computation | `QmdbStateRoot::transition()` | Root scaling with changeset size |
| ChangeSet merge | `ChangeSet::merge()` | Merge overhead |
| QMDB commit | `QmdbLedger::commit_changes()` | Persistence throughput |
| Overlay state access | `OverlayState::nonce/balance/storage` | State read latency |
| Mempool build (N txs) | `InMemoryMempool::build(N, excluded)` | Build scaling |
| Signature verification | BLS12-381 threshold verification | Crypto bottleneck |
| Block serialization | Block encode/decode | Codec throughput |

### No Profiling

- No flamegraph configuration (cargo-flamegraph)
- No perf annotations
- No allocation tracking (DHAT, jemalloc profiling)
- No async runtime metrics (Tokio console)

### No Integration Benchmarks

- No end-to-end block production throughput measurement
- No consensus round latency benchmark
- No finalization pipeline throughput test
- No state growth benchmark (how performance degrades over time)

### Recommended Benchmark Suite

```
benches/
  ecdsa_recovery.rs       - criterion bench for signature recovery throughput
  block_execution.rs      - criterion bench for N-tx block execution
  state_root.rs           - criterion bench for transition root computation
  mempool_operations.rs   - criterion bench for build/insert/prune
  changeset_merge.rs      - criterion bench for merge with various sizes
  overlay_state.rs        - criterion bench for layered state access
  block_codec.rs          - criterion bench for block serialization
  e2e_block_production.rs - integration bench for full pipeline
```

### How to Measure Current Performance

Until benchmarks are added, use these approaches:

1. **Block build timing**: Set `RUST_LOG=debug` and grep for "built block" logs
2. **Consensus throughput**: Use `devnet-stats.sh` or Grafana "Blocks/sec" panel
3. **Finalization latency**: Grafana "Finalization Latency" panel
4. **CPU profiling**: `cargo flamegraph --bin kora -- validator ...` (requires cargo-flamegraph)
5. **Memory profiling**: Track `runtime_process_rss` in Grafana over time
6. **Load testing**: Use loadgen with various `--total-txs` and `--concurrency` values
