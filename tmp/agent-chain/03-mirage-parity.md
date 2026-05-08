# Mirage Feature Parity — What daeji Must Replicate

mirage-rs is an in-process EVM fork simulator that roko agents use for local
chain interaction. daeji must provide equivalent or better capabilities as a
real consensus network. This document maps every mirage feature to its daeji
implementation path.

---

## Architecture Comparison

```
mirage-rs (single process)          daeji (validator network)
┌──────────────────────┐           ┌──────────────────────────┐
│  DirtyStore          │           │  Transaction mempool     │
│  ↓ miss              │           │  ↓ included              │
│  ReadCache           │           │  REVM execution          │
│  ↓ miss              │           │  ↓ state writes          │
│  UpstreamRpc         │           │  QMDB (3 partitions)     │
└──────────────────────┘           └──────────────────────────┘

In-process, mutable,               Consensus-validated,
single-node, instant               multi-validator, 400ms finality
```

Key tension: mirage is fast and mutable (great for simulation). daeji is
consensus-bound (great for trust). The bridge is the roko trait layer —
same interface, different backend.

---

## Feature Matrix

### Core EVM

| mirage feature | Status in daeji | Notes |
|---------------|----------------|-------|
| `eth_call` | ✅ Implemented | Standard REVM execution |
| `eth_estimateGas` | ✅ Implemented | Binary search estimation |
| `eth_sendRawTransaction` | ✅ Implemented | Through mempool → consensus |
| `eth_getBalance` | ✅ Implemented | Via StateProvider |
| `eth_getCode` | ✅ Implemented | Via StateProvider |
| `eth_getStorageAt` | ✅ Implemented | Via StateProvider |
| `eth_getTransactionReceipt` | ✅ Implemented | Via BlockIndex |
| `eth_getLogs` | ✅ Implemented | Via BlockIndex with LogFilter |
| `eth_feeHistory` | ✅ Implemented | Historical base fee data |
| Block production | ✅ Implemented | Simplex BFT, ~400ms blocks |
| Transaction types | ✅ Legacy, 2930, 1559, 4844, 7702 | Full EIP support |

**Verdict:** Core EVM is at full parity. No gaps.

### Chain Extensions (feature-gated in mirage)

| mirage feature | Status in daeji | Required work |
|---------------|----------------|---------------|
| `HdcIndex` | ❌ Not implemented | HDC precompile at 0x09 (see 01-hdc-precompile.md) |
| `InsightEntry` storage | ❌ Not implemented | InsightBoard contract + HDC integration |
| `InsightEntry` 6 types | ❌ Not implemented | Contract enum + HDC type-specific encoding |
| `InsightEntry` lifecycle | ❌ Not implemented | Contract state machine (Active→Decaying→Archived→Purged) |
| `InsightEntry` decay | ❌ Not implemented | Block-based TTL in contract |
| Pheromone signaling | ❌ Not implemented | PheromoneRegistry contract |
| Pheromone types (THREAT/OPP/WISDOM) | ❌ Not implemented | Contract enum with type-specific decay |
| Pheromone decay curves | ❌ Not implemented | Exponential decay computed at read time |
| WebSocket streaming | ❌ Not implemented | eth_subscribe + kora_subscribe |
| PheromoneBus (WS push) | ❌ Not implemented | kora_subscribe("pheromones") |
| InsightBus (WS push) | ❌ Not implemented | kora_subscribe("insights") |

### Simulation Features

| mirage feature | daeji equivalent | Notes |
|---------------|-----------------|-------|
| Copy-on-write branching | N/A | Not applicable — daeji is the canonical chain, not a simulator |
| Scenario forking | N/A | Agents can use mirage locally against daeji as upstream RPC |
| Atomic JSON snapshots | QMDB snapshots | Built into storage layer |
| Resource profiles (Micro/Standard/Power) | Validator hardware | Not application-level concern |

**Important:** Daeji does NOT need to replicate mirage's simulation/forking features.
Those are local agent capabilities. Mirage continues to exist as a local simulator
that forks from daeji's live state. Daeji is the source of truth; mirage is the
scratch pad.

### Roko Trait Bridges (feature-gated in mirage)

| mirage trait | daeji equivalent | Required work |
|-------------|-----------------|---------------|
| `SimulationGate` | N/A | mirage-only concept (local sim control) |
| `HdcSubstrate` | HDC precompile wrapper | Thin adapter: roko HdcSubstrate trait → eth_call to 0x09 |
| `ChainSubstrate` | Native RPC client | Thin adapter: roko ChainSubstrate trait → daeji RPC |

---

## Feature-by-Feature Implementation Plan

### 1. InsightBoard Contract

mirage stores insights in an in-memory `HashMap<H256, InsightEntry>`. daeji needs
this as a smart contract with HDC integration.

```solidity
contract InsightBoard {
    enum Kind { OBSERVATION, INFERENCE, PREDICTION, STRATEGY, MEMORY, AXIOM }
    enum State { ACTIVE, DECAYING, ARCHIVED, PURGED }

    struct Insight {
        Kind kind;
        State state;
        address author;         // agent that created it
        uint64 createdAt;       // block number
        uint64 expiresAt;       // block number (TTL)
        bytes32 hdcKey;         // key in HDC index
        bytes content;          // raw insight data
        uint16 confidence;      // 0-10000 (basis points)
    }

    // Submit new insight (stores in HDC, checks duplicates)
    function submit(Kind kind, bytes calldata content, uint64 ttl) external returns (bytes32);

    // Query similar insights
    function searchSimilar(bytes calldata query, uint8 topK) external view returns (bytes32[] memory);

    // Lifecycle management
    function decay(bytes32 insightId) external;    // Called by keeper or block hook
    function archive(bytes32 insightId) external;
    function purge(bytes32 insightId) external;

    // Retention tiers (from roko v2-depth spec)
    // Tier 1 (CORE): 2,048 blocks, never auto-purge
    // Tier 2 (WORKING): 512 blocks, decay after TTL
    // Tier 3 (REFERENCE): 128 blocks, aggressive decay
    // Tier 4 (EPHEMERAL): 32 blocks, purge after TTL
}
```

### 2. Pheromone System

mirage's pheromone system is a signaling layer — agents deposit "scent" that decays
over time, guiding swarm behavior.

```solidity
contract PheromoneRegistry {
    enum PheromoneType { THREAT, OPPORTUNITY, WISDOM }

    struct Pheromone {
        PheromoneType pType;
        address depositor;
        uint64 depositBlock;
        uint64 intensity;       // initial strength (0-10000)
        uint16 halfLife;        // blocks until half-decay
        bytes32 location;       // spatial/conceptual location hash
        bytes metadata;         // type-specific data
    }

    // Deposit a pheromone
    function deposit(PheromoneType pType, bytes32 location, uint64 intensity, uint16 halfLife, bytes calldata metadata) external;

    // Read current intensity at a location (computed with decay)
    function intensityAt(bytes32 location, PheromoneType pType) external view returns (uint64);

    // Scan for pheromones near a location
    function scan(bytes32 location, uint64 radius, PheromoneType pType) external view returns (Pheromone[] memory);
}
```

Decay formula: `currentIntensity = initialIntensity × 2^(-(currentBlock - depositBlock) / halfLife)`

Computed at read time, not stored — saves gas on writes.

### 3. WebSocket Subscriptions

mirage pushes events via `PheromoneBus` and `InsightBus`. daeji equivalent:

```
eth_subscribe("logs", { address: PheromoneRegistry, topics: [DEPOSIT_EVENT] })
  → Push pheromone deposits to connected agents in real-time

eth_subscribe("logs", { address: InsightBoard, topics: [SUBMIT_EVENT] })
  → Push new insights to connected agents in real-time

kora_subscribe("pheromones")
  → Higher-level subscription: pre-decoded pheromone events with computed decay

kora_subscribe("insights")
  → Higher-level subscription: pre-decoded insight events with similarity metadata
```

Standard `eth_subscribe("logs")` gets us 80% there. The `kora_subscribe` variants
are convenience wrappers that decode events and add computed fields (decay, similarity).

### 4. Agent Registry

mirage has implicit agent identity (in-process, single agent). daeji needs explicit
on-chain identity.

```solidity
// ERC-8004: Agent Identity (soulbound ERC-721)
contract AgentRegistry is ERC721 {
    struct AgentIdentity {
        address controller;     // EOA or multisig that controls the agent
        bytes32 codeHash;       // hash of agent's runtime code (for verification)
        uint64 capabilities;    // bitmask of granted capabilities
        uint64 reputation;      // composite reputation score
        bytes32 teeAttestation; // TEE attestation hash (optional)
    }

    function registerAgent(bytes32 codeHash, uint64 capabilities) external returns (uint256 tokenId);
    function updateCapabilities(uint256 tokenId, uint64 capabilities) external;
    function getIdentity(uint256 tokenId) external view returns (AgentIdentity memory);
}
```

### 5. Job Marketplace

mirage doesn't have this (it's a simulator, not a market). But the agent-chainv2
spec requires it for inter-agent commerce.

```solidity
// ERC-8183: Agentic Commerce
contract JobMarketplace {
    enum JobState { OPEN, ASSIGNED, ACTIVE, EVALUATING, DISPUTED, COMPLETED, CANCELLED }
    enum HiringModel { OPEN_BID, DIRECT_HIRE, TOURNAMENT }

    struct Job {
        address poster;
        address worker;         // assigned agent
        address evaluator;      // third-party quality evaluator
        JobState state;
        HiringModel model;
        uint256 escrowAmount;
        bytes32 specHash;       // IPFS hash of job specification
        uint64 deadline;        // block number
    }

    function postJob(HiringModel model, address evaluator, uint64 deadline, bytes32 specHash) external payable;
    function bid(uint256 jobId, uint256 price) external;
    function assign(uint256 jobId, address worker) external;
    function submitWork(uint256 jobId, bytes32 deliverableHash) external;
    function evaluate(uint256 jobId, bool accepted, bytes calldata feedback) external;
    function dispute(uint256 jobId) external;
    function release(uint256 jobId) external;
}
```

---

## What daeji Does NOT Need From mirage

| mirage feature | Why daeji skips it |
|---------------|-------------------|
| `DirtyStore → ReadCache → UpstreamRpc` | daeji IS the upstream — no layered caching needed |
| Copy-on-write scenario branching | Simulation feature, not consensus feature |
| `SimulationGate` trait | Local simulation control, meaningless on-chain |
| Atomic JSON snapshots | QMDB handles persistence natively |
| Resource profiles | Infrastructure concern, not protocol concern |
| `serve_forever()` HTTP server | daeji has its own RPC server (jsonrpsee + axum) |

**The relationship:** mirage forks FROM daeji. Agents run mirage locally to simulate
transactions before submitting them to daeji. mirage is the sandbox; daeji is production.

---

## Priority Order

| Priority | Feature | Effort | Blocks |
|----------|---------|--------|--------|
| P0 | HDC precompile | High | Agent cognition |
| P0 | InsightBoard contract | Medium | Knowledge sharing |
| P1 | AgentRegistry (ERC-8004) | Medium | Agent identity |
| P1 | PheromoneRegistry | Medium | Swarm coordination |
| P1 | eth_subscribe (newHeads, logs) | Medium | Real-time events |
| P2 | JobMarketplace (ERC-8183) | Medium | Agent commerce |
| P2 | kora_subscribe (pheromones, insights) | Low | Convenience layer |
| P2 | ISFR precompile | Medium-High | Rate oracle |
| P3 | Yield perpetual contracts | High | DeFi markets |
