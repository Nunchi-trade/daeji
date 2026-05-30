# Kora Audit: 49 Issues Across 7 Categories -- Tracking Issue

**Category**: audit, tracking
**Severity**: critical (contains P0 through P3 issues)

**Labels**: `bug`, `security`, `correctness`, `reliability`, `performance`, `consensus`, `executor`, `rpc`, `storage`, `txpool`, `p2p`, `dkg`, `config`, `shutdown`, `recovery`

## Summary

A comprehensive audit of the entire Kora codebase (an EVM execution client built from scratch on the Commonware consensus framework) identified 49 unique issues across 7 categories. The audit included 150+ code investigations across 4 rounds, 15+ live devnet tests with real transactions on a 10-node cluster at `65.21.232.29`, and 80+ investigation files with 5 verification passes. Issues range from P0 (crash nodes, corrupt data, expose cryptographic secrets) to P3 (minor specification gaps). This is a meta-tracking issue for all audit findings, with a recommended fix timeline organized into 7 phases.

Kora is NOT a fork of Geth, Reth, or any existing Ethereum client -- every subsystem (consensus integration, executor, RPC, txpool, state management, networking) is a new implementation, each requiring independent review.

## Problem

### P0 -- Must Fix Before Any Deployment (5 issues)

| # | Title | Status |
|---|-------|--------|
| #258 | Silenced `DatabaseCommit` error causes silent state divergence | Fixed (PR #325) |
| #262 | Memory OOM via unbounded `BlockIndex` HashMap growth | Fixed (PR #313) |
| #251 | DKG ceremony transmits key shares over raw unencrypted TCP | **OPEN** |
| #255 | Commonware runtime defaults to only 2 tokio worker threads | Fixed (PR #315) |
| #276 | EVM `block_in_place` starves async runtime on constrained thread pool | Fixed (PR #334) |

### P1 -- Must Fix Before Multi-Operator Devnet (19 issues)

**Critical Stability (DEF-01)**

| # | Title | Status |
|---|-------|--------|
| #252 | Disk exhaustion via unpruned QMDB journals | Fixed (PR #322) |
| #257 | No graceful shutdown: SIGTERM drops runtime, QMDB not flushed | Fixed (PR #318) |
| #271 | QMDB cross-partition crash inconsistency (no WAL, sequential writes) | **OPEN** |
| #274 | Storage read-only slot leak: `extract_changes()` writes unchanged slots | Fixed (PR #303) |

**Tooling Compatibility (DEF-02)**

| # | Title | Status |
|---|-------|--------|
| #280 | `eth_call`/`eth_estimateGas` nonce defaults to 0, breaking all active accounts | Fixed (PR #304) |
| #254 | Revert data lost in error responses | Fixed (PR #307) |
| #263 | No CORS middleware on JSON-RPC server | Fixed (PR #308) |
| #253 | Unlimited batch RPC requests | Fixed (PR #309) |

**Ethereum Specification (DEF-03)**

| # | Title | Status |
|---|-------|--------|
| #260 | Base fee hardcoded to 1 gwei; `calculate_base_fee()` is dead code | Fixed (PR #346) |
| #259 | State root is keccak hash chain, not MPT (no `eth_getProof`, no light clients) | **OPEN** |

**RPC Security (DEF-04)**

| # | Title | Status |
|---|-------|--------|
| #256 | `eth_getLogs` unbounded range DoS | Fixed (PR #338) |
| #264 | Global rate limiter shared across all clients | Fixed (PR #331) |

**Consensus & Recovery (DEF-05)**

| # | Title | Status |
|---|-------|--------|
| #279 | Catch-up creates silent state divergence via empty changesets | **OPEN** |
| #261 | Timestamp validation is dead code | Fixed (PR #314) |
| #270 | Equivocation evidence silently discarded | Fixed (PR #333) |
| #269 | Finalization failure leaves node running with diverged QMDB state | Fixed (PR #335) |
| #278 | Node restart broken: resolver rejects all block data after restart | **OPEN** (upstream fix) |

**Security Hardening (DEF-06)**

| # | Title | Status |
|---|-------|--------|
| #281 | All cryptographic keys stored as plaintext with 0644 permissions | Fixed (PR #312) |
| #265 | `NoOpBlocker` disables peer banning | Fixed (PR #327) |

### P2 -- Should Fix Before Public Testnet (15 issues)

| # | Title | Status |
|---|-------|--------|
| #273 | `eth_getTransactionCount("pending")` ignores mempool | Fixed (PR #320) |
| #287 | `eth_estimateGas` gasPrice defaults to 0 | Fixed (PR #304) |
| #275 | Beneficiary is `Address::ZERO`: all priority fees burned | Fixed (PR #321) |
| #268 | `transactionsRoot` and `receiptsRoot` hardcoded to `B256::ZERO` | Fixed (PR #350) |
| #285 | Block `logsBloom` all zeros | Fixed (PR #317) |
| #267 | Blob base fee not forwarded to REVM `BlockEnv` | Fixed (PR #310) |
| #277 | Missing `newHeads`/`logs` WebSocket subscriptions | **OPEN** |
| #289 | WebSocket has no ping/pong keep-alive | Fixed (PR #302) |
| #266 | `eth_syncing` hardcoded to false | Fixed (PR #324) |
| #297 | Snapshot eviction chain gap under stress | Fixed (PR #319) |
| #283 | Static validator set, no key rotation | **OPEN** |
| #292 | Full MEV exposure; `effective_tip()` is dead code | **OPEN** |
| #284 | Genesis `chain_id` silently discarded from config | Fixed (PR #305) |
| #288 | Missing operational metrics | Fixed (PR #328) |
| #291 | Log verbosity not configurable per component | Fixed (PR #316) |

### P3 / Informational (10 issues)

| # | Title | Status |
|---|-------|--------|
| #272 | Block `size` field always returns `0x0` | Fixed (PR #311) |
| #286 | `SpecId::CANCUN` blocks EIP-7702 account abstraction | **OPEN** |
| #290 | Missing `withdrawals` field in RPC block response | Fixed (PR #306) |
| #294 | Missing EIP-4788 beacon root contract | **OPEN** |
| #282 | `eth_feeHistory` percentile calculation uses gas limit instead of gas used | Fixed (PR #336) |
| #299 | 30+ missing standard RPC methods | **OPEN** |
| #295 | Docker health check does not verify consensus participation | Fixed (PR #332) |
| #298 | 18 dead code items found across the codebase | Fixed (PR #330) |
| #293 | Block tags with explicit numbers race with head for state queries | **OPEN** |
| #296 | 10 blob handling gaps (KZG not verified, no sidecar storage, blob gas not tracked) | **OPEN** |

## Impact

- **P0 issues** can crash running nodes, corrupt state databases, or expose cryptographic key material. Any P0 issue can take down a running network.
- **P1 issues** cause data loss, break standard Ethereum tooling (MetaMask, Hardhat, Foundry), enable denial-of-service, or allow consensus violations. These block multi-operator deployments.
- **P2 issues** cause incorrect economic behavior, missing RPC functionality, or security weaknesses that matter for public networks.
- **P3 issues** are specification gaps that affect compatibility but not safety.

## Root Cause

Kora is a from-scratch EVM execution client -- not a fork of Geth, Reth, or any existing Ethereum client. Every subsystem (consensus integration, executor, RPC, txpool, state management, networking) is a new implementation, each requiring independent review and validation against the Ethereum specification.

## Suggested Fix

Follow the recommended fix timeline organized into 7 phases:

| Phase | Timeframe | Focus | Key Issues |
|-------|-----------|-------|------------|
| **0** | Week 1 | Critical stability | #258, #262, #255, #257 |
| **1** | Week 2 | Tooling compatibility | #280, #254, #263, #253, #274 |
| **2** | Weeks 3-4 | Ethereum spec compliance | #260, #275, #261, #286 |
| **3** | Weeks 5-6 | Security hardening | #251, #281, #264, #270 |
| **4** | Weeks 7-8 | Performance | #276, #288 |
| **5** | Weeks 9-12 | Feature completeness | #277, #266, #296 |
| **6** | Month 2+ | Strategic | #259, #283, #271, #278 |

### Production Readiness Gates

| Gate | Blockers |
|------|----------|
| Single-operator devnet (<2h) | Working today with manual restarts |
| Single-operator devnet (long-running) | #262, #252, #258 |
| Multi-operator devnet | All above + #251, #281, #278, #265, #257 |
| Public testnet | All above + #264, #253, #263, #280, #254, #260, #261, #255 |
| Mainnet | All above + #271, #283, #259 + comprehensive test coverage |

## Files to Modify

This is a tracking issue. Individual issues reference specific files. The 49 linked issues cover files across the entire codebase including:
- `crates/node/runner/src/` -- Core validator runner, app logic, shutdown
- `crates/node/rpc/src/` -- RPC server, rate limiting, subscriptions
- `crates/node/reporters/src/` -- Finalization, metrics, consensus reporting
- `crates/node/metrics/src/` -- Prometheus metrics
- `crates/node/dkg/src/` -- DKG ceremony
- `crates/storage/` -- QMDB integration, indexer, state management
- `crates/node/executor/src/` -- EVM execution
- `docker/` -- Deployment configuration

## Verified Safe Areas (21)

These areas were investigated and found to be correctly implemented:
Gas refund handling, SSTORE gas metering, BFT instant finality, certificate cryptography, P2P transport encryption, peer authorization, chain ID enforcement, gas price oracle, transaction hash computation, concurrent RPC safety, filter lifecycle, precompiles, historical state queries, WebSocket subscriptions, eth_feeHistory, key material logging, access list gas, EIP-2929 warm/cold pricing, cross-node state consistency, Docker security hardening, receipt logs on revert.

## Related Issues

- `103-dkg-resharing-tracking.md` -- DKG resharing (parent tracking for #94 key rotation and #251 cleartext TCP)
- `100-metrics-comprehensive-coverage.md` -- Metrics coverage (#288 in the audit)
- `101-shutdown-graceful-shutdown.md` -- Graceful shutdown (#257, #384, #388 in the audit)
- All other files in this directory (091-103) cover specific findings from this audit
