# Agent-Chain Requirements — Overview

Documents in this folder specify what daeji needs to become the native chain layer
for the agent-chainv2 stack: roko agents running on-chain with HDC cognition,
ISFR price discovery, and mirage-equivalent simulation capabilities.

---

## Document Index

| Doc | Topic |
|-----|-------|
| `01-hdc-precompile.md` | HDC precompile at 0x09 — vector storage, similarity search, Merkle proofs |
| `02-isfr-oracle.md` | ISFR composite index at 0xA01 — yield aggregation, circuit breaker, perpetuals |
| `03-mirage-parity.md` | Feature-by-feature analysis of mirage-rs capabilities daeji must replicate |
| `04-roko-native.md` | Roko runtime integration — trait bridges, registries, bus fabric, feeds |
| `05-roadmap.md` | Prioritized implementation plan with dependency graph |

---

## Current State Summary

**What daeji has:**
- Simplex BFT consensus (commonware) with BLS12-381 threshold signatures
- REVM execution engine (Cancun spec level)
- QMDB three-partition storage (accounts, storage, code)
- ~400ms block time, 1-second finality
- 25 standard eth_ RPC methods (poll-only, no subscriptions)
- daeji-chat P2P coordination layer (lobby + 64-slot pool + AEAD rooms)
- Standard REVM precompiles only — no custom precompiles

**What the spec requires:**
- 5 custom precompiles (HDC, QMDB proofs, BTLE, ISFR, Agent namespace)
- 7+ on-chain contracts (ERC-8004, ERC-8183, InsightBoard, ReputationRegistry, etc.)
- WebSocket subscriptions (eth_subscribe + kora-specific)
- Roko trait bridges (ChainClient, ChainWallet, HdcSubstrate, ChainSubstrate)
- Mirage-equivalent simulation layer (knowledge, pheromones, HDC index)
- ISFR oracle with Byzantine aggregation and yield perpetual markets

**Gap magnitude:** daeji is a solid EVM L1. It needs domain-specific extensions
to become an agent-native chain. The core infrastructure is sound — the work is
additive, not corrective.
