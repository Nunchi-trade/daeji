# Comprehensive E2E Test Coverage: Major Gaps in Node Restart, Partition, and Execution Tests

**Category**: e2e, testing
**Severity**: medium

**Labels**: `enhancement`, `reliability`, `correctness`

## Summary

The current E2E test harness (`crates/e2e/src/harness.rs`) has significant coverage gaps. The `TestHarness::run()` method operates as a black box -- it starts all nodes, waits for finalization, and exits without allowing mid-test manipulation such as stopping/restarting nodes, partitioning the network, or deploying contracts. This architectural limitation blocks critical test categories including node crash/restart recovery, network partition handling, contract execution through the consensus pipeline, transaction rejection edge cases, and Byzantine behavior detection. Several production bugs (e.g., the marshal genesis anchor panic and epocher mismatch, both part of issue #241) would have been caught by restart tests that currently cannot be written.

## Problem

### Monolithic test harness architecture

The test harness spawns a thread, starts all nodes, waits for a target block count, verifies state convergence, and exits. The `SimControl` (which provides network manipulation primitives) is created internally and never exposed to test code. `TestNode::submit_tx()` exists but cannot be called mid-run since test code has no handle to nodes during execution.

```rust
// /Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs:44
const EPOCH_LENGTH: u64 = u64::MAX;

// The harness creates SimControl internally but does not expose it:
// SimControl has add_link and connect_all but no remove_link method
```

### Coverage gaps

**1. Node crash/restart recovery** -- The marshal genesis anchor panic (#241) and epocher mismatch (#241) were production bugs that could only have been caught by a restart test. The entire recovery pipeline (QMDB checkpoint restore, archive tail replay, snapshot cache prepopulation) is never exercised in CI.

**2. Network partition and message loss** -- No partition test exists. `SimControl` in `crates/network/transport-sim/src/provider.rs` has `add_link` and `connect_all` but no `remove_link` method, making partition simulation impossible.

**3. Contract deployment and transaction rejection** -- The test suite only tests EIP-1559 ETH transfers. The entire EVM execution layer beyond simple transfers (contract creation via `TxKind::Create`, SSTORE/SLOAD, CALL/DELEGATECALL, LOG opcodes) is untested through the consensus pipeline.

**4. Equivocation detection, gas limit edge cases, and transaction gossip** -- Byzantine behavior detection, gas limit boundary conditions, and cross-node transaction propagation have no coverage.

### Existing test files are minimal

The test directory structure exists but coverage is thin:

- `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/consensus.rs` -- Basic consensus tests only
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/execution.rs` -- Simple transfer tests only
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/resilience.rs` -- No partition or restart tests

## Code Reference

**File**: `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs`, lines 1-80 (harness structure, no cluster handle returned)
**File**: `/Users/will/dev/nunchi/daeji/crates/e2e/src/node.rs` (TestNode without lifecycle management)
**File**: `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/execution.rs` (simple transfers only)
**File**: `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/resilience.rs` (empty or minimal)

## Impact

| Category | Risk if untested |
|----------|-----------------|
| Node restart | Regression blindness for recovery bugs (e.g., #241 was a production crash that took down nodes) |
| Network partition | Critical P2P bugs invisible (e.g., the dialable-addr bug caused 65% nullification) |
| Contract execution | EVM correctness through the full consensus pipeline is unverified |
| Transaction rejection | Invalid transaction handling bugs (wrong chain ID, insufficient balance, nonce gaps) are not caught |
| Base fee dynamics | Core economic mechanism (EIP-1559 base fee adjustment) has no E2E coverage |
| Byzantine behavior | Equivocation detection and peer banning mechanisms are never exercised |
| Transaction gossip | The critical path for non-proposer nodes receiving transactions has no coverage |

## Root Cause

The `TestHarness::run()` architecture is monolithic. It spawns nodes, waits, and verifies -- with no hooks for mid-test manipulation. The `SimControl` provides network topology control but is not exposed, and it lacks a `remove_link` method needed for partition simulation.

## Suggested Fix

### Prerequisite: Harness refactoring

Refactor `TestHarness::run()` to return a live `ClusterHandle` with methods for node and network control:

```rust
let mut cluster = TestHarness::start(config).await;
cluster.wait_for_blocks(10).await;
cluster.stop_node(2).await;
cluster.restart_node(2).await;
cluster.remove_link(node_a, node_b).await;  // requires SimControl::remove_link
cluster.add_link(node_a, node_b).await;
cluster.verify_state_convergence().await;
```

### Test scenarios to implement

**Node restart tests**:
- Single node restart during consensus
- Restart during the node's leader turn
- Multiple restarts while maintaining quorum
- Restart after long downtime (large gap catch-up)
- `Start::Floor` recovery path verification

**Network partition tests**:
- Symmetric partition (two halves cannot communicate)
- Asymmetric partition (A can send to B, but B cannot send to A)
- Message loss with `success_rate: 0.8` in `SimLinkConfig`
- Multi-node minority/majority split

**Contract execution tests**:
- Contract creation (`TxKind::Create`)
- SSTORE/SLOAD across blocks
- CALL/DELEGATECALL between contracts
- LOG opcodes and receipt verification

**Transaction rejection tests**:
- Wrong chain ID
- Insufficient balance
- Nonce gaps and nonce replay
- Gas limit exceeded
- Oversized transactions
- `max_fee_per_gas < base_fee`

**Dynamic base fee tests**:
- Base fee adjustment based on gas usage
- Transactions invalidated by base fee increase
- Fee recipient accumulation

**Byzantine behavior tests**:
- Double-voting / equivocation detection
- `GraduatedBlocker` mechanism
- Permanent finalization failure abort

**Transaction gossip tests**:
- Cross-node propagation with `tx_gossip` channels enabled

## Files to Modify

- `/Users/will/dev/nunchi/daeji/crates/e2e/src/harness.rs` -- Refactor `TestHarness::run()` to return `ClusterHandle`; expose `SimControl`
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/node.rs` -- Add lifecycle management to `TestNode` (cancel handle for stopping, state preservation for restart)
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/setup.rs` -- Add contract deployment helpers
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/execution.rs` -- Add contract, rejection, and base fee tests
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/resilience.rs` -- Add partition and restart tests
- `/Users/will/dev/nunchi/daeji/crates/e2e/src/tests/consensus.rs` -- Add equivocation and gossip tests
- New file needed for `SimControl::remove_link` in the transport-sim crate

## Related Issues

- `098-docker-oom-restart-loop.md` -- OOM restart loop (catch-up behavior untested in E2E)
- `096-ci-docker-build-smoke-test.md` -- Docker compose smoke test complements E2E tests
- `101-shutdown-graceful-shutdown.md` -- Graceful shutdown (restart tests would exercise shutdown path)
