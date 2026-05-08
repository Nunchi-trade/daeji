# Implementation Roadmap

Prioritized plan to take daeji from "solid EVM L1" to "agent-native chain with
HDC cognition, ISFR oracle, mirage parity, and roko substrate support."

---

## Dependency Graph

```
                          ┌──────────────────┐
                          │  eth_subscribe   │
                          │  (newHeads, logs) │
                          └────────┬─────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              │                    │                    │
     ┌────────▼────────┐  ┌───────▼───────┐  ┌────────▼────────┐
     │  HDC Precompile │  │ AgentRegistry │  │ PheromoneReg.  │
     │  (0x09)         │  │ (ERC-8004)    │  │ (contract)     │
     └────────┬────────┘  └───────┬───────┘  └────────┬────────┘
              │                   │                    │
     ┌────────▼────────┐  ┌──────▼────────┐           │
     │  InsightBoard   │  │ JobMarketplace│           │
     │  (contract+HDC) │  │ (ERC-8183)   │           │
     └────────┬────────┘  └──────┬────────┘           │
              │                  │                     │
              └──────────┬───────┘                     │
                         │                             │
              ┌──────────▼──────────┐                  │
              │  roko-daeji adapter │◄─────────────────┘
              │  (substrate traits) │
              └──────────┬──────────┘
                         │
              ┌──────────▼──────────┐
              │  ISFR Precompile   │
              │  (0xA01)           │
              └──────────┬──────────┘
                         │
              ┌──────────▼──────────┐
              │  Yield Perpetuals  │
              │  (ClearingHouse)   │
              └────────────────────┘
```

---

## Phase 0: Foundation (Weeks 1-2)

**Goal:** Unblock everything else. No new features visible to users yet.

### 0.1 Custom Precompile Registry

**What:** Extend REVM executor to support custom precompiles alongside standard ones.

**Where:** `crates/executor/src/lib.rs`, new `crates/executor/src/precompiles/mod.rs`

**Work:**
- Add `CustomPrecompileSet` that wraps standard precompiles + custom ones
- Wire into `RevmExecutor::execute_transaction`
- Add precompile address validation (reject calls to reserved but unimplemented addresses)
- Unit tests: custom precompile is callable, gas metering works

**Effort:** ~2 days

### 0.2 eth_subscribe Infrastructure

**What:** WebSocket subscription support in the RPC layer.

**Where:** `crates/node/rpc/src/eth.rs`, `crates/node/rpc/src/server.rs`

**Work:**
- Add `#[subscription]` methods to EthApi trait (jsonrpsee supports this)
- Wire `LedgerEvent` notifications to subscription broadcast channels
- Implement `newHeads` subscription (push block headers on finalization)
- Implement basic `logs` subscription (push matching logs on finalization)
- Integration tests: connect WS, subscribe, receive events

**Effort:** ~3-4 days

### 0.3 Contract Deployment Infrastructure

**What:** Ability to deploy genesis contracts and governance-upgradeable contracts.

**Where:** `crates/ledger/src/genesis.rs` or new genesis builder

**Work:**
- Genesis contract deployment (bytecode in genesis config → deployed at chain start)
- Deterministic contract addresses for well-known contracts
- Governance contract for post-genesis upgrades (optional, can defer)

**Effort:** ~2 days

---

## Phase 1: Agent Identity & Knowledge (Weeks 3-5)

**Goal:** Agents can register on-chain and share knowledge.

### 1.1 HDC Precompile (0x09)

**What:** Full HDC precompile — store, search, delete, bundle, bind vectors.

**Where:** New `crates/executor/src/precompiles/hdc.rs`

**Work:**
- Implement `HdcIndex` (brute-force, auto-switch to HNSW at 100K)
- Implement all 6 precompile operations with gas metering
- Deterministic projection matrix from genesis seed
- State sync: rebuild index from storage on node restart
- Snapshot support: persist index alongside QMDB snapshots
- Comprehensive tests (algebra, gas, consensus determinism, performance)

**Effort:** ~8-10 days

### 1.2 AgentRegistry Contract (ERC-8004)

**What:** Soulbound ERC-721 for agent identity.

**Where:** Solidity contract, deployed at genesis

**Work:**
- Write contract (register, update capabilities, query)
- Capability bitmask design (which bits mean what)
- TEE attestation field (optional, can be zero)
- Deploy script / genesis inclusion
- Integration tests with roko agent registration flow

**Effort:** ~3 days

### 1.3 InsightBoard Contract

**What:** Knowledge sharing layer with HDC duplicate detection.

**Where:** Solidity contract, depends on HDC precompile

**Work:**
- Write contract (submit, search, lifecycle management)
- 6 insight types, 4 retention tiers
- HDC integration: call precompile for storage and search
- Decay logic (block-based TTL, computed at read time)
- Events for subscription-based notification

**Effort:** ~4 days

---

## Phase 2: Coordination & Commerce (Weeks 6-8)

**Goal:** Agents can coordinate via pheromones and trade via job marketplace.

### 2.1 PheromoneRegistry Contract

**What:** Stigmergic signaling with exponential decay.

**Where:** Solidity contract

**Work:**
- Write contract (deposit, intensityAt, scan)
- 3 pheromone types with configurable half-life
- Decay computed at read time (no storage updates needed)
- Events for real-time pheromone push

**Effort:** ~3 days

### 2.2 JobMarketplace Contract (ERC-8183)

**What:** Full job lifecycle with escrow and evaluation.

**Where:** Solidity contract

**Work:**
- 7-state job lifecycle (OPEN → ASSIGNED → ACTIVE → EVALUATING → COMPLETED)
- 3 hiring models (open bid, direct hire, tournament)
- Escrow management (lock on post, release on completion)
- Evaluator pattern (third-party quality assessment)
- Dispute resolution (basic: evaluator decides; advanced: governance vote)

**Effort:** ~5 days

### 2.3 roko-daeji Adapter Crate

**What:** Rust crate implementing all roko substrate traits against daeji.

**Where:** New crate, possibly in roko repo or as standalone

**Work:**
- `ChainClient` impl (thin wrapper over alloy/ethers provider)
- `ChainWallet` impl (local signer + nonce management)
- `HdcSubstrate` impl (calls to precompile 0x09)
- `ChainSubstrate` impl (calls to deployed contracts)
- `SignalBus` impl (emit as tx, subscribe via logs)
- `PulseSource` impl (subscribe to newHeads)
- Integration tests: full roko agent lifecycle on daeji testnet

**Effort:** ~5 days

### 2.4 kora_subscribe Extensions

**What:** Convenience subscriptions for agent-specific events.

**Where:** `crates/node/rpc/src/kora.rs`

**Work:**
- `kora_subscribe("pheromones")` — decoded pheromone events with computed decay
- `kora_subscribe("insights")` — decoded insight events with similarity metadata
- `kora_subscribe("agents")` — agent registration/update events

**Effort:** ~2 days

---

## Phase 3: ISFR & Markets (Weeks 9-12)

**Goal:** Rate oracle operational, foundation for yield markets.

### 3.1 ISFR Precompile (0xA01) — Phase 1

**What:** ISFR precompile with operator-published rates.

**Where:** New `crates/executor/src/precompiles/isfr.rs`

**Work:**
- Implement precompile operations (current, at, twap, history, classRates)
- Ring buffer storage for rate history
- Circuit breaker state machine
- Governance transaction to publish rates (single trusted operator)
- Tests: rate publication, TWAP calculation, circuit breaker transitions

**Effort:** ~5 days

### 3.2 ReputationRegistry Contract

**What:** 7-domain reputation system for agent quality signals.

**Where:** Solidity contract

**Work:**
- 7 reputation domains (from agent-chainv2 spec)
- Stake-weighted reputation updates
- Decay over time (reputation freshness)
- Cross-domain composite score
- Integration with AgentRegistry

**Effort:** ~4 days

### 3.3 Additional Precompiles

**What:** QMDB proofs (0x0B) and BTLE encryption (0x0C).

**Where:** `crates/executor/src/precompiles/`

**Work:**
- QMDB proof precompile: generate and verify Merkle inclusion proofs
- BTLE precompile: encrypt/decrypt with chain-managed keys

**Effort:** ~5 days each

---

## Phase 4: Advanced Features (Weeks 13-20)

**Goal:** Full agent-chainv2 spec compliance.

### 4.1 ISFR Phase 2 — Validator-Computed Rates

**What:** Validators independently observe source rates and reach consensus.

**Work:**
- Source data collector framework (validators run oracle feeds)
- Block proposal extension (include ISFR observations)
- Validation logic (compare peer observations, reject outliers)
- Two-level Byzantine aggregation (intra-class + inter-validator)
- Modify `consensus/src/application.rs` for ISFR-aware proposals

**Effort:** ~10 days

### 4.2 Yield Perpetual Contracts

**What:** ClearingHouse, ClearingProfile, InsuranceFund, LiquidationEngine.

**Work:**
- Full perpetual mechanics (long/short, funding rate, margin)
- Liquidation engine with insurance fund backstop
- ISFR integration for mark-to-market

**Effort:** ~15 days

### 4.3 Debug/Trace APIs

**What:** `debug_traceTransaction`, `debug_storageRangeAt`, `trace_block`.

**Work:**
- REVM Inspector implementation for trace collection
- Re-execution tracing (replay transaction with inspector)
- RPC methods in `crates/node/rpc/`

**Effort:** ~8 days

### 4.4 GroupRegistry Contract

**What:** Persistent agent collectives with 4 coordination modes.

**Effort:** ~4 days

### 4.5 GovernanceContract

**What:** On-chain governance for parameter updates and contract upgrades.

**Effort:** ~5 days

---

## Milestone Summary

| Milestone | Week | Deliverable | Agent Impact |
|-----------|------|-------------|--------------|
| M0 | 2 | Custom precompiles + eth_subscribe + genesis contracts | Infrastructure ready |
| M1 | 5 | HDC + AgentRegistry + InsightBoard | Agents can think and share knowledge |
| M2 | 8 | Pheromones + Jobs + roko-daeji adapter | Agents can coordinate and trade |
| M3 | 12 | ISFR oracle + Reputation + QMDB/BTLE precompiles | Price discovery and trust |
| M4 | 20 | Validator ISFR + Perpetuals + Debug APIs + Groups | Full spec compliance |

---

## Resource Estimate

| Phase | Engineering-weeks | Dependencies |
|-------|-------------------|-------------|
| Phase 0 | 1.5 | None |
| Phase 1 | 3.5 | Phase 0 |
| Phase 2 | 3 | Phase 1 (HDC, AgentRegistry) |
| Phase 3 | 3 | Phase 0 (precompile registry) |
| Phase 4 | 6.5 | Phases 1-3 |
| **Total** | **~17.5 engineering-weeks** | |

With 2 engineers working in parallel on independent tracks:
- Track A: Precompiles (HDC → ISFR → QMDB/BTLE)
- Track B: Contracts + RPC (subscribe → contracts → roko adapter)

Estimated calendar time: **10-12 weeks** to M3, **18-20 weeks** to M4.

---

## Risk Factors

| Risk | Impact | Mitigation |
|------|--------|-----------|
| HDC determinism across validators | Consensus failure | Extensive fuzzing, fixed-point arithmetic only |
| HNSW transition block | State divergence | Hard-fork style: activate at predetermined block height |
| ISFR source availability | No rate to publish | Phase 1 uses operator-published rates (no external deps) |
| Gas cost estimation for precompiles | Under/over-charging | Benchmark on target hardware, add safety margin |
| roko trait interface changes | Adapter breakage | Pin roko version, update adapter in lockstep |
| Contract upgrade path | Locked-in bugs | Proxy pattern or governance-controlled migration |
