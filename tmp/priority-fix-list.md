# Kora Priority Fix List -- Master Reference

## What is Kora?

Kora is a blockchain node implementation written in Rust. It executes Ethereum-compatible (EVM) transactions using REVM, achieves consensus via Simplex BFT (from the Commonware framework), and persists state in QMDB (a custom Merkle database). The network currently runs as a 4-validator devnet with BLS12-381 threshold signatures providing both consensus voting and on-chain randomness (prevrandao via threshold VRF).

Kora's transaction pipeline: user submits a signed transaction via `eth_sendRawTransaction` JSON-RPC, the transaction is validated and inserted into a per-node mempool, a randomly-elected leader builds a block by pulling transactions from its mempool, the block is proposed to consensus, validators re-execute and vote, and finalized blocks are persisted to QMDB.

## Purpose of This Document

This document is the **single authoritative list** of all known bugs, gaps, and improvement areas in the Kora codebase. Each issue includes: exact file locations, line numbers, severity assessment, cross-references to detailed analysis documents, estimated fix effort, and a proposed solution summary. Use this document to plan sprint work, onboard new contributors, and track progress toward production readiness.

---

## Priority Levels

| Level | Meaning | Criteria |
|-------|---------|----------|
| **P0** | Chain death | Bug causes permanent consensus stall or data corruption. No workaround. Must fix before any production deployment. |
| **P1** | Fragile | Chain works but is brittle. Specific sequences of events or moderate load can trigger failures. Workarounds exist but are manual or fragile. |
| **P2** | Degradation | Performance, reliability, or correctness impaired. Does not kill the chain but degrades user/operator experience. |
| **P3** | Nice-to-have | Missing features, cosmetic issues, or improvements that enhance DX, observability, or spec compliance without affecting core liveness. |

---

## P0: Chain-Killing Bugs

These bugs can cause **permanent, unrecoverable consensus stalls** with no automated recovery path.

---

### P0-1: Executor Fatal Abort on Invalid Transaction

| Field | Value |
|-------|-------|
| **File** | `crates/node/executor/src/revm.rs` |
| **Lines** | 391 (`decode_tx_env`), 395 (`evm.replay()`) |
| **Caller** | `crates/node/runner/src/app.rs` lines 125-136 (`build_block`) |
| **Detailed Doc** | `executor-fatal-abort.md`, `evm-execution-issues.md` (Issue 1) |
| **Effort** | Small (~20 LOC change + tests) |

**What Happens:**
The transaction processing loop uses Rust's `?` operator on two fallible operations. A single invalid transaction (malformed RLP, stale nonce, insufficient balance) aborts the **entire block** via `Err(ExecutionError)`. The caller (`build_block`) maps this error to `return None`, nullifying the consensus view.

**Why It Kills the Chain:**
The offending transaction is never removed from the mempool (pruning only occurs on finalization). Every subsequent leader picks the same bad transaction, hits the same error, and nullifies. This repeats **forever**.

**Trigger Conditions:**
- Malformed RLP encoding in mempool
- Stale-nonce transaction (nonce already consumed by a prior finalized block)
- Balance change between mempool admission and execution time
- ECDSA signature recovery failure

**Proposed Fix:**
Replace `?` with `match` + `continue`. Skip failing transactions and record them in a `skipped_txs` set for immediate mempool eviction:

```rust
let tx_env = match decode_tx_env(tx_bytes, self.config.chain_id) {
    Ok(env) => env,
    Err(err) => {
        warn!(?tx_hash, error = ?err, "skipping failed tx decode");
        skipped_txs.push(tx_hash);
        continue;
    }
};
```

Only true system errors (State/DB failures) should abort the block. All transaction-level failures should skip-and-continue.

---

### P0-2: Mempool Never Prunes Stale Transactions After Finalization Errors

| Field | Value |
|-------|-------|
| **File** | `crates/node/reporters/src/lib.rs` |
| **Lines** | 216-219 (early return on persist failure), 226 (prune call) |
| **Related Lines** | 143 (execution failure), 155 (root computation failure), 166 (state root mismatch), 197 (missing parent), 211 (JoinError) |
| **Detailed Doc** | `mempool-pruning-bug.md`, `mempool-architecture-and-gaps.md` (Gap 3, Gap 7) |
| **Effort** | Medium (restructure error handling + add nonce-based pruning) |

**What Happens:**
The `FinalizedReporter::report()` function has 6 early-return paths that call `ack.acknowledge()` (allowing consensus to advance) but skip `prune_mempool()`. When any error occurs during persistence, finalized transactions remain in the mempool.

Additionally, the `prune_mempool` call only removes transactions by their exact hash (those literally included in the finalized block). It does NOT remove stale-nonce transactions that became invalid because a different transaction from the same sender consumed the nonce.

**Why It Kills the Chain:**
1. Block B is consensus-final (has 2/3+ BLS threshold signatures -- irreversible).
2. Persistence fails (disk full, I/O error, etc.) -- or the stale tx was submitted to a different validator.
3. `prune_mempool(B.txs)` is SKIPPED (or only removes the specific hashes from B).
4. B's transactions (or same-nonce duplicates) remain in the mempool.
5. Next leader includes these stale transactions.
6. Executor: NonceTooLow -> `?` -> abort block (see P0-1).
7. View nullified. Repeat forever.

**Proposed Fix (two-part):**

Part A -- Always prune regardless of persist result:
```rust
// Prune BEFORE any error-path returns:
state.prune_mempool(&block.txs).await;

if let Err(err) = persist_result {
    error!(?digest, error = ?err, "failed to persist finalized block");
    ack.acknowledge();
    return;
}
```

Part B -- Nonce-based pruning (remove all stale txs, not just by hash):
After finalization, for each sender in the mempool, query the finalized state nonce and remove all transactions with `nonce < finalized_nonce`. The `TransactionPool::remove_confirmed(sender, confirmed_nonce)` method already implements this logic.

---

### P0-3: Resolver Permanently Blocks Peers During Catch-Up

| Field | Value |
|-------|-------|
| **File** | Commonware library (upstream) -- invoked via `crates/node/runner/src/runner.rs` lines 492-498 |
| **Related** | `crates/node/runner/src/app.rs` lines 176-179 (verify_block parent snapshot check) |
| **Detailed Doc** | `resolver-catchup-failure.md` |
| **Effort** | Depends on upstream (workaround possible) |

**What Happens:**
When a node restarts and needs to catch up, the Commonware resolver requests missing blocks from peers. For each fetched block, `verify_block()` is called, which requires the parent block's state snapshot. A freshly restarted node has an empty snapshot cache, so verification fails:

1. `verify_block(H)` fails -- missing parent snapshot for block H-1.
2. Resolver interprets this as the peer sending invalid data.
3. Resolver permanently blocks that peer (via `transport.oracle.clone()` used as blocker).
4. Tries next peer -- same failure -- blocks that peer too.
5. All peers blocked -- no block sources remain.
6. Node is permanently isolated, can never catch up.

**Observed Impact:**
- Node stuck 680+ blocks behind for 120+ seconds after restart.
- Post-restart: severe degradation (0.5-1.0 blocks/sec vs baseline 4.5).
- 452 nullifications in 180 seconds.

**Proposed Fix (workaround until upstream fix):**

Option A: Periodic peer unblocking -- add a timer that clears the block list every N seconds during catch-up mode.

Option B: Sequential block fetch -- modify catch-up to request blocks in order (height 1, 2, 3...) so parent state is always available before verifying the next block.

Option C: Trust finality proofs -- accept blocks with valid BLS threshold certificates (2/3+ signatures) without full re-execution during catch-up.

---

## P1: Serious Bugs (Chain Works But Fragile)

These issues do not immediately kill the chain but create conditions where specific event sequences cause failures, wasted capacity, or degraded liveness.

---

### P1-1: Nonce Validation Gap (Pool Accepts Stale/Duplicate Nonces)

| Field | Value |
|-------|-------|
| **File** | `crates/node/txpool/src/validator.rs` |
| **Lines** | 91-100 (nonce check against QMDB only) |
| **Instantiation** | `crates/node/runner/src/runner.rs` (validator created per-request with cloned QmdbState) |
| **Detailed Doc** | `nonce-validation-gap.md` |
| **Effort** | Medium (wire pool state into validation, or switch to TransactionPool) |

**What Happens:**
The `TransactionValidator` checks a transaction's nonce only against the persisted QMDB state (which updates only after finalization). It does NOT check:
- Whether the mempool already contains a pending transaction at the same nonce for the same sender.
- Whether the nonce was consumed by a proposed-but-not-yet-persisted block.
- The "effective nonce" (highest pending nonce + 1) for the sender.

**Impact:**
- Multiple transactions with the same nonce (but different payloads/hashes) can coexist in the mempool.
- When a block includes both, the second one fails with NonceTooLow at execution time.
- Combined with P0-1 (fatal abort), this can stall the chain.
- Trivially exploitable: any user submits two transactions with the same nonce.

**Proposed Fix:**
The codebase already contains a fully-featured `TransactionPool` (in `crates/node/txpool/src/pool.rs`) with per-sender `SenderQueue`, `next_nonce` tracking, nonce-ordered pending/queued classification, and replacement-by-gas-price. Wire it as the production mempool and validate against `effective_nonce = max(state_nonce, highest_pending_nonce + 1)`.

---

### P1-2: No Transaction Gossip (Mempools Are Isolated Islands)

| Field | Value |
|-------|-------|
| **File** | All mempool implementations (no gossip layer exists) |
| **Detailed Doc** | `p2p-networking-gaps.md` (Issue 5), `mempool-architecture-and-gaps.md` (Gap 2) |
| **Effort** | Large (new protocol implementation) |

**What Happens:**
Each validator maintains a completely independent mempool. There is no mechanism for validators to share pending transactions. A transaction submitted to validator A's RPC exists only in validator A's mempool.

**Impact:**
- If validator B becomes leader, it cannot include transactions only submitted to validator A.
- Users must externally broadcast to all validators (fragile, causes stale-copy problems).
- After finalization on one validator, others retain stale copies that were submitted externally but not pruned (different hashes than the finalized tx).

**Current Workaround:** External tooling (`loadgen --broadcast-rpc-urls`) sends transactions to all validators. This causes the nonce duplication problem described in P1-1.

**Proposed Fix:** Implement transaction gossip using Commonware's networking layer. When a validator receives a new valid transaction, broadcast it to peers. Peers validate and insert into their own pools.

---

### P1-3: 26% Idle Nullification Rate

| Field | Value |
|-------|-------|
| **File** | `crates/node/runner/src/app.rs` lines 285-318 (`propose()` function) |
| **Lines** | `ancestry.next().await?` returns None when parent unavailable |
| **Detailed Doc** | `idle-nullification-26-percent.md` |
| **Effort** | Medium-Large (protocol-level change) |

**What Happens:**
With zero external load, the devnet wastes 26% of consensus rounds (nullified views). Root cause: VRF-based random leader election + asymmetric startup heights between nodes. A leader elected at a lower height lacks the parent block needed to build a proposal.

**Impact:**
- Theoretical capacity: ~148 blocks/sec. Actual: ~108 blocks/sec.
- 26% of consensus bandwidth is wasted on empty rounds.
- Decreases over time as nodes converge, but re-appears after any restart event.

**Proposed Fix:**
- Short-term: Synchronized start mechanism (all nodes begin in the same round).
- Long-term: Height-aware leader election (don't elect nodes that are behind).

---

### P1-4: DKG Timeout and Coordination Issues

| Field | Value |
|-------|-------|
| **File** | `crates/node/dkg/src/ceremony.rs` |
| **Lines** | 346 (silent send_to failure), 388-395 (debug-level broadcast failure) |
| **Related** | `crates/node/dkg/src/protocol.rs` (1055 lines, state machine) |
| **Detailed Doc** | `dkg-key-management.md` (Known Issues section) |
| **Effort** | Medium |

**What Happens:**
Three compounding problems in the DKG ceremony:

1. **No overall timeout:** If one validator is unreachable, others wait indefinitely. The ceremony hangs with no error or deadline.
2. **Silent network failures:** `let _ = network.send_to(leader_pk, &request_msg)` silently discards errors. Broadcast failures are logged at debug level only. A network hiccup during DKG causes a silent hang.
3. **Leader dependency:** Validator 0 is hardcoded as DKG leader/coordinator. If validator 0 fails during DKG, the entire ceremony stalls permanently.

**Impact:**
DKG must complete before consensus can start. A hung DKG means the entire network never boots. In production, this manifests as "validators started but nothing happens" with no actionable error.

**Proposed Fix:**
- Add configurable overall DKG timeout (e.g., 60 seconds) with clear error reporting.
- Upgrade silent `let _` sends to error-level logging with retry logic.
- Add leader failover: if validator 0's messages are not received within timeout, elect a new DKG coordinator.

---

## P2: Degradation Issues

These issues impair performance, reliability, or correctness without directly killing the chain.

---

### P2-1: BLOCKHASH Opcode Always Returns Zero

| Field | Value |
|-------|-------|
| **File** | `crates/node/executor/src/adapter.rs` |
| **Lines** | 76-79 (`block_hash_ref` returns `B256::ZERO`) |
| **Detailed Doc** | `evm-execution-issues.md` (Issue 2) |
| **Effort** | Medium (new storage structure + trait method) |

**What Happens:**
The `BLOCKHASH` EVM opcode (opcode `0x40`) allows smart contracts to access the hash of one of the 256 most recent blocks. In Kora, this always returns `0x0000...0000`. The `StateDbRead` trait has no `block_hash` method, and QMDB does not maintain a block hash ring buffer.

**Impact:**
- Randomness schemes using `blockhash(block.number - 1)` as entropy get zero (predictable).
- Commit-reveal protocols that verify against block hashes break.
- Some DeFi protocols and oracle designs malfunction.
- Basic transfers, token operations, and DEX swaps are unaffected.

**Proposed Fix:**
1. Add `block_hash(&self, number: u64) -> Result<B256, StateDbError>` to `StateDbRead`.
2. Maintain a ring buffer of the last 256 block hashes (in QMDB or a sidecar structure).
3. Wire through `StateDbAdapter::block_hash_ref()`.

---

### P2-2: Pending Transaction Memory Leak in RPC

| Field | Value |
|-------|-------|
| **File** | `crates/node/rpc/src/eth.rs` |
| **Structure** | `pending_txs: Arc<RwLock<HashMap<B256, RpcTransaction>>>` |
| **Detailed Doc** | `rpc-server-issues.md` (Issue 1) |
| **Effort** | Small (~15 LOC) |

**What Happens:**
Submitted transactions are cached in a `HashMap` to enable `eth_getTransactionByHash` lookups before finalization. There is no TTL, size limit, or eviction policy. Entries for transactions that never finalize (failed, replaced, or abandoned) accumulate indefinitely.

**Impact:**
On a long-running node under sustained load, this HashMap grows without bound, causing gradual memory consumption increase. Eventually contributes to OOM.

**Proposed Fix:**
Add time-based eviction (remove entries older than 5 minutes) or cap the HashMap size with LRU eviction.

---

### P2-3: Rate Limiting Configured But Not Enforced

| Field | Value |
|-------|-------|
| **File** | `crates/node/rpc/src/config.rs` |
| **Config** | `RateLimitConfig { requests_per_second: 100, burst_size: 200 }` |
| **Detailed Doc** | `rpc-server-issues.md` (Issue 2) |
| **Effort** | Small (~5-10 LOC to wire existing config) |

**What Happens:**
`RateLimitConfig` defines rate limit parameters (100 req/s, burst 200), but these values are stored in config without being wired into actual middleware. Only the `max_connections: 100` concurrency limit (via Tower) is enforced.

**Impact:**
A single client can flood the RPC with unlimited requests. Under attack, this degrades validator performance during consensus (CPU spent handling RPC instead of consensus messages).

**Proposed Fix:**
Wire `RateLimitConfig` into a `tower::limit::RateLimitLayer` or use jsonrpsee's built-in rate limiting. The config already exists; this is purely a wiring change.

---

### P2-4: No Mempool Size Limits (Hard Enforcement)

| Field | Value |
|-------|-------|
| **File** | `crates/node/txpool/src/pool.rs` lines 125-139 |
| **Backup** | `crates/node/consensus/src/components/mempool.rs` line 16 (InMemoryMempool) |
| **Detailed Doc** | `mempool-architecture-and-gaps.md` (Gap 1, Gap 6) |
| **Effort** | Small-Medium |

**What Happens:**
The `TransactionPool` has `max_pending_txs: 4096` and `max_queued_txs: 1024`, but these are **soft limits** -- when exceeded, the pool logs a warning but still accepts the transaction. The `InMemoryMempool` has no limits at all (unbounded BTreeMap).

**Impact:**
Under sustained spam or high load, memory consumption grows linearly with submitted transactions. No backpressure. Eventually OOM on a long-running node under attack.

**Proposed Fix:**
Make pool size limits hard (reject inserts beyond capacity with `TxPoolError::PoolFull`). Implement eviction of lowest-priority (lowest gas price) transactions to make room for higher-value ones.

---

### P2-5: DKG Silent Network Failures

| Field | Value |
|-------|-------|
| **File** | `crates/node/dkg/src/ceremony.rs` |
| **Lines** | 346, 388-395 |
| **Related** | `crates/network/transport/src/error.rs` (no runtime error types) |
| **Detailed Doc** | `p2p-networking-gaps.md` (Issue 1, Issue 2) |
| **Effort** | Medium |

**What Happens:**
The network transport error types only cover configuration parsing errors (invalid addresses, keys, ports). There are NO error types for runtime failures (connection drops, delivery failures, timeouts, buffer overflow). DKG broadcast failures are silently dropped or logged at debug level.

**Impact:**
Cannot distinguish between "peer is down", "network is partitioned", and "messages silently dropped". DKG ceremony hangs without clear error reporting. Operators see no actionable information in logs.

**Proposed Fix:**
- Add runtime error types to `TransportError` (ConnectionFailed, MessageDropped, Timeout, BufferFull).
- Upgrade DKG network calls from `let _` to proper error handling with error-level logging.
- Add `network_send_failures_total` and `network_broadcast_failures_total` Prometheus counters.

---

### P2-6: No Block Gas Limit Enforcement During Execution

| Field | Value |
|-------|-------|
| **File** | `crates/node/executor/src/revm.rs` |
| **Lines** | 386-410 (cumulative_gas tracked but never checked against limit) |
| **Detailed Doc** | `evm-execution-issues.md` (Issue 5) |
| **Effort** | Small (~5 LOC) |

**What Happens:**
The executor accumulates `cumulative_gas` but never checks if it exceeds `context.header.gas_limit`. A proposer could theoretically include more transactions than the block gas limit allows, producing a `gas_used > gas_limit` block.

**Current Mitigation:** The `max_txs` configuration (10,000) and 250M gas limit mean you would need ~12,000 basic transfers to overflow. In practice, this does not trigger.

**Proposed Fix:**
Add a break condition: `if cumulative_gas + estimated_next_tx_gas > block_gas_limit { break; }` in the execution loop.

---

## P3: Nice-to-Have Improvements

These are missing features, cosmetic issues, or spec compliance gaps that do not affect core liveness.

---

### P3-1: No WebSocket/Subscription Support

| Field | Value |
|-------|-------|
| **File** | `crates/node/rpc/src/server.rs` |
| **Detailed Doc** | `rpc-server-issues.md` (Issue 5) |
| **Effort** | Medium |

**What Happens:**
No `eth_subscribe`/`eth_unsubscribe` support. All interaction is request/response over HTTP.

**Impact:** Applications cannot receive real-time block/event notifications. Must poll `eth_blockNumber` to detect new blocks. Standard Ethereum tooling (ethers.js WebSocket providers, etc.) cannot function in subscription mode.

---

### P3-2: No Historical State Queries

| Field | Value |
|-------|-------|
| **File** | `crates/node/rpc/src/indexed_provider.rs` |
| **Detailed Doc** | `rpc-server-issues.md` (Issue 4) |
| **Effort** | Large (requires archive mode in QMDB) |

**What Happens:**
Block number parameter in `eth_getBalance`, `eth_getTransactionCount`, etc. is ignored. All queries return the latest finalized state regardless of the requested block number. `Finalized`, `Safe`, and `Pending` tags all resolve to the same thing.

**Impact:** DApps that query historical state (balance at block N-10, storage at a past block) get incorrect results. This deviates from the Ethereum JSON-RPC specification.

---

### P3-3: No Mempool Introspection RPC Endpoints

| Field | Value |
|-------|-------|
| **File** | `crates/node/rpc/src/` (entirely missing) |
| **Detailed Doc** | `rpc-server-issues.md` (Issue 3) |
| **Effort** | Small (add txpool_status, txpool_content handlers) |

**What Happens:**
No way to query pending transactions or mempool state via RPC. Missing: `txpool_status`, `txpool_content`, `eth_pendingTransactions`.

**Impact:** Debugging chain stalls is extremely difficult. When the chain stalls due to mempool poisoning, there is no way to identify which transactions are causing the problem without reading source code and adding logs. Operator visibility is zero.

---

### P3-4: Peer Count Always Reports Zero

| Field | Value |
|-------|-------|
| **File** | `crates/node/rpc/src/state.rs` line 29 |
| **Method** | `set_peer_count()` defined but never called |
| **Detailed Doc** | `peer-count-reporting-bug.md` |
| **Effort** | Small (~10 LOC) |

**What Happens:**
`NodeState.peer_count` is initialized to 0 and never updated. The `set_peer_count()` method exists but is never invoked from anywhere. The `kora_nodeStatus` RPC always returns `peers: 0` even when validators are actively communicating.

Note: The separate `net_peerCount` endpoint (via `NetApiImpl`) works correctly (returns `validators - 1`).

**Impact:** Misleading monitoring. During diagnosis, `peers: 0` suggested P2P connectivity was broken when it was actually fine. Cosmetic but actively harmful for debugging.

**Proposed Fix:**
```rust
// During RPC initialization in runner.rs:
node_state.set_peer_count(self.scheme.participants().len().saturating_sub(1) as u64);
```

---

### P3-5: No Key Rotation / Epoch Mechanism

| Field | Value |
|-------|-------|
| **File** | `crates/node/runner/src/scheme.rs` |
| **Config** | `EPOCH_LENGTH = u64::MAX` (infinite) |
| **Detailed Doc** | `dkg-key-management.md` (Key Rotation section) |
| **Effort** | Large (protocol-level feature) |

**What Happens:**
The validator set is fixed forever. `EPOCH_LENGTH = u64::MAX` means no epoch transitions ever occur. The `ConstantSchemeProvider` returns the same threshold scheme for all epochs. Infrastructure exists (round tracking, certificate Provider trait) but is not activated.

**Impact:** Cannot add/remove validators, cannot rotate compromised keys, cannot implement slashing. The network is permanently fixed to its initial configuration.

---

### P3-6: Hardcoded Gas Price and Fee History

| Field | Value |
|-------|-------|
| **File** | `crates/node/rpc/src/eth.rs` |
| **Config** | `base_fee_per_gas: 0`, `eth_gasPrice` returns 1 Gwei |
| **Detailed Doc** | `rpc-server-issues.md` (Issue 6) |
| **Effort** | Low (non-blocking while base_fee = 0) |

**What Happens:**
- `eth_gasPrice` returns hardcoded 1 Gwei.
- `eth_feeHistory` returns uniform base fee with no variance.
- No market-based fee discovery.
- `max_fee_per_gas: 0` is accepted (no minimum gas price enforced).

**Impact:** Fee estimation tools get static values regardless of demand. Currently a non-issue because fees are effectively zero, but blocks dynamic fee market implementation.

---

### P3-7: No Transaction Expiration / TTL

| Field | Value |
|-------|-------|
| **File** | All mempool implementations |
| **Related** | `crates/node/txpool/src/ordering.rs` line 20 (`timestamp` field exists but unused for expiration) |
| **Detailed Doc** | `mempool-architecture-and-gaps.md` (Gap 5) |
| **Effort** | Small-Medium |

**What Happens:**
Transactions remain in the mempool indefinitely. The `OrderedTransaction` has a `timestamp` field but it is only used for ordering tie-breaking, never for expiration. Stale transactions from previous test runs accumulate forever.

**Impact:** Memory waste and polluted block proposals. Does not cause chain death but contributes to cruft accumulation.

---

## Quick Wins Table

Fixes that are high-impact and require fewer than 20 lines of code change:

| Priority | Issue | Lines of Code | Impact | Unblocks |
|----------|-------|---------------|--------|----------|
| P0-1 | Executor skip-and-continue | ~20 LOC | Eliminates all single-bad-tx stall scenarios | Everything |
| P2-2 | Pending tx HashMap eviction | ~15 LOC | Prevents unbounded RPC memory growth | -- |
| P2-3 | Wire rate limiting config | ~5-10 LOC | Prevents RPC flooding/DoS | -- |
| P3-4 | Fix peer count reporting | ~10 LOC | Accurate monitoring, faster debugging | -- |
| P2-6 | Block gas limit check | ~5 LOC | Prevents over-gas blocks | -- |
| P0-2a | Move prune before persist error paths | ~10 LOC | Prevents mempool poisoning on persist failure | P0-2b |

---

## Testing Plan After Each Fix

### After P0-1 (Executor skip-and-continue):

```bash
# Stress test: inject invalid transactions alongside valid ones
./target/release/loadgen \
  --total-txs 50000 \
  --accounts 50 \
  --concurrency 200 \
  --rpc-url http://127.0.0.1:8545 \
  --broadcast-rpc-urls http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548
```

**Expected:** Chain continues finalizing blocks. Invalid transactions are skipped with warning logs. No view nullifications caused by bad transactions.

**Verification:**
- No `"build_block: execution failed"` in logs.
- Skipped transactions logged at WARN level with tx hash and error reason.
- Chain height advances monotonically.

---

### After P0-2 (Mempool pruning):

**Test:** Submit duplicate-nonce transactions to multiple validators, wait for finalization, verify stale copies are pruned.

**Expected:**
- After finalization, `mempool.len()` decreases to reflect only truly pending (non-stale) transactions.
- No stale-nonce transactions re-appear in subsequent block proposals.
- If `txpool_status` is implemented (P3-3), it should show declining pending count after each finalization.

---

### After P0-3 (Resolver fix):

**Test:** Stop one validator for 60 seconds (let it fall 200+ blocks behind), restart, observe catch-up.

**Expected:**
- Node catches up to current height within 30 seconds.
- No `"invalid data received"` warnings that persist beyond initial reconnection.
- Blocked peer count metric stays at 0 after catch-up completes.

---

### After P1-1 (Nonce validation):

**Test:** Submit two transactions with the same nonce from the same sender to the same validator.

**Expected:**
- Second transaction is rejected with `NonceTooLow` (or accepted only if gas price is 10%+ higher -- replacement).
- No duplicate-nonce transactions coexist in the mempool.

---

### After P0-1 + P0-2 combined:

**Expected:** Chain should be unkillable by external transaction load. Only Byzantine validator behavior (>1/3 malicious) should be able to stall consensus. The chain gracefully handles:
- Malformed transactions
- Stale-nonce transactions
- Persistence failures
- Node restarts under load

---

## Dependency Graph

```
P0-1 (Executor skip-and-continue)
  |
  |-- Immediately makes chain resilient to bad txs in mempool
  |-- Reduces urgency of P1-1 (nonce gap) since duplicates are skipped, not fatal
  |-- Reduces urgency of P0-2 (pruning) since unpruned stale txs are skipped, not fatal
  |
  v
P0-2 (Mempool pruning)
  |
  |-- Prevents stale tx accumulation even with skip-and-continue
  |-- Prerequisite for: clean metrics, accurate txpool_status
  |-- Reduces urgency of P1-2 (gossip) since cross-validator stale copies get pruned
  |
  v
P1-1 (Nonce validation)
  |
  |-- Prevents duplicates from entering pool in the first place
  |-- Makes P1-2 (gossip) safe to implement (gossipped txs won't create duplicates)
  |
  v
P1-2 (Transaction gossip)
  |
  |-- Requires P1-1 to be safe (otherwise gossip amplifies duplicate problem)
  |-- Enables single-endpoint transaction submission
  |
  v
P3-3 (Mempool introspection RPC)
  |
  |-- Requires working mempool (P0-2 + P1-1) to report meaningful data

P0-3 (Resolver fix)
  |
  |-- Independent of other fixes
  |-- Blocks: production deployments with validator restarts

P2-3 (Rate limiting)
  |
  |-- Independent, can be done anytime
  |-- Prerequisite for: public RPC exposure

P2-4 (Mempool size limits)
  |
  |-- Independent, can be done anytime
  |-- Combines well with P2-3 for full DoS protection
```

**Recommended fix order:**
1. P0-1 (Executor) -- immediate chain safety, 20 minutes of work
2. P0-2a (Move prune call) -- 10 minutes, prevents the persistence-failure cascade
3. P0-2b (Nonce-based pruning) -- 2-4 hours, comprehensive stale tx removal
4. P2-3 + P2-2 (Rate limit + pending tx eviction) -- quick wins, 30 minutes total
5. P3-4 (Peer count) -- 5 minutes, improves debugging
6. P1-1 (Nonce validation) -- 4-8 hours, correct long-term solution
7. P0-3 (Resolver) -- depends on upstream engagement
8. P1-4 (DKG timeouts) -- important for production but not blocking devnet
9. Everything else in priority order

---

## Cross-Reference Index

| Issue ID | Detailed Document |
|----------|-------------------|
| P0-1 | `executor-fatal-abort.md`, `evm-execution-issues.md` |
| P0-2 | `mempool-pruning-bug.md`, `mempool-architecture-and-gaps.md` |
| P0-3 | `resolver-catchup-failure.md` |
| P1-1 | `nonce-validation-gap.md` |
| P1-2 | `p2p-networking-gaps.md`, `mempool-architecture-and-gaps.md` |
| P1-3 | `idle-nullification-26-percent.md` |
| P1-4 | `dkg-key-management.md` |
| P2-1 | `evm-execution-issues.md` |
| P2-2 | `rpc-server-issues.md` |
| P2-3 | `rpc-server-issues.md` |
| P2-4 | `mempool-architecture-and-gaps.md` |
| P2-5 | `p2p-networking-gaps.md` |
| P2-6 | `evm-execution-issues.md` |
| P3-1 | `rpc-server-issues.md` |
| P3-2 | `rpc-server-issues.md` |
| P3-3 | `rpc-server-issues.md` |
| P3-4 | `peer-count-reporting-bug.md` |
| P3-5 | `dkg-key-management.md` |
| P3-6 | `rpc-server-issues.md` |
| P3-7 | `mempool-architecture-and-gaps.md` |

---

## Summary Statistics

| Priority | Count | Chain Impact | Estimated Total Effort |
|----------|-------|--------------|------------------------|
| P0 | 3 | Permanent stall / chain death | 1-3 days (plus upstream for P0-3) |
| P1 | 4 | Fragile liveness, wasted capacity | 2-3 weeks |
| P2 | 6 | Performance/reliability degradation | 1-2 weeks |
| P3 | 7 | Missing features, cosmetic | 3-4 weeks |
| **Total** | **20** | | |

**Minimum viable production:** Fix all P0 issues + P1-1 (nonce validation). Everything else can ship with known limitations documented.
